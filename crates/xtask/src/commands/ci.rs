use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use super::{Result, report};
use crate::cli::CiCommand;
use crate::process::{
    EnvironmentProfile, NetworkPolicy, ProcessLimits, ProcessRequest, ProcessRunner,
};
use crate::root::RepositoryRoot;
use serde::Deserialize;
use serde::Serialize;

#[path = "ci_support.rs"]
mod ci_support;

const FAST_SURFACES: [&str; 3] = ["precommit", "prepush", "pull_request"];
const SUPPORTED_SURFACES: [&str; 4] = ["precommit", "prepush", "pull_request", "main"];

pub(super) fn run(root: &RepositoryRoot, command: &CiCommand, runner: &ProcessRunner) -> Result {
    let surface = command.surface();
    let group = command.group();
    let skip_groups = command.skip_groups();
    let contract = Contract::load(root)?;
    contract.validate()?;
    if let Some(group) = group {
        contract.validate_group(surface, group)?;
    }
    let result =
        execute_surface_with_scheduler(root, &contract, surface, group, skip_groups, runner);
    match result {
        Ok(receipt) => Ok(report(
            root,
            &format!("ci.{surface}"),
            serde_json::to_value(receipt).map_err(|error| error.to_string())?,
        )),
        Err(error) => Err(error),
    }
}

impl CiCommand {
    const fn surface(&self) -> &'static str {
        match self {
            Self::Precommit => "precommit",
            Self::Prepush => "prepush",
            Self::Pr { .. } => "pull_request",
            Self::Main { .. } => "main",
        }
    }

    fn group(&self) -> Option<&str> {
        match self {
            Self::Pr { group } | Self::Main { group, .. } => group.as_deref(),
            Self::Precommit | Self::Prepush => None,
        }
    }

    fn skip_groups(&self) -> &[String] {
        match self {
            Self::Main { skip_groups, .. } => skip_groups,
            Self::Precommit | Self::Prepush | Self::Pr { .. } => &[],
        }
    }
}

#[derive(Debug, Deserialize)]
struct Contract {
    budgets: BTreeMap<String, u64>,
    checks: Vec<Check>,
}

#[derive(Debug, Deserialize)]
struct Check {
    id: String,
    label: String,
    program: String,
    #[serde(default)]
    args: Vec<String>,
    owner: String,
    #[serde(default)]
    prerequisites: Vec<String>,
    #[serde(default)]
    prerequisites_by_surface: BTreeMap<String, Vec<String>>,
    surfaces: Vec<String>,
    parallel_group: String,
    #[serde(default)]
    parallel_group_by_surface: BTreeMap<String, String>,
    #[serde(default)]
    resources: Vec<String>,
    network: String,
    #[serde(default)]
    artifacts: Vec<String>,
    platforms: Vec<String>,
    severity: String,
    #[serde(default)]
    merge_only: bool,
    #[serde(default)]
    accept_warning: bool,
}

