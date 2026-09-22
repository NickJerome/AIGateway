//! HTTP service facade. Runtime scheduling, transport and administration are separate adapters.
use crate::{
    config::Config,
    network::Network,
    plugins::{PluginManager, Routes},
};
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;

mod admin;
mod auth;
mod body;
mod identity;
mod ingress;
mod scheduler;
mod server;
mod workers;
pub use server::Server;
use workers::Workers;

struct App {
    identity: crate::identity::Service,
    config: Config,
    network: Network,
    telemetry: crate::telemetry::Telemetry,
    manager: Arc<tokio::sync::Mutex<PluginManager>>,
    routes: Routes,
    workers: Workers,
    token: Option<String>,
    mutation: Arc<Semaphore>,
    websocket_tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    schedule: Mutex<(
        crate::scheduled::Schedule,
        std::collections::HashSet<(u64, String)>,
    )>,
}
