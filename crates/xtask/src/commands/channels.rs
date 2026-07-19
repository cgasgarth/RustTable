use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{Result, report};
use crate::cli::{ChannelRefreshArgs, ChannelVerifyArgs, ChannelsCommand};
use crate::process::{EnvironmentProfile, ProcessLimits, ProcessRequest, ProcessRunner};
use crate::root::RepositoryRoot;

const CHANNELS_PATH: &str = "quality/rust-channels.toml";
const LABS_PATH: &str = "quality/nightly-labs.toml";
const EXCEPTIONS_PATH: &str = "quality/compiler-channel-exceptions.toml";
const FIXTURES_PATH: &str = "quality/compiler-channel-fixtures.toml";
const PRIMARY_TOOLCHAIN_PATH: &str = "rust-toolchain.toml";
const LOCKFILE_PATH: &str = "Cargo.lock";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(45);
const OUTPUT_LIMIT: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ChannelId {
    PrimaryBeta,
    RollingBeta,
    PreviousStable,
    CurrentNightly,
    PinnedNightlyLab,
}

impl ChannelId {
    const ALL: [Self; 5] = [
        Self::PrimaryBeta,
        Self::RollingBeta,
        Self::PreviousStable,
        Self::CurrentNightly,
        Self::PinnedNightlyLab,
    ];

    fn parse(value: &str) -> Result<Self> {
        match value {
            "primary-beta" => Ok(Self::PrimaryBeta),
            "rolling-beta" => Ok(Self::RollingBeta),
            "previous-stable" => Ok(Self::PreviousStable),
            "current-nightly" => Ok(Self::CurrentNightly),
            "pinned-nightly-lab" => Ok(Self::PinnedNightlyLab),
            other => Err(format!("unknown compiler channel {other}")),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::PrimaryBeta => "primary-beta",
            Self::RollingBeta => "rolling-beta",
            Self::PreviousStable => "previous-stable",
            Self::CurrentNightly => "current-nightly",
            Self::PinnedNightlyLab => "pinned-nightly-lab",
        }
    }
}

#[derive(Debug, Deserialize)]
struct ChannelPolicy {
    schema: String,
    authoritative: String,
    product_artifacts: Vec<String>,
    channels: Vec<ChannelSpec>,
}

