//! An active plugin generation owns its request and WebSocket lifetimes.
use super::{Route, route_matches};
use crate::{
    runtime::request::{Phase, RequestHandle},
    storage::{Database, Namespace},
};
use anyhow::{Result, bail};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
pub struct Plugin {
    pub auth_routes: Vec<Route>,
    pub trace_body: Option<bool>,
    pub name: String,
    pub code: Arc<crate::bytecode::CompiledArtifact>,
    pub vars: BTreeMap<String, serde_json::Value>,
    pub bindings: BTreeMap<String, Namespace>,
    pub durable: BTreeMap<String, String>,
    pub unsafe_bindings: BTreeMap<String, String>,
    pub generation: u64,
    pub crons: Vec<crate::scheduled::Cron>,
    pub(crate) database: Database,
    pub(super) active: Mutex<(bool, Vec<RequestHandle>)>,
    pub(super) sockets: Mutex<Vec<std::sync::Weak<crate::websocket::Socket>>>,
}
impl Plugin {
    pub fn route_auth(&self, path: &str) -> crate::identity::Auth {
        self.auth_routes
            .iter()
            .filter(|r| route_matches(r.pattern(), path))
            .max_by_key(|r| (!r.pattern().ends_with("/*"), r.pattern().len()))
            .map_or(crate::identity::Auth::Public, Route::auth)
    }
    pub(crate) fn close_sockets(&self) {
        for socket in self
            .sockets
            .lock()
            .unwrap()
            .iter()
            .filter_map(std::sync::Weak::upgrade)
        {
            let _ = socket.close(1001, "gateway shutdown".into());
        }
    }
    pub(crate) fn cancel_sockets(&self) {
        for socket in self
            .sockets
            .lock()
            .unwrap()
            .iter()
            .filter_map(std::sync::Weak::upgrade)
        {
            socket.cancel();
        }
    }
    pub(crate) fn socket(&self, socket: &Arc<crate::websocket::Socket>) -> Result<()> {
        let active = self.active.lock().unwrap();
        if !active.0 {
            socket.cancel();
            bail!("plugin unavailable");
        }
        let mut sockets = self.sockets.lock().unwrap();
        sockets.retain(|s| s.strong_count() > 0);
        sockets.push(Arc::downgrade(socket));
        Ok(())
    }
    /// Admission and invalidation share a lock: no request can start using a
    /// snapshot acquired just before a configuration change.
    pub fn attach(&self, handle: RequestHandle) -> Result<()> {
        let mut active = self.active.lock().unwrap();
        if !active.0 {
            handle.cancel();
            bail!("plugin unavailable");
        }
        active.1.retain(|h| h.phase() != Phase::Closed);
        active.1.push(handle);
        Ok(())
    }
    pub(super) async fn stop(&self) {
        let handles = {
            let mut active = self.active.lock().unwrap();
            active.0 = false;
            std::mem::take(&mut active.1)
        };
        for socket in self
            .sockets
            .lock()
            .unwrap()
            .drain(..)
            .filter_map(|w| w.upgrade())
        {
            socket.cancel();
        }
        for h in &handles {
            h.cancel();
        }
        for h in handles {
            h.closed().await;
        }
    }
}
