//! Authenticated management HTTP adapters; no worker execution or database access.
use crate::plugins::Overrides;
use anyhow::{Context, Result};
use bytes::Bytes;
use futures::{StreamExt, stream::BoxStream};
use http_body_util::{BodyExt, StreamBody};
use hyper::{
    Request, Response,
    body::{Frame, Incoming},
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use super::{
    App,
    body::{Body, SyncStream, collect, error, full, identity_error, json},
};
impl App {
    pub(super) fn bootstrap(&self, request: &Request<Incoming>) -> bool {
        self.token
            .as_ref()
            .zip(super::auth::bearer(request.headers()))
            .is_some_and(|(expected, actual)| crate::identity::secret_equal(expected, actual))
    }
    async fn telemetry_list(
        &self,
        uri: &hyper::Uri,
        plugin: Option<String>,
        kind: crate::telemetry::Kind,
    ) -> Response<Body> {
        let query: std::collections::HashMap<String, String> =
            url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
                .into_owned()
                .collect();
        let since = query
            .get("since")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        let limit = query
            .get("limit")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(100);
        let after = query
            .get("after_id")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        let cursor = match query.get("order").map(String::as_str) {
            None | Some("asc") => crate::telemetry::ListCursor::After(after),
            Some("desc") => {
                let before = match query.get("before_id") {
                    Some(value) => match value.parse::<i64>() {
                        Ok(id) if id > 0 => id,
                        _ => {
                            return error(
                                400,
                                "query_error",
                                "invalid before_id",
                                plugin.as_deref(),
                            );
                        }
                    },
                    None => i64::MAX,
                };
                crate::telemetry::ListCursor::Before(before)
            }
            _ => {
                return error(
                    400,
                    "query_error",
                    "order must be asc or desc",
                    plugin.as_deref(),
                );
            }
        };
        match self
            .telemetry
            .list(plugin.clone(), kind, since, cursor, limit)
            .await
        {
            Ok(value) => json(200, value),
            Err(e) => error(400, "query_error", &e.to_string(), plugin.as_deref()),
        }
    }
    pub(super) async fn admin(self: Arc<Self>, request: Request<Incoming>) -> Response<Body> {
        if !self.bootstrap(&request) {
            let Some(token) = super::auth::bearer(request.headers()) else {
                return error(
                    401,
                    "unauthorized",
                    "administrator authorization required",
                    None,
                );
            };
            if let Err(e) = self
                .identity
                .authenticate(token, crate::identity::Auth::Admin)
                .await
            {
                return identity_error(e);
            }
        }
        let method = request.method().clone();
        let path = request.uri().path().to_owned();
        if method == hyper::Method::GET && path == "/__gateway/metrics" {
            let ((requests, js, host), (objects, do_js, do_host)) = self.workers.memory();
            return json(
                200,
                serde_json::json!({"requests":requests,"request_js_bytes":js,"request_host_bytes":host,"do_engines":objects,"do_js_bytes":do_js,"do_host_bytes":do_host,"websockets":self.config.websocket.max_connections-self.workers.ws_capacity.available_permits(),"telemetry":self.telemetry.counters()}),
            );
        }
        if method == hyper::Method::GET {
            let kind = match path.as_str() {
                "/__gateway/logs" => Some(crate::telemetry::Kind::Logs),
                "/__gateway/traces" => Some(crate::telemetry::Kind::Traces),
                "/__gateway/usage/records" => Some(crate::telemetry::Kind::Usage),
                _ => None,
            };
            if let Some(kind) = kind {
                return self.telemetry_list(request.uri(), None, kind).await;
            }
        }
        let suffix = path.strip_prefix("/__gateway/plugins");
        let Some(suffix) = suffix else {
            return error(404, "not_found", "unknown management endpoint", None);
        };
        let segments = suffix
            .trim_matches('/')
            .split('/')
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if !suffix.is_empty() && !suffix.starts_with('/') {
            return error(404, "not_found", "unknown management endpoint", None);
        }
        let name = segments.first().cloned();
        if let Some(name) = &name
            && crate::store::validate_name(name).is_err()
        {
            return error(400, "invalid_name", "invalid plugin name", Some(name));
        }
        if method == hyper::Method::GET && segments.len() == 2 && segments[1] == "config" {
            let result = self.manager.lock().await.configuration(&segments[0]);
            let mut response = match result {
                Ok(value) => json(200, value),
                Err(_) => error(404, "not_found", "plugin not installed", name.as_deref()),
            };
            response
                .headers_mut()
                .insert("cache-control", "no-store".parse().unwrap());
            return response;
        }
        if method == hyper::Method::GET {
            let kind = match segments.as_slice() {
                [_, kind] if kind == "logs" => Some(crate::telemetry::Kind::Logs),
                [_, kind] if kind == "traces" => Some(crate::telemetry::Kind::Traces),
                [_, kind, records] if kind == "usage" && records == "records" => {
                    Some(crate::telemetry::Kind::Usage)
                }
                _ => None,
            };
            if let Some(kind) = kind {
                return self.telemetry_list(request.uri(), name.clone(), kind).await;
            }
        }
        if method == hyper::Method::GET && segments.len() >= 2 {
            let query: std::collections::HashMap<String, String> =
                url::form_urlencoded::parse(request.uri().query().unwrap_or("").as_bytes())
                    .into_owned()
                    .collect();
            let telemetry = self.telemetry.clone();
            let plugin = segments[0].clone();
            let since = query
                .get("since")
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(0);
            if segments.len() == 4 && segments[1] == "traces" && segments[3] == "body" {
                let side = query
                    .get("side")
                    .cloned()
                    .unwrap_or_else(|| "response".into());
                if !matches!(side.as_str(), "request" | "response") {
                    return error(
                        400,
                        "invalid_side",
                        "expected request or response",
                        Some(&plugin),
                    );
                }
                let id = segments[2].clone();
                if telemetry.detail(plugin.clone(), id.clone()).await.is_err() {
                    return error(404, "not_found", "trace not found", Some(&plugin));
                }
                let stream = futures::stream::try_unfold(
                    (telemetry, plugin, id, side, 0u64),
                    |(t, p, id, side, seq)| async move {
                        match t.chunk(p.clone(), id.clone(), side.clone(), seq).await? {
                            Some(bytes) => Ok(Some((
                                Frame::data(Bytes::from(bytes)),
                                (t, p, id, side, seq + 1),
                            ))),
                            None => Ok::<_, anyhow::Error>(None),
                        }
                    },
                );
                let stream: BoxStream<'static, Result<Frame<Bytes>>> = stream.boxed();
                return Response::new(BodyExt::boxed(StreamBody::new(SyncStream(Mutex::new(
                    stream,
                )))));
            }
            let result = match segments.as_slice() {
                [_, kind] if kind == "usage" => {
                    telemetry
                        .query(
                            plugin.clone(),
                            since,
                            query
                                .get("groupBy")
                                .map(|s| {
                                    s.split(',')
                                        .filter(|s| !s.is_empty())
                                        .map(str::to_owned)
                                        .collect()
                                })
                                .unwrap_or_default(),
                        )
                        .await
                }
                [_, kind, id] if kind == "traces" => {
                    telemetry.detail(plugin.clone(), id.clone()).await
                }
                _ => return error(404, "not_found", "unknown endpoint", Some(&plugin)),
            };
            return match result {
                Ok(v) => json(200, v),
                Err(e) => error(400, "query_error", &e.to_string(), Some(&plugin)),
            };
        }
        if method == hyper::Method::GET {
            let summaries = self.routes.list();
            return match segments.as_slice() {
                [] => json(200, serde_json::json!(summaries)),
                [name] => match summaries
                    .into_iter()
                    .find(|v| v["name"].as_str() == Some(name))
                {
                    Some(value) => json(200, value),
                    None => error(404, "not_found", "plugin not installed", Some(name)),
                },
                _ => error(404, "not_found", "unknown endpoint", name.as_deref()),
            };
        }
        let Ok(permit) = self.mutation.clone().try_acquire_owned() else {
            return error(
                409,
                "conflict",
                "plugin management is busy",
                name.as_deref(),
            );
        };
        let upload = method == hyper::Method::POST && segments.is_empty();
        if upload
            && request
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.split(';').next().unwrap_or("").trim())
                != Some("application/zip")
        {
            return error(
                400,
                "invalid_package",
                "Content-Type must be application/zip",
                None,
            );
        }
        let limit = if upload {
            self.config.zip_upload_bytes
        } else {
            1 << 20
        };
        let body = match collect(
            request.into_body(),
            limit,
            Duration::from_millis(self.config.http.idle_timeout_ms),
        )
        .await
        {
            Ok(b) => b,
            Err(r) => return *r,
        };
        let manager = self.manager.clone();
        let runtime = tokio::runtime::Handle::current();
        let result = tokio::task::spawn_blocking(move || {
            runtime.block_on(async move {
                let _permit = permit;
                let mut manager = manager.lock().await;
                match (method.as_str(), segments.as_slice()) {
                    ("POST", []) => manager
                        .install(body.to_vec())
                        .await
                        .map(|(new, value)| (if new { 201 } else { 200 }, value)),
                    ("PUT", [name, action]) if action == "enabled" => {
                        #[derive(serde::Deserialize)]
                        #[serde(deny_unknown_fields)]
                        struct Enabled {
                            enabled: bool,
                        }
                        let value: Enabled = serde_json::from_slice(&body)
                            .context(crate::plugins::Error::InvalidConfig)?;
                        manager.enable(name, value.enabled).await.map(|v| (200, v))
                    }
                    ("PUT", [name, action]) if action == "config" => {
                        let value: Overrides = serde_json::from_slice(&body)
                            .context(crate::plugins::Error::InvalidConfig)?;
                        manager.configure(name, value).await.map(|v| (200, v))
                    }
                    ("DELETE", [name]) => manager
                        .uninstall(name)
                        .await
                        .map(|_| (204, serde_json::Value::Null)),
                    _ => Err(crate::plugins::Error::NotFound.into()),
                }
            })
        })
        .await;
        match result {
            Ok(Ok((204, _))) => Response::builder()
                .status(204)
                .body(full(Bytes::new()))
                .unwrap(),
            Ok(Ok((status, value))) => json(status, value),
            Ok(Err(e)) => {
                let text = format!("{e:#}");
                let status = plugin_error_status(&e);
                error(status, "plugin_management", &text, name.as_deref())
            }
            Err(_) => error(500, "host_error", "management task failed", name.as_deref()),
        }
    }
}

fn plugin_error_status(error: &anyhow::Error) -> u16 {
    use crate::plugins::Error;
    match error.downcast_ref::<Error>() {
        Some(Error::NotFound) => 404,
        Some(Error::RouteConflict) => 409,
        Some(Error::UploadLimit) => 413,
        Some(Error::InvalidPackage | Error::InvalidConfig) => 400,
        None if error.downcast_ref::<std::io::Error>().is_some()
            || error.downcast_ref::<rusqlite::Error>().is_some() =>
        {
            500
        }
        None => 422,
    }
}

#[cfg(test)]
mod tests {
    use super::plugin_error_status;
    use crate::plugins::Error;

    #[test]
    fn management_status_uses_error_kind_not_diagnostic_words() {
        let conflict = anyhow::Error::from(Error::RouteConflict).context("ZIP wrangler binding");
        assert_eq!(plugin_error_status(&conflict), 409);
        let io = anyhow::Error::from(std::io::Error::other("invalid_package ZIP binding"));
        assert_eq!(plugin_error_status(&io), 500);
        assert_eq!(
            plugin_error_status(&anyhow::anyhow!("not_found: plain plugin execution error")),
            422
        );
    }
}