#[derive(Debug, Deserialize)]
struct ChannelSpec {
    id: String,
    source: String,
    toolchain: String,
    archive: String,
    release_line: String,
    stabilization_date: String,
    gating: String,
    update_cadence: String,
    moving: bool,
    required_components: Vec<String>,
    targets: Vec<String>,
    commands: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct LabPolicy {
    schema: String,
    toolchain: String,
    labs: Vec<LabSpec>,
}

#[derive(Debug, Deserialize)]
struct LabSpec {
    id: String,
    owner_issue: u64,
    owner: String,
    feature_gates: Vec<String>,
    allowed_unstable_flags: Vec<String>,
    package: String,
    target: String,
    commands: Vec<String>,
    outputs: Vec<String>,
    platforms: Vec<String>,
    resource_timeout_seconds: u64,
    resource_memory_mb: u64,
    #[serde(default)]
    expected_exclusions: Vec<String>,
    #[serde(default)]
    artifact_retention_days: u32,
    promotion_condition: String,
    removal_condition: String,
    forbidden_product_edges: Vec<String>,
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct ExceptionPolicy {
    schema: String,
    version: u32,
    policy: ExceptionRules,
    #[serde(default)]
    exceptions: Vec<ExceptionSpec>,
}

#[derive(Debug, Deserialize)]
struct ExceptionRules {
    required_fields: Vec<String>,
    rolling_beta_max_days: u32,
    current_nightly_max_days: u32,
    pinned_lab_allowed: bool,
    unused_is_error: bool,
    fingerprint_mismatch_is_error: bool,
}

#[derive(Debug, Deserialize)]
struct ExceptionSpec {
    id: String,
    channel: String,
    fingerprint: String,
    command: String,
    reproducer: String,
    owner: String,
    first_seen: String,
    platforms: Vec<String>,
    risk: String,
    expires: String,
}

#[derive(Debug, Deserialize)]
struct FixturePolicy {
    schema: String,
    cases: Vec<FixtureCase>,
}

#[derive(Debug, Deserialize)]
struct FixtureCase {
    id: String,
    channel: String,
    expected: String,
}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoPackage>,
    workspace_members: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CargoPackage {
    id: String,
    name: String,
    manifest_path: String,
    #[serde(default)]
    features: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Serialize)]
struct ChannelReceipt {
    schema: &'static str,
    channel: String,
    toolchain: String,
    source: String,
    gating: String,
    status: String,
    compiler: CompilerFingerprint,
    components: Vec<String>,
    installed_components: Vec<String>,
    targets: Vec<String>,
    installed_targets: Vec<String>,
    dist_manifest_hash: String,
    commands: Vec<String>,
    artifact_allowed: bool,
    graph_isolated: bool,
    exception_count: usize,
}

#[derive(Debug, Serialize)]
struct CompilerFingerprint {
    rustc: String,
    release: String,
    commit_hash: String,
    commit_date: String,
    host: String,
    llvm_version: String,
    cargo: String,
    rustfmt: String,
    clippy: String,
}

#[derive(Debug, Serialize)]
struct RefreshReceipt {
    schema: &'static str,
    authoritative_channel: String,
    primary_toolchain: String,
    proposed_primary_toolchain: Option<String>,
    moving_channels: Vec<RefreshChannel>,
    release_sources: Vec<String>,
    applied: bool,
    rust_toolchain_unchanged: bool,
    cargo_lock_unchanged: bool,
}

#[derive(Debug, Serialize)]
struct RefreshChannel {
    channel: String,
    toolchain: String,
    status: String,
    fingerprint: Option<CompilerFingerprint>,
    finding_class: Option<String>,
}

pub(super) fn run(
    root: &RepositoryRoot,
    command: &ChannelsCommand,
    runner: &ProcessRunner,
) -> Result {
    match command {
        ChannelsCommand::Verify(arguments) => verify(root, arguments, runner),
        ChannelsCommand::Refresh(arguments) => refresh(root, arguments, runner),
    }
}

fn verify(root: &RepositoryRoot, arguments: &ChannelVerifyArgs, runner: &ProcessRunner) -> Result {
    let policy = Policy::load(root)?;
    policy.validate(root)?;
    let selected = selected_channels(&arguments.channels)?;
    if arguments.artifact && selected != [ChannelId::PrimaryBeta].into_iter().collect() {
        return Err("package provenance: --artifact requires only primary-beta".to_owned());
    }
    if env::var_os("RUSTC_BOOTSTRAP").is_some() {
        return Err("package provenance: RUSTC_BOOTSTRAP is prohibited".to_owned());
    }

    let mut receipts = Vec::new();
    for channel in selected {
        let spec = policy.channel(channel)?;
        let fingerprint = capture_fingerprint(root, spec, runner)?;
        validate_fingerprint(spec, &fingerprint)?;
        let (installed_components, installed_targets) =
            installed_toolchain(root, spec, runner, &fingerprint.host)?;
        let metadata = cargo_metadata(root, spec, runner)?;
        let graph_isolated = guard_product_graph(&metadata, &policy.labs)?;
        if arguments.artifact && channel != ChannelId::PrimaryBeta {
            return Err(format!(
                "package provenance: {} cannot produce artifacts",
                channel.as_str()
            ));
        }
        let artifact_allowed = channel == ChannelId::PrimaryBeta && arguments.artifact;
        let commands = commands_for(spec, channel, &policy.labs)?;
        let receipt = ChannelReceipt {
            schema: "rusttable.compiler-channel-receipt.v1",
            channel: channel.as_str().to_owned(),
            toolchain: spec.toolchain.clone(),
            source: spec.source.clone(),
            gating: spec.gating.clone(),
            status: "ok".to_owned(),
            compiler: fingerprint,
            components: spec.required_components.clone(),
            installed_components,
            targets: spec.targets.clone(),
            installed_targets,
            dist_manifest_hash: digest(format!("{}\n{}", spec.archive, spec.toolchain).as_bytes()),
            commands,
            artifact_allowed,
            graph_isolated,
            exception_count: policy.exceptions.exceptions.len(),
        };
        receipts.push(receipt);
    }
    let data = serde_json::json!({
        "schema": "rusttable.compiler-channel-verification.v1",
        "channels": receipts,
        "fixtures": policy.fixtures.cases.len(),
        "receipt_policy": "paths and environment values are excluded",
    });
    write_receipt(arguments.receipt.as_deref(), &data)?;
    Ok(report(root, "ecosystem.channels.verify", data))
}

fn refresh(
    root: &RepositoryRoot,
    arguments: &ChannelRefreshArgs,
    runner: &ProcessRunner,
) -> Result {
    let policy = Policy::load(root)?;
    policy.validate(root)?;
    let toolchain_before = fs::read(root.join(PRIMARY_TOOLCHAIN_PATH))
        .map_err(|error| format!("{PRIMARY_TOOLCHAIN_PATH}: {error}"))?;
    let lock_before =
        fs::read(root.join(LOCKFILE_PATH)).map_err(|error| format!("{LOCKFILE_PATH}: {error}"))?;
    let primary = policy.channel(ChannelId::PrimaryBeta)?;
    let mut moving_channels = Vec::new();
    for channel in [ChannelId::RollingBeta, ChannelId::CurrentNightly] {
        let spec = policy.channel(channel)?;
        let result = capture_fingerprint(root, spec, runner);
        match result {
            Ok(fingerprint) => moving_channels.push(RefreshChannel {
                channel: channel.as_str().to_owned(),
                toolchain: spec.toolchain.clone(),
                status: "resolved".to_owned(),
                fingerprint: Some(fingerprint),
                finding_class: None,
            }),
            Err(error) if is_channel_unavailable(&error) => moving_channels.push(RefreshChannel {
                channel: channel.as_str().to_owned(),
                toolchain: spec.toolchain.clone(),
                status: "channel-unavailable".to_owned(),
                fingerprint: None,
                finding_class: Some("ChannelUnavailable".to_owned()),
            }),
            Err(error) => return Err(error),
        }
    }
    let data = serde_json::json!(RefreshReceipt {
        schema: "rusttable.compiler-channel-refresh.v1",
        authoritative_channel: policy.channels.authoritative.clone(),
        primary_toolchain: primary.toolchain.clone(),
        proposed_primary_toolchain: None,
        moving_channels,
        release_sources: policy
            .channels
            .channels
            .iter()
            .filter(|channel| channel.moving)
            .map(|channel| channel.archive.clone())
            .collect(),
        applied: false,
        rust_toolchain_unchanged: toolchain_before
            == fs::read(root.join(PRIMARY_TOOLCHAIN_PATH)).unwrap_or_default(),
        cargo_lock_unchanged: lock_before == fs::read(root.join(LOCKFILE_PATH)).unwrap_or_default(),
    });
    write_receipt(arguments.receipt.as_deref(), &data)?;
    Ok(report(root, "ecosystem.channels.refresh", data))
}

struct Policy {
    channels: ChannelPolicy,
    labs: LabPolicy,
    exceptions: ExceptionPolicy,
    fixtures: FixturePolicy,
}

impl Policy {
    fn load(root: &RepositoryRoot) -> Result<Self> {
        Ok(Self {
            channels: parse_toml(root, CHANNELS_PATH)?,
            labs: parse_toml(root, LABS_PATH)?,
            exceptions: parse_toml(root, EXCEPTIONS_PATH)?,
            fixtures: parse_toml(root, FIXTURES_PATH)?,
        })
    }

