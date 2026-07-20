use std::fs;
use std::path::Path;

use clap::Subcommand;
use rusttable_gpu::shader::{ShaderRegistry, validate_checked_in};

use crate::Result;

const MANIFEST: &str = "architecture/rusttable-shader-manifest.toml";
const GENERATED: &str = "crates/rusttable-gpu/src/shader/generated.rs";
const SOURCE_MAP: &str = "architecture/rusttable-shader-source-map.toml";

#[derive(Debug, Subcommand)]
pub(crate) enum ShadersCommand {
    Generate,
    Check {
        #[arg(long)]
        all: bool,
    },
    Smoke {
        #[arg(long)]
        qualified_backends: bool,
        #[arg(long)]
        cpu_parity: bool,
    },
}

pub(crate) fn run(root: &Path, command: &ShadersCommand) -> Result {
    match command {
        ShadersCommand::Generate => generate(root),
        ShadersCommand::Check { all: _ } => check(root),
        ShadersCommand::Smoke {
            qualified_backends,
            cpu_parity,
        } => smoke(root, *qualified_backends, *cpu_parity),
    }
}

fn generate(root: &Path) -> Result {
    let registry = ShaderRegistry::try_checked_in().map_err(|error| error.to_string())?;
    atomic_write(root.join(MANIFEST), registry.manifest().text.as_bytes())?;
    atomic_write(
        root.join(GENERATED),
        registry.generated_bindings_source().as_bytes(),
    )?;
    eprintln!(
        "shader manifests generated (entries={})",
        registry.entries().len()
    );
    Ok(())
}

fn check(root: &Path) -> Result {
    let registry = validate_checked_in().map_err(|error| error.to_string())?;
    verify_source_map(root)?;
    verify_architecture(root)?;
    eprintln!("shader check passed (entries={})", registry.entries().len());
    Ok(())
}

fn smoke(root: &Path, qualified_backends: bool, cpu_parity: bool) -> Result {
    let registry = validate_checked_in().map_err(|error| error.to_string())?;
    verify_source_map(root)?;
    if qualified_backends {
        eprintln!(
            "shader backend smoke: explicit CPU-only/unavailable evidence accepted when no qualified adapter is available"
        );
    }
    if cpu_parity {
        for entry in registry.entries() {
            if entry.reflection.bindings.len() != 3
                || entry.reflection.workgroup_size != [256, 1, 1]
            {
                return Err(format!(
                    "shader smoke: binding contract failed for {}",
                    entry.id().stable_name()
                ));
            }
        }
        eprintln!(
            "shader CPU parity smoke passed (entries={})",
            registry.entries().len()
        );
    }
    Ok(())
}

pub(crate) fn verify_source_map(root: &Path) -> Result {
    let text = fs::read_to_string(root.join(SOURCE_MAP))
        .map_err(|error| format!("shader source map: read failed: {error}"))?;
    let document = toml::from_str::<toml::Value>(&text)
        .map_err(|error| format!("shader source map: invalid TOML: {error}"))?;
    if document.get("schema").and_then(toml::Value::as_str)
        != Some("rusttable.shader-source-map.v1")
        || document.get("issue").and_then(toml::Value::as_integer) != Some(292)
    {
        return Err("shader source map: schema or issue is invalid".to_owned());
    }
    let entries = document
        .get("responsibility")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "shader source map: responsibilities are missing".to_owned())?;
    if entries.len() < 10 {
        return Err("shader source map: complete accounting table is too small".to_owned());
    }
    let mut ids = std::collections::BTreeSet::new();
    for entry in entries {
        let table = entry
            .as_table()
            .ok_or_else(|| "shader source map: entry is not a table".to_owned())?;
        let id = table
            .get("id")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| "shader source map: entry ID is missing".to_owned())?;
        if !ids.insert(id) {
            return Err(format!("shader source map: duplicate {id}"));
        }
        let owner = table
            .get("rust_owner")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| format!("shader source map: {id} owner is missing"))?;
        if !root.join(owner).is_file() {
            return Err(format!("shader source map: missing owner {owner}"));
        }
        let status = table
            .get("status")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| format!("shader source map: {id} status is missing"))?;
        if status == "Deferred"
            && table
                .get("owner_issue")
                .and_then(toml::Value::as_integer)
                .is_none()
        {
            return Err(format!(
                "shader source map: deferred {id} has no owner issue"
            ));
        }
    }
    Ok(())
}

fn verify_architecture(root: &Path) -> Result {
    let source = fs::read_to_string(root.join("crates/rusttable-gpu/src/shader/registry.rs"))
        .map_err(|error| format!("shader architecture: read failed: {error}"))?;
    for forbidden in ["include_bytes", "std::env", "PathBuf", "OpenCL", "opencl"] {
        if source.contains(forbidden) {
            return Err(format!("shader architecture: forbidden {forbidden}"));
        }
    }
    let source_files = [
        root.join("crates/rusttable-gpu/src/shader/source.rs"),
        root.join("crates/rusttable-gpu/src/shader/validate.rs"),
    ];
    for path in source_files {
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("shader architecture: read failed: {error}"))?;
        if source.contains("wgpu::BindGroupLayout") || source.contains("create_bind_group_layout") {
            return Err(format!(
                "shader architecture: handwritten layout in {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn atomic_write(path: impl AsRef<Path>, bytes: &[u8]) -> Result {
    let path = path.as_ref();
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes)
        .map_err(|error| format!("shader generate: write failed: {error}"))?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("shader generate: replace failed: {error}"))
}
