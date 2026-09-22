//! Validated plugin manifest and host binding configuration.
use super::{Overrides, package::safe_path, routing::validate_route};
use crate::store::validate_name;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    pub main: String,
    #[serde(default)]
    pub routes: Vec<Route>,
    #[serde(default)]
    pub vars: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub kv_namespaces: Vec<KvBinding>,
    pub compatibility_date: Option<String>,
    pub build: Option<Build>,
    pub durable_objects: Option<Durable>,
    #[serde(default)]
    pub migrations: Vec<Migration>,
    pub triggers: Option<Triggers>,
    #[serde(rename = "unsafe")]
    pub unsafe_config: Option<UnsafeBindings>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Route {
    Path(String),
    Pattern(RoutePattern),
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoutePattern {
    pub pattern: String,
    #[serde(default)]
    pub auth: crate::identity::Auth,
}
impl Route {
    pub fn auth(&self) -> crate::identity::Auth {
        match self {
            Self::Path(_) => crate::identity::Auth::Public,
            Self::Pattern(route) => route.auth,
        }
    }
    pub fn pattern(&self) -> &str {
        match self {
            Self::Path(s) => s,
            Self::Pattern(pattern) => &pattern.pattern,
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KvBinding {
    pub binding: String,
    pub id: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Build {
    pub command: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Durable {
    pub bindings: Vec<DoBinding>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DoBinding {
    pub name: String,
    pub class_name: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Migration {
    pub tag: String,
    #[serde(default)]
    pub new_sqlite_classes: Vec<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Triggers {
    pub crons: Vec<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UnsafeBindings {
    pub bindings: Vec<UnsafeBinding>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UnsafeBinding {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
}
impl Manifest {
    pub fn parse(text: &str) -> Result<Self> {
        let manifest: Self = toml::from_str(text).context("invalid wrangler.toml")?;
        validate_name(&manifest.name)?;
        safe_path(&manifest.main)?;
        if let Some(date) = &manifest.compatibility_date {
            validate_date(date)?;
        }
        let mut names = HashSet::new();
        if manifest.vars.contains_key("IDENTITY") {
            bail!("IDENTITY is a reserved host binding");
        }
        for binding in &manifest.kv_namespaces {
            if binding.binding.is_empty()
                || binding.binding == "IDENTITY"
                || !names.insert(&binding.binding)
                || manifest.vars.contains_key(&binding.binding)
            {
                bail!("duplicate or empty binding name");
            }
            if binding.id.is_empty() || binding.id.len() > 512 {
                bail!("invalid KV namespace ID");
            }
        }
        let mut routes = HashSet::new();
        for route in &manifest.routes {
            let route = route.pattern();
            validate_route(route)?;
            if !routes.insert(route) {
                bail!("duplicate route");
            }
        }
        Ok(manifest)
    }
    pub(super) fn variables(
        &self,
        overrides: &Overrides,
    ) -> Result<BTreeMap<String, serde_json::Value>> {
        let mut vars = self.vars.clone();
        vars.extend(overrides.vars.clone());
        if vars.contains_key("IDENTITY") {
            bail!("IDENTITY is a reserved host binding");
        }
        for key in overrides.bindings.keys() {
            if !self.kv_namespaces.iter().any(|b| &b.binding == key) {
                bail!("unknown binding override {key}");
            }
        }
        for b in &self.kv_namespaces {
            if vars.contains_key(&b.binding) {
                bail!("variable conflicts with binding {}", b.binding);
            }
            let id = overrides.bindings.get(&b.binding).unwrap_or(&b.id);
            if id.is_empty() || id.len() > 512 {
                bail!("invalid binding namespace ID");
            }
        }
        let other_bindings = self
            .durable_objects
            .iter()
            .flat_map(|d| d.bindings.iter().map(|b| &b.name))
            .chain(
                self.unsafe_config
                    .iter()
                    .flat_map(|u| u.bindings.iter().map(|b| &b.name)),
            );
        if other_bindings
            .into_iter()
            .any(|name| vars.contains_key(name))
        {
            bail!("variable conflicts with host binding");
        }
        Ok(vars)
    }
    pub(super) fn supported(&self) -> Result<()> {
        let mut tags = HashSet::new();
        let mut classes = HashSet::new();
        for m in &self.migrations {
            if m.tag.is_empty() || !tags.insert(&m.tag) {
                bail!("duplicate or empty DO migration tag");
            }
            for class in &m.new_sqlite_classes {
                if class.is_empty() || !classes.insert(class) {
                    bail!("duplicate or empty DO class");
                }
            }
        }
        let mut names: HashSet<&str> = self
            .kv_namespaces
            .iter()
            .map(|b| b.binding.as_str())
            .collect();
        if let Some(durable) = &self.durable_objects {
            for binding in &durable.bindings {
                if !classes.contains(&binding.class_name) {
                    bail!("DO class lacks a migration");
                }
                if binding.name.is_empty()
                    || binding.name == "IDENTITY"
                    || !names.insert(&binding.name)
                    || self.vars.contains_key(&binding.name)
                {
                    bail!("duplicate DO binding");
                }
            }
        }
        for binding in self.unsafe_config.iter().flat_map(|u| &u.bindings) {
            if !matches!(binding.kind.as_str(), "ir" | "usage") {
                bail!("unsupported unsafe binding type");
            }
            if binding.name.is_empty()
                || binding.name == "IDENTITY"
                || !names.insert(&binding.name)
                || self.vars.contains_key(&binding.name)
            {
                bail!("duplicate binding");
            }
        }
        for cron in self.triggers.iter().flat_map(|t| &t.crons) {
            crate::scheduled::Cron::parse(cron)?;
        }
        Ok(())
    }
}
fn validate_date(date: &str) -> Result<()> {
    let b = date.as_bytes();
    if b.len() != 10
        || b[4] != b'-'
        || b[7] != b'-'
        || b.iter()
            .enumerate()
            .any(|(i, c)| i != 4 && i != 7 && !c.is_ascii_digit())
    {
        bail!("compatibility_date must be YYYY-MM-DD");
    }
    let y = date[..4].parse::<u32>()?;
    let m = date[5..7].parse::<u32>()?;
    let d = date[8..].parse::<u32>()?;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400)) {
                29
            } else {
                28
            }
        }
        _ => 0,
    };
    if y == 0 || d == 0 || d > days {
        bail!("invalid compatibility_date");
    }
    Ok(())
}