    // Keep the cross-file policy invariants in one bounded validation pass.
    #[allow(clippy::too_many_lines)]
    fn validate(&self, root: &RepositoryRoot) -> Result<()> {
        if self.channels.schema != "rusttable.rust-channels.v1"
            || self.channels.authoritative != "primary-beta"
            || self.channels.product_artifacts != ["primary-beta"]
        {
            return Err(format!(
                "{CHANNELS_PATH}: invalid authoritative channel policy"
            ));
        }
        let expected = ChannelId::ALL
            .iter()
            .map(|channel| channel.as_str())
            .collect::<BTreeSet<_>>();
        let actual = self
            .channels
            .channels
            .iter()
            .map(|channel| channel.id.as_str())
            .collect::<BTreeSet<_>>();
        if actual != expected {
            return Err(format!(
                "{CHANNELS_PATH}: must declare exactly five compiler identities"
            ));
        }
        for channel in &self.channels.channels {
            if channel.id.is_empty()
                || channel.toolchain.is_empty()
                || channel.archive.is_empty()
                || channel.release_line.is_empty()
                || channel.stabilization_date.len() != 10
                || channel.gating.is_empty()
                || channel.update_cadence.is_empty()
                || channel.required_components.is_empty()
                || channel.targets.is_empty()
                || channel.commands.is_empty()
            {
                return Err(format!(
                    "{CHANNELS_PATH}: channel {} has missing fields",
                    channel.id
                ));
            }
            if channel.moving != (channel.source == "moving-channel") {
                return Err(format!(
                    "{CHANNELS_PATH}: {} has inconsistent moving source",
                    channel.id
                ));
            }
            if channel.id == "primary-beta" && channel.toolchain != primary_toolchain(root)? {
                return Err(format!(
                    "{CHANNELS_PATH}: primary-beta must match rust-toolchain.toml"
                ));
            }
            if channel.id == "pinned-nightly-lab" && channel.gating != "lab-only" {
                return Err(format!(
                    "{CHANNELS_PATH}: pinned-nightly-lab must be lab-only"
                ));
            }
        }
        if self.labs.schema != "rusttable.nightly-labs.v1"
            || self.labs.toolchain != "pinned-nightly-lab"
        {
            return Err(format!("{LABS_PATH}: invalid lab policy"));
        }
        let mut lab_ids = BTreeSet::new();
        for lab in &self.labs.labs {
            if lab.id.is_empty()
                || !lab_ids.insert(&lab.id)
                || lab.owner_issue == 0
                || lab.owner.is_empty()
                || lab.feature_gates.iter().any(String::is_empty)
                || lab.allowed_unstable_flags.iter().any(String::is_empty)
                || lab.package.is_empty()
                || lab.target.is_empty()
                || lab.commands.is_empty()
                || lab.outputs.is_empty()
                || lab.platforms.is_empty()
                || lab.resource_timeout_seconds == 0
                || lab.resource_memory_mb == 0
                || lab.expected_exclusions.is_empty()
                || lab.artifact_retention_days == 0
                || lab.artifact_retention_days > 30
                || lab.promotion_condition.is_empty()
                || lab.removal_condition.is_empty()
                || lab.forbidden_product_edges.is_empty()
            {
                return Err(format!("{LABS_PATH}: lab has missing or duplicate fields"));
            }
            if lab
                .commands
                .iter()
                .any(|command| command.contains("RUSTC_BOOTSTRAP"))
            {
                return Err(format!(
                    "{LABS_PATH}: lab {} contains a forbidden command",
                    lab.id
                ));
            }
        }
        if self.exceptions.schema != "rusttable.compiler-channel-exceptions.v1"
            || self.exceptions.version != 1
            || self.exceptions.policy.pinned_lab_allowed
            || self.exceptions.policy.rolling_beta_max_days != 7
            || self.exceptions.policy.current_nightly_max_days != 30
            || !self.exceptions.policy.unused_is_error
            || !self.exceptions.policy.fingerprint_mismatch_is_error
        {
            return Err(format!("{EXCEPTIONS_PATH}: invalid exception policy"));
        }
        let required_exception_fields = [
            "id",
            "channel",
            "fingerprint",
            "command",
            "reproducer",
            "owner",
            "first_seen",
            "platforms",
            "risk",
            "expires",
        ];
        if self.exceptions.policy.required_fields
            != required_exception_fields
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        {
            return Err(format!(
                "{EXCEPTIONS_PATH}: required exception fields drifted"
            ));
        }
        validate_exceptions(&self.exceptions.exceptions)?;
        if self.fixtures.schema != "rusttable.compiler-channel-fixtures.v1"
            || self.fixtures.cases.len() < 14
        {
            return Err(format!(
                "{FIXTURES_PATH}: required failure matrix is incomplete"
            ));
        }
        let mut fixture_ids = BTreeSet::new();
        for fixture in &self.fixtures.cases {
            if fixture.id.is_empty()
                || !fixture_ids.insert(&fixture.id)
                || ChannelId::parse(&fixture.channel).is_err()
                || fixture.expected.is_empty()
            {
                return Err(format!("{FIXTURES_PATH}: invalid fixture {}", fixture.id));
            }
        }
        Ok(())
    }

