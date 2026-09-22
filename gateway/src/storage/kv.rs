//! SQLite-backed plugin KV namespaces, expiration and listing.
use super::Database;
use anyhow::{Result, anyhow, bail};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

impl Database {
    pub async fn namespace(&self, id: &str) -> Result<Namespace> {
        if id.is_empty() || id.len() > 512 {
            bail!("namespace ID must contain 1–512 bytes");
        }
        let id = id.to_owned();
        let table = self
            .run(move |conn| {
                // Only the hash-derived table name is interpolated; user values are bound.
                let table = format!("kv_{}", crate::bytecode::source_sha256(&id));
                let tx = conn.transaction()?;
                tx.execute(
                "INSERT OR IGNORE INTO resources(id, kind, physical_name) VALUES (?1, 'kv', ?2)",
                params![id, table],
            )?;
                let (kind, stored): (String, String) = tx.query_row(
                    "SELECT kind, physical_name FROM resources WHERE id=?1",
                    [&id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                if kind != "kv" || stored != table {
                    bail!("resource identity conflicts with KV namespace");
                }
                tx.execute_batch(&format!(
                    "CREATE TABLE IF NOT EXISTS {table}
                 (key TEXT PRIMARY KEY, value BLOB NOT NULL, expires_at INTEGER)"
                ))?;
                tx.commit()?;
                Ok(table)
            })
            .await?;
        Ok(Namespace {
            db: self.clone(),
            table,
        })
    }
}
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}
fn key_valid(key: &str) -> Result<()> {
    if key.is_empty() || key.len() > 512 {
        bail!("KV key must contain 1–512 UTF-8 bytes");
    }
    Ok(())
}
#[derive(Clone)]
pub struct Namespace {
    db: Database,
    table: String,
}
#[derive(Debug, Serialize)]
pub struct Key {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiration: Option<i64>,
}
#[derive(Debug, Serialize)]
pub struct KeyList {
    pub keys: Vec<Key>,
    pub list_complete: bool,
}
impl Namespace {
    pub async fn get(&self, key: String) -> Result<Option<Vec<u8>>> {
        self.read(key, None).await
    }
    #[doc(hidden)]
    pub async fn get_at(&self, key: String, time: i64) -> Result<Option<Vec<u8>>> {
        self.read(key, Some(time)).await
    }
    async fn read(&self, key: String, time: Option<i64>) -> Result<Option<Vec<u8>>> {
        key_valid(&key)?;
        let table = self.table.clone();
        self.db
            .run(move |c| {
                Ok(c.query_row(
                    &format!(
                        "SELECT value FROM {table}
                          WHERE key=?1 AND (expires_at IS NULL OR expires_at>?2)"
                    ),
                    params![key, time.unwrap_or_else(now)],
                    |r| r.get(0),
                )
                .optional()?)
            })
            .await
    }
    pub async fn put(
        &self,
        key: String,
        value: impl Into<bytes::Bytes>,
        ttl: Option<u64>,
    ) -> Result<()> {
        self.write(key, value.into(), ttl, None).await
    }
    #[doc(hidden)]
    pub async fn put_at(
        &self,
        key: String,
        value: impl Into<bytes::Bytes>,
        ttl: Option<u64>,
        time: i64,
    ) -> Result<()> {
        self.write(key, value.into(), ttl, Some(time)).await
    }
    async fn write(
        &self,
        key: String,
        value: bytes::Bytes,
        ttl: Option<u64>,
        time: Option<i64>,
    ) -> Result<()> {
        key_valid(&key)?;
        if value.len() > 25 * 1024 * 1024 {
            bail!("KV value exceeds 25 MiB");
        }
        if ttl.is_some_and(|ttl| ttl < 60 || ttl > i64::MAX as u64) {
            bail!("expirationTtl must be an integer between 60 and i64::MAX seconds");
        }
        let table = self.table.clone();
        self.db
            .run(move |c| {
                let time = time.unwrap_or_else(now);
                let expiration = ttl
                    .map(|ttl| {
                        time.checked_add(ttl as i64)
                            .ok_or_else(|| anyhow!("expirationTtl overflow"))
                    })
                    .transpose()?;
                c.execute(
                    &format!(
                        "INSERT INTO {table}(key, value, expires_at) VALUES (?1, ?2, ?3)
                          ON CONFLICT(key) DO UPDATE SET
                          value=excluded.value, expires_at=excluded.expires_at"
                    ),
                    params![key, value.as_ref(), expiration],
                )?;
                Ok(())
            })
            .await
    }
    pub async fn delete(&self, key: String) -> Result<()> {
        key_valid(&key)?;
        let table = self.table.clone();
        self.db
            .run(move |c| {
                c.execute(&format!("DELETE FROM {table} WHERE key=?1"), [key])?;
                Ok(())
            })
            .await
    }
    pub async fn list(&self, prefix: String, limit: usize) -> Result<KeyList> {
        if !(1..=1000).contains(&limit) {
            bail!("KV list limit must be 1–1000");
        }
        let table = self.table.clone();
        self.db
            .run(move |c| {
                let mut stmt = c.prepare(&format!(
                    "SELECT key, expires_at FROM {table}
                 WHERE substr(CAST(key AS BLOB), 1, length(CAST(?1 AS BLOB)))=CAST(?1 AS BLOB)
                   AND (expires_at IS NULL OR expires_at>?2)
                 ORDER BY key COLLATE BINARY LIMIT ?3"
                ))?;
                let mut keys = stmt
                    .query_map(params![prefix, now(), (limit + 1) as i64], |r| {
                        Ok(Key {
                            name: r.get(0)?,
                            expiration: r.get(1)?,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let list_complete = keys.len() <= limit;
                keys.truncate(limit);
                Ok(KeyList {
                    keys,
                    list_complete,
                })
            })
            .await
    }
}
