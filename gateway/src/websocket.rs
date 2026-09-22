//! Owned websocket endpoints; raw relays never enter a JS Runtime.
use crate::network::{Budget, Chunk};
use anyhow::{Result, anyhow, bail};
use futures::{SinkExt, StreamExt};
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
};
use tokio::sync::Notify;
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        Message,
        protocol::{CloseFrame, frame::coding::CloseCode},
    },
};
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub max_message_bytes: usize,
    pub max_connections: usize,
    pub close_timeout_ms: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            max_message_bytes: 1 << 20,
            max_connections: 1024,
            close_timeout_ms: 5000,
        }
    }
}
pub struct Event {
    pub kind: &'static str,
    pub data: Option<Chunk>,
    pub code: u16,
    pub reason: String,
}
impl Event {
    fn close(code: u16, reason: impl Into<String>) -> Self {
        Self {
            kind: "close",
            data: None,
            code,
            reason: reason.into(),
        }
    }
}
pub struct Socket {
    js_closed: AtomicBool,
    js_done: Notify,
    pub close_info: Mutex<(u16, String)>,
    pub frames_in: AtomicU64,
    pub frames_out: AtomicU64,
    pub bytes_in: AtomicU64,
    pub bytes_out: AtomicU64,
    tx: async_channel::Sender<Event>,
    rx: async_channel::Receiver<Event>,
    peer: Mutex<Weak<Socket>>,
    pub accepted: AtomicBool,
    pub detached: AtomicBool,
    pub state: AtomicU8,
    cancelled: AtomicBool,
    notify: Notify,
    pub capacity: Mutex<Option<Arc<tokio::sync::OwnedSemaphorePermit>>>,
    budget: Budget,
    max: usize,
    pub protocol: Mutex<String>,
    pub url: String,
}
impl Socket {
    pub fn pair(budget: Budget, max: usize, queue: usize, url: String) -> (Arc<Self>, Arc<Self>) {
        let (a_tx, a_rx) = async_channel::bounded(queue.max(1));
        let (b_tx, b_rx) = async_channel::bounded(queue.max(1));
        let make = |tx, rx| {
            Arc::new(Self {
                js_closed: AtomicBool::new(false),
                js_done: Notify::new(),
                close_info: Mutex::new((1006, "transport interrupted".into())),
                frames_in: AtomicU64::new(0),
                frames_out: AtomicU64::new(0),
                bytes_in: AtomicU64::new(0),
                bytes_out: AtomicU64::new(0),
                tx,
                rx,
                peer: Mutex::new(Weak::new()),
                accepted: AtomicBool::new(false),
                detached: AtomicBool::new(false),
                state: AtomicU8::new(1),
                cancelled: AtomicBool::new(false),
                notify: Notify::new(),
                capacity: Mutex::new(None),
                budget: budget.clone(),
                max,
                protocol: Mutex::new(String::new()),
                url: url.clone(),
            })
        };
        let a = make(a_tx, b_rx);
        let b = make(b_tx, a_rx);
        *a.peer.lock().unwrap() = Arc::downgrade(&b);
        *b.peer.lock().unwrap() = Arc::downgrade(&a);
        (a, b)
    }
    pub fn js_closed(&self) {
        self.js_closed.store(true, Ordering::SeqCst);
        self.js_done.notify_waiters();
    }
    pub async fn wait_js_closed(&self, timeout_ms: u64) {
        let peer = self.peer.lock().unwrap().upgrade();
        if let Some(peer) = peer
            && peer.accepted.load(Ordering::SeqCst)
        {
            let wait = peer.js_done.notified();
            tokio::pin!(wait);
            wait.as_mut().enable();
            if !peer.js_closed.load(Ordering::SeqCst) {
                let _ =
                    tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), wait).await;
            }
        }
    }
    pub fn keep_engine(&self) -> bool {
        self.peer
            .lock()
            .unwrap()
            .upgrade()
            .is_some_and(|s| s.accepted.load(Ordering::SeqCst))
    }
    pub fn send(&self, kind: &'static str, bytes: bytes::Bytes) -> Result<()> {
        if self.state.load(Ordering::SeqCst) != 1 {
            bail!("WebSocket is not open");
        }
        if bytes.len() > self.max {
            let _ = self.close(1009, "message too large".into());
            bail!("WebSocket message exceeds limit");
        }
        let data = Chunk::new(bytes, &self.budget)?;
        self.tx
            .try_send(Event {
                kind,
                data: Some(data),
                code: 0,
                reason: String::new(),
            })
            .map_err(|_| anyhow!("WebSocket send queue full"))
    }
    pub fn close(&self, code: u16, reason: String) -> Result<()> {
        if self.state.fetch_max(2, Ordering::SeqCst) >= 2 {
            return Ok(());
        }
        self.tx
            .try_send(Event::close(code, reason))
            .map_err(|_| anyhow!("WebSocket close queue full"))
    }
    pub async fn receive(&self) -> Result<Event> {
        self.rx
            .recv()
            .await
            .map_err(|_| anyhow!("WebSocket closed"))
    }
    pub fn cancel(&self) {
        // Forced Engine teardown cannot run JS close callbacks.
        self.js_closed();
        self.cancel_one();
        if let Some(peer) = self.peer.lock().unwrap().upgrade() {
            peer.js_closed();
            peer.cancel_one();
        }
    }
    fn cancel_one(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.state.store(3, Ordering::SeqCst);
        self.notify.notify_waiters();
        self.tx.close();
        self.rx.close();
    }
    async fn incoming(&self, message: Message) -> Result<bool> {
        self.frames_in.fetch_add(1, Ordering::Relaxed);
        self.bytes_in
            .fetch_add(message.len() as u64, Ordering::Relaxed);
        let (kind, bytes) = match message {
            Message::Text(t) => ("message_text", bytes::Bytes::from(t.to_string())),
            Message::Binary(b) => ("message_binary", b),
            Message::Close(c) => {
                let (code, reason) = c.map_or((1000, String::new()), |c| {
                    (u16::from(c.code), c.reason.to_string())
                });
                *self.close_info.lock().unwrap() = (code, reason.clone());
                let _ = self.tx.send(Event::close(code, reason)).await;
                return Ok(false);
            }
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => return Ok(true),
        };
        if bytes.len() > self.max {
            bail!("message too large");
        }
        self.tx
            .send(Event {
                kind,
                data: Some(Chunk::new(bytes, &self.budget)?),
                code: 0,
                reason: String::new(),
            })
            .await
            .map_err(|_| anyhow!("WebSocket receiver closed"))?;
        Ok(true)
    }
}
pub async fn transport<S>(socket: Arc<Socket>, wire: WebSocketStream<S>, close_ms: u64)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let notified = socket.notify.notified();
    tokio::pin!(notified);
    notified.as_mut().enable();
    if !socket.cancelled.load(Ordering::SeqCst) {
        tokio::select! {_=notified=>{},_=transport_inner(socket.clone(),wire,close_ms)=>{}}
    }
    let _ = socket.tx.try_send(Event::close(1006, "transport closed"));
    socket.cancel_one();
}
async fn transport_inner<S>(socket: Arc<Socket>, mut wire: WebSocketStream<S>, close_ms: u64)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let mut deadline = None;
    loop {
        if socket.cancelled.load(Ordering::SeqCst) {
            break;
        }
        let notification = socket.notify.notified();
        let sleep = async {
            if let Some(at) = deadline {
                tokio::time::sleep_until(at).await
            } else {
                std::future::pending::<()>().await
            }
        };
        tokio::select! {
         _=notification=>break,
         _=sleep=>break,
         incoming=wire.next()=>{match incoming{
          Some(Ok(message))=>{match socket.incoming(message).await{Ok(true)=>{},Ok(false)=>{let _=wire.flush().await;break;},Err(_)=>{let _=wire.close(Some(CloseFrame{code:CloseCode::Size,reason:"message limit".into()})).await;break;}}},
          Some(Err(error))=>{
            let code=if matches!(error,tokio_tungstenite::tungstenite::Error::Capacity(_)){1009}else{1002};
            let reason=error.to_string();
            *socket.close_info.lock().unwrap()=(code,reason.clone());
            let _=socket.tx.send(Event{kind:"error",data:None,code,reason:reason.clone()}).await;
            let _=socket.tx.send(Event::close(code,reason)).await;
            let _=wire.close(Some(CloseFrame{code:code.into(),reason:"protocol error".into()})).await;
            break;
          },
          None=>break
         }},
         outgoing=socket.receive()=>{let Ok(event)=outgoing else{break;};let message=match event.kind{
          "message_text"=>Message::Text(String::from_utf8_lossy(event.data.as_ref().unwrap().bytes.as_ref()).into_owned().into()),
          "message_binary"=>Message::Binary(event.data.as_ref().unwrap().bytes.clone()),
          "close"=>{deadline=Some(tokio::time::Instant::now()+std::time::Duration::from_millis(close_ms));Message::Close(Some(CloseFrame{code:event.code.into(),reason:event.reason.into()}))},_=>break};
          socket.frames_out.fetch_add(1,Ordering::Relaxed);socket.bytes_out.fetch_add(message.len() as u64,Ordering::Relaxed);
          if wire.send(message).await.is_err(){break;}
         }
        }
    }
    let _ = socket.tx.try_send(Event::close(1006, "transport closed"));
    socket.cancel_one();
}
pub fn wire_config(config: &Config) -> tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
    let mut result = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default();
    result.max_message_size = Some(config.max_message_bytes);
    result.max_frame_size = Some(config.max_message_bytes);
    result.max_write_buffer_size = config.max_message_bytes.saturating_mul(2).max(256 << 10);
    result
}