    fn channel(&self, id: ChannelId) -> Result<&ChannelSpec> {
        self.channels
            .channels
            .iter()
            .find(|channel| channel.id == id.as_str())
            .ok_or_else(|| format!("{CHANNELS_PATH}: missing {}", id.as_str()))
    }
}

fn parse_toml<T: for<'de> Deserialize<'de>>(root: &RepositoryRoot, path: &str) -> Result<T> {
    let full_path = root.join(path);
    let source = fs::read_to_string(&full_path).map_err(|error| format!("{path}: {error}"))?;
    toml::from_str(&source).map_err(|error| format!("{path}: invalid TOML: {error}"))
}

fn selected_channels(values: &[String]) -> Result<BTreeSet<ChannelId>> {
    if values.is_empty() {
        return Ok([ChannelId::PrimaryBeta].into_iter().collect());
    }
    values.iter().map(|value| ChannelId::parse(value)).collect()
}

fn primary_toolchain(root: &RepositoryRoot) -> Result<String> {
    let source = fs::read_to_string(root.join(PRIMARY_TOOLCHAIN_PATH))
        .map_err(|error| format!("{PRIMARY_TOOLCHAIN_PATH}: {error}"))?;
    let channel = source.lines().find_map(|line| {
        let trimmed = line.trim();
        trimmed.strip_prefix("channel = \"")?.strip_suffix('"')
    });
    channel
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{PRIMARY_TOOLCHAIN_PATH}: channel is missing"))
}

fn capture_fingerprint(
    root: &RepositoryRoot,
    spec: &ChannelSpec,
    runner: &ProcessRunner,
) -> Result<CompilerFingerprint> {
    let rustc = rustup(runner, root, &spec.toolchain, ["rustc", "-vV"])?;
    let cargo = rustup(runner, root, &spec.toolchain, ["cargo", "-V"])?;
    let rustfmt = rustup(runner, root, &spec.toolchain, ["rustfmt", "--version"])?;
    let clippy = rustup(runner, root, &spec.toolchain, ["cargo", "clippy", "-V"])?;
    let fields = version_fields(&rustc);
    Ok(CompilerFingerprint {
        rustc: rustc.trim().to_owned(),
        release: field(&fields, "release")?,
        commit_hash: field(&fields, "commit-hash")?,
        commit_date: field(&fields, "commit-date")?,
        host: field(&fields, "host")?,
        llvm_version: field(&fields, "LLVM version")?,
        cargo: cargo.trim().to_owned(),
        rustfmt: rustfmt.trim().to_owned(),
        clippy: clippy.trim().to_owned(),
    })
}

fn installed_toolchain(
    root: &RepositoryRoot,
    spec: &ChannelSpec,
    runner: &ProcessRunner,
    host: &str,
) -> Result<(Vec<String>, Vec<String>)> {
    let components = rustup_control(
        runner,
        root,
        [
            "component",
            "list",
            "--installed",
            "--toolchain",
            spec.toolchain.as_str(),
        ],
    )?
    .lines()
    .map(str::trim)
    .filter(|line| !line.is_empty())
    .map(ToOwned::to_owned)
    .collect::<Vec<_>>();
    for required in &spec.required_components {
        let installed_name = if required == "llvm-tools-preview" {
            "llvm-tools"
        } else {
            required.as_str()
        };
        if !components.iter().any(|component| {
            component == installed_name || component.starts_with(&format!("{installed_name}-"))
        }) {
            return Err(format!(
                "{}: missing required component {}",
                spec.id, required
            ));
        }
    }
    let targets = rustup_control(
        runner,
        root,
        [
            "target",
            "list",
            "--installed",
            "--toolchain",
            spec.toolchain.as_str(),
        ],
    )?
    .lines()
    .map(str::trim)
    .filter(|line| !line.is_empty())
    .map(ToOwned::to_owned)
    .collect::<Vec<_>>();
    if !targets.iter().any(|target| target == host) {
        return Err(format!(
            "{}: host target {} is not installed",
            spec.id, host
        ));
    }
    Ok((components, targets))
}

fn rustup<const N: usize>(
    runner: &ProcessRunner,
    root: &RepositoryRoot,
    toolchain: &str,
    command: [&str; N],
) -> Result<String> {
    let mut args = vec!["run", toolchain];
    args.extend(command);
    rustup_control(runner, root, args)
        .map_err(|error| format!("ChannelUnavailable: {toolchain}: {error}"))
}

fn rustup_control<I, S>(runner: &ProcessRunner, root: &RepositoryRoot, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let request = ProcessRequest::new("rustup", args)
        .profile(EnvironmentProfile::RustTool)
        .current_dir(root.path())
        .limits(ProcessLimits {
            max_stdout_bytes: OUTPUT_LIMIT,
            max_stderr_bytes: OUTPUT_LIMIT,
            timeout: Some(COMMAND_TIMEOUT),
        });
    let result = runner.run(request).map_err(|error| error.to_string())?;
    if !result.receipt.success() {
        let error = String::from_utf8_lossy(&result.stderr).trim().to_owned();
        return Err(format!("rustup command failed: {error}"));
    }
    Ok(String::from_utf8_lossy(&result.stdout).into_owned())
}

fn cargo_metadata(
    root: &RepositoryRoot,
    spec: &ChannelSpec,
    runner: &ProcessRunner,
) -> Result<CargoMetadata> {
    let output = rustup(
        runner,
        root,
        &spec.toolchain,
        [
            "cargo",
            "metadata",
            "--locked",
            "--no-deps",
            "--format-version",
            "1",
        ],
    )?;
    serde_json::from_str(&output).map_err(|error| format!("{} cargo metadata: {error}", spec.id))
}

fn validate_fingerprint(spec: &ChannelSpec, fingerprint: &CompilerFingerprint) -> Result<()> {
    if !fingerprint.release.starts_with(&spec.release_line) {
        return Err(format!(
            "{}: compiler release {} is outside {}",
            spec.id, fingerprint.release, spec.release_line
        ));
    }
    if fingerprint.commit_hash.is_empty()
        || fingerprint.commit_date.is_empty()
        || fingerprint.llvm_version.is_empty()
    {
        return Err(format!("{}: incomplete rustc -vV fingerprint", spec.id));
    }
    Ok(())
}

fn guard_product_graph(metadata: &CargoMetadata, labs: &LabPolicy) -> Result<bool> {
    let workspace_ids = metadata.workspace_members.iter().collect::<BTreeSet<_>>();
    for package in &metadata.packages {
        if !workspace_ids.contains(&package.id) || package.name == "xtask" {
            continue;
        }
        let lower = package.name.to_ascii_lowercase();
        if lower.contains("nightly") || lower.contains("portable_simd") || lower.contains("lab") {
            return Err(format!(
                "product graph: lab-like package {} is reachable",
                package.name
            ));
        }
        if package.features.keys().any(|feature| {
            let lower = feature.to_ascii_lowercase();
            lower.contains("nightly")
                || lower.contains("unstable")
                || lower.contains("portable_simd")
        }) {
            return Err(format!(
                "product graph: unstable feature is reachable from {}",
                package.name
            ));
        }
        let manifest = fs::read_to_string(&package.manifest_path)
            .map_err(|error| format!("product graph: {}: {error}", package.name))?;
        if manifest.contains("RUSTC_BOOTSTRAP")
            || manifest.contains("-Z")
            || manifest.contains("#![feature(")
        {
            return Err(format!(
                "product graph: unstable compiler setting in {}",
                package.name
            ));
        }
        let source_root = Path::new(&package.manifest_path)
            .parent()
            .map(|path| path.join("src"));
        if let Some(source_root) = source_root.filter(|path| path.is_dir()) {
            scan_product_source(&source_root, &package.name)?;
        }
    }
    if labs
        .labs
        .iter()
        .any(|lab| lab.enabled && workspace_ids.iter().any(|id| id.contains(&lab.package)))
    {
        return Err("product graph: enabled lab package is a workspace member".to_owned());
    }
    Ok(true)
}

fn scan_product_source(path: &Path, package: &str) -> Result<()> {
    let entries =
        fs::read_dir(path).map_err(|error| format!("product graph: {package}: {error}"))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("product graph: {package}: {error}"))?;
        let entry_path = entry.path();
        if entry_path.is_dir() {
            scan_product_source(&entry_path, package)?;
        } else if entry_path
            .extension()
            .is_some_and(|extension| extension == "rs")
        {
            let source = fs::read_to_string(&entry_path)
                .map_err(|error| format!("product graph: {}: {error}", entry_path.display()))?;
            if source.contains("#![feature(")
                || source.contains("RUSTC_BOOTSTRAP")
                || source.contains("-Z")
            {
                return Err(format!(
                    "product graph: unstable setting in package {package}"
                ));
            }
        }
    }
    Ok(())
}