impl Contract {
    fn load(root: &RepositoryRoot) -> Result<Self> {
        let path = root.join("quality/validation-surfaces.toml");
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("validation contract {}: {error}", path.display()))?;
        toml::from_str(&source).map_err(|error| format!("validation contract: {error}"))
    }

    fn validate(&self) -> Result<()> {
        for surface in SUPPORTED_SURFACES {
            if surface == "precommit" {
                if self.budgets.contains_key(surface) {
                    return Err(
                        "validation contract: precommit must not declare a time budget".to_owned(),
                    );
                }
                continue;
            }
            let budget = self
                .budgets
                .get(surface)
                .ok_or_else(|| format!("validation contract: missing budget for {surface}"))?;
            if surface != "main" && *budget == 0 {
                return Err(format!(
                    "validation contract: {surface} budget must be positive"
                ));
            }
        }
        let mut ids = BTreeSet::<String>::new();
        let mut labels = BTreeSet::<String>::new();
        for check in &self.checks {
            check.validate(&mut ids, &mut labels)?;
        }
        self.validate_prerequisites(&ids)?;
        self.validate_commands()?;
        self.validate_surface_drift()
    }

    fn validate_prerequisites(&self, ids: &BTreeSet<String>) -> Result<()> {
        for check in &self.checks {
            for surface in &check.surfaces {
                for prerequisite in check.prerequisites_for(surface) {
                    let prerequisite_check = self
                        .checks
                        .iter()
                        .find(|candidate| candidate.id == prerequisite)
                        .ok_or_else(|| {
                            format!(
                                "validation contract: check {} references unknown prerequisite {}",
                                check.id, prerequisite
                            )
                        })?;
                    if !ids.contains(prerequisite) {
                        return Err(format!(
                            "validation contract: check {} references unknown prerequisite {}",
                            check.id, prerequisite
                        ));
                    }
                    if !prerequisite_check.on(surface) {
                        return Err(format!(
                            "validation contract: prerequisite {prerequisite} does not run on {surface}"
                        ));
                    }
                    let prerequisite_group = prerequisite_check.parallel_group_for(surface);
                    let check_group = check.parallel_group_for(surface);
                    if prerequisite_group == check_group {
                        return Err(format!(
                            "validation contract: check {} has incompatible prerequisite group {}",
                            check.id, check_group
                        ));
                    }
                    if prerequisite_group >= check_group {
                        return Err(format!(
                            "validation contract: prerequisite {} must run before {} on {}",
                            prerequisite, check.id, surface
                        ));
                    }
                }
            }
        }
        for surface in SUPPORTED_SURFACES {
            self.validate_prerequisite_cycles(surface)?;
        }
        Ok(())
    }

    fn validate_prerequisite_cycles(&self, surface: &str) -> Result<()> {
        let ids = self
            .checks
            .iter()
            .filter(|check| check.on(surface))
            .map(|check| check.id.clone())
            .collect::<BTreeSet<_>>();
        let mut visiting = BTreeSet::new();
        let mut visited = BTreeSet::new();
        for id in &ids {
            if has_cycle(self, surface, id, &ids, &mut visiting, &mut visited) {
                return Err(format!(
                    "validation contract: prerequisite cycle detected on {surface} at {id}"
                ));
            }
        }
        Ok(())
    }

    fn validate_commands(&self) -> Result<()> {
        for (index, check) in self.checks.iter().enumerate() {
            let command = command_body(check);
            for other in self.checks.iter().skip(index + 1) {
                if command == command_body(other)
                    && check.surfaces.iter().any(|surface| other.on(surface))
                {
                    return Err(format!(
                        "duplicate command body on an overlapping surface: {} and {}",
                        check.id, other.id
                    ));
                }
            }
        }
        Ok(())
    }

    fn validate_surface_drift(&self) -> Result<()> {
        for check in &self.checks {
            if check.on("pull_request") && !check.on("main") {
                return Err(format!(
                    "validation contract: pull_request check {} drifts from main",
                    check.id
                ));
            }
        }
        Ok(())
    }

    fn validate_group(&self, surface: &str, group: &str) -> Result<()> {
        if self
            .checks
            .iter()
            .any(|check| check.on(surface) && check.parallel_group_for(surface) == group)
        {
            return Ok(());
        }
        Err(format!(
            "validation contract: no checks use parallel group {group} on {surface}"
        ))
    }
}

fn has_cycle(
    contract: &Contract,
    surface: &str,
    id: &str,
    ids: &BTreeSet<String>,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
) -> bool {
    if visiting.contains(id) {
        return true;
    }
    if visited.contains(id) {
        return false;
    }
    visiting.insert(id.to_owned());
    if let Some(check) = contract.checks.iter().find(|check| check.id == id) {
        for prerequisite in check.prerequisites_for(surface) {
            if ids.contains(prerequisite)
                && has_cycle(contract, surface, prerequisite, ids, visiting, visited)
            {
                return true;
            }
        }
    }
    visiting.remove(id);
    visited.insert(id.to_owned());
    false
}

