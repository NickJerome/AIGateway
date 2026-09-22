//! Ordered SQLite migrations. Each version commits atomically; never erase on failure.
use anyhow::{Result, bail};
use rusqlite::Connection;

pub(super) fn migrate(conn: &mut Connection) -> Result<()> {
    const MIGRATIONS: &[&str] = &[
        include_str!("001.sql"),
        include_str!("002.sql"),
        include_str!("003.sql"),
        include_str!("004.sql"),
    ];
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version < 0 || version > MIGRATIONS.len() as i64 {
        bail!(
            "database schema {version} is newer than supported schema {}",
            MIGRATIONS.len()
        );
    }
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (index + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}
