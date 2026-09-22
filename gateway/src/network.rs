//! Pooled transport and request-scoped native bodies. No JS references cross threads.
use anyhow::{Result, anyhow, bail};
use bytes::Bytes;
use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HttpLimits {
    pub request_body_bytes: usize,
    pub host_buffer_bytes: usize,
    pub queue_bytes: usize,
    pub connect_timeout_ms: u64,
    pub headers_timeout_ms: u64,
    pub idle_timeout_ms: u64,
    pub drain_timeout_ms: u64,
}
impl Default for HttpLimits {
    fn default() -> Self {
        Self {
            request_body_bytes: 16 << 20,
            host_buffer_bytes: 32 << 20,
            queue_bytes: 256 << 10,
            connect_timeout_ms: 10_000,
            headers_timeout_ms: 120_000,
            idle_timeout_ms: 300_000,
            drain_timeout_ms: 30_000,
        }
    }
}
#[derive(Clone)]
pub struct Budget {
    used: Arc<AtomicUsize>,
    limit: usize,
}
pub struct Reservation {
    budget: Budget,
    size: usize,
}
impl Budget {
    pub fn new(limit: usize) -> Self {
        Self {
            used: Arc::new(AtomicUsize::new(0)),
            limit,
        }
    }
    pub fn reserve(&self, size: usize) -> Result<Reservation> {
        self.used
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                n.checked_add(size).filter(|n| *n <= self.limit)
            })
            .map_err(|_| anyhow!("request host buffer limit exceeded"))?;
        Ok(Reservation {
            budget: self.clone(),
            size,
        })
    }
    pub fn used(&self) -> usize {
        self.used.load(Ordering::SeqCst)
    }
}
impl Reservation {
    pub fn grow(&mut self, size: usize) -> Result<()> {
        self.budget
            .used
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                n.checked_add(size).filter(|n| *n <= self.budget.limit)
            })
            .map_err(|_| anyhow!("request host buffer limit exceeded"))?;
        self.size += size;
        Ok(())
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.used.fetch_sub(self.size, Ordering::SeqCst);
    }
}
struct ChargedBytes {
    bytes: Bytes,
    _reservation: Reservation,
}
impl AsRef<[u8]> for ChargedBytes {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}
pub struct Chunk {
    pub bytes: Bytes,
}
impl Chunk {
    pub fn new(bytes: Bytes, budget: &Budget) -> Result<Self> {
        let reservation = budget.reserve(bytes.len())?;
        Ok(Self {
            bytes: Bytes::from_owner(ChargedBytes {
                bytes,
                _reservation: reservation,
            }),
        })
    }
}