impl Check {
    #[allow(clippy::too_many_lines)]
    fn validate(&self, ids: &mut BTreeSet<String>, labels: &mut BTreeSet<String>) -> Result<()> {
        if self.id.is_empty() || !ids.insert(self.id.clone()) {
            return Err(format!(
                "validation contract: duplicate or empty check id {}",
                self.id
            ));
        }
        if self.label.is_empty() || !labels.insert(self.label.clone()) {
            return Err(format!(
                "validation contract: duplicate or empty check label {}",
                self.label
            ));
        }
        if self.program.is_empty() || self.owner.is_empty() || self.parallel_group.is_empty() {
            return Err(format!(
                "validation contract: check {} has a missing required field",
                self.id
            ));
        }
        for (surface, group) in &self.parallel_group_by_surface {
            if !SUPPORTED_SURFACES.contains(&surface.as_str())
                || group.is_empty()
                || !self.on(surface)
            {
                return Err(format!(
                    "validation contract: check {} has an invalid parallel group for {}",
                    self.id, surface
                ));
            }
        }
        let mut resources = BTreeSet::new();
        for resource in &self.resources {
            if resource.is_empty() || !resources.insert(resource) {
                return Err(format!(
                    "validation contract: check {} has an empty or duplicate resource",
                    self.id
                ));
            }
        }
        for surface in self.prerequisites_by_surface.keys() {
            if !SUPPORTED_SURFACES.contains(&surface.as_str()) || !self.on(surface) {
                return Err(format!(
                    "validation contract: check {} has prerequisites for an invalid surface {}",
                    self.id, surface
                ));
            }
        }
        if self.platforms.is_empty() {
            return Err(format!(
                "validation contract: check {} has an empty platform matrix",
                self.id
            ));
        }
        for platform in &self.platforms {
            if !["all", "linux", "macos", "windows"].contains(&platform.as_str()) {
                return Err(format!(
                    "validation contract: check {} has unknown platform {}",
                    self.id, platform
                ));
            }
        }
        if !["none", "read"].contains(&self.network.as_str()) {
            return Err(format!(
                "validation contract: check {} has invalid network policy {}",
                self.id, self.network
            ));
        }
        if !["error", "warning"].contains(&self.severity.as_str()) {
            return Err(format!(
                "validation contract: check {} has invalid failure severity {}",
                self.id, self.severity
            ));
        }
        if self.accept_warning && self.severity != "warning" {
            return Err(format!(
                "validation contract: check {} accepts warnings but is not warning severity",
                self.id
            ));
        }
        if self.surfaces.is_empty() {
            return Err(format!(
                "validation contract: check {} has no surface",
                self.id
            ));
        }
        self.validate_surfaces()?;
        if self.recursive() {
            return Err(format!(
                "validation contract: check {} recursively invokes a surface wrapper",
                self.id
            ));
        }
        if self
            .prerequisites
            .iter()
            .any(|prerequisite| prerequisite == &self.id)
        {
            return Err(format!(
                "validation contract: check {} has a recursive prerequisite",
                self.id
            ));
        }
        Ok(())
    }

    fn validate_surfaces(&self) -> Result<()> {
        let mut surfaces = BTreeSet::new();
        for surface in &self.surfaces {
            if !SUPPORTED_SURFACES.contains(&surface.as_str()) {
                return Err(format!(
                    "validation contract: check {} has unknown surface {}",
                    self.id, surface
                ));
            }
            if !surfaces.insert(surface) {
                return Err(format!(
                    "validation contract: check {} repeats surface {}",
                    self.id, surface
                ));
            }
            if FAST_SURFACES.contains(&surface.as_str()) && self.network != "none" {
                return Err(format!(
                    "validation contract: fast check {} must have network = none",
                    self.id
                ));
            }
            if surface == "pull_request" && self.merge_only {
                return Err(format!(
                    "validation contract: merge-only check {} is assigned to pull_request",
                    self.id
                ));
            }
            if surface != "main" && self.merge_only {
                return Err(format!(
                    "validation contract: merge-only check {} is assigned to {surface}",
                    self.id
                ));
            }
        }
        Ok(())
    }

