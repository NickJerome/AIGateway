use anyhow::{Result, anyhow};
use rquickjs::{Context, Module, Runtime, WriteOptions};
use sha2::{Digest, Sha256};

pub const BUILD_ID: &str = env!("GATEWAY_BUILD_ID");

/// Immutable bytecode created by this compiler or the validating store.
/// Public callers cannot construct an artifact from unchecked bytes.
/// ```compile_fail
/// let code: gateway::bytecode::CompiledArtifact = vec![0u8; 32].into();
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledArtifact(std::sync::Arc<[u8]>);
impl CompiledArtifact {
    pub(crate) fn validated(bytes: Vec<u8>) -> Self {
        Self(bytes.into())
    }
}
impl std::ops::Deref for CompiledArtifact {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}
impl AsRef<[u8]> for CompiledArtifact {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct Artifact {
    pub code: CompiledArtifact,
    pub source_sha256: String,
    pub build_id: String,
}

pub fn source_sha256(source: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn compile(name: &str, source: &str) -> Result<Artifact> {
    let rt = Runtime::new()?;
    let ctx = Context::full(&rt)?;
    let code = ctx.with(|ctx| -> Result<Vec<u8>> {
        declare_builtin(&ctx)?;
        let module =
            Module::declare(ctx.clone(), name, source).map_err(|e| describe(&ctx, name, e))?;
        module
            .write(WriteOptions::default())
            .map_err(|e| describe(&ctx, name, e))
    })?;
    Ok(Artifact {
        code: CompiledArtifact::validated(code),
        source_sha256: source_sha256(source),
        build_id: BUILD_ID.to_string(),
    })
}

fn describe(ctx: &rquickjs::Ctx<'_>, name: &str, err: rquickjs::Error) -> anyhow::Error {
    if matches!(err, rquickjs::Error::Exception) {
        let caught = ctx.catch();
        let text = caught
            .as_exception()
            .and_then(|e| e.message())
            .unwrap_or_else(|| "unknown exception".to_string());
        anyhow!("compile {name}: {text}")
    } else {
        anyhow!("compile {name}: {err}")
    }
}

pub(crate) fn declare_builtin(ctx: &rquickjs::Ctx<'_>) -> Result<()> {
    Module::declare(
        ctx.clone(),
        "cloudflare:workers",
        "export class DurableObject {constructor(ctx,env){this.ctx=ctx;this.env=env;}}",
    )?;
    Ok(())
}

/// Link named imports against a module that throws before plugin statements.
/// Missing exports are link errors; the intentional null exception is success.
pub fn validate_exports(source: &str, required: &[String]) -> Result<()> {
    if required.is_empty() {
        return Ok(());
    }
    let runtime = Runtime::new()?;
    runtime.set_memory_limit(32 << 20);
    let context = Context::full(&runtime)?;
    context.with(|ctx| -> Result<()> {
        declare_builtin(&ctx)?;
        let _source = Module::declare(
            ctx.clone(),
            "plugin-source",
            format!("throw null;\n{source}"),
        )?;
        let imports = required
            .iter()
            .enumerate()
            .map(|(i, name)| format!("{} as binding{i}", serde_json::to_string(name).unwrap()))
            .collect::<Vec<_>>()
            .join(",");
        let probe = Module::declare(
            ctx.clone(),
            "export-validation",
            format!("import {{{imports}}} from 'plugin-source';"),
        )?;
        match probe.eval() {
            Ok((_, promise)) => {
                let _ = promise.result::<()>();
            }
            Err(rquickjs::Error::Exception) => {}
            Err(error) => return Err(error.into()),
        }
        let exception = ctx.catch();
        if exception.is_null() || exception.is_undefined() {
            return Ok(());
        }
        let message = exception
            .as_exception()
            .and_then(|e| e.message())
            .unwrap_or_else(|| "invalid Durable Object exports".into());
        anyhow::bail!("{message}")
    })
}