fn commands_for(spec: &ChannelSpec, channel: ChannelId, labs: &LabPolicy) -> Result<Vec<String>> {
    if channel != ChannelId::PinnedNightlyLab {
        if spec
            .commands
            .iter()
            .any(|command| command.contains("-Z") || command.contains("RUSTC_BOOTSTRAP"))
        {
            return Err(format!("{}: unstable command is not registered", spec.id));
        }
        return Ok(spec.commands.clone());
    }
    let mut commands = Vec::new();
    for lab in labs.labs.iter().filter(|lab| lab.enabled) {
        for command in &lab.commands {
            let has_unregistered_flag = command.contains("-Z")
                && lab
                    .allowed_unstable_flags
                    .iter()
                    .all(|flag| !command.contains(flag));
            if command.contains("RUSTC_BOOTSTRAP") || has_unregistered_flag {
                return Err(format!("{}: lab command is not registered safely", lab.id));
            }
            commands.push(command.clone());
        }
    }
    Ok(commands)
}

fn version_fields(output: &str) -> BTreeMap<String, String> {
    output
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect::<BTreeMap<_, _>>()
}

fn field(fields: &BTreeMap<String, String>, name: &str) -> Result<String> {
    fields
        .get(name)
        .cloned()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("rustc -vV: missing {name}"))
}

