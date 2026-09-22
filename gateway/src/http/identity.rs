//! Identity HTTP routing, bounded JSON decoding and authorization policy.
use super::{
    App,
    body::{Body, collect, error, full, identity_error, json},
};
use crate::identity::{
    Auth, Identity, KeyScope,
    error::{ErrorKind, fail, invalid},
};
use anyhow::Result;
use bytes::Bytes;
use hyper::{Request, Response, body::Incoming};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

fn parse<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T> {
    serde_json::from_slice(body)
        .map_err(|_| fail(ErrorKind::InvalidInput, "invalid request fields"))
}
impl App {
    async fn identity_endpoint(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        bootstrap: bool,
        has_bootstrap: bool,
        body: &[u8],
    ) -> Result<(u16, Value)> {
        let service = &self.identity;
        if method == "POST" && path == "/__gateway/auth/login" {
            return Ok((200, json!(service.login(parse(body)?).await?)));
        }
        let segments: Vec<_> = path.trim_start_matches('/').split('/').collect();
        let personal = matches!(segments.get(1), Some(&"me") | Some(&"auth"));
        let actor = if bootstrap && !personal {
            Identity {
                user_id: None,
                key_id: None,
                kind: "platform",
                administrator: true,
            }
        } else {
            service
                .authenticate(
                    token.ok_or_else(invalid)?,
                    if personal { Auth::User } else { Auth::Admin },
                )
                .await?
        };
        let value = match (method, segments.as_slice()) {
            ("POST", ["__gateway", "auth", "logout"]) => {
                service.logout(token.ok_or_else(invalid)?).await?;
                return Ok((204, Value::Null));
            }
            ("GET", ["__gateway", "me"]) => {
                json!(service.user(actor.user_id.ok_or_else(invalid)?).await?)
            }
            ("GET", ["__gateway", "users"]) => json!(service.users().await?),
            ("POST", ["__gateway", "users"]) => {
                return Ok((201, json!(service.create_user(parse(body)?).await?)));
            }
            ("PUT", ["__gateway", "users", id]) => json!(
                service
                    .update_user((*id).into(), parse(body)?, has_bootstrap)
                    .await?
            ),
            ("POST", ["__gateway", "keys"]) => {
                return Ok((
                    201,
                    json!(service.create_key(parse(body)?, KeyScope::All).await?),
                ));
            }
            ("POST", ["__gateway", "me", "keys"]) => {
                return Ok((
                    201,
                    json!(
                        service
                            .create_key(
                                parse(body)?,
                                KeyScope::User(actor.user_id.ok_or_else(invalid)?)
                            )
                            .await?
                    ),
                ));
            }
            ("GET", ["__gateway", "keys"]) | ("GET", ["__gateway", "me", "keys"]) => {
                json!(
                    service
                        .keys(if personal {
                            KeyScope::User(actor.user_id.ok_or_else(invalid)?)
                        } else {
                            KeyScope::All
                        })
                        .await?
                )
            }
            ("DELETE", ["__gateway", "keys", id]) | ("DELETE", ["__gateway", "me", "keys", id]) => {
                service
                    .revoke_key(
                        (*id).into(),
                        if personal {
                            KeyScope::User(actor.user_id.ok_or_else(invalid)?)
                        } else {
                            KeyScope::All
                        },
                    )
                    .await?;
                return Ok((204, Value::Null));
            }
            _ => return Err(fail(ErrorKind::NotFound, "unknown identity endpoint")),
        };
        Ok((200, value))
    }
    pub(super) async fn identity_request(
        self: Arc<Self>,
        request: Request<Incoming>,
    ) -> Response<Body> {
        let bootstrap = self.bootstrap(&request);
        let token = super::auth::bearer(request.headers()).map(str::to_owned);
        let method = request.method().to_string();
        let path = request.uri().path().to_owned();
        let body = match tokio::time::timeout(
            Duration::from_secs(10),
            collect(request.into_body(), 8192, Duration::from_secs(10)),
        )
        .await
        {
            Ok(Ok(body)) => body,
            Ok(Err(response)) => return *response,
            Err(_) => {
                return error(
                    408,
                    "request_timeout",
                    "identity request body timed out",
                    None,
                );
            }
        };
        let mut response = match self
            .identity_endpoint(
                &method,
                &path,
                token.as_deref(),
                bootstrap,
                self.token.is_some(),
                &body,
            )
            .await
        {
            Ok((204, _)) => Response::builder()
                .status(204)
                .body(full(Bytes::new()))
                .unwrap(),
            Ok((status, value)) => json(status, value),
            Err(e) => identity_error(e),
        };
        response.headers_mut().insert(
            "cache-control",
            hyper::header::HeaderValue::from_static("no-store"),
        );
        response
    }
}
