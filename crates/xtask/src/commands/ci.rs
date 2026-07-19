use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde::Serialize;

use super::{Result, report};
use crate::cli::CiCommand;
use crate::process::{
    EnvironmentProfile, NetworkPolicy, ProcessLimits, ProcessRequest, ProcessRunner,
};
use crate::root::RepositoryRoot;

const FAST_SURFACES: [&str; 3] = ["precommit", "prepush", "pull_request"];
const SUPPORTED_SURFACES: [&str; 4] = ["precommit", "prepush", "pull_request", "main"];

pub(super) fn run(root: &RepositoryRoot, command: &CiCommand, runner: &ProcessRunner) -> Result {
    let surface = command.surface();
    let group = command.group();
    let contract = Contract::load(root)?;
    contract.validate()?;
    if let Some(group) = group {
        contract.validate_group(surface, group)?;
    }
    let result = execute_surface_with_scheduler(root, &contract, surface, group, runner);
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
            Self::Main => "main",
        }
    }

    fn group(&self) -> Option<&str> {
        match self {
            Self::Pr { group } => group.as_deref(),
            Self::Precommit | Self::Prepush | Self::Main => None,
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
    timeout_seconds: u64,
    #[serde(default)]
    timeout_seconds_by_surface: BTreeMap<String, u64>,
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
        self.validate_surface_drift()?;
        self.validate_budgets()
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

    fn validate_budgets(&self) -> Result<()> {
        for surface in FAST_SURFACES {
            let mut group_max = BTreeMap::<&str, u64>::new();
            for check in self.checks.iter().filter(|check| check.on(surface)) {
                group_max
                    .entry(check.parallel_group_for(surface))
                    .and_modify(|value| *value = (*value).max(check.timeout_for(surface)))
                    .or_insert(check.timeout_for(surface));
            }
            let configured = if surface == "pull_request" {
                group_max.values().copied().max().unwrap_or_default()
            } else {
                group_max.values().sum::<u64>()
            };
            let budget = self.budgets[surface];
            if configured > budget {
                return Err(format!(
                    "validation contract: {surface} parallel-group timeout budget {configured}s exceeds budget {budget}s"
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

impl Check {
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
        for surface in self.prerequisites_by_surface.keys() {
            if !SUPPORTED_SURFACES.contains(&surface.as_str()) || !self.on(surface) {
                return Err(format!(
                    "validation contract: check {} has prerequisites for an invalid surface {}",
                    self.id, surface
                ));
            }
        }
        if self.timeout_seconds == 0 || self.platforms.is_empty() {
            return Err(format!(
                "validation contract: check {} has an invalid timeout or platform matrix",
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
        for (surface, timeout) in &self.timeout_seconds_by_surface {
            if !SUPPORTED_SURFACES.contains(&surface.as_str()) || *timeout == 0 {
                return Err(format!(
                    "validation contract: check {} has an invalid surface timeout for {}",
                    self.id, surface
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

    fn timeout_for(&self, surface: &str) -> u64 {
        self.timeout_seconds_by_surface
            .get(surface)
            .copied()
            .unwrap_or(self.timeout_seconds)
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
    budget_seconds: u64,
    checks: Vec<CheckReceipt>,
    omitted: Vec<OmittedCheck>,
}

#[derive(Debug, Serialize)]
struct CheckReceipt {
    id: String,
    label: String,
    status: String,
    duration_ms: u128,
    severity: String,
    artifacts: Vec<String>,
    detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    process: Option<crate::process::CommandReceipt>,
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

fn execute_surface_with_scheduler(
    root: &RepositoryRoot,
    contract: &Contract,
    surface: &str,
    selected_group: Option<&str>,
    runner: &ProcessRunner,
) -> Result<SurfaceReceipt> {
    let budget_seconds = contract.budgets[surface];
    let surface_start = Instant::now();
    let active = contract
        .checks
        .iter()
        .filter(|check| {
            check.on(surface)
                && selected_group.is_none_or(|group| check.parallel_group_for(surface) == group)
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
                "ci.{surface}: dependency graph made no progress for {:?}",
                pending
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
                artifacts: Vec::new(),
                detail: Some(
                    "a prerequisite did not pass its declared acceptance policy".to_owned(),
                ),
                process: None,
            });
        }
        if ready.is_empty() {
            continue;
        }
        let cancellation = Arc::new(AtomicBool::new(false));
        let results = Mutex::new(Vec::<(String, CheckReceipt)>::new());
        std::thread::scope(|scope| {
            for check in ready {
                let cancellation = Arc::clone(&cancellation);
                let results = &results;
                scope.spawn(move || {
                    let receipt = execute_check(root, check, surface, runner, &cancellation);
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
        if surface_start.elapsed() > Duration::from_secs(budget_seconds) {
            return Err(format!(
                "ci.{surface}: configured budget of {budget_seconds}s exceeded; receipt={}",
                serde_json::to_string(&receipts).map_err(|error| error.to_string())?
            ));
        }
        if receipts.iter().any(|receipt| receipt.status == "failed") {
            receipts.sort_by(|left, right| left.id.cmp(&right.id));
            return Err(format!(
                "ci.{surface}: validation wave failed; receipt={}",
                serde_json::to_string(&receipts).map_err(|error| error.to_string())?
            ));
        }
    }
    receipts.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(SurfaceReceipt {
        surface: surface.to_owned(),
        group: selected_group.map(str::to_owned),
        budget_seconds,
        checks: receipts,
        omitted,
    })
}

#[cfg(test)]
fn execute_surface_for_group(
    root: &RepositoryRoot,
    contract: &Contract,
    surface: &str,
    selected_group: Option<&str>,
    runner: &ProcessRunner,
) -> Result<SurfaceReceipt> {
    execute_surface_with_scheduler(root, contract, surface, selected_group, runner)
}

fn execute_check(
    root: &RepositoryRoot,
    check: &Check,
    surface: &str,
    runner: &ProcessRunner,
    cancellation: &Arc<AtomicBool>,
) -> CheckReceipt {
    const CHECK_OUTPUT_LIMIT: usize = 64 * 1024;
    const DAG_RECEIPT_OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
    let start = Instant::now();
    let (args, artifacts, process_artifacts) = check_process_contract(check, surface);
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
            timeout: Duration::from_secs(check.timeout_for(surface)),
        });
    if let Some((stdout, stderr)) = process_artifacts {
        request = request.artifacts(root.join(stdout), root.join(stderr));
    }
    let result = runner.run(request);
    let duration_ms = start.elapsed().as_millis();
    match result {
        Ok(result) if result.receipt.success() => CheckReceipt {
            id: check.id.clone(),
            label: check.label.clone(),
            status: "passed".to_owned(),
            duration_ms,
            severity: check.severity.clone(),
            artifacts,
            detail: None,
            process: Some(result.receipt),
        },
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
                artifacts,
                detail: Some(format!(
                    "process status {}, exit code {:?}",
                    result.receipt.status, result.receipt.exit_code
                )),
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
                artifacts,
                detail: Some(error.to_string()),
                process: None,
            }
        }
    }
}

fn check_process_contract(
    check: &Check,
    surface: &str,
) -> (Vec<String>, Vec<String>, Option<(String, String)>) {
    if check.id != "workspace-dag" {
        return (check.args.clone(), check.artifacts.clone(), None);
    }
    let directory = "target/validation/workspace-dag";
    let report = format!("{directory}/{surface}.json");
    let stdout = format!("{directory}/{surface}.stdout.log");
    let stderr = format!("{directory}/{surface}.stderr.log");
    let mut args = check.args.clone();
    args.extend(["--artifact".to_owned(), report.clone()]);
    let mut artifacts = check.artifacts.clone();
    artifacts.extend([report, stdout.clone(), stderr.clone()]);
    (args, artifacts, Some((stdout, stderr)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(id: &str, group: &str, timeout: u64) -> Check {
        Check {
            id: id.to_owned(),
            label: id.to_owned(),
            program: "true".to_owned(),
            args: Vec::new(),
            owner: "test".to_owned(),
            prerequisites: Vec::new(),
            prerequisites_by_surface: BTreeMap::new(),
            surfaces: vec!["precommit".to_owned()],
            parallel_group: group.to_owned(),
            parallel_group_by_surface: BTreeMap::new(),
            timeout_seconds: timeout,
            timeout_seconds_by_surface: BTreeMap::new(),
            network: "none".to_owned(),
            artifacts: Vec::new(),
            platforms: vec!["all".to_owned()],
            severity: "error".to_owned(),
            merge_only: false,
            accept_warning: false,
        }
    }

    fn contract(checks: Vec<Check>) -> Contract {
        Contract {
            budgets: BTreeMap::from([
                ("precommit".to_owned(), 30),
                ("prepush".to_owned(), 170),
                ("pull_request".to_owned(), 150),
                ("main".to_owned(), 2_700),
            ]),
            checks,
        }
    }

    #[test]
    fn rejects_duplicate_ids_and_labels() {
        let error = contract(vec![check("same", "a", 1), check("same", "b", 1)])
            .validate()
            .expect_err("duplicate");
        assert!(error.contains("duplicate"));
    }

    #[test]
    fn rejects_fast_network_checks() {
        let mut item = check("network", "a", 1);
        item.network = "read".to_owned();
        let error = contract(vec![item]).validate().expect_err("network");
        assert!(error.contains("network = none"));
    }

    #[test]
    fn rejects_unknown_platforms() {
        let mut item = check("platform", "a", 1);
        item.platforms = vec!["solaris".to_owned()];
        let error = contract(vec![item]).validate().expect_err("platform");
        assert!(error.contains("unknown platform solaris"));
    }

    #[test]
    fn rejects_sequential_budget_overflow() {
        let mut first = check("a", "a", 20);
        first.args = vec!["a".to_owned()];
        let mut second = check("b", "b", 20);
        second.args = vec!["b".to_owned()];
        let error = contract(vec![first, second])
            .validate()
            .expect_err("budget");
        assert!(error.contains("exceeds budget"));
    }

    #[test]
    fn rejects_recursive_surface_invocation() {
        let mut item = check("recursive", "a", 1);
        item.program = "cargo".to_owned();
        item.args = vec!["xtask".to_owned(), "ci".to_owned(), "precommit".to_owned()];
        let error = contract(vec![item]).validate().expect_err("recursive");
        assert!(error.contains("recursively"));
    }

    #[test]
    fn rejects_unknown_prerequisites() {
        let mut item = check("dependent", "dependent", 1);
        item.prerequisites = vec!["missing".to_owned()];
        let error = contract(vec![item]).validate().expect_err("prerequisite");
        assert!(error.contains("unknown prerequisite"));
    }

    #[test]
    fn rejects_incompatible_prerequisite_groups() {
        let mut dependent = check("dependent", "same", 1);
        dependent.prerequisites = vec!["prerequisite".to_owned()];
        let prerequisite = check("prerequisite", "same", 1);
        let error = contract(vec![dependent, prerequisite])
            .validate()
            .expect_err("parallel group");
        assert!(error.contains("incompatible prerequisite group"));
    }

    #[test]
    fn rejects_surface_prerequisite_that_runs_after_dependent() {
        let mut dependent = check("dependent", "dependent", 1);
        dependent.surfaces = vec!["prepush".to_owned()];
        dependent.prerequisites_by_surface =
            BTreeMap::from([("prepush".to_owned(), vec!["prerequisite".to_owned()])]);
        let mut prerequisite = check("prerequisite", "prerequisite", 1);
        prerequisite.surfaces = vec!["prepush".to_owned()];
        dependent.parallel_group_by_surface =
            BTreeMap::from([("prepush".to_owned(), "rust-02".to_owned())]);
        prerequisite.parallel_group_by_surface =
            BTreeMap::from([("prepush".to_owned(), "rust-03".to_owned())]);
        let error = contract(vec![dependent, prerequisite])
            .validate()
            .expect_err("ordered prerequisite");
        assert!(error.contains("must run before"));
    }

    #[test]
    fn rejects_pull_request_checks_that_drift_from_main() {
        let mut item = check("pr", "pr", 1);
        item.surfaces = vec!["pull_request".to_owned()];
        let error = contract(vec![item]).validate().expect_err("surface drift");
        assert!(error.contains("drifts from main"));
    }

    #[test]
    fn rejects_merge_only_checks_on_a_fast_surface() {
        let mut item = check("merge-only", "main", 1);
        item.merge_only = true;
        item.surfaces = vec!["prepush".to_owned(), "main".to_owned()];
        let error = contract(vec![item])
            .validate()
            .expect_err("merge-only fast check");
        assert!(error.contains("merge-only"));
    }

    #[test]
    fn checked_in_contract_keeps_local_coverage_and_merge_only_exhaustiveness() {
        let runner = ProcessRunner::new();
        let root = RepositoryRoot::discover(&runner).expect("repository root");
        let contract = Contract::load(&root).expect("validation contract");
        contract.validate().expect("valid validation contract");
        assert_eq!(contract.budgets["precommit"], 30);
        assert_eq!(contract.budgets["prepush"], 170);
        assert_eq!(contract.budgets["pull_request"], 150);

        for id in [
            "fmt",
            "metadata",
            "rust-check",
            "rust-clippy",
            "rust-test",
            "source",
            "workflow-policy",
            "repository-files",
            "workspace-dag",
            "workspace-rust-version",
            "workspace-layout",
            "cache-workflow-policy",
        ] {
            let item = contract
                .checks
                .iter()
                .find(|check| check.id == id)
                .unwrap_or_else(|| panic!("missing {id}"));
            assert!(item.on("prepush"), "{id} must run on prepush");
        }
        let library_lint = contract
            .checks
            .iter()
            .find(|check| check.id == "rust-clippy")
            .expect("library lint");
        assert_eq!(library_lint.timeout_for("precommit"), 20);
        assert_eq!(library_lint.timeout_for("prepush"), 35);

        let rust_check = contract
            .checks
            .iter()
            .find(|check| check.id == "rust-check")
            .expect("library check");
        let rust_test = contract
            .checks
            .iter()
            .find(|check| check.id == "rust-test")
            .expect("library test");
        let workflow_policy = contract
            .checks
            .iter()
            .find(|check| check.id == "workflow-policy")
            .expect("workflow policy check");
        assert_eq!(
            workflow_policy.parallel_group_for("pull_request"),
            "rust-02-workflow"
        );
        assert_eq!(workflow_policy.timeout_for("pull_request"), 20);
        for surface in ["prepush", "pull_request"] {
            if surface == "pull_request" {
                assert!(!rust_check.on(surface));
                assert_eq!(rust_test.parallel_group_for(surface), "rust-01-test");
                assert_eq!(rust_test.timeout_for(surface), 110);
                assert!(rust_test.prerequisites_for(surface).is_empty());
                continue;
            }
            assert_eq!(rust_check.parallel_group_for(surface), "rust-03-check");
            assert_eq!(rust_test.parallel_group_for(surface), "rust-01-test");
            assert_eq!(rust_check.timeout_for(surface), 25);
            assert_eq!(rust_test.timeout_for(surface), 75);
            let expected_prerequisites = if surface == "prepush" {
                vec!["rust-clippy"]
            } else {
                vec!["rust-test"]
            };
            assert_eq!(
                rust_check.prerequisites_for(surface),
                expected_prerequisites
            );
            assert!(rust_test.prerequisites_for(surface).is_empty());
        }
        assert!(library_lint.on("prepush"));
        assert!(!library_lint.on("pull_request"));
        assert_eq!(library_lint.parallel_group_for("prepush"), "rust-02-clippy");
        assert_eq!(library_lint.timeout_for("prepush"), 35);
        assert_eq!(library_lint.prerequisites_for("prepush"), vec!["rust-test"]);
        assert_eq!(rust_check.parallel_group_for("precommit"), "rust");
        assert_eq!(library_lint.parallel_group_for("precommit"), "rust");

        for id in ["rust-test-all", "rust-clippy-all"] {
            let item = contract
                .checks
                .iter()
                .find(|check| check.id == id)
                .unwrap_or_else(|| panic!("missing {id}"));
            assert_eq!(item.surfaces, vec!["main".to_owned()]);
            assert!(item.merge_only);
        }
        assert!(
            contract
                .checks
                .iter()
                .filter(|check| check.on("prepush"))
                .all(|check| !check.merge_only)
        );
    }

    #[test]
    fn records_a_failed_check_receipt() {
        let runner = ProcessRunner::new();
        let root = RepositoryRoot::discover(&runner).expect("repository root");
        let mut item = check("failure", "a", 5);
        item.program = "sh".to_owned();
        item.args = vec!["-c".to_owned(), "exit 7".to_owned()];
        let receipt = execute_check(
            &root,
            &item,
            "precommit",
            &runner,
            &Arc::new(AtomicBool::new(false)),
        );
        assert_eq!(receipt.status, "failed");
        assert!(
            receipt
                .detail
                .expect("failure detail")
                .contains("completed")
        );
    }

    #[test]
    fn warning_failure_does_not_cancel_siblings_or_group() {
        let runner = ProcessRunner::new();
        let root = RepositoryRoot::discover(&runner).expect("repository root");
        let mut warning = check("warning", "a", 5);
        warning.program = "sh".to_owned();
        warning.args = vec!["-c".to_owned(), "exit 7".to_owned()];
        warning.severity = "warning".to_owned();
        let cancellation = Arc::new(AtomicBool::new(false));
        let receipt = execute_check(&root, &warning, "precommit", &runner, &cancellation);
        assert_eq!(receipt.status, "warning");
        assert!(!cancellation.load(Ordering::Acquire));
    }

    #[test]
    fn records_a_timed_out_check_receipt() {
        let runner = ProcessRunner::new();
        let root = RepositoryRoot::discover(&runner).expect("repository root");
        let mut item = check("timeout", "a", 1);
        item.program = "sh".to_owned();
        item.args = vec!["-c".to_owned(), "sleep 2".to_owned()];
        let receipt = execute_check(
            &root,
            &item,
            "precommit",
            &runner,
            &Arc::new(AtomicBool::new(false)),
        );
        assert_eq!(receipt.status, "failed");
        assert!(
            receipt
                .detail
                .expect("timeout detail")
                .contains("timed-out")
        );
    }

    #[test]
    fn selected_group_receipt_omits_other_parallel_groups() {
        let runner = ProcessRunner::new();
        let root = RepositoryRoot::discover(&runner).expect("repository root");
        let mut rust_check = check("rust", "rust", 5);
        rust_check.surfaces = vec!["pull_request".to_owned()];
        let mut repository_check = check("repository", "repository", 5);
        repository_check.surfaces = vec!["pull_request".to_owned()];
        let contract = contract(vec![rust_check, repository_check]);

        let receipt =
            execute_surface_for_group(&root, &contract, "pull_request", Some("rust"), &runner)
                .expect("selected group succeeds");

        assert_eq!(receipt.group.as_deref(), Some("rust"));
        assert_eq!(
            receipt
                .checks
                .iter()
                .map(|check| check.id.as_str())
                .collect::<Vec<_>>(),
            vec!["rust"]
        );
        assert_eq!(
            receipt.omitted[0].reason,
            "not assigned to selected group rust"
        );
    }

    #[test]
    fn selected_group_rejects_unmet_prerequisites() {
        let runner = ProcessRunner::new();
        let root = RepositoryRoot::discover(&runner).expect("repository root");
        let mut dependent = check("dependent", "dependent", 5);
        dependent.surfaces = vec!["pull_request".to_owned()];
        dependent.prerequisites = vec!["prerequisite".to_owned()];
        let mut prerequisite = check("prerequisite", "prerequisite", 5);
        prerequisite.surfaces = vec!["pull_request".to_owned()];
        let contract = Contract {
            budgets: BTreeMap::from([(String::from("pull_request"), 150)]),
            checks: vec![dependent, prerequisite],
        };

        let error =
            execute_surface_for_group(&root, &contract, "pull_request", Some("dependent"), &runner)
                .expect_err("selected group with prerequisite");
        assert!(error.contains("unmet prerequisite prerequisite"));
    }

    #[test]
    fn omits_checks_for_other_platforms_before_execution() {
        let runner = ProcessRunner::new();
        let root = RepositoryRoot::discover(&runner).expect("repository root");
        let mut item = check("other-platform", "rust", 5);
        item.platforms = vec![if current_platform() == "linux" {
            "macos".to_owned()
        } else {
            "linux".to_owned()
        }];
        let contract = contract(vec![item]);

        let receipt = execute_surface_for_group(&root, &contract, "precommit", None, &runner)
            .expect("platform omission succeeds");

        assert!(receipt.checks.is_empty());
        assert_eq!(
            receipt.omitted[0].reason,
            format!("not assigned to platform {}", current_platform())
        );
    }

    #[test]
    fn workspace_dag_receipt_contract_has_deterministic_artifacts() {
        let item = check("workspace-dag", "repository", 5);
        let (args, artifacts, process_artifacts) = check_process_contract(&item, "pull_request");
        assert!(args.ends_with(&[
            "--artifact".to_owned(),
            "target/validation/workspace-dag/pull_request.json".to_owned(),
        ]));
        assert_eq!(
            artifacts,
            vec![
                "target/validation/workspace-dag/pull_request.json".to_owned(),
                "target/validation/workspace-dag/pull_request.stdout.log".to_owned(),
                "target/validation/workspace-dag/pull_request.stderr.log".to_owned(),
            ]
        );
        assert_eq!(
            process_artifacts,
            Some((
                "target/validation/workspace-dag/pull_request.stdout.log".to_owned(),
                "target/validation/workspace-dag/pull_request.stderr.log".to_owned(),
            ))
        );
    }

    #[test]
    fn cancels_siblings_and_keeps_failure_receipts_sorted() {
        let runner = ProcessRunner::new();
        let root = RepositoryRoot::discover(&runner).expect("repository root");
        let mut failure = check("a-failure", "group", 5);
        failure.program = "sh".to_owned();
        failure.args = vec!["-c".to_owned(), "exit 7".to_owned()];
        failure.surfaces = vec!["prepush".to_owned()];
        let mut sibling = check("z-sibling", "group", 5);
        sibling.program = "sh".to_owned();
        sibling.args = vec!["-c".to_owned(), "sleep 10".to_owned()];
        sibling.surfaces = vec!["prepush".to_owned()];
        let contract = Contract {
            budgets: BTreeMap::from([(String::from("prepush"), 150)]),
            checks: vec![failure, sibling],
        };

        let error = execute_surface_for_group(&root, &contract, "prepush", None, &runner)
            .expect_err("failing group");
        let failure_position = error.find("\"id\":\"a-failure\"").expect("failure receipt");
        let sibling_position = error.find("\"id\":\"z-sibling\"").expect("sibling receipt");
        assert!(failure_position < sibling_position);
        assert!(error.contains("cancelled"));
    }
}
