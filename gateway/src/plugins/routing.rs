//! Read-only routing snapshots, exact/prefix matching and route validation.
use super::Plugin;
use anyhow::{Result, bail};
use std::sync::{Arc, RwLock};
pub(super) fn validate_route(route: &str) -> Result<()> {
    if !route.starts_with('/')
        || route.contains(['?', '#', '\\'])
        || route.contains('*') && (!route.ends_with("/*") || route[..route.len() - 1].contains('*'))
        || route == "/__gateway"
        || route.starts_with("/__gateway/")
    {
        bail!("invalid or reserved route {route}");
    }
    Ok(())
}
pub fn route_matches(pattern: &str, path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path);
    match pattern.strip_suffix("/*") {
        Some(prefix) => {
            path == prefix
                || path
                    .strip_prefix(prefix)
                    .is_some_and(|s| s.starts_with('/'))
        }
        None => path == pattern,
    }
}
#[derive(Clone)]
pub(super) struct RouteEntry {
    pub(super) pattern: String,
    pub(super) name: String,
    pub(super) plugin: Option<Arc<Plugin>>,
}
#[derive(Default)]
pub(super) struct Snapshot {
    pub(super) plugins: Vec<Arc<Plugin>>,
    pub(super) routes: Vec<RouteEntry>,
    pub(super) summaries: Vec<serde_json::Value>,
}
#[derive(Clone, Default)]
pub struct Routes(pub(super) Arc<RwLock<Snapshot>>);
pub enum Matched {
    Missing,
    Unavailable,
    Ready(Arc<Plugin>),
}
impl Routes {
    pub fn enabled(&self) -> Vec<Arc<Plugin>> {
        self.0.read().unwrap().plugins.clone()
    }
    pub fn list(&self) -> Vec<serde_json::Value> {
        self.0.read().unwrap().summaries.clone()
    }
    pub fn lookup(&self, path: &str) -> Matched {
        if path == "/__gateway" || path.starts_with("/__gateway/") {
            return Matched::Missing;
        }
        let routes = self.0.read().unwrap();
        match routes
            .routes
            .iter()
            .filter(|r| route_matches(&r.pattern, path))
            .max_by_key(|r| (!r.pattern.ends_with("/*"), r.pattern.len()))
        {
            None => Matched::Missing,
            Some(r) => r
                .plugin
                .clone()
                .map_or(Matched::Unavailable, Matched::Ready),
        }
    }
}
