use std::fs;

use super::{Result, files, report};
use crate::cli::RepoCommand;
use crate::process::ProcessRunner;
use crate::root::RepositoryRoot;

pub(super) fn run(root: &RepositoryRoot, command: &RepoCommand, runner: &ProcessRunner) -> Result {
    match command {
        RepoCommand::Dag(arguments) => super::dag::run(root, runner, arguments.artifact.as_deref()),
        RepoCommand::Files(arguments) => files::run(root, arguments, runner),
        RepoCommand::Workflows => verify_workflows(root),
        RepoCommand::NativeBoundaries(arguments) => {
            super::native_boundaries::run(root, runner, arguments.receipt.as_deref())
        }
    }
}

const EXPECTED_WORKFLOWS: [(&str, WorkflowKind); 2] = [
    ("rust-main.yml", WorkflowKind::Main),
    ("rust-pr.yml", WorkflowKind::PullRequest),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkflowKind {
    PullRequest,
    Main,
}

impl WorkflowKind {
    const fn timeout_limit(self) -> u32 {
        match self {
            Self::PullRequest => 10,
            Self::Main => 45,
        }
    }

    const fn shell_budget(self) -> u32 {
        match self {
            Self::PullRequest => 150,
            Self::Main => 2_700,
        }
    }
}

fn verify_workflows(root: &RepositoryRoot) -> Result {
    let directory = root.join(".github/workflows");
    let mut files = fs::read_dir(&directory)
        .map_err(|error| format!(".github/workflows: cannot read directory: {error}"))?
        .map(|entry| {
            let entry = entry.map_err(|error| format!(".github/workflows: {error}"))?;
            let file_type = entry.file_type().map_err(|error| {
                format!(".github/workflows: {}: {error}", entry.path().display())
            })?;
            if file_type.is_file() {
                Ok(Some(entry.file_name().to_string_lossy().into_owned()))
            } else {
                Ok(None)
            }
        })
        .collect::<std::result::Result<Vec<_>, String>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    files.sort();

    let expected = EXPECTED_WORKFLOWS
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>();
    if files != expected {
        return Err(format!(
            ".github/workflows: executable inventory mismatch: found [{}], required [{}]",
            files.join(", "),
            expected.join(", ")
        ));
    }

    for (name, kind) in EXPECTED_WORKFLOWS {
        let path = directory.join(name);
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("{name}: cannot read workflow: {error}"))?;
        validate_workflow(name, kind, &source)?;
    }

    Ok(report(
        root,
        "repo.verify-workflows",
        serde_json::json!({
            "workflows": files,
            "policy": "rusttable.workflow.v1",
        }),
    ))
}

