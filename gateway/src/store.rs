use crate::bytecode::{Artifact, BUILD_ID, CompiledArtifact, compile, source_sha256};
use anyhow::{Context as _, Result, bail};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(serde::Serialize, serde::Deserialize)]
struct Meta {
    format_version: u32,
    bytecode_sha256: String,
    bytecode_len: usize,
    source_sha256: String,
    build_id: String,
}

pub struct BytecodeStore {
    root: PathBuf,
    cache: HashMap<String, (String, Arc<CompiledArtifact>)>,
    memory_hits: u64,
    disk_hits: u64,
    recompiles: u64,
}

impl BytecodeStore {
    pub fn new(root: PathBuf) -> Self {
        BytecodeStore {
            root,
            cache: HashMap::new(),
            memory_hits: 0,
            disk_hits: 0,
            recompiles: 0,
        }
    }
    pub fn stats(&self) -> (u64, u64, u64) {
        (self.memory_hits, self.disk_hits, self.recompiles)
    }
    pub fn invalidate(&mut self, name: &str) -> Result<()> {
        validate_name(name)?;
        self.cache.remove(name);
        Ok(())
    }

    pub fn install(&mut self, name: &str, source: &str) -> Result<Arc<CompiledArtifact>> {
        validate_name(name)?;
        let artifact = compile("plugin.mjs", source)?;
        self.write_artifact(name, &artifact)?;
        self.recompiles += 1;
        let code: Arc<CompiledArtifact> = Arc::new(artifact.code);
        self.cache
            .insert(name.to_string(), (source_sha256(source), Arc::clone(&code)));
        Ok(code)
    }

    pub fn get(&mut self, name: &str, source: &str) -> Result<Arc<CompiledArtifact>> {
        validate_name(name)?;
        if let Some((hash, code)) = self.cache.get(name)
            && *hash == source_sha256(source)
        {
            self.memory_hits += 1;
            return Ok(Arc::clone(code));
        }
        if let Some(code) = self.read_artifact(name, source)? {
            self.disk_hits += 1;
            let code: Arc<CompiledArtifact> = Arc::new(CompiledArtifact::validated(code));
            self.cache
                .insert(name.to_string(), (source_sha256(source), Arc::clone(&code)));
            return Ok(code);
        }
        self.install(name, source)
    }

    /// Validate the configured root. Only PluginManager's SQLite registry
    /// decides which manifests to load; arbitrary folders are never plugins.
    pub fn check_root(&self) -> Result<()> {
        fs::create_dir_all(&self.root)
            .with_context(|| format!("create plugin root {}", self.root.display()))?;
        Ok(())
    }

    fn compiled_dir(&self, name: &str) -> PathBuf {
        self.root.join(name).join("compiled")
    }

    fn write_artifact(&self, name: &str, artifact: &Artifact) -> Result<()> {
        let dir = self.compiled_dir(name);
        fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        atomic_write(&dir.join("plugin.qbc"), &artifact.code)?;
        let meta = toml::to_string(&Meta {
            format_version: 1,
            source_sha256: artifact.source_sha256.clone(),
            build_id: artifact.build_id.clone(),
            bytecode_len: artifact.code.len(),
            bytecode_sha256: hex::encode(Sha256::digest(&artifact.code)),
        })?;
        atomic_write(&dir.join("meta.toml"), meta.as_bytes())?;
        fs::File::open(dir.parent().expect("compiled parent"))?.sync_all()?;
        Ok(())
    }

    fn read_artifact(&self, name: &str, source: &str) -> Result<Option<Vec<u8>>> {
        let dir = self.compiled_dir(name);
        let text = match fs::read_to_string(dir.join("meta.toml")) {
            Ok(t) => t,
            Err(_) => return Ok(None),
        };
        let meta: Meta = match toml::from_str(&text) {
            Ok(m) => m,
            Err(_) => return Ok(None),
        };
        if meta.format_version != 1
            || meta.build_id != BUILD_ID
            || meta.source_sha256 != source_sha256(source)
        {
            return Ok(None);
        }
        match fs::read(dir.join("plugin.qbc")) {
            Ok(b)
                if b.len() == meta.bytecode_len
                    && hex::encode(Sha256::digest(&b)) == meta.bytecode_sha256 =>
            {
                Ok(Some(b))
            }
            Ok(_) => Ok(None),
            Err(_) => Ok(None),
        }
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    let mut file = fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path).with_context(|| format!("rename into {}", path.display()))?;
    if let Some(parent) = path.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

/// Names are identities, never filesystem paths.
pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        || !name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
    {
        bail!("invalid plugin name: expected [a-z0-9][a-z0-9_-]{{0,63}}");
    }
    Ok(())
}
