//! HTTP plugin dispatch, upstream streaming and transport cancellation.
use crate::{
    engine::Invocation,
    network::filter_headers,
    plugins::Matched,
    runtime::io::{DurableIo, IoConfig},
};
use anyhow::Result;
use bytes::Bytes;
use futures::{StreamExt, stream::BoxStream};
use http_body_util::{BodyExt, StreamBody};
use hyper::{
    Request, Response,
    body::{Frame, Incoming},
};
use hyper_util::rt::TokioIo;
use std::{
    sync::{Arc, Mutex, atomic::Ordering},
    time::Duration,
};

use super::{
    App,
    body::{Body, SyncStream, collect, error, full, identity_error, trace_failure},
};
impl App {
    pub(super) async fn handle(self: Arc<Self>, mut request: Request<Incoming>) -> Response<Body> {
        if request.uri().path() == "/__gateway" || request.uri().path().starts_with("/__gateway/") {
            let family = request.uri().path().split('/').nth(2).unwrap_or("");
            if matches!(family, "auth" | "me" | "users" | "keys") {
                return self.identity_request(request).await;
            }
            let mut response = self.admin(request).await;
            response
                .headers_mut()
                .insert("cache-control", "no-store".parse().unwrap());
            return response;
        }
        let plugin = match self.routes.lookup(request.uri().path()) {
            Matched::Missing => return error(404, "not_found", "no matching plugin", None),
            Matched::Unavailable => return error(503, "unavailable", "plugin unavailable", None),
            Matched::Ready(p) => p,
        };
        let mode = plugin.route_auth(request.uri().path());
        let identity = if mode == crate::identity::Auth::Public {
            None
        } else if mode == crate::identity::Auth::Admin && self.bootstrap(&request) {
            Some(crate::identity::Identity {
                user_id: None,
                key_id: None,
                kind: "platform",
                administrator: true,
            })
        } else {
            let Some(token) = super::auth::bearer(request.headers()) else {
                return error(
                    401,
                    "unauthorized",
                    "bearer credentials required",
                    Some(&plugin.name),
                );
            };
            match self.identity.authenticate(token, mode).await {
                Ok(identity) => Some(identity),
                Err(e) => return identity_error(e),
            }
        };
        if mode != crate::identity::Auth::Public {
            // Plugins receive identity, never the original Core credential. This
            // also keeps Core secrets out of request traces and upstream forwarding.
            request.headers_mut().remove("authorization");
        }
        let Some(reservation) = self.workers.reserve() else {
            return error(
                503,
                "overloaded",
                "worker capacity exhausted",
                Some(&plugin.name),
            );
        };
        let upgrade = if request.version() == hyper::Version::HTTP_11
            && request
                .headers()
                .get("upgrade")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
        {
            Some(hyper::upgrade::on(&mut request))
        } else {
            None
        };
        let websocket_key = request
            .headers()
            .get("sec-websocket-key")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let (parts, body) = request.into_parts();
        let body = match collect(
            body,
            self.config.http.request_body_bytes,
            Duration::from_millis(self.config.http.idle_timeout_ms),
        )
        .await
        {
            Ok(body) => body,
            Err(e) => return *e,
        };
        let authority = parts
            .uri
            .authority()
            .map(|a| a.to_string())
            .or_else(|| {
                parts
                    .headers
                    .get("host")
                    .and_then(|h| h.to_str().ok())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "localhost".into());
        let url = if parts.uri.scheme().is_some() {
            parts.uri.to_string()
        } else {
            format!("http://{authority}{}", parts.uri)
        };
        let mut headers = match parts
            .headers
            .iter()
            .map(|(k, v)| v.to_str().map(|v| (k.to_string(), v.to_owned())))
            .collect::<std::result::Result<Vec<_>, _>>()
        {
            Ok(v) => filter_headers(v),
            Err(_) => {
                return error(
                    400,
                    "invalid_headers",
                    "header value is not text",
                    Some(&plugin.name),
                );
            }
        };
        if upgrade.is_some() {
            headers.push(("upgrade".into(), "websocket".into()));
        }
        let head = parts.method == hyper::Method::HEAD;
        let trace = match crate::telemetry::Trace::begin_configured(
            self.telemetry.clone(),
            plugin.name.clone(),
            parts.method.to_string(),
            url.clone(),
            headers.clone(),
            plugin.trace_body,
        )
        .await
        {
            Ok(t) => t,
            Err(_) => return error(503, "telemetry", "trace unavailable", None),
        };
        if trace.chunk(0, body.clone()).await.is_err() {
            let _ = trace.finish(false, Some("trace write failed".into())).await;
            return error(503, "telemetry", "trace unavailable", None);
        }
        let io = IoConfig {
            identity,
            websocket: self.config.websocket.clone(),
            ws_capacity: self.workers.ws_capacity.clone(),
            upgrade: upgrade.is_some(),
            telemetry: Some(self.telemetry.clone()),
            request_id: trace.id().into(),
            trace: Some(trace.clone()),
            durable: Some(DurableIo {
                router: self.workers.durable.clone(),
                plugin: plugin.clone(),
                service: None,
            }),
            network: self.network.clone(),
            runtime: tokio::runtime::Handle::current(),
            vars: plugin.vars.clone(),
            bindings: plugin.bindings.clone(),
            headers,
            body: (!body.is_empty()).then_some(body),
        };
        let Some(rx) = reservation.submit(
            plugin,
            Invocation {
                url,
                method: parts.method.to_string(),
            },
            io,
        ) else {
            trace_failure(&trace, 503, "worker dispatch queue full").await;
            return error(503, "overloaded", "worker dispatch queue full", None);
        };
        let mut guard = match rx.await {
            Ok(Ok(h)) => h,
            Ok(Err(_)) => {
                trace_failure(&trace, 503, "plugin unavailable").await;
                return error(503, "unavailable", "plugin unavailable", None);
            }
            Err(_) => {
                trace_failure(&trace, 503, "worker stopped").await;
                return error(503, "unavailable", "worker stopped", None);
            }
        };
        guard.trace = Some(trace.clone());
        let handle = guard.handle.clone();
        let response = match handle.response().await {
            Some(r) => r,
            None => {
                trace_failure(&trace, 500, "plugin execution failed").await;
                return error(500, "plugin_error", "plugin execution failed", None);
            }
        };
        if trace.metadata(serde_json::json!({"event":"headers","status":response.status,"response_headers":response.headers})).await.is_err(){return error(503,"telemetry","trace unavailable",None);}
        if let Some(socket) = response.web_socket {
            let (Some(upgrade), Some(key)) = (upgrade, websocket_key) else {
                return error(400, "upgrade", "missing WebSocket handshake", None);
            };
            if parts.method != hyper::Method::GET
                || parts
                    .headers
                    .get("sec-websocket-version")
                    .and_then(|v| v.to_str().ok())
                    != Some("13")
            {
                return error(400, "upgrade", "invalid WebSocket handshake", None);
            }
            let keep_engine = socket.keep_engine();
            let protocol = response
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("sec-websocket-protocol"))
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| socket.protocol.lock().unwrap().clone());
            if !protocol.is_empty()
                && !parts
                    .headers
                    .get("sec-websocket-protocol")
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| v.split(',').any(|p| p.trim() == protocol))
            {
                socket.cancel();
                return error(400, "upgrade", "invalid selected subprotocol", None);
            }
            {
                use base64::Engine;
                if base64::engine::general_purpose::STANDARD
                    .decode(&key)
                    .map_or(true, |v| v.len() != 16)
                {
                    socket.cancel();
                    return error(400, "upgrade", "invalid WebSocket key", None);
                }
            }
            let config = self.config.websocket.clone();
            let mut builder = Response::builder()
                .status(101)
                .header("connection", "Upgrade")
                .header("upgrade", "websocket")
                .header(
                    "sec-websocket-accept",
                    tokio_tungstenite::tungstenite::handshake::derive_accept_key(key.as_bytes()),
                );
            for (k, v) in response.headers {
                if !matches!(
                    k.to_ascii_lowercase().as_str(),
                    "connection"
                        | "upgrade"
                        | "sec-websocket-accept"
                        | "sec-websocket-protocol"
                        | "sec-websocket-extensions"
                        | "content-length"
                ) {
                    builder = builder.header(k, v);
                }
            }
            if !protocol.is_empty() {
                builder = builder.header("sec-websocket-protocol", protocol);
            }
            let task = tokio::spawn(async move {
                if !keep_engine {
                    guard.finish();
                }
                match upgrade.await {
                    Ok(wire) => {
                        let wire = tokio_tungstenite::WebSocketStream::from_raw_socket(
                            TokioIo::new(wire),
                            tokio_tungstenite::tungstenite::protocol::Role::Server,
                            Some(crate::websocket::wire_config(&config)),
                        )
                        .await;
                        crate::websocket::transport(socket.clone(), wire, config.close_timeout_ms)
                            .await;
                        if keep_engine {
                            socket.wait_js_closed(config.close_timeout_ms).await;
                        }
                        let (code, reason) = socket.close_info.lock().unwrap().clone();
                        let _=trace.metadata(serde_json::json!({"event":"websocket","close_code":code,"close_reason":reason,"frames_in":socket.frames_in.load(Ordering::Relaxed),"frames_out":socket.frames_out.load(Ordering::Relaxed),"bytes_in":socket.bytes_in.load(Ordering::Relaxed),"bytes_out":socket.bytes_out.load(Ordering::Relaxed)})).await;
                        let _ = trace
                            .finish(
                                code != 1006 && code != 1002 && code != 1009,
                                if code == 1006 { Some(reason) } else { None },
                            )
                            .await;
                    }
                    Err(_) => {
                        socket.cancel();
                        let _ = trace.finish(false, Some("upgrade failed".into())).await;
                    }
                }
                guard.finish();
            });
            {
                let mut tasks = self.websocket_tasks.lock().unwrap();
                tasks.retain(|t| !t.is_finished());
                tasks.push(task);
            }
            return builder.body(full(Bytes::new())).unwrap();
        }
        let mut builder = Response::builder().status(response.status);
        for (k, v) in filter_headers(response.headers) {
            if !k.eq_ignore_ascii_case("content-length") {
                builder = builder.header(k, v);
            }
        }
        let stream = response
            .stream
            .filter(|_| !head && !matches!(response.status, 204 | 205 | 304));
        let body = if let Some(source) = stream {
            let stream =
                futures::stream::try_unfold((source, guard), |(source, mut guard)| async move {
                    let next = match source.read().await {
                        Ok(next) => next,
                        Err(error) => {
                            if let Some(trace) = &guard.trace {
                                let _ = trace.finish(false, Some(error.to_string())).await;
                            }
                            return Err(error);
                        }
                    };
                    match next {
                        Some(chunk) => {
                            if let Some(trace) = &guard.trace {
                                trace.chunk(1, chunk.bytes.clone()).await?;
                            }
                            Ok(Some((Frame::data(chunk.bytes.clone()), (source, guard))))
                        }
                        None => {
                            if let Some(trace) = &guard.trace {
                                trace.finish(true, None).await?;
                            }
                            guard.finish();
                            Ok::<_, anyhow::Error>(None)
                        }
                    }
                });
            let stream: BoxStream<'static, Result<Frame<Bytes>>> = stream.boxed();
            BodyExt::boxed(StreamBody::new(SyncStream(Mutex::new(stream))))
        } else {
            let _ = trace.finish(true, None).await;
            guard.finish();
            if head {
                // An unknown size avoids inventing Content-Length: 0 for HEAD.
                BodyExt::boxed(StreamBody::new(futures::stream::empty::<
                    Result<Frame<Bytes>>,
                >()))
            } else {
                full(Bytes::from(response.body))
            }
        };
        builder
            .body(body)
            .unwrap_or_else(|_| error(500, "plugin_error", "invalid response headers", None))
    }
}
