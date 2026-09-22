//! Persist plugin registrations and overrides; lifecycle decisions stay in PluginManager.
use crate::storage::Database;
use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Overrides {
    #[serde(default)]
    pub trace_body: Option<bool>,
    #[serde(default)]
    pub vars: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub bindings: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginRecord {
    pub name: String,
    pub state: String,
    pub enabled: bool,
    pub manifest: String,
    pub error: Option<String>,
    pub overrides: Overrides,
    pub installed: bool,
}
impl PluginRecord {
    pub(super) async fn recover(db: &Database) -> Result<()> {
        db.run(|conn| {
            conn.execute("UPDATE plugin_settings SET state='failed', error='interrupted installation', enabled=0 WHERE state='installing'", [])?;
            Ok(())
        }).await
    }

    pub async fn list(db: &Database) -> Result<Vec<Self>> {
        db.run(|conn| {
            let mut stmt = conn.prepare(
                "SELECT name, state, enabled, manifest, error, overrides, installed
                 FROM plugin_settings ORDER BY name",
            )?;
            let mut rows = stmt.query([])?;
            let mut records = Vec::new();
            while let Some(row) = rows.next()? {
                records.push(Self {
                    name: row.get(0)?,
                    state: row.get(1)?,
                    enabled: row.get(2)?,
                    manifest: row.get(3)?,
                    error: row.get(4)?,
                    overrides: serde_json::from_str(&row.get::<_, String>(5)?)?,
                    installed: row.get(6)?,
                });
            }
            Ok(records)
        })
        .await
    }

    pub async fn save(self, db: &Database) -> Result<()> {
        db.run(move |conn| {
            conn.execute(
                "INSERT INTO plugin_settings
                    (name, state, enabled, manifest, error, overrides, installed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(name) DO UPDATE SET
                    state=excluded.state, enabled=excluded.enabled,
                    manifest=excluded.manifest, error=excluded.error,
                    overrides=excluded.overrides, installed=excluded.installed",
                params![
                    self.name,
                    self.state,
                    self.enabled,
                    self.manifest,
                    self.error,
                    serde_json::to_string(&self.overrides)?,
                    self.installed
                ],
            )?;
            Ok(())
        })
        .await
    }
}