fn validate_workflow(
    name: &str,
    kind: WorkflowKind,
    source: &str,
) -> std::result::Result<(), String> {
    let lines = source.lines().collect::<Vec<_>>();
    let mut errors = Vec::new();
    let top_level = top_level_keys(&lines);
    require_top_level(name, &top_level, "name", &mut errors);
    require_top_level(name, &top_level, "on", &mut errors);
    require_top_level(name, &top_level, "permissions", &mut errors);
    require_top_level(name, &top_level, "jobs", &mut errors);

    validate_events(name, kind, &lines, &mut errors);
    validate_permissions(name, &lines, &mut errors);
    validate_jobs(name, kind, &lines, &mut errors);
    validate_actions(name, &lines, &mut errors);
    validate_prohibited_content(name, &lines, &mut errors);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

fn top_level_keys<'a>(lines: &'a [&'a str]) -> Vec<&'a str> {
    lines
        .iter()
        .filter_map(|line| {
            let trimmed = line.trim();
            if line.starts_with(' ') || trimmed.is_empty() || trimmed.starts_with('#') {
                return None;
            }
            trimmed.split_once(':').map(|(key, _)| key.trim())
        })
        .collect()
}

fn require_top_level(name: &str, keys: &[&str], key: &str, errors: &mut Vec<String>) {
    if !keys.contains(&key) {
        errors.push(format!("{name}: YAML path {key}: required key is missing"));
    }
}

fn validate_events(name: &str, kind: WorkflowKind, lines: &[&str], errors: &mut Vec<String>) {
    let Some(start) = section_start(lines, "on") else {
        return;
    };
    let end = section_end(lines, start);
    let mut events = Vec::new();
    for line in &lines[start + 1..end] {
        if indent(line) == 2
            && let Some((key, _)) = key_value(line)
        {
            events.push(key.to_owned());
        }
    }
    let expected = match kind {
        WorkflowKind::PullRequest => vec!["pull_request"],
        WorkflowKind::Main => vec!["push"],
    };
    if events != expected {
        errors.push(format!(
            "{name}: YAML path on: events must be [{}], found [{}]",
            expected.join(", "),
            events.join(", ")
        ));
    }
    if kind == WorkflowKind::Main {
        let push_start = lines[start + 1..end]
            .iter()
            .position(|line| {
                indent(line) == 2 && key_value(line).is_some_and(|(key, _)| key == "push")
            })
            .map(|offset| start + 1 + offset);
        if let Some(push_start) = push_start {
            let push_end = section_end_from(lines, push_start, end);
            let branches = lines[push_start + 1..push_end].iter().find_map(|line| {
                (indent(line) == 4 && key_value(line).is_some_and(|(key, _)| key == "branches"))
                    .then_some(())
            });
            let main_branch = lines[push_start + 1..push_end]
                .iter()
                .any(|line| indent(line) == 6 && line.trim() == "- main");
            if branches.is_none() || !main_branch {
                errors.push(format!(
                    "{name}: YAML path on.push.branches: must contain main"
                ));
            }
        }
    }
}

fn validate_permissions(name: &str, lines: &[&str], errors: &mut Vec<String>) {
    let Some(start) = section_start(lines, "permissions") else {
        return;
    };
    let end = section_end(lines, start);
    let permissions = lines[start + 1..end]
        .iter()
        .filter(|line| indent(line) == 2)
        .filter_map(|line| key_value(line))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect::<Vec<_>>();
    if permissions != [("contents".to_owned(), "read".to_owned())] {
        errors.push(format!(
            "{name}: YAML path permissions: require exactly contents: read"
        ));
    }
}

fn validate_jobs(name: &str, kind: WorkflowKind, lines: &[&str], errors: &mut Vec<String>) {
    let Some(start) = section_start(lines, "jobs") else {
        return;
    };
    let end = section_end(lines, start);
    let job_starts = lines[start + 1..end]
        .iter()
        .enumerate()
        .filter_map(|(offset, line)| {
            (indent(line) == 2 && key_value(line).is_some()).then_some(start + 1 + offset)
        })
        .collect::<Vec<_>>();
    if job_starts.is_empty() {
        errors.push(format!(
            "{name}: YAML path jobs: at least one job is required"
        ));
        return;
    }
    for (index, job_start) in job_starts.iter().enumerate() {
        let job_end = job_starts.get(index + 1).copied().unwrap_or(end);
        let job_name = key_value(lines[*job_start]).map_or("<unknown>", |(key, _)| key);
        let body = &lines[*job_start + 1..job_end];
        let summary_job = body.iter().any(|line| {
            indent(line) == 4 && key_value(line).is_some_and(|(key, _)| key == "needs")
        }) && body
            .iter()
            .any(|line| indent(line) == 4 && line.contains("if: ${{ always() }}"));
        let timeout = body.iter().find_map(|line| {
            if indent(line) == 4 && key_value(line).is_some_and(|(key, _)| key == "timeout-minutes")
            {
                key_value(line).and_then(|(_, value)| value.parse::<u32>().ok())
            } else {
                None
            }
        });
        match timeout {
            Some(value) if value <= kind.timeout_limit() => {}
            Some(value) => errors.push(format!(
                "{name}: YAML path jobs.{job_name}.timeout-minutes: {value} exceeds {} minutes",
                kind.timeout_limit()
            )),
            None => errors.push(format!(
                "{name}: YAML path jobs.{job_name}.timeout-minutes: required and must be bounded"
            )),
        }
        if !body.iter().any(|line| {
            indent(line) == 4 && key_value(line).is_some_and(|(key, _)| key == "runs-on")
        }) {
            errors.push(format!(
                "{name}: YAML path jobs.{job_name}.runs-on: runner label is required"
            ));
        }
        if !summary_job {
            let budget = format!("scripts/with-validation-budget.sh {}", kind.shell_budget());
            if !body.iter().any(|line| {
                line.contains("shell:") && line.contains(&budget) && line.contains("{0}")
            }) {
                errors.push(format!(
                    "{name}: YAML path jobs.{job_name}.defaults.run.shell: must use {budget} ... {{0}}"
                ));
            }
        }
    }
}

fn validate_actions(name: &str, lines: &[&str], errors: &mut Vec<String>) {
    let mut checkout_needs_credentials = false;
    for line in lines {
        let trimmed = line.trim();
        let action_entry = trimmed.strip_prefix("- ").unwrap_or(trimmed);
        if let Some(uses) = action_entry.strip_prefix("uses:") {
            let uses = uses.split_whitespace().next().unwrap_or_default();
            let Some((action, sha)) = uses.rsplit_once('@') else {
                errors.push(format!(
                    "{name}: YAML path uses: action must be pinned by full commit SHA"
                ));
                continue;
            };
            if sha.len() != 40 || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                errors.push(format!(
                    "{name}: YAML path uses: {action} is not pinned by full commit SHA"
                ));
            }
            if !line.contains('#') || !line[line.find('#').unwrap_or(line.len())..].contains('v') {
                errors.push(format!(
                    "{name}: YAML path uses: {action} SHA requires a version comment"
                ));
            }
            if action == "actions/checkout" {
                checkout_needs_credentials = true;
            }
        } else if checkout_needs_credentials && trimmed.starts_with("persist-credentials:") {
            if trimmed != "persist-credentials: false" {
                errors.push(format!(
                    "{name}: YAML path steps.checkout.persist-credentials: must be false"
                ));
            }
            checkout_needs_credentials = false;
        } else if checkout_needs_credentials
            && !trimmed.is_empty()
            && !trimmed.starts_with('#')
            && indent(line) <= 6
        {
            errors.push(format!(
                "{name}: YAML path steps.checkout.persist-credentials: must be false"
            ));
            checkout_needs_credentials = false;
        }
    }
    if checkout_needs_credentials {
        errors.push(format!(
            "{name}: YAML path steps.checkout.persist-credentials: must be false"
        ));
    }
}

fn validate_prohibited_content(name: &str, lines: &[&str], errors: &mut Vec<String>) {
    let prohibited = [
        ("workflow_dispatch", "manual dispatch"),
        ("workflow_run", "workflow_run event"),
        (
            "pull_request_target",
            "privileged pull_request_target event",
        ),
        ("docker:", "Docker job/container"),
        ("secrets.", "secret reference"),
        ("permissions: write", "write permission"),
        (
            "actions/cache@",
            "use the cache policy's pinned action only",
        ),
    ];
    for (needle, replacement) in prohibited {
        if lines.iter().any(|line| line.contains(needle)) && needle != "actions/cache@" {
            errors.push(format!("{name}: prohibited {needle}; use {replacement}"));
        }
    }
    if lines
        .iter()
        .any(|line| line.contains("github.event.pull_request.head"))
    {
        errors.push(format!(
            "{name}: fork-controlled expression must not execute in workflow commands"
        ));
    }
}

fn section_start(lines: &[&str], key: &str) -> Option<usize> {
    lines.iter().position(|line| {
        indent(line) == 0 && key_value(line).is_some_and(|(candidate, _)| candidate == key)
    })
}

fn section_end(lines: &[&str], start: usize) -> usize {
    section_end_from(lines, start, lines.len())
}

fn section_end_from(lines: &[&str], start: usize, limit: usize) -> usize {
    lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .take(limit.saturating_sub(start + 1))
        .find_map(|(index, line)| (indent(line) == 0 && !line.trim().is_empty()).then_some(index))
        .unwrap_or(limit)
}

fn key_value(line: &str) -> Option<(&str, &str)> {
    line.trim()
        .split_once(':')
        .map(|(key, value)| (key.trim(), value.trim()))
}

fn indent(line: &str) -> usize {
    line.bytes().take_while(|byte| *byte == b' ').count()
}

#[cfg(test)]
mod tests {
    use super::{WorkflowKind, validate_workflow};

    const VALID: &str = "name: RustTable PR\non:\n  pull_request:\npermissions:\n  contents: read\njobs:\n  validate:\n    runs-on: ubuntu-latest\n    timeout-minutes: 3\n    defaults:\n      run:\n        shell: bash scripts/with-validation-budget.sh 150 workflow-step {0}\n    steps:\n      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4.2.2\n        with:\n          persist-credentials: false\n";

    #[test]
    fn accepts_the_minimal_pr_contract() {
        validate_workflow("rust-pr.yml", WorkflowKind::PullRequest, VALID).expect("valid");
    }

    #[test]
    fn accepts_an_always_run_summary_job_without_a_shell_budget() {
        let valid = "name: RustTable PR\non:\n  pull_request:\npermissions:\n  contents: read\njobs:\n  validate-groups:\n    runs-on: ubuntu-latest\n    timeout-minutes: 3\n    defaults:\n      run:\n        shell: bash scripts/with-validation-budget.sh 150 workflow-step {0}\n  validate:\n    if: ${{ always() }}\n    needs: validate-groups\n    runs-on: ubuntu-latest\n    timeout-minutes: 1\n";
        validate_workflow("rust-pr.yml", WorkflowKind::PullRequest, valid).expect("valid");
    }

    #[test]
    fn pr_workflow_keeps_workflow_inventory_inside_pr_validation() {
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../.github/workflows/rust-pr.yml"
        ));
        assert!(source.contains("run: bash scripts/pr-ci.sh"));
        assert!(!source.contains("cargo xtask repo verify-workflows"));
    }

    #[test]
    fn rejects_extra_events_and_floating_actions() {
        let invalid = VALID
            .replace("  pull_request:", "  pull_request:\n  workflow_dispatch:")
            .replace("@11bd71901bbe5b1630ceea73d27597364c9af683", "@v4");
        let error = validate_workflow("fixture.yaml", WorkflowKind::PullRequest, &invalid)
            .expect_err("invalid workflow");
        assert!(error.contains("workflow_dispatch"));
        assert!(error.contains("not pinned by full commit SHA"));
    }

    #[test]
    fn rejects_write_permissions_and_unbounded_jobs() {
        let invalid = VALID
            .replace("contents: read", "contents: write")
            .replace("timeout-minutes: 3", "timeout-minutes: 11");
        let error = validate_workflow("fixture.yml", WorkflowKind::PullRequest, &invalid)
            .expect_err("invalid workflow");
        assert!(error.contains("permissions"));
        assert!(error.contains("timeout-minutes"));
    }
}
