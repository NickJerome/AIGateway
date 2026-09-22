//! One SQLite file and a bounded queue serviced by persistent connections.
use anyhow::{Result, anyhow, bail};
use futures::channel::oneshot;
use rusqlite::Connection;

mod kv;
mod schema;
pub use kv::{Key, KeyList, Namespace};
use std::{path::Path, sync::Arc, thread::JoinHandle, time::Duration};

type Job = Box<dyn FnOnce(&mut Connection) + Send>;
struct Pool {
    queue: async_channel::Sender<Job>,
    workers: Vec<JoinHandle<()>>,
}
impl Drop for Pool {
    fn drop(&mut self) {
        self.queue.close();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}
#[derive(Clone)]
pub struct Database(Arc<Pool>);

impl Database {
    /// Migrate before spawning workers or accepting requests. No destructive recovery.
    pub fn open(data_dir: &Path, threads: usize, capacity: usize) -> Result<Self> {
        if threads == 0 || threads > 4 || capacity == 0 {
            bail!("database requires 1–4 workers and a positive queue capacity");
        }
        std::fs::create_dir_all(data_dir)?;
        let path = data_dir.join("gateway.sqlite");
        let mut first = connect(&path)?;
        schema::migrate(&mut first)?;
        let mut connections = vec![first];
        for _ in 1..threads {
            connections.push(connect(&path)?);
        }
        let (queue, rx) = async_channel::bounded::<Job>(capacity);
        let mut workers = Vec::new();
        for (id, mut conn) in connections.into_iter().enumerate() {
            let rx = rx.clone();
            workers.push(
                std::thread::Builder::new()
                    .name(format!("sqlite-{id}"))
                    .spawn(move || {
                        while let Ok(job) = rx.recv_blocking() {
                            job(&mut conn);
                        }
                    })?,
            );
        }
        Ok(Self(Arc::new(Pool { queue, workers })))
    }
    /// Cancellation before start skips the job. Once started, the transaction runs
    /// to completion even if its receiver goes away; no callback can enter dead JS.
    pub async fn run<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (tx, rx) = oneshot::channel();
        self.0
            .queue
            .send(Box::new(move |conn| {
                if !tx.is_canceled() {
                    let result = f(conn);
                    let _ = tx.send(result);
                }
            }))
            .await
            .map_err(|_| anyhow!("database queue closed"))?;
        rx.await.map_err(|_| anyhow!("database worker stopped"))?
    }
}
fn connect(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_millis(5000))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "FULL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(conn)
}