enum Source {
    Segments(std::collections::VecDeque<Chunk>),
    Bytes(Option<Chunk>),
    Upstream(reqwest::Response),
    Upload(async_channel::Receiver<Result<Chunk>>),
}
pub struct NativeBody {
    source: tokio::sync::Mutex<(Source, Option<Chunk>)>,
    budget: Budget,
    idle: Duration,
    cancelled: AtomicBool,
    cancel: tokio::sync::Notify,
}
impl NativeBody {
    fn new(source: Source, budget: Budget, idle: Duration) -> Arc<Self> {
        Arc::new(Self {
            source: tokio::sync::Mutex::new((source, None)),
            budget,
            idle,
            cancelled: AtomicBool::new(false),
            cancel: tokio::sync::Notify::new(),
        })
    }
    pub fn segments(parts: Vec<Bytes>, budget: Budget) -> Result<Arc<Self>> {
        // Original slices already own their reservation; only newly encoded parts
        // need charging. Callers retain the original body for this snapshot.
        let chunks = parts.into_iter().map(|bytes| Chunk { bytes }).collect();
        Ok(Self::new(Source::Segments(chunks), budget, Duration::ZERO))
    }
    pub fn bytes(bytes: Bytes, budget: Budget) -> Result<Arc<Self>> {
        let data = Chunk::new(bytes, &budget)?;
        Ok(Self::new(
            Source::Bytes(Some(data)),
            budget,
            Duration::from_secs(300),
        ))
    }
    pub(crate) fn replayable(&self) -> Option<Bytes> {
        let state = self.source.try_lock().ok()?;
        match &state.0 {
            Source::Bytes(Some(chunk)) if state.1.is_none() => Some(chunk.bytes.clone()),
            _ => None,
        }
    }
    pub fn fork(&self) -> Result<Option<Arc<Self>>> {
        let state = self
            .source
            .try_lock()
            .map_err(|_| anyhow!("body is being read"))?;
        match &state.0 {
            Source::Bytes(Some(chunk)) if state.1.is_none() => Ok(Some(Self::new(
                Source::Bytes(Some(Chunk {
                    bytes: chunk.bytes.clone(),
                })),
                self.budget.clone(),
                self.idle,
            ))),
            _ => Ok(None),
        }
    }
    fn upstream(response: reqwest::Response, budget: Budget, idle: Duration) -> Arc<Self> {
        Self::new(Source::Upstream(response), budget, idle)
    }
    pub fn upload(
        budget: Budget,
        queue_bytes: usize,
    ) -> (Arc<Self>, async_channel::Sender<Result<Chunk>>) {
        let (tx, rx) = async_channel::bounded((queue_bytes / (64 << 10)).max(1));
        (
            Self::new(Source::Upload(rx), budget, Duration::from_secs(300)),
            tx,
        )
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.cancel.notify_waiters();
        if let Ok(mut state) = self.source.try_lock() {
            *state = (Source::Bytes(None), None);
        }
    }
    pub async fn read(&self) -> Result<Option<Chunk>> {
        let notified = self.cancel.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.cancelled.load(Ordering::SeqCst) {
            bail!("body cancelled");
        }
        tokio::select! {
            biased;
            _ = notified => {
                if let Ok(mut state)=self.source.try_lock(){*state=(Source::Bytes(None),None);}
                Err(anyhow!("body cancelled"))
            }
            result = self.read_chunk() => {
                if self.cancelled.load(Ordering::SeqCst){Err(anyhow!("body cancelled"))}else{result}
            }
        }
    }
    async fn read_chunk(&self) -> Result<Option<Chunk>> {
        let mut state = self.source.lock().await;
        let next = if let Some(chunk) = state.1.take() {
            Some(chunk)
        } else {
            match &mut state.0 {
                Source::Bytes(bytes) => bytes.take(),
                Source::Segments(parts) => parts.pop_front(),
                Source::Upstream(response) => {
                    match tokio::time::timeout(self.idle, response.chunk()).await {
                        Ok(Ok(Some(bytes))) => Some(Chunk::new(bytes, &self.budget)?),
                        Ok(Ok(None)) => None,
                        Ok(Err(_)) => bail!("upstream body transport failed"),
                        Err(_) => bail!("upstream body idle timeout"),
                    }
                }
                Source::Upload(rx) => match rx.recv().await {
                    Ok(v) => Some(v?),
                    Err(_) => None,
                },
            }
        };
        if let Some(mut chunk) = next {
            if chunk.bytes.len() > 64 << 10 {
                let tail = chunk.bytes.split_off(64 << 10);
                // Bytes slices share the reservation until the last transport,
                // tee or native-body reference is released.
                state.1 = Some(Chunk { bytes: tail });
            }
            Ok(Some(chunk))
        } else {
            Ok(None)
        }
    }
}
#[derive(Deserialize)]
pub struct FetchRequest {
    pub url: String,
    pub method: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub redirect: String,
    #[serde(default)]
    pub proxy: String,
}
#[derive(Clone)]
pub struct Network {
    clients: Arc<Mutex<HashMap<String, reqwest::Client>>>,
    pub limits: HttpLimits,
    roots: Arc<Vec<reqwest::Certificate>>,
}
pub struct FetchResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub url: String,
    pub redirected: bool,
    pub body: Arc<NativeBody>,
}
impl Network {
    pub fn new(limits: HttpLimits) -> Self {
        Self {
            clients: Arc::new(Mutex::new(HashMap::new())),
            limits,
            roots: Arc::new(Vec::new()),
        }
    }
    /// Additional administrator-supplied trust anchors; validation stays enabled.
    pub fn with_roots(mut self, roots: Vec<reqwest::Certificate>) -> Self {
        self.roots = Arc::new(roots);
        self.clients = Arc::new(Mutex::new(HashMap::new()));
        self
    }
    fn client(&self, proxy: &str, redirect: &str) -> Result<reqwest::Client> {
        let key = format!("{redirect}\n{proxy}");
        let mut clients = self.clients.lock().unwrap();
        if let Some(client) = clients.get(&key) {
            return Ok(client.clone());
        }
        let policy = match redirect {
            "" | "follow" => reqwest::redirect::Policy::limited(20),
            "manual" | "manual-ws" => reqwest::redirect::Policy::none(),
            "error" => reqwest::redirect::Policy::custom(|a| a.error("redirect forbidden")),
            _ => bail!("invalid redirect mode"),
        };
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .redirect(policy)
            .connect_timeout(Duration::from_millis(self.limits.connect_timeout_ms))
            .http2_initial_stream_window_size(self.limits.queue_bytes.min(u32::MAX as usize) as u32)
            .http2_adaptive_window(false);
        if redirect == "manual-ws" {
            builder = builder.http1_only();
        }
        for root in self.roots.iter() {
            builder = builder.add_root_certificate(root.clone());
        }
        if !proxy.is_empty() && proxy != "direct" {
            let parsed = url::Url::parse(proxy).map_err(|_| anyhow!("invalid proxy URL"))?;
            if !matches!(parsed.scheme(), "http" | "https" | "socks5" | "socks5h") {
                bail!("unsupported proxy scheme");
            }
            builder = builder.proxy(
                reqwest::Proxy::all(proxy).map_err(|_| anyhow!("invalid proxy configuration"))?,
            );
        }
        let client = builder
            .build()
            .map_err(|_| anyhow!("build HTTP client failed"))?;
        clients.insert(key, client.clone());
        Ok(client)
    }
    pub async fn upgrade(
        &self,
        request: FetchRequest,
        budget: Budget,
        config: crate::websocket::Config,
    ) -> Result<(FetchResponse, Option<Arc<crate::websocket::Socket>>)> {
        use tokio_tungstenite::tungstenite::{
            handshake::{client::generate_key, derive_accept_key},
            protocol::Role,
        };
        let mut url = url::Url::parse(&request.url)?;
        match url.scheme() {
            "ws" => {
                url.set_scheme("http").unwrap();
            }
            "wss" => {
                url.set_scheme("https").unwrap();
            }
            "http" | "https" => {}
            _ => bail!("invalid WebSocket URL"),
        };
        if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
            bail!("WebSocket URL cannot contain credentials or fragments");
        }
        let key = generate_key();
        let mut protocols = String::new();
        let client = self.client(&request.proxy, "manual-ws")?;
        let mut outgoing = client.get(url.clone());
        for (k, v) in filter_headers(request.headers) {
            if k.eq_ignore_ascii_case("sec-websocket-protocol") {
                protocols = v.clone();
            }
            if !k.eq_ignore_ascii_case("sec-websocket-key")
                && !k.eq_ignore_ascii_case("sec-websocket-extensions")
            {
                outgoing = outgoing.header(k, v);
            }
        }
        let response = tokio::time::timeout(
            Duration::from_millis(self.limits.headers_timeout_ms),
            outgoing
                .header("connection", "Upgrade")
                .header("upgrade", "websocket")
                .header("sec-websocket-version", "13")
                .header("sec-websocket-key", &key)
                .send(),
        )
        .await
        .map_err(|_| anyhow!("WebSocket headers timeout"))?
        .map_err(|_| anyhow!("WebSocket connection failed"))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_owned()))
            .collect::<Vec<_>>();
        if status != 101 {
            return Ok((
                FetchResponse {
                    status,
                    headers,
                    url: url.to_string(),
                    redirected: false,
                    body: Self::upstream_body(response, budget, self.limits.idle_timeout_ms),
                },
                None,
            ));
        }
        if response
            .headers()
            .get("sec-websocket-accept")
            .and_then(|v| v.to_str().ok())
            != Some(derive_accept_key(key.as_bytes()).as_str())
        {
            bail!("invalid WebSocket accept key");
        }
        if !response
            .headers()
            .get("upgrade")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
            || !response
                .headers()
                .get("connection")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| {
                    v.split(',')
                        .any(|p| p.trim().eq_ignore_ascii_case("upgrade"))
                })
        {
            bail!("invalid WebSocket upgrade headers");
        }
        if response.headers().contains_key("sec-websocket-extensions") {
            bail!("WebSocket compression is not supported");
        }
        let protocol = response
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        if !protocol.is_empty() && !protocols.split(',').any(|p| p.trim() == protocol) {
            bail!("invalid WebSocket subprotocol");
        }
        let wire = response.upgrade().await?;
        let wire = tokio_tungstenite::WebSocketStream::from_raw_socket(
            wire,
            Role::Client,
            Some(crate::websocket::wire_config(&config)),
        )
        .await;
        let (socket, transport) = crate::websocket::Socket::pair(
            budget.clone(),
            config.max_message_bytes,
            4,
            url.to_string(),
        );
        *socket.protocol.lock().unwrap() = protocol;
        tokio::spawn(crate::websocket::transport(
            transport,
            wire,
            config.close_timeout_ms,
        ));
        Ok((
            FetchResponse {
                status,
                headers,
                url: url.to_string(),
                redirected: false,
                body: NativeBody::bytes(Bytes::new(), budget)?,
            },
            Some(socket),
        ))
    }
    fn upstream_body(response: reqwest::Response, budget: Budget, idle: u64) -> Arc<NativeBody> {
        NativeBody::new(
            Source::Upstream(response),
            budget,
            Duration::from_millis(idle),
        )
    }
    pub async fn fetch(
        &self,
        request: FetchRequest,
        body: Option<Arc<NativeBody>>,
        budget: Budget,
    ) -> Result<FetchResponse> {
        let url = url::Url::parse(&request.url).map_err(|_| anyhow!("invalid fetch URL"))?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            bail!("fetch URL requires HTTP(S) without credentials");
        }
        let method = reqwest::Method::from_bytes(request.method.as_bytes())?;
        if matches!(method, reqwest::Method::GET | reqwest::Method::HEAD) && body.is_some() {
            bail!("GET/HEAD cannot have a body");
        }
        let client = self.client(&request.proxy, &request.redirect)?;
        let mut headers = reqwest::header::HeaderMap::new();
        for (k, v) in filter_headers(request.headers) {
            headers.append(
                reqwest::header::HeaderName::from_bytes(k.as_bytes())?,
                reqwest::header::HeaderValue::from_str(&v)?,
            );
        }
        // Body length may change through a transform. Let the transport frame it.
        headers.remove(reqwest::header::CONTENT_LENGTH);
        headers.remove(reqwest::header::HOST);
        let mut outgoing = client.request(method, url.clone()).headers(headers);
        let retained = body.clone();
        let mut replayable = true;
        if let Some(body) = body {
            if let Some(bytes) = body.replayable() {
                outgoing = outgoing.body(bytes);
            } else {
                replayable = false;
                let stream = futures::stream::try_unfold(body, |body| async move {
                    match body.read().await? {
                        Some(chunk) => Ok(Some((chunk.bytes.clone(), body))),
                        None => Ok::<_, anyhow::Error>(None),
                    }
                });
                outgoing = outgoing.body(reqwest::Body::wrap_stream(stream));
            }
        }
        let response = tokio::time::timeout(
            Duration::from_millis(self.limits.headers_timeout_ms),
            outgoing.send(),
        )
        .await
        .map_err(|_| anyhow!("upstream response headers timeout"))?
        .map_err(|e| {
            anyhow!(if e.is_timeout() {
                "upstream connection timeout"
            } else if e.is_connect() {
                "upstream connection/TLS/proxy failed"
            } else {
                "upstream request/redirect failed"
            })
        })?;
        drop(retained);
        if !replayable
            && matches!(request.redirect.as_str(), "" | "follow")
            && matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308)
            && response.headers().contains_key(reqwest::header::LOCATION)
        {
            bail!("redirect requires a replayable body");
        }
        let status = response.status().as_u16();
        if status == 407 && !matches!(request.proxy.as_str(), "" | "direct") {
            bail!("proxy authentication rejected");
        }
        let final_url = response.url().to_string();
        let headers = response
            .headers()
            .iter()
            .map(|(k, v)| Ok((k.to_string(), v.to_str()?.to_owned())))
            .collect::<Result<Vec<_>>>()?;
        Ok(FetchResponse {
            status,
            headers: filter_headers(headers),
            redirected: final_url != url.as_str(),
            url: final_url,
            body: NativeBody::upstream(
                response,
                budget,
                Duration::from_millis(self.limits.idle_timeout_ms),
            ),
        })
    }
}
pub fn filter_headers(headers: Vec<(String, String)>) -> Vec<(String, String)> {
    let connection = headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("connection"))
        .flat_map(|(_, v)| v.split(',').map(|s| s.trim().to_ascii_lowercase()))
        .collect::<Vec<_>>();
    headers
        .into_iter()
        .filter(|(k, _)| {
            let k = k.to_ascii_lowercase();
            !matches!(
                k.as_str(),
                "connection"
                    | "keep-alive"
                    | "proxy-authenticate"
                    | "proxy-authorization"
                    | "te"
                    | "trailer"
                    | "transfer-encoding"
                    | "upgrade"
            ) && !connection.contains(&k)
        })
        .collect()
}