fn validate_exceptions(exceptions: &[ExceptionSpec]) -> Result<()> {
    let mut ids = BTreeSet::new();
    for exception in exceptions {
        if !ids.insert(&exception.id)
            || exception.channel == "pinned-nightly-lab"
            || exception.fingerprint.is_empty()
            || exception.command.is_empty()
            || exception.reproducer.is_empty()
            || exception.owner.is_empty()
            || exception.first_seen.len() != 10
            || exception.expires.len() != 10
            || exception.platforms.is_empty()
            || exception.risk.is_empty()
        {
            return Err(format!(
                "{EXCEPTIONS_PATH}: invalid exception {}",
                exception.id
            ));
        }
        if exception.expires <= exception.first_seen {
            return Err(format!(
                "{EXCEPTIONS_PATH}: exception {} has invalid expiry",
                exception.id
            ));
        }
    }
    Ok(())
}

fn is_channel_unavailable(error: &str) -> bool {
    error.starts_with("ChannelUnavailable:")
}

fn write_receipt(path: Option<&Path>, value: &serde_json::Value) -> Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("receipt {}: {error}", path.display()))?;
    }
    let serialized =
        serde_json::to_vec_pretty(value).map_err(|error| format!("receipt: {error}"))?;
    fs::write(path, serialized).map_err(|error| format!("receipt {}: {error}", path.display()))
}

fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::{ChannelId, selected_channels, version_fields};

    #[test]
    fn selects_primary_by_default() {
        assert_eq!(
            selected_channels(&[]).expect("selection"),
            [ChannelId::PrimaryBeta].into_iter().collect()
        );
    }

    #[test]
    fn accepts_multiple_named_channels() {
        let selected =
            selected_channels(&["rolling-beta".to_owned(), "current-nightly".to_owned()])
                .expect("selection");
        assert!(selected.contains(&ChannelId::RollingBeta));
        assert!(selected.contains(&ChannelId::CurrentNightly));
    }

    #[test]
    fn parses_complete_rustc_fingerprint_fields() {
        let fields = version_fields("release: 1.98.0-beta.4\ncommit-hash: abc\n");
        assert_eq!(fields["release"], "1.98.0-beta.4");
        assert_eq!(fields["commit-hash"], "abc");
    }
}
