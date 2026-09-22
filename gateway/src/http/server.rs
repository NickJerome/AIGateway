//! Service assembly, connection ownership and graceful shutdown.
use crate::{
    config::Config,
    network::Network,
    plugins::{PluginManager, ZipLimits},
    storage::Database,
};
use anyhow::{Result, anyhow};
use hyper::service::service_fn;
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
};
use std::{
    convert::Infallible,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Semaphore;

use super::{App, workers::Workers};
use crate::telemetry::now as epoch_ms;
pub struct Server {
    pub address: SocketAddr,
    pub plugins: usize,
    pub loaded: u64,
    pub recompiled: u64,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<Result<()>>,
    app: Arc<App>,
}
impl Server {
    pub async fn start(config: Config) -> Result<Self> {
        let token = match &config.admin_token_env {
            Some(name) => {
                let value = std::env::var(name)
                    .map_err(|_| anyhow!("administrator token environment variable is missing"))?;
                if value.is_empty() {
                    return Err(anyhow!("administrator token is empty"));
                }
                Some(value)
            }
            None => None,
        };
        let db = Database::open(
            &config.data_dir,
            config.database_threads,
            config.database_queue,
        )?;
        let manager = PluginManager::open(
            config.plugins_dir.clone(),
            db.clone(),
            ZipLimits {
                upload: config.zip_upload_bytes,
                expanded: config.zip_expanded_bytes,
                files: config.zip_files,
            },
        )
        .await?;
        let plugins = manager.list().len();
        let (_, loaded, recompiled) = manager.stats();
        let routes = manager.routes();
        let listener = tokio::net::TcpListener::bind(&config.listen).await?;
        let address = listener.local_addr()?;
        let identity = crate::identity::Service::new(db.clone());
        let telemetry =
            crate::telemetry::Telemetry::open(&config.data_dir, db, config.telemetry.clone())?;
        let workers = Workers::new(&config, telemetry.clone())?;
        let app = Arc::new(App {
            identity,
            network: Network::new(config.http),
            telemetry,
            config,
            manager: Arc::new(tokio::sync::Mutex::new(manager)),
            routes,
            workers,
            token,
            mutation: Arc::new(Semaphore::new(1)),
            websocket_tasks: Mutex::new(Vec::new()),
            schedule: Mutex::new((Default::default(), Default::default())),
        });
        app.schedule.lock().unwrap().0.advance(epoch_ms());
        let server_app = app.clone();
        let (stop, mut stopped) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            let (shutdown, shutdown_rx) = tokio::sync::watch::channel(false);
            let mut ticks = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    _=ticks.tick()=>{app.tick(epoch_ms());},
                    _ = &mut stopped => break,
                    accepted = listener.accept() => {
                        let (socket, _) = accepted?;
                        connections.spawn(serve_connection(socket, app.clone(), shutdown_rx.clone()));
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
            drop(listener);
            app.workers.close();
            for plugin in app.routes.enabled() {
                plugin.close_sockets();
            }
            let _ = shutdown.send(true);
            let deadline = tokio::time::Instant::now()
                + Duration::from_millis(app.config.http.drain_timeout_ms);
            let drained = tokio::time::timeout_at(deadline, async {
                while connections.join_next().await.is_some() {}
                app.workers.drained().await;
            })
            .await;
            if drained.is_err() {
                connections.abort_all();
                app.workers.cancel();
            }
            while connections.join_next().await.is_some() {}
            for plugin in app.routes.enabled() {
                plugin.cancel_sockets();
            }
            app.workers.cancel();
            let tasks = std::mem::take(&mut *app.websocket_tasks.lock().unwrap());
            for mut task in tasks {
                if tokio::time::timeout_at(deadline, &mut task).await.is_err() {
                    task.abort();
                    let _ = task.await;
                }
            }
            app.workers.join().await;
            app.telemetry
                .shutdown_with_timeout(
                    deadline.saturating_duration_since(tokio::time::Instant::now()),
                )
                .await?;
            Ok(())
        });
        Ok(Self {
            address,
            plugins,
            loaded,
            recompiled,
            stop: Some(stop),
            task,
            app: server_app,
        })
    }
    #[doc(hidden)]
    pub fn tick(&self, now_ms: i64) {
        self.app.tick(now_ms);
    }
    pub async fn shutdown(mut self) -> Result<()> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        self.task.await?
    }
}
async fn serve_connection(
    socket: tokio::net::TcpStream,
    app: Arc<App>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let queue_bytes = app.config.http.queue_bytes;
    let service = service_fn(move |request| {
        let app = app.clone();
        async move { Ok::<_, Infallible>(app.handle(request).await) }
    });
    let mut builder = Builder::new(TokioExecutor::new());
    builder.http1().max_buf_size(queue_bytes);
    builder
        .http2()
        .max_send_buf_size(queue_bytes)
        .initial_stream_window_size(queue_bytes.min(u32::MAX as usize) as u32);
    let connection = builder.serve_connection_with_upgrades(TokioIo::new(socket), service);
    tokio::pin!(connection);
    tokio::select! {
        _ = &mut connection => {},
        _ = shutdown.changed() => {
            connection.as_mut().graceful_shutdown();
            let _ = connection.await;
        }
    }
}