    fn on(&self, surface: &str) -> bool {
        self.surfaces.iter().any(|candidate| candidate == surface)
    }

    fn runs_on_current_platform(&self) -> bool {
        self.platforms
            .iter()
            .any(|platform| platform == "all" || platform == current_platform())
    }

    fn parallel_group_for(&self, surface: &str) -> &str {
        self.parallel_group_by_surface
            .get(surface)
            .map_or(self.parallel_group.as_str(), String::as_str)
    }

    fn resources(&self) -> &[String] {
        &self.resources
    }

    fn prerequisites_for(&self, surface: &str) -> Vec<&str> {
        self.prerequisites
            .iter()
            .map(String::as_str)
            .chain(
                self.prerequisites_by_surface
                    .get(surface)
                    .into_iter()
                    .flatten()
                    .map(String::as_str),
            )
            .collect()
    }

    fn recursive(&self) -> bool {
        let command = std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        command.contains("cargo xtask ci")
            || command.contains("scripts/precommit-fast.sh")
            || command.contains("scripts/prepush-fast.sh")
            || command.contains("scripts/pr-ci.sh")
            || command.contains("scripts/main-ci.sh")
    }
}

fn current_platform() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "linux"
    }
    #[cfg(target_os = "macos")]
    {
        "macos"
    }
    #[cfg(target_os = "windows")]
    {
        "windows"
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        "other"
    }
}

