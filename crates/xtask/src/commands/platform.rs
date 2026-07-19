use std::collections::BTreeSet;
use std::fs;

use rusttable_core::platform::{
    ApplicationMode, BackendPreference, CpuArchitecture, CpuFallbackPolicy, GraphicsBackend,
    LinuxLibc, OperatingSystem, OsVersion, PlatformRegistry, PlatformRequirement, SupportLevel,
    TargetTriple, WindowSystem, current_platform_identity, fallback_identity,
};
use serde::Deserialize;

use super::{Result as CommandResult, report};
use crate::cli::{PlatformCommand, PlatformVerifyArgs};
use crate::root::RepositoryRoot;

#[derive(Debug, Deserialize)]
struct PlatformFile {
    schema_version: u32,
    targets: Vec<PlatformTarget>,
}

#[derive(Debug, Deserialize)]
struct PlatformTarget {
    triple: String,
    os: String,
    architecture: String,
    minimum_os: String,
    libc: String,
    minimum_libc: Option<String>,
    desktop: bool,
    headless: bool,
    window_systems: Vec<String>,
    backends: Vec<String>,
    cpu_fallback: String,
    runner: String,
    package_target: String,
    support_level: String,
}

pub(super) fn run(root: &RepositoryRoot, command: &PlatformCommand) -> CommandResult {
    match command {
        PlatformCommand::Verify(arguments) => verify(root, arguments),
    }
}

#[allow(clippy::too_many_lines)]
fn verify(root: &RepositoryRoot, arguments: &PlatformVerifyArgs) -> CommandResult {
    let path = root.join("architecture/platform-support.toml");
    let source = fs::read_to_string(&path)
        .map_err(|error| format!("{}: cannot read matrix: {error}", path.display()))?;
    let file: PlatformFile = toml::from_str(&source)
        .map_err(|error| format!("{}: invalid matrix: {error}", path.display()))?;
    if file.schema_version != 1 {
        return Err(format!("{}: schema_version must be 1", path.display()));
    }
    let mut triples = BTreeSet::new();
    let mut runners = BTreeSet::new();
    let mut package_targets = BTreeSet::new();
    let mut requirements = Vec::with_capacity(file.targets.len());
    let mut target_receipts = Vec::with_capacity(file.targets.len());
    for target in &file.targets {
        if target.support_level != "supported" {
            return Err(format!(
                "{}: initial matrix entries must declare support_level = supported",
                target.triple
            ));
        }
        if !triples.insert(target.triple.clone()) {
            return Err(format!("platform matrix repeats target {}", target.triple));
        }
        if !runners.insert(target.runner.clone()) {
            return Err(format!("platform matrix repeats runner {}", target.runner));
        }
        if !package_targets.insert(target.package_target.clone()) {
            return Err(format!(
                "platform matrix repeats package target {}",
                target.package_target
            ));
        }
        if target.triple != target.package_target {
            return Err(format!(
                "{}: package_target must equal triple",
                target.triple
            ));
        }
        let requirement = to_requirement(target)?;
        target_receipts.push(serde_json::json!({
            "triple": target.triple,
            "os": target.os,
            "architecture": target.architecture,
            "minimum_os": target.minimum_os,
            "libc": target.libc,
            "minimum_libc": target.minimum_libc,
            "desktop": target.desktop,
            "headless": target.headless,
            "window_systems": target.window_systems,
            "backends": target.backends,
            "cpu_fallback": target.cpu_fallback,
            "runner": target.runner,
            "package_target": target.package_target,
            "support_level": target.support_level,
        }));
        requirements.push(requirement);
    }
    if arguments.all_targets && requirements.len() != 3 {
        return Err(format!(
            "platform matrix must contain three supported targets, found {}",
            requirements.len()
        ));
    }
    let registry = PlatformRegistry::new(requirements);
    verify_against_core_policy(&registry)?;
    let workflow = fs::read_to_string(root.join(".github/workflows/rust-main.yml"))
        .map_err(|error| format!("rust-main.yml: cannot read workflow: {error}"))?;
    if !workflow.contains("scripts/platform-support.ts") || !workflow.contains("fromJSON") {
        return Err(
            "rust-main.yml: portable target matrix is not derived from platform-support.ts"
                .to_owned(),
        );
    }
    let app_manifest = fs::read_to_string(root.join("crates/rusttable-app/Cargo.toml"))
        .map_err(|error| format!("rusttable-app/Cargo.toml: cannot read manifest: {error}"))?;
    if !app_manifest.contains("features = [\"wayland\", \"x11\"]") {
        return Err(
            "rusttable-app/Cargo.toml: Linux Iced policy must enable Wayland and X11".to_owned(),
        );
    }
    let runtime = if arguments.runtime_current {
        let identity = current_platform_identity().unwrap_or_else(fallback_identity);
        let decision = registry.evaluate(identity, ApplicationMode::Headless);
        Some(serde_json::json!({
            "level": support_level(decision.level()),
            "target": decision.identity().target().as_str(),
            "findings": decision.findings().iter().map(ToString::to_string).collect::<Vec<_>>(),
        }))
    } else {
        None
    };
    if arguments.verify_startup_preflight {
        let decision = registry.evaluate(fallback_identity(), ApplicationMode::Headless);
        if decision.level() != SupportLevel::UnsupportedPlatform {
            return Err(
                "startup preflight did not reject the synthetic unknown platform".to_owned(),
            );
        }
    }
    Ok(report(
        root,
        "platform.verify",
        serde_json::json!({
            "schema": "rusttable.platform-support-receipt.v1",
            "targets": target_receipts,
            "target_count": triples.len(),
            "runtime_current": runtime,
            "startup_preflight_verified": arguments.verify_startup_preflight,
            "workflow_matrix_source": "scripts/platform-support.ts",
        }),
    ))
}

