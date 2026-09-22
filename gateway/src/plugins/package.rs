//! Bounded ZIP decoding and safe package paths; no installation side effects.
use super::Manifest;
use anyhow::{Context, Result, anyhow, bail};
use std::{
    collections::{BTreeMap, HashSet},
    io::{Cursor, Read},
};
pub(super) fn safe_path(name: &str) -> Result<()> {
    if name.is_empty()
        || name.starts_with('/')
        || name.contains(['\\', ':', '\0'])
        || name
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
        || name.split('/').next() == Some("compiled")
    {
        bail!("unsafe or reserved ZIP path");
    }
    Ok(())
}
#[derive(Clone, Copy)]
pub struct ZipLimits {
    pub upload: usize,
    pub expanded: usize,
    pub files: usize,
}
impl Default for ZipLimits {
    fn default() -> Self {
        Self {
            upload: 32 << 20,
            expanded: 128 << 20,
            files: 1024,
        }
    }
}
pub(super) struct Package {
    pub(super) manifest: Manifest,
    pub(super) text: String,
    pub(super) files: BTreeMap<String, Vec<u8>>,
}
impl Package {
    pub(super) fn read(bytes: Vec<u8>, limits: ZipLimits) -> Result<Self> {
        if bytes.len() > limits.upload {
            bail!("upload_limit: ZIP exceeds upload limit");
        }
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).context("invalid ZIP")?;
        if zip.len() > limits.files {
            bail!("ZIP file count exceeded");
        }
        let mut files = BTreeMap::new();
        let mut seen = HashSet::new();
        let mut total = 0usize;
        for i in 0..zip.len() {
            let mut file = zip.by_index(i)?;
            let name = file.name().trim_end_matches('/').to_string();
            safe_path(&name)?;
            if !seen.insert(name.clone()) {
                bail!("duplicate ZIP entry");
            }
            if let Some(mode) = file.unix_mode() {
                let kind = mode & 0o170000;
                if kind != 0 && kind != 0o100000 && kind != 0o040000 {
                    bail!("ZIP links and special files are forbidden");
                }
            }
            if file.is_dir() {
                continue;
            }
            let mut data = Vec::new();
            file.by_ref()
                .take((limits.expanded - total + 1) as u64)
                .read_to_end(&mut data)?;
            total += data.len();
            if total > limits.expanded {
                bail!("ZIP expanded size exceeded");
            }
            files.insert(name, data);
        }
        for name in files.keys() {
            for (i, _) in name.match_indices('/') {
                if files.contains_key(&name[..i]) {
                    bail!("ZIP file/directory conflict");
                }
            }
            if seen.iter().any(|n| n.starts_with(&format!("{name}/"))) {
                bail!("ZIP file/directory conflict");
            }
        }
        let text = String::from_utf8(
            files
                .get("wrangler.toml")
                .ok_or_else(|| anyhow!("missing wrangler.toml"))?
                .clone(),
        )?;
        let manifest = Manifest::parse(&text)?;
        if !files.contains_key(&manifest.main) {
            bail!("main file is absent from ZIP");
        }
        Ok(Self {
            manifest,
            text,
            files,
        })
    }
}
