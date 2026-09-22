//! Cron admission and overlap tracking on the same bounded worker pool.
use crate::{
    engine::Invocation,
    runtime::io::{DurableIo, IoConfig},
};
use std::sync::Arc;

use super::App;
impl App {
    pub(super) fn tick(self: &Arc<Self>, now: i64) {
        let mut schedule = self.schedule.lock().unwrap();
        let Some(at) = schedule.0.advance(now) else {
            return;
        };
        for plugin in self.routes.enabled() {
            for cron in &plugin.crons {
                if !cron.matches(at) {
                    continue;
                }
                let key = (plugin.generation, cron.expression.clone());
                if schedule.1.contains(&key) {
                    eprintln!("cron skipped plugin={} reason=overlap", plugin.name);
                    continue;
                }
                let hash =
                    crate::bytecode::source_sha256(&format!("{}:{}", plugin.name, cron.expression));
                let Some(reservation) = self.workers.reserve_scheduled(&hash) else {
                    eprintln!("cron skipped plugin={} reason=capacity", plugin.name);
                    continue;
                };
                let io = IoConfig {
                    identity: None,
                    websocket: self.config.websocket.clone(),
                    ws_capacity: self.workers.ws_capacity.clone(),
                    upgrade: false,
                    telemetry: Some(self.telemetry.clone()),
                    request_id: format!("cron-{at}-{}", plugin.generation),
                    trace: None,
                    network: self.network.clone(),
                    runtime: tokio::runtime::Handle::current(),
                    vars: plugin.vars.clone(),
                    bindings: plugin.bindings.clone(),
                    headers: vec![],
                    body: None,
                    durable: Some(DurableIo {
                        router: self.workers.durable.clone(),
                        plugin: plugin.clone(),
                        service: None,
                    }),
                };
                let Some(rx) = reservation.submit(
                    plugin.clone(),
                    Invocation {
                        url: serde_json::json!({"cron":cron.expression,"scheduledTime":at})
                            .to_string(),
                        method: "__SCHEDULED".into(),
                    },
                    io,
                ) else {
                    eprintln!("cron skipped plugin={} reason=queue", plugin.name);
                    continue;
                };
                schedule.1.insert(key.clone());
                let app = self.clone();
                tokio::spawn(async move {
                    if let Ok(Ok(mut guard)) = rx.await {
                        if guard.handle.response().await.is_none() {
                            eprintln!("cron failed generation={}", key.0);
                        }
                        guard.finish();
                        guard.handle.closed().await;
                    }
                    app.schedule.lock().unwrap().1.remove(&key);
                });
            }
        }
    }
}
