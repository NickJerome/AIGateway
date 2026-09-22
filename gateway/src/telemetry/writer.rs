//! Single writer: bounded batches, retry, retention and final flush.
use super::{Command, Config, Kind, Row, now};
use anyhow::{Result, anyhow};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
pub(super) fn run(
    mut conn: Connection,
    rx: mpsc::Receiver<Command>,
    config: Config,
    failed: Arc<AtomicU64>,
) {
    let mut batch = Vec::new();
    let mut deadline: Option<Instant> = None;
    let mut last_error = None;
    let mut checkpoint = Instant::now();
    loop {
        let result = match deadline {
            Some(at) => rx.recv_timeout(at.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected),
        };
        let mut reply = None;
        let mut stop = false;
        let mut stop_deadline = None;
        match result {
            Ok(Command::Row(row)) => {
                if batch.is_empty() {
                    deadline = Some(Instant::now() + Duration::from_millis(config.flush_ms));
                }
                batch.push(row);
                if batch.len() < config.batch_rows {
                    continue;
                }
            }
            Ok(Command::Flush(tx)) => reply = Some(tx),
            Ok(Command::Stop(tx, at)) => {
                reply = Some(tx);
                stop = true;
                stop_deadline = Some(at);
                let _ = conn.busy_timeout(
                    at.saturating_duration_since(Instant::now())
                        .min(Duration::from_secs(5)),
                );
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => stop = true,
        }
        if !batch.is_empty() {
            match commit(&mut conn, &batch, &config) {
                Ok(()) => {
                    batch.clear();
                    last_error = None;
                    deadline = None;
                }
                Err(error) => {
                    failed.fetch_add(1, Ordering::Relaxed);
                    eprintln!("telemetry commit failed: {error}; pending={}", batch.len());
                    last_error = Some(error.to_string());
                    deadline = Some(Instant::now() + Duration::from_millis(config.flush_ms));
                }
            }
        }
        if let Some(at) = stop_deadline {
            while !batch.is_empty() && Instant::now() < at {
                std::thread::sleep(
                    Duration::from_millis(config.flush_ms)
                        .min(at.saturating_duration_since(Instant::now())),
                );
                let _ = conn.busy_timeout(
                    at.saturating_duration_since(Instant::now())
                        .min(Duration::from_secs(5)),
                );
                match commit(&mut conn, &batch, &config) {
                    Ok(()) => {
                        batch.clear();
                        last_error = None;
                    }
                    Err(error) => {
                        failed.fetch_add(1, Ordering::Relaxed);
                        last_error = Some(error.to_string());
                    }
                }
            }
        }
        if (checkpoint.elapsed() >= Duration::from_secs(60) || stop) && last_error.is_none() {
            if let Err(error) = conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE)") {
                failed.fetch_add(1, Ordering::Relaxed);
                eprintln!("telemetry checkpoint failed: {error}");
            }
            checkpoint = Instant::now();
        }
        if let Some(tx) = reply {
            let _ = tx.send(
                last_error
                    .as_ref()
                    .map_or(Ok(()), |e| Err(anyhow!("{e}; uncommitted={}", batch.len()))),
            );
        }
        if stop {
            break;
        }
    }
}
fn commit(conn: &mut Connection, rows: &[Row], config: &Config) -> Result<()> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    for row in rows {
        let text = serde_json::to_string(&row.data)?;
        match row.kind {
            Kind::Usage | Kind::Logs => {
                let table = if row.kind == Kind::Usage {
                    "usage"
                } else {
                    "logs"
                };
                tx.execute(
                    &format!("INSERT INTO {table}(plugin,request,time,value) VALUES(?1,?2,?3,?4)"),
                    params![row.plugin, row.request, row.time, text],
                )?;
            }
            Kind::Traces => {
                let event = row.data["event"].as_str().unwrap_or("");
                if event == "start" {
                    tx.execute("INSERT INTO traces(plugin,request,time,value,active,bytes) VALUES(?1,?2,?3,?4,1,0)",params![row.plugin,row.request,row.time,text])?;
                } else if event == "chunk" {
                    let body = row
                        .body
                        .as_ref()
                        .ok_or_else(|| anyhow!("missing trace chunk"))?;
                    tx.execute(
                        "INSERT INTO trace_chunks VALUES(?1,?2,?3,?4,?5)",
                        params![
                            row.plugin,
                            row.request,
                            row.data["side"].as_str(),
                            row.data["seq"].as_i64(),
                            body.as_ref()
                        ],
                    )?;
                    tx.execute(
                        "UPDATE traces SET bytes=bytes+?3 WHERE plugin=?1 AND request=?2",
                        params![row.plugin, row.request, body.len() as i64],
                    )?;
                } else {
                    let mut value: Value = serde_json::from_str(&tx.query_row(
                        "SELECT value FROM traces WHERE plugin=?1 AND request=?2",
                        params![row.plugin, row.request],
                        |r| r.get::<_, String>(0),
                    )?)?;
                    if event == "attempt" {
                        if !value["attempts"].is_array() {
                            value["attempts"] = json!([]);
                        }
                        value["attempts"]
                            .as_array_mut()
                            .unwrap()
                            .push(row.data.clone());
                    } else if let Some(fields) = row.data.as_object() {
                        for (k, v) in fields {
                            value[k] = v.clone();
                        }
                    }
                    tx.execute("UPDATE traces SET value=?3,active=CASE WHEN ?4='end' THEN 0 ELSE active END WHERE plugin=?1 AND request=?2",params![row.plugin,row.request,serde_json::to_string(&value)?,event])?;
                }
            }
        }
    }
    let cutoff = now().saturating_sub(
        (config.retain_days.saturating_mul(86_400_000)).min(i64::MAX as u64) as i64,
    );
    tx.execute(
        "DELETE FROM usage WHERE time<?1",
        [now().saturating_sub(
            config
                .usage_days
                .saturating_mul(86_400_000)
                .min(i64::MAX as u64) as i64,
        )],
    )?;
    tx.execute("DELETE FROM logs WHERE time<?1 OR id IN (SELECT id FROM logs ORDER BY id DESC LIMIT -1 OFFSET ?2)",params![cutoff,config.retain_rows as i64])?;
    tx.execute("DELETE FROM traces WHERE active=0 AND (time<?1 OR rowid IN (SELECT rowid FROM traces WHERE active=0 ORDER BY rowid DESC LIMIT -1 OFFSET ?2))",params![cutoff,config.retain_rows as i64])?;
    loop {
        let total: i64 = tx.query_row(
            "SELECT (SELECT COALESCE(sum(bytes+length(CAST(value AS BLOB))),0) FROM traces) + (SELECT COALESCE(sum(length(CAST(value AS BLOB))),0) FROM logs)",
            [],
            |r| r.get(0),
        )?;
        if total as u64 <= config.retain_bytes {
            break;
        }
        let oldest: Option<(String, i64)> = tx.query_row(
            "SELECT kind,id FROM (SELECT 'traces' AS kind,rowid AS id,time FROM traces WHERE active=0 UNION ALL SELECT 'logs',id,time FROM logs) ORDER BY time,id LIMIT 1", [], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((table, id)) = oldest else {
            break;
        };
        tx.execute(&format!("DELETE FROM {table} WHERE rowid=?1"), [id])?;
    }
    tx.execute("DELETE FROM trace_chunks WHERE NOT EXISTS(SELECT 1 FROM traces WHERE traces.plugin=trace_chunks.plugin AND traces.request=trace_chunks.request)",[])?;
    tx.commit()?;
    Ok(())
}

pub(super) fn open(dir: &std::path::Path) -> Result<Connection> {
    let conn = Connection::open(dir.join("gateway.sqlite"))?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.execute("UPDATE traces SET active=0,value=json_set(value,'$.complete',json('false'),'$.error','process interrupted') WHERE active=1", [])?;
    Ok(conn)
}
