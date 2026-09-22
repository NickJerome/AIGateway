//! Plugin lifecycle: validate, persist, compile, activate and publish routing snapshots.
use crate::{
    storage::Database,
    store::{BytecodeStore, validate_name},
};
use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeMap, HashSet},
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
};
mod error;
pub use error::Error;
mod instance;
mod manifest;
mod package;
mod persistence;
mod routing;
pub use instance::Plugin;
pub use manifest::{
    Build, DoBinding, Durable, KvBinding, Manifest, Migration, Route, RoutePattern, Triggers,
    UnsafeBinding, UnsafeBindings,
};
use package::Package;
pub use package::ZipLimits;
pub use persistence::{Overrides, PluginRecord};
pub use routing::{Matched, Routes, route_matches};
use routing::{RouteEntry, Snapshot};

struct Installed {
    record: PluginRecord,
    manifest: Manifest,
    plugin: Option<Arc<Plugin>>,
    hash: Option<String>,
}
pub struct PluginManager {
    root: PathBuf,
    db: Database,
    store: BytecodeStore,
    entries: BTreeMap<String, Installed>,
    routes: Routes,
    limits: ZipLimits,
}
impl PluginManager {
    pub async fn open(root: PathBuf, db: Database, limits: ZipLimits) -> Result<Self> {
        std::fs::create_dir_all(&root)?;
        let mut manager = Self {
            store: BytecodeStore::new(root.clone()),
            root,
            db,
            entries: BTreeMap::new(),
            routes: Routes::default(),
            limits,
        };
        PluginRecord::recover(&manager.db).await?;
        for mut record in PluginRecord::list(&manager.db).await? {
            if !record.installed {
                continue;
            }
            let manifest = Manifest::parse(&record.manifest)?;
            let name = record.name.clone();
            validate_name(&name)?;
            let failed = record.state == "failed";
            let wanted = record.enabled;
            record.enabled = false;
            manager.entries.insert(
                name.clone(),
                Installed {
                    record,
                    manifest,
                    plugin: None,
                    hash: None,
                },
            );
            if failed {
                continue;
            }
            let result = manager.load(&name, wanted).await;
            if let Err(e) = result {
                manager.fail(&name, &e).await?;
            }
        }
        manager.publish();
        Ok(manager)
    }
    pub fn routes(&self) -> Routes {
        self.routes.clone()
    }
    pub fn stats(&self) -> (u64, u64, u64) {
        self.store.stats()
    }
    pub fn list(&self) -> Vec<serde_json::Value> {
        self.entries.values().map(summary).collect()
    }
    pub fn detail(&self, name: &str) -> Result<serde_json::Value> {
        validate_name(name)?;
        Ok(summary(
            self.entries
                .get(name)
                .ok_or_else(|| anyhow::Error::from(Error::NotFound))?,
        ))
    }
    /// Configuration is available only through the dedicated administrator endpoint.
    /// Keep values out of the list/detail snapshots, which are used for navigation.
    pub fn configuration(&self, name: &str) -> Result<serde_json::Value> {
        validate_name(name)?;
        let entry = self
            .entries
            .get(name)
            .ok_or_else(|| anyhow::Error::from(Error::NotFound))?;
        Ok(serde_json::json!({
            "defaults": entry.manifest.vars,
            "overrides": entry.record.overrides,
        }))
    }
    fn publish(&self) {
        let routes = self
            .entries
            .values()
            .flat_map(|e| {
                e.manifest.routes.iter().map(|r| RouteEntry {
                    pattern: r.pattern().to_owned(),
                    name: e.record.name.clone(),
                    plugin: e.plugin.clone(),
                })
            })
            .collect();
        *self.routes.0.write().unwrap() = Snapshot {
            plugins: self
                .entries
                .values()
                .filter_map(|e| e.plugin.clone())
                .collect(),
            routes,
            summaries: self.list(),
        };
    }
    fn check_routes(&self, manifest: &Manifest) -> Result<()> {
        for route in &manifest.routes {
            if self
                .routes
                .0
                .read()
                .unwrap()
                .routes
                .iter()
                .any(|r| r.name != manifest.name && r.pattern == route.pattern())
            {
                return Err(Error::RouteConflict.into());
            }
        }
        Ok(())
    }
    async fn stop(&mut self, name: &str) -> Result<()> {
        let entry = self
            .entries
            .get_mut(name)
            .ok_or_else(|| anyhow::Error::from(Error::NotFound))?;
        let plugin = entry.plugin.take();
        entry.record.enabled = false;
        entry.record.state = "disabled".into();
        self.publish();
        self.store.invalidate(name)?;
        if let Some(plugin) = plugin {
            plugin.stop().await;
        }
        Ok(())
    }
    async fn fail(&mut self, name: &str, error: &anyhow::Error) -> Result<()> {
        let entry = self.entries.get_mut(name).unwrap();
        entry.plugin = None;
        entry.record.enabled = false;
        entry.record.state = "failed".into();
        entry.record.error = Some(format!("{error:#}"));
        entry.record.clone().save(&self.db).await?;
        self.publish();
        Ok(())
    }
    async fn load(&mut self, name: &str, enabled: bool) -> Result<()> {
        let entry = self.entries.get_mut(name).unwrap();
        let disk = std::fs::read_to_string(self.root.join(name).join("wrangler.toml"))
            .context("read installed manifest")?;
        if disk != entry.record.manifest {
            bail!("installed manifest differs from registration");
        }
        let source = std::fs::read_to_string(self.root.join(name).join(&entry.manifest.main))
            .context("read plugin main")?;
        let code = self.store.get(name, &source)?;
        entry.hash = Some(crate::bytecode::source_sha256(&source));
        let vars = entry.manifest.variables(&entry.record.overrides)?;
        if enabled {
            entry.manifest.supported().context(Error::InvalidConfig)?;
            let classes: Vec<String> = entry
                .manifest
                .durable_objects
                .iter()
                .flat_map(|d| d.bindings.iter().map(|b| b.class_name.clone()))
                .collect();
            crate::bytecode::validate_exports(&source, &classes)?;
            self.db
                .migrate_durable(name.into(), entry.manifest.migrations.clone())
                .await?;
            let durable: BTreeMap<String, String> = entry
                .manifest
                .durable_objects
                .iter()
                .flat_map(|d| {
                    d.bindings
                        .iter()
                        .map(|b| (b.name.clone(), b.class_name.clone()))
                })
                .collect();
            let mut bindings = BTreeMap::new();
            for b in &entry.manifest.kv_namespaces {
                let id = entry
                    .record
                    .overrides
                    .bindings
                    .get(&b.binding)
                    .unwrap_or(&b.id);
                bindings.insert(b.binding.clone(), self.db.namespace(id).await?);
            }
            entry.plugin = Some(Arc::new(Plugin {
                auth_routes: entry.manifest.routes.clone(),
                trace_body: entry.record.overrides.trace_body,
                name: name.into(),
                code,
                vars,
                bindings,
                crons: {
                    let mut seen = HashSet::new();
                    entry
                        .manifest
                        .triggers
                        .iter()
                        .flat_map(|t| &t.crons)
                        .map(|c| crate::scheduled::Cron::parse(c))
                        .collect::<Result<Vec<_>>>()?
                        .into_iter()
                        .filter(|c| seen.insert(c.expression.clone()))
                        .collect()
                },
                unsafe_bindings: entry
                    .manifest
                    .unsafe_config
                    .iter()
                    .flat_map(|u| u.bindings.iter().map(|b| (b.name.clone(), b.kind.clone())))
                    .collect(),
                durable,
                database: self.db.clone(),
                generation: {
                    static NEXT: std::sync::atomic::AtomicU64 =
                        std::sync::atomic::AtomicU64::new(1);
                    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                },
                active: Mutex::new((true, Vec::new())),
                sockets: Mutex::new(Vec::new()),
            }));
        }
        entry.record.enabled = enabled;
        entry.record.state = if enabled { "enabled" } else { "disabled" }.into();
        entry.record.error = None;
        entry.record.clone().save(&self.db).await?;
        Ok(())
    }
    pub async fn install(&mut self, bytes: Vec<u8>) -> Result<(bool, serde_json::Value)> {
        if bytes.len() > self.limits.upload {
            return Err(Error::UploadLimit.into());
        }
        let package = Package::read(bytes, self.limits).context(Error::InvalidPackage)?;
        self.check_routes(&package.manifest)?;
        let name = package.manifest.name.clone();
        let existed = self.entries.contains_key(&name);
        let old = PluginRecord::list(&self.db)
            .await?
            .into_iter()
            .find(|p| p.name == name);
        let record = PluginRecord {
            name: name.clone(),
            state: "installing".into(),
            enabled: false,
            manifest: package.text,
            error: None,
            overrides: old.map(|r| r.overrides).unwrap_or_default(),
            installed: true,
        };
        record.clone().save(&self.db).await?;
        if existed {
            self.stop(&name).await?;
        }
        self.entries.insert(
            name.clone(),
            Installed {
                record,
                manifest: package.manifest,
                plugin: None,
                hash: None,
            },
        );
        self.publish();
        let result = async {
            let dir = self.root.join(&name);
            if dir.exists() {
                std::fs::remove_dir_all(&dir)?;
            }
            std::fs::create_dir_all(&dir)?;
            for (path, data) in package.files {
                let path = dir.join(path);
                std::fs::create_dir_all(path.parent().unwrap())?;
                let mut f = std::fs::File::create(path)?;
                f.write_all(&data)?;
                f.sync_all()?;
            }
            sync_directory_tree(&dir)?;
            std::fs::File::open(&self.root)?.sync_all()?;
            self.load(&name, true).await
        }
        .await;
        if let Err(e) = result {
            self.fail(&name, &e).await?;
            return Err(e);
        }
        self.publish();
        Ok((!existed, self.detail(&name)?))
    }
    pub async fn enable(&mut self, name: &str, enabled: bool) -> Result<serde_json::Value> {
        validate_name(name)?;
        self.stop(name).await?;
        if let Err(e) = self.load(name, enabled).await {
            self.fail(name, &e).await?;
            return Err(e);
        }
        self.publish();
        self.detail(name)
    }
    pub async fn configure(
        &mut self,
        name: &str,
        overrides: Overrides,
    ) -> Result<serde_json::Value> {
        validate_name(name)?;
        let enabled = self
            .entries
            .get(name)
            .ok_or_else(|| anyhow::Error::from(Error::NotFound))?
            .record
            .enabled;
        self.entries[name]
            .manifest
            .variables(&overrides)
            .context(Error::InvalidConfig)?;
        self.stop(name).await?;
        self.entries.get_mut(name).unwrap().record.overrides = overrides;
        self.entries[name].record.clone().save(&self.db).await?;
        if let Err(e) = self.load(name, enabled).await {
            self.fail(name, &e).await?;
            return Err(e);
        }
        self.publish();
        self.detail(name)
    }
    pub async fn uninstall(&mut self, name: &str) -> Result<()> {
        validate_name(name)?;
        self.stop(name).await?;
        let mut record = self.entries[name].record.clone();
        record.installed = false;
        record.save(&self.db).await?;
        let dir = self.root.join(name);
        if dir.exists() {
            std::fs::remove_dir_all(dir)?;
        }
        self.entries.remove(name);
        self.publish();
        Ok(())
    }
}
fn summary(entry: &Installed) -> serde_json::Value {
    serde_json::json!({"name":entry.record.name,"state":entry.record.state,"enabled":entry.record.enabled,"error":entry.record.error,"source_hash":entry.hash,
        "main":entry.manifest.main,"routes":entry.manifest.routes,"bindings":entry.manifest.kv_namespaces.iter().map(|b|serde_json::json!({"binding":b.binding,"id":entry.record.overrides.bindings.get(&b.binding).unwrap_or(&b.id)})).collect::<Vec<_>>(),"build_id":crate::bytecode::BUILD_ID})
}

fn sync_directory_tree(dir: &std::path::Path) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            sync_directory_tree(&entry.path())?;
        }
    }
    std::fs::File::open(dir)?.sync_all()?;
    Ok(())
}