fn to_requirement(target: &PlatformTarget) -> std::result::Result<PlatformRequirement, String> {
    PlatformRequirement::new(
        TargetTriple::new(target.triple.clone()).map_err(|error| error.to_string())?,
        parse_os(&target.os)?,
        parse_architecture(&target.architecture)?,
        Some(parse_version(&target.minimum_os)?),
        parse_libc(&target.libc)?,
        target
            .minimum_libc
            .as_deref()
            .map(parse_version)
            .transpose()?,
        target.desktop,
        target.headless,
        target
            .window_systems
            .iter()
            .map(|value| parse_window_system(value))
            .collect::<std::result::Result<Vec<_>, _>>()?,
        BackendPreference::new(
            target
                .backends
                .iter()
                .map(|value| parse_backend(value))
                .collect::<std::result::Result<Vec<_>, _>>()?,
        )
        .map_err(|error| error.to_string())?,
        parse_fallback(&target.cpu_fallback)?,
        target.runner.clone(),
        TargetTriple::new(target.package_target.clone()).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

fn verify_against_core_policy(registry: &PlatformRegistry) -> std::result::Result<(), String> {
    let expected = PlatformRegistry::default();
    if registry.requirements() != expected.requirements() {
        return Err("platform matrix differs from the core policy contract".to_owned());
    }
    Ok(())
}

fn parse_os(value: &str) -> std::result::Result<OperatingSystem, String> {
    match value {
        "linux" => Ok(OperatingSystem::Linux),
        "macos" => Ok(OperatingSystem::MacOs),
        "windows" => Ok(OperatingSystem::Windows),
        _ => Err(format!("unsupported OS {value}")),
    }
}
fn parse_architecture(value: &str) -> std::result::Result<CpuArchitecture, String> {
    match value {
        "x86_64" => Ok(CpuArchitecture::X86_64),
        "aarch64" => Ok(CpuArchitecture::Aarch64),
        _ => Err(format!("unsupported architecture {value}")),
    }
}
fn parse_libc(value: &str) -> std::result::Result<LinuxLibc, String> {
    match value {
        "gnu" => Ok(LinuxLibc::Gnu),
        "musl" => Ok(LinuxLibc::Musl),
        "unknown" => Ok(LinuxLibc::Unknown),
        "not-applicable" => Ok(LinuxLibc::NotApplicable),
        _ => Err(format!("unsupported libc {value}")),
    }
}
fn parse_window_system(value: &str) -> std::result::Result<WindowSystem, String> {
    match value {
        "wayland" => Ok(WindowSystem::Wayland),
        "x11" => Ok(WindowSystem::X11),
        _ => Err(format!("unsupported window system {value}")),
    }
}
fn parse_backend(value: &str) -> std::result::Result<GraphicsBackend, String> {
    match value {
        "vulkan" => Ok(GraphicsBackend::Vulkan),
        "opengl" => Ok(GraphicsBackend::OpenGl),
        "metal" => Ok(GraphicsBackend::Metal),
        "direct3d12" => Ok(GraphicsBackend::Direct3D12),
        _ => Err(format!("unsupported graphics backend {value}")),
    }
}
fn parse_fallback(value: &str) -> std::result::Result<CpuFallbackPolicy, String> {
    match value {
        "required" => Ok(CpuFallbackPolicy::Required),
        "forbidden" => Ok(CpuFallbackPolicy::Forbidden),
        _ => Err(format!("unsupported CPU fallback policy {value}")),
    }
}
fn parse_version(value: &str) -> std::result::Result<OsVersion, String> {
    OsVersion::parse(value).map_err(|error| format!("{value}: {error}"))
}
fn support_level(level: SupportLevel) -> &'static str {
    match level {
        SupportLevel::SupportedGpuCandidate => "supported-gpu-candidate",
        SupportLevel::SupportedCpuOnly => "supported-cpu-only",
        SupportLevel::UnsupportedPlatform => "unsupported-platform",
    }
}