fn command_body(check: &Check) -> String {
    std::iter::once(check.program.as_str())
        .chain(check.args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join("\u{1f}")
}

#[derive(Debug, Serialize)]
struct SurfaceReceipt {
    surface: String,
    group: Option<String>,
    budget_seconds: Option<u64>,
    checks: Vec<CheckReceipt>,
    omitted: Vec<OmittedCheck>,
    resource_waves: Vec<ResourceWaveReceipt>,
}

#[derive(Debug, Serialize)]
struct ResourceWaveReceipt {
    checks: Vec<String>,
    resources: Vec<String>,
}

#[derive(Debug, Serialize)]
struct CheckReceipt {
    id: String,
    label: String,
    status: String,
    duration_ms: u128,
    severity: String,
    resources: Vec<String>,
    artifacts: Vec<String>,
    artifact_receipts: Vec<DeclaredArtifactReceipt>,
    detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    process: Option<crate::process::CommandReceipt>,
}

#[derive(Debug, Serialize)]
struct DeclaredArtifactReceipt {
    path: String,
    size_bytes: u64,
    sha256: String,
    fresh: bool,
}

#[derive(Debug, Serialize)]
struct OmittedCheck {
    id: String,
    reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckState {
    Passed,
    Warning,
    Failed,
    SkippedPrerequisite,
    OmittedPlatform,
}

impl CheckState {
    const fn acceptable_prerequisite(self, accept_warning: bool) -> bool {
        matches!(self, Self::Passed) || (accept_warning && matches!(self, Self::Warning))
    }
}

#[allow(clippy::too_many_lines)]
fn execute_surface_with_scheduler(
    root: &RepositoryRoot,
    contract: &Contract,
    surface: &str,
    selected_group: Option<&str>,
    skipped_groups: &[String],
    runner: &ProcessRunner,
) -> Result<SurfaceReceipt> {
    let budget_seconds = contract.budgets.get(surface).copied();
    let surface_start = Instant::now();
    let surface_deadline = budget_seconds.map(|budget| surface_start + Duration::from_secs(budget));
    let active = contract
        .checks
        .iter()
        .filter(|check| {
            check.on(surface)
                && selected_group.is_none_or(|group| check.parallel_group_for(surface) == group)
                && !skipped_groups
                    .iter()
                    .any(|group| group == check.parallel_group_for(surface))
                && check.runs_on_current_platform()
        })
        .collect::<Vec<_>>();
    let active_ids = active
        .iter()
        .map(|check| check.id.as_str())
        .collect::<BTreeSet<_>>();
    let omitted = contract
        .checks
        .iter()
        .filter(|check| {
            !check.on(surface)
                || selected_group.is_some_and(|group| check.parallel_group_for(surface) != group)
                || skipped_groups
                    .iter()
                    .any(|group| group == check.parallel_group_for(surface))
                || !check.runs_on_current_platform()
        })
        .map(|check| OmittedCheck {
            id: check.id.clone(),
            reason: if !check.on(surface) {
                format!("not assigned to {surface}")
            } else if selected_group.is_some_and(|group| check.parallel_group_for(surface) != group)
            {
                format!(
                    "not assigned to selected group {}",
                    selected_group.unwrap_or_default()
                )
            } else if skipped_groups
                .iter()
                .any(|group| group == check.parallel_group_for(surface))
            {
                "skipped by selected merge job group policy".to_owned()
            } else {
                format!("not assigned to platform {}", current_platform())
            },
        })
        .collect::<Vec<_>>();
    if let Some(group) = selected_group {
        for check in &active {
            if let Some(prerequisite) = check
                .prerequisites_for(surface)
                .into_iter()
                .find(|id| !active_ids.contains(id))
            {
                return Err(format!(
                    "ci.{surface}: selected group {group} has unmet prerequisite {prerequisite} for {}; run the full surface",
                    check.id
                ));
            }
        }
    }

    let omitted_ids = omitted
        .iter()
        .map(|check| check.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut pending = active_ids.clone();
    let mut states = BTreeMap::<String, CheckState>::new();
    for id in &omitted_ids {
        states.insert((*id).to_owned(), CheckState::OmittedPlatform);
    }
    let mut receipts = Vec::new();
    let mut resource_waves = Vec::new();
    while !pending.is_empty() {
        let mut ready = Vec::new();
        let mut skipped = Vec::new();
        for check in &active {
            if !pending.contains(check.id.as_str()) {
                continue;
            }
            let prerequisites = check.prerequisites_for(surface);
            if prerequisites.iter().all(|id| states.contains_key(*id)) {
                if prerequisites.iter().any(|id| {
                    states
                        .get(*id)
                        .is_some_and(|state| !state.acceptable_prerequisite(check.accept_warning))
                }) {
                    skipped.push(*check);
                } else {
                    ready.push(*check);
                }
            }
        }
        if ready.is_empty() && skipped.is_empty() {
            return Err(format!(
                "ci.{surface}: dependency graph made no progress for {pending:?}"
            ));
        }
        for check in skipped {
            pending.remove(check.id.as_str());
            states.insert(check.id.clone(), CheckState::SkippedPrerequisite);
            receipts.push(CheckReceipt {
                id: check.id.clone(),
                label: check.label.clone(),
                status: "skipped-prerequisite".to_owned(),
                duration_ms: 0,
                severity: check.severity.clone(),
                resources: check.resources.clone(),
                artifacts: Vec::new(),
                artifact_receipts: Vec::new(),
                detail: Some(
                    "a prerequisite did not pass its declared acceptance policy".to_owned(),
                ),
                process: None,
            });
        }
        if ready.is_empty() {
            continue;
        }
        let mut remaining_ready = ready;
        while !remaining_ready.is_empty() {
            if surface_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(surface_budget_error(
                    surface,
                    budget_seconds.expect("bounded surface has a budget"),
                    &receipts,
                )?);
            }
            let wave = take_resource_compatible_batch(&mut remaining_ready);
            let mut wave_resources = BTreeSet::new();
            for check in &wave {
                wave_resources.extend(check.resources().iter().cloned());
            }
            resource_waves.push(ResourceWaveReceipt {
                checks: wave.iter().map(|check| check.id.clone()).collect(),
                resources: wave_resources.into_iter().collect(),
            });
            let cancellation = Arc::new(AtomicBool::new(false));
            let results = Mutex::new(Vec::<(String, CheckReceipt)>::new());
            std::thread::scope(|scope| {
                for check in wave {
                    let cancellation = Arc::clone(&cancellation);
                    let results = &results;
                    scope.spawn(move || {
                        let receipt = execute_check(
                            root,
                            check,
                            surface,
                            runner,
                            &cancellation,
                            surface_deadline,
                        );
                        results
                            .lock()
                            .expect("receipt mutex")
                            .push((check.id.clone(), receipt));
                    });
                }
            });
            let wave = results
                .into_inner()
                .map_err(|_| "receipt mutex poisoned".to_owned())?;
            for (id, receipt) in wave {
                pending.remove(id.as_str());
                let state = match receipt.status.as_str() {
                    "passed" => CheckState::Passed,
                    "warning" => CheckState::Warning,
                    _ => CheckState::Failed,
                };
                states.insert(id, state);
                receipts.push(receipt);
            }
            if surface_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(surface_budget_error(
                    surface,
                    budget_seconds.expect("bounded surface has a budget"),
                    &receipts,
                )?);
            }
            if receipts.iter().any(|receipt| receipt.status == "failed") {
                receipts.sort_by(|left, right| left.id.cmp(&right.id));
                return Err(format!(
                    "ci.{surface}: validation wave failed; receipt={}",
                    serde_json::to_string(&receipts).map_err(|error| error.to_string())?
                ));
            }
        }
    }
    receipts.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(SurfaceReceipt {
        surface: surface.to_owned(),
        group: selected_group.map(str::to_owned),
        budget_seconds,
        checks: receipts,
        omitted,
        resource_waves,
    })
}

fn surface_budget_error(
    surface: &str,
    budget_seconds: u64,
    receipts: &[CheckReceipt],
) -> Result<String> {
    Ok(format!(
        "ci.{surface}: surface budget of {budget_seconds}s exhausted; receipt={}",
        serde_json::to_string(receipts).map_err(|error| error.to_string())?
    ))
}

fn take_resource_compatible_batch<'a>(ready: &mut Vec<&'a Check>) -> Vec<&'a Check> {
    let mut locked = BTreeSet::new();
    let mut batch = Vec::new();
    let mut deferred = Vec::new();
    for check in ready.drain(..) {
        if check
            .resources()
            .iter()
            .all(|resource| !locked.contains(resource))
        {
            locked.extend(check.resources().iter().cloned());
            batch.push(check);
        } else {
            deferred.push(check);
        }
    }
    *ready = deferred;
    batch
}

#[cfg(test)]
fn execute_surface_for_group(
    root: &RepositoryRoot,
    contract: &Contract,
    surface: &str,
    selected_group: Option<&str>,
    runner: &ProcessRunner,
) -> Result<SurfaceReceipt> {
    execute_surface_with_scheduler(root, contract, surface, selected_group, &[], runner)
}

#[allow(clippy::too_many_lines)]
fn execute_check(
    root: &RepositoryRoot,
    check: &Check,
    surface: &str,
    runner: &ProcessRunner,
    cancellation: &Arc<AtomicBool>,
    surface_deadline: Option<Instant>,
) -> CheckReceipt {
    const CHECK_OUTPUT_LIMIT: usize = 64 * 1024;
    const DAG_RECEIPT_OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
    let start = Instant::now();
    let started_at = SystemTime::now();
    let timeout = if let Some(surface_deadline) = surface_deadline {
        let Some(timeout) = surface_deadline.checked_duration_since(start) else {
            cancellation.store(true, Ordering::Release);
            return expired_surface_receipt(check, surface);
        };
        Some(timeout)
    } else {
        None
    };
    let (args, artifacts, process_artifacts) = ci_support::check_process_contract(check, surface);
    let profile = match check.program.as_str() {
        "cargo" | "rustc" | "rustup" => EnvironmentProfile::RustTool,
        "git" => EnvironmentProfile::GitTool,
        "curl" => EnvironmentProfile::GitHubApi,
        "bash" | "bun" => EnvironmentProfile::PlatformTool,
        _ => EnvironmentProfile::Empty,
    };
    let network = if check.network == "read" {
        NetworkPolicy::Read
    } else {
        NetworkPolicy::None
    };
    let mut request = ProcessRequest::new(check.program.clone(), args)
        .profile(profile)
        .network(network)
        .current_dir(root.path())
        .cancellation(cancellation.clone())
        .limits(ProcessLimits {
            // The DAG receipt intentionally includes every target/feature edge;
            // keep its cap bounded without truncating a valid deterministic receipt.
            max_stdout_bytes: if check.id == "workspace-dag" {
                DAG_RECEIPT_OUTPUT_LIMIT
            } else {
                CHECK_OUTPUT_LIMIT
            },
            max_stderr_bytes: CHECK_OUTPUT_LIMIT,
            timeout,
        });
    if let Some((stdout, stderr)) = process_artifacts {
        request = request.artifacts(root.join(stdout), root.join(stderr));
    }
    let result = runner.run(request);
    let duration_ms = start.elapsed().as_millis();
    match result {
        Ok(result) if result.receipt.success() => {
            match ci_support::verify_declared_artifacts(root, &artifacts, started_at) {
                Ok(artifact_receipts) => CheckReceipt {
                    id: check.id.clone(),
                    label: check.label.clone(),
                    status: "passed".to_owned(),
                    duration_ms,
                    severity: check.severity.clone(),
                    resources: check.resources.clone(),
                    artifacts,
                    artifact_receipts,
                    detail: None,
                    process: Some(result.receipt),
                },
                Err(error) => {
                    let status = if check.severity == "warning" {
                        "warning"
                    } else {
                        cancellation.store(true, Ordering::Release);
                        "failed"
                    };
                    CheckReceipt {
                        id: check.id.clone(),
                        label: check.label.clone(),
                        status: status.to_owned(),
                        duration_ms,
                        severity: check.severity.clone(),
                        resources: check.resources.clone(),
                        artifacts,
                        artifact_receipts: Vec::new(),
                        detail: Some(error),
                        process: Some(result.receipt),
                    }
                }
            }
        }
        Ok(result) => {
            let status = if check.severity == "warning" {
                "warning"
            } else {
                cancellation.store(true, Ordering::Release);
                "failed"
            };
            CheckReceipt {
                id: check.id.clone(),
                label: check.label.clone(),
                status: status.to_owned(),
                duration_ms,
                severity: check.severity.clone(),
                resources: check.resources.clone(),
                artifacts,
                artifact_receipts: Vec::new(),
                detail: Some(process_failure_detail(&result.receipt, surface)),
                process: Some(result.receipt),
            }
        }
        Err(error) => {
            let status = if check.severity == "warning" {
                "warning"
            } else {
                cancellation.store(true, Ordering::Release);
                "failed"
            };
            CheckReceipt {
                id: check.id.clone(),
                label: check.label.clone(),
                status: status.to_owned(),
                duration_ms,
                severity: check.severity.clone(),
                resources: check.resources.clone(),
                artifacts,
                artifact_receipts: Vec::new(),
                detail: Some(error.to_string()),
                process: None,
            }
        }
    }
}

fn expired_surface_receipt(check: &Check, surface: &str) -> CheckReceipt {
    CheckReceipt {
        id: check.id.clone(),
        label: check.label.clone(),
        status: "failed".to_owned(),
        duration_ms: 0,
        severity: check.severity.clone(),
        resources: check.resources.clone(),
        artifacts: Vec::new(),
        artifact_receipts: Vec::new(),
        detail: Some(format!(
            "surface deadline exhausted before {} could start on {surface}",
            check.id
        )),
        process: None,
    }
}

fn process_failure_detail(receipt: &crate::process::CommandReceipt, surface: &str) -> String {
    if receipt.status == "timed-out" {
        format!("surface deadline exhausted on {surface}")
    } else {
        format!(
            "process status {}, exit code {:?}",
            receipt.status, receipt.exit_code
        )
    }
}

#[cfg(test)]
#[path = "ci_tests.rs"]
mod tests;
