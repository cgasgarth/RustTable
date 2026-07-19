use std::fs;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use super::report;
use crate::cli::{GithubCommand, VerifyPrContractArgs, VerifyQueueArgs};
use crate::process::{ProcessLimits, ProcessRequest, ProcessRunner};
use crate::root::RepositoryRoot;

const TARGET_REPOSITORY: &str = "cgasgarth/RustTable";
const COMMIT_PAGE_SIZE: usize = 100;
const MAX_COMMIT_PAGES: u32 = 20;
const MAX_ISSUE_PAGES: u32 = 20;
const ISSUE_PAGE_SIZE: usize = 25;
const PRIORITY_LABELS: [&str; 5] = [
    "priority: P0",
    "priority: P1",
    "priority: P2",
    "priority: P3",
    "priority: P4",
];
const CLOSING_KEYWORDS: [&str; 9] = [
    "close", "closes", "closed", "fix", "fixes", "fixed", "resolve", "resolves", "resolved",
];

type Result<T = crate::output::Report, E = String> = std::result::Result<T, E>;

pub(super) fn run(
    root: &RepositoryRoot,
    command: &GithubCommand,
    runner: &ProcessRunner,
) -> Result {
    match command {
        GithubCommand::VerifyPrContract(arguments) => verify_pr_contract(root, arguments, runner),
        GithubCommand::VerifyQueue(arguments) => verify_queue(root, arguments, runner),
    }
}

fn verify_pr_contract(
    root: &RepositoryRoot,
    arguments: &VerifyPrContractArgs,
    runner: &ProcessRunner,
) -> Result {
    let event_path = root.join(&arguments.event);
    let event_source = fs::read_to_string(&event_path)
        .map_err(|error| format!("event payload {}: {error}", event_path.display()))?;
    let event = PullRequestEvent::parse(&event_source)?;
    let api: Box<dyn ReadApi> = match &arguments.api_fixture {
        Some(path) => Box::new(FixtureApi::read(&root.join(path))?),
        None => Box::new(GitHubApi::from_environment(runner)?),
    };
    let receipt = verify_contract(&event, api.as_ref())?;
    Ok(report(
        root,
        "github.verify-pr-contract",
        serde_json::json!({
            "repository": event.repository,
            "pull_request": event.number,
            "issue": receipt.issue_number,
            "title": receipt.title,
            "priority": receipt.priority,
            "branch": event.branch,
            "commit_pages": receipt.commit_pages,
        }),
    ))
}

fn verify_queue(
    root: &RepositoryRoot,
    arguments: &VerifyQueueArgs,
    runner: &ProcessRunner,
) -> Result {
    let api: Box<dyn ReadApi> = match &arguments.api_fixture {
        Some(path) => Box::new(FixtureApi::read(&root.join(path))?),
        None => Box::new(GitHubApi::from_environment(runner)?),
    };
    let receipt = build_queue_receipt(api.as_ref())?;
    Ok(report(
        root,
        "github.verify-queue",
        serde_json::to_value(receipt).map_err(|error| format!("queue receipt: {error}"))?,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PullRequestEvent {
    repository: String,
    number: u64,
    title: String,
    body: String,
    branch: String,
    base_repository: String,
}

impl PullRequestEvent {
    fn parse(source: &str) -> Result<Self, String> {
        let value: Value = serde_json::from_str(source)
            .map_err(|error| format!("event payload: malformed JSON: {error}"))?;
        let repository =
            required_string(&value, &["repository", "full_name"], "repository.full_name")?;
        let pull_request = value
            .get("pull_request")
            .ok_or_else(|| "event payload: pull_request: required object is missing".to_owned())?;
        let number = required_u64(pull_request, &["number"], "pull_request.number")?;
        let title = required_string(pull_request, &["title"], "pull_request.title")?;
        let body = optional_string(pull_request, &["body"]).unwrap_or_default();
        let branch = required_string(pull_request, &["head", "ref"], "pull_request.head.ref")?;
        let base_repository = required_string(
            pull_request,
            &["base", "repo", "full_name"],
            "pull_request.base.repo.full_name",
        )?;
        Ok(Self {
            repository,
            number,
            title,
            body,
            branch,
            base_repository,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IssueSnapshot {
    number: u64,
    state: String,
    state_reason: Option<String>,
    title: String,
    body: String,
    repository: String,
    milestone: Option<u64>,
    is_pull_request: bool,
    labels: Vec<String>,
}

impl IssueSnapshot {
    fn parse(value: &Value) -> Result<Self, String> {
        let number = required_u64(value, &["number"], "issue.number")?;
        let state = required_string(value, &["state"], "issue.state")?;
        let title = required_string(value, &["title"], "issue.title")?;
        let body = optional_string(value, &["body"]).unwrap_or_default();
        let repository = repository_name(value).ok_or_else(|| {
            "GitHub API issue response: repository identity is missing".to_owned()
        })?;
        let milestone = value
            .get("milestone")
            .and_then(|milestone| milestone.get("number"))
            .and_then(Value::as_u64);
        let is_pull_request = value.get("pull_request").is_some();
        let labels = value
            .get("labels")
            .and_then(Value::as_array)
            .map(|labels| {
                labels
                    .iter()
                    .filter_map(|label| label.get("name").and_then(Value::as_str))
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            number,
            state,
            state_reason: optional_string(value, &["state_reason"]),
            title,
            body,
            repository,
            milestone,
            is_pull_request,
            labels,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ContractReceipt {
    issue_number: u64,
    title: String,
    priority: String,
    commit_pages: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct QueueEntry {
    issue: u64,
    title: String,
    milestone: Option<u64>,
    priority: String,
    depends_on: Vec<u64>,
    ready: bool,
    selected: bool,
    blocking_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct QueueReceipt {
    repository: String,
    parent_issue: u64,
    selected_issue: Option<u64>,
    issues: Vec<QueueEntry>,
}

struct ParsedQueueIssue {
    issue: IssueSnapshot,
    title: String,
    priority: String,
    depends_on: Vec<u64>,
}

trait ReadApi {
    fn issue(&self, repository: &str, number: u64) -> Result<IssueSnapshot, String>;
    fn issues(&self, repository: &str) -> Result<Vec<IssueSnapshot>, String>;
    fn commit_pages(&self, repository: &str, number: u64) -> Result<Vec<Vec<String>>, String>;
}

#[allow(clippy::too_many_lines)]
fn build_queue_receipt(api: &dyn ReadApi) -> Result<QueueReceipt, String> {
    let issues = api.issues(TARGET_REPOSITORY)?;
    let mut by_number = std::collections::BTreeMap::new();
    for issue in issues {
        if issue.repository != TARGET_REPOSITORY {
            continue;
        }
        if by_number.insert(issue.number, issue).is_some() {
            return Err("queue: duplicate issue number in API response".to_owned());
        }
    }
    let parsed = parse_queue_issues(
        by_number.values().filter(|issue| {
            issue.state == "open" && !issue.is_pull_request && is_child_issue(issue)
        }),
        &by_number,
    )?;
    detect_dependency_cycles(&parsed, &by_number)?;
    let readiness = parsed
        .iter()
        .map(|item| {
            (
                item.issue.number,
                dependency_blockers(&item.depends_on, &by_number),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let selected_issue = parsed
        .iter()
        .filter(|item| readiness[&item.issue.number].is_empty())
        .min_by(|left, right| {
            priority_rank(&left.priority)
                .cmp(&priority_rank(&right.priority))
                .then_with(|| left.issue.number.cmp(&right.issue.number))
        })
        .map(|item| item.issue.number);
    let ready_p0 = parsed
        .iter()
        .filter(|item| item.priority == "priority: P0" && readiness[&item.issue.number].is_empty())
        .map(|item| item.issue.number)
        .min();
    let entries = build_queue_entries(&parsed, &readiness, selected_issue, ready_p0);
    Ok(QueueReceipt {
        repository: TARGET_REPOSITORY.to_owned(),
        parent_issue: 158,
        selected_issue,
        issues: entries,
    })
}

fn parse_queue_issues<'a, I>(
    issues: I,
    by_number: &std::collections::BTreeMap<u64, IssueSnapshot>,
) -> Result<Vec<ParsedQueueIssue>, String>
where
    I: IntoIterator<Item = &'a IssueSnapshot>,
{
    let mut normalized_titles = std::collections::BTreeMap::<String, u64>::new();
    let mut parsed = Vec::new();
    for issue in issues {
        let priority = priority_label(issue)?.to_owned();
        if let Some(prefix) = numeric_title_prefix(&issue.title) {
            return Err(format!(
                "issue #{}: numeric title prefix {prefix} is prohibited",
                issue.number
            ));
        }
        let title = normalize_outcome_title(&issue.title);
        if let Some(previous) = normalized_titles.insert(title.to_ascii_lowercase(), issue.number) {
            return Err(format!(
                "queue: duplicate active outcome title between #{} and #{}",
                previous, issue.number
            ));
        }
        let depends_on = parse_dependencies(issue)?;
        for dependency in &depends_on {
            if !by_number.contains_key(dependency) {
                return Err(format!(
                    "issue #{}: dependency #{} is outside the repository",
                    issue.number, dependency
                ));
            }
        }
        parsed.push(ParsedQueueIssue {
            issue: issue.clone(),
            title,
            priority,
            depends_on,
        });
    }
    Ok(parsed)
}

fn build_queue_entries(
    parsed: &[ParsedQueueIssue],
    readiness: &std::collections::BTreeMap<u64, Vec<String>>,
    selected_issue: Option<u64>,
    ready_p0: Option<u64>,
) -> Vec<QueueEntry> {
    let mut entries = parsed
        .iter()
        .map(|item| {
            let blockers = &readiness[&item.issue.number];
            let ready = blockers.is_empty();
            let selected = selected_issue == Some(item.issue.number);
            let blocking_reason = if !ready {
                Some(blockers.join(", "))
            } else if selected {
                None
            } else if let Some(number) = ready_p0 {
                Some(format!("lower priority than ready P0 issue #{number}"))
            } else {
                Some("lower priority than the selected ready issue".to_owned())
            };
            QueueEntry {
                issue: item.issue.number,
                title: item.title.clone(),
                milestone: item.issue.milestone,
                priority: item.priority.clone(),
                depends_on: item.depends_on.clone(),
                ready,
                selected,
                blocking_reason,
            }
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        priority_rank(&left.priority)
            .cmp(&priority_rank(&right.priority))
            .then_with(|| left.issue.cmp(&right.issue))
    });
    entries
}

fn verify_contract(event: &PullRequestEvent, api: &dyn ReadApi) -> Result<ContractReceipt, String> {
    if event.repository != TARGET_REPOSITORY || event.base_repository != TARGET_REPOSITORY {
        return Err(format!(
            "pull request repository: expected {TARGET_REPOSITORY}, found {} / {}",
            event.repository, event.base_repository
        ));
    }
    let closing_numbers = event
        .body
        .lines()
        .filter_map(parse_canonical_closing_line)
        .collect::<Vec<_>>();
    if closing_numbers.len() != 1 {
        return Err(format!(
            "pull request body: expected exactly one Closes #<number> line, found {}",
            closing_numbers.len()
        ));
    }
    let issue_number = closing_numbers[0];
    let issue = api.issue(TARGET_REPOSITORY, issue_number)?;
    validate_issue(&issue, issue_number)?;
    let normalized_title = normalize_outcome_title(&issue.title);
    if numeric_title_prefix(&event.title).is_some()
        || !normalize_outcome_title(&event.title).starts_with(&normalized_title)
    {
        return Err(format!(
            "pull request title: expected normalized issue outcome {normalized_title:?} without a numeric prefix"
        ));
    }
    if !valid_branch(&event.branch, issue_number) {
        return Err(format!(
            "pull request head branch: expected issue-{issue_number}-<slug>, found {}",
            event.branch
        ));
    }
    validate_sections(&event.body)?;
    let canonical_line = format!("Closes #{issue_number}");
    let body_without_canonical = event
        .body
        .lines()
        .filter(|line| line.trim() != canonical_line)
        .collect::<Vec<_>>()
        .join("\n");
    if find_issue_references(&body_without_canonical)
        .next()
        .is_some()
    {
        return Err(
            "pull request body: only the canonical Closes line may contain a closing issue reference"
                .to_owned(),
        );
    }
    let pages = api.commit_pages(TARGET_REPOSITORY, event.number)?;
    for message in pages.iter().flatten() {
        if find_issue_references(message).next().is_some() {
            return Err(
                "pull request commits: an additional close/fix/resolve issue reference was found"
                    .to_owned(),
            );
        }
    }
    Ok(ContractReceipt {
        issue_number,
        title: normalized_title,
        priority: priority_label(&issue)?.to_owned(),
        commit_pages: pages.len(),
    })
}

fn validate_issue(issue: &IssueSnapshot, expected_number: u64) -> Result<(), String> {
    if issue.number != expected_number {
        return Err(format!(
            "issue reference: API returned #{}, requested #{expected_number}",
            issue.number
        ));
    }
    if issue.repository != TARGET_REPOSITORY {
        return Err(format!(
            "issue #{expected_number}: repository must be {TARGET_REPOSITORY}, found {}",
            issue.repository
        ));
    }
    if issue.is_pull_request {
        return Err(format!(
            "issue #{expected_number}: pull requests cannot be the closing item"
        ));
    }
    if issue.state != "open" {
        return Err(format!(
            "issue #{expected_number}: state must be open, found {}",
            issue.state
        ));
    }
    if numeric_title_prefix(&issue.title).is_some() {
        return Err(format!(
            "issue #{expected_number}: title must not begin with a numeric sequence prefix"
        ));
    }
    if issue.milestone.is_none() {
        return Err(format!("issue #{expected_number}: milestone is required"));
    }
    if !issue.body.lines().any(|line| line.trim() == "Parent: #158") {
        return Err(format!(
            "issue #{expected_number}: parent reference Parent: #158 is required"
        ));
    }
    priority_label(issue).map(|_| ())?;
    Ok(())
}

fn validate_sections(body: &str) -> Result<(), String> {
    for section in ["Why", "Implementation", "Validation", "Out of scope"] {
        let marker = format!("## {section}");
        if !body.lines().any(|line| line.trim() == marker) {
            return Err(format!(
                "pull request body: required section ## {section} is missing"
            ));
        }
    }
    Ok(())
}

fn priority_label(issue: &IssueSnapshot) -> Result<&str, String> {
    let priority_labels = issue
        .labels
        .iter()
        .filter(|label| label.starts_with("priority:"))
        .collect::<Vec<_>>();
    if priority_labels.len() != 1 || !PRIORITY_LABELS.contains(&priority_labels[0].as_str()) {
        return Err(format!(
            "issue #{}: expected exactly one priority label (priority: P0 through priority: P4)",
            issue.number
        ));
    }
    Ok(priority_labels[0].as_str())
}

fn normalize_outcome_title(title: &str) -> String {
    title.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn numeric_title_prefix(title: &str) -> Option<String> {
    let (prefix, _) = title.trim_start().split_once(']')?;
    let prefix = prefix.strip_prefix('[')?;
    let valid = (prefix.len() == 4 && prefix.bytes().all(|byte| byte.is_ascii_digit()))
        || (prefix.len() == 5
            && prefix[..4].bytes().all(|byte| byte.is_ascii_digit())
            && prefix.as_bytes()[4].is_ascii_uppercase());
    valid.then(|| format!("[{prefix}]"))
}

fn is_child_issue(issue: &IssueSnapshot) -> bool {
    issue.body.lines().any(|line| line.trim() == "Parent: #158")
}

fn priority_rank(priority: &str) -> usize {
    PRIORITY_LABELS
        .iter()
        .position(|label| *label == priority)
        .unwrap_or(PRIORITY_LABELS.len())
}

fn parse_dependencies(issue: &IssueSnapshot) -> Result<Vec<u64>, String> {
    let mut dependencies = Vec::new();
    for line in issue.body.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("Depends on") else {
            continue;
        };
        if !rest.starts_with(char::is_whitespace) {
            continue;
        }
        if rest.contains('[') || rest.contains(']') {
            return Err(format!(
                "issue #{}: legacy sequence dependency is prohibited",
                issue.number
            ));
        }
        let bytes = rest.as_bytes();
        let mut index = 0;
        let mut found = false;
        while index < bytes.len() {
            if bytes[index] != b'#' {
                index += 1;
                continue;
            }
            if index > 0 && (bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'/') {
                return Err(format!(
                    "issue #{}: dependency must use a direct local #<issue> link",
                    issue.number
                ));
            }
            let start = index + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end == start {
                return Err(format!(
                    "issue #{}: malformed dependency link",
                    issue.number
                ));
            }
            let number = rest[start..end]
                .parse::<u64>()
                .map_err(|_| format!("issue #{}: dependency number is invalid", issue.number))?;
            dependencies.push(number);
            found = true;
            index = end;
        }
        if !found {
            return Err(format!(
                "issue #{}: dependency line must contain a direct #<issue> link",
                issue.number
            ));
        }
    }
    dependencies.sort_unstable();
    dependencies.dedup();
    Ok(dependencies)
}

fn dependency_blockers(
    dependencies: &[u64],
    by_number: &std::collections::BTreeMap<u64, IssueSnapshot>,
) -> Vec<String> {
    dependencies
        .iter()
        .filter_map(|dependency| {
            let target = by_number.get(dependency)?;
            if target.state == "open" {
                return Some(format!("depends on open #{dependency}"));
            }
            if matches!(
                target.state_reason.as_deref(),
                Some("not planned" | "duplicate")
            ) && completed_replacement(target, by_number).is_none()
            {
                return Some(format!(
                    "depends on #{dependency} closed as {}",
                    target.state_reason.as_deref().unwrap_or("unresolved")
                ));
            }
            None
        })
        .collect()
}

fn completed_replacement(
    issue: &IssueSnapshot,
    by_number: &std::collections::BTreeMap<u64, IssueSnapshot>,
) -> Option<u64> {
    issue
        .body
        .lines()
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("replacement") || lower.contains("superseded by")
        })
        .flat_map(|line| issue_numbers_in_text(line).into_iter())
        .find(|number| {
            by_number.get(number).is_some_and(|replacement| {
                replacement.state == "closed"
                    && replacement.state_reason.as_deref() == Some("completed")
            })
        })
}

fn issue_numbers_in_text(text: &str) -> Vec<u64> {
    text.split('#')
        .skip(1)
        .filter_map(|part| {
            let digits = part
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>();
            (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
        })
        .collect()
}

fn detect_dependency_cycles(
    parsed: &[ParsedQueueIssue],
    by_number: &std::collections::BTreeMap<u64, IssueSnapshot>,
) -> Result<(), String> {
    let dependencies = parsed
        .iter()
        .map(|item| (item.issue.number, item.depends_on.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut visiting = std::collections::BTreeSet::new();
    let mut visited = std::collections::BTreeSet::new();
    for item in parsed {
        visit_dependency(
            item.issue.number,
            &dependencies,
            by_number,
            &mut visiting,
            &mut visited,
        )?;
    }
    Ok(())
}

fn visit_dependency(
    issue: u64,
    dependencies: &std::collections::BTreeMap<u64, Vec<u64>>,
    by_number: &std::collections::BTreeMap<u64, IssueSnapshot>,
    visiting: &mut std::collections::BTreeSet<u64>,
    visited: &mut std::collections::BTreeSet<u64>,
) -> Result<(), String> {
    if visited.contains(&issue)
        || by_number
            .get(&issue)
            .is_some_and(|item| item.state == "closed")
    {
        return Ok(());
    }
    if !visiting.insert(issue) {
        return Err(format!("queue: dependency cycle includes #{issue}"));
    }
    if let Some(next) = dependencies.get(&issue) {
        for dependency in next {
            visit_dependency(*dependency, dependencies, by_number, visiting, visited)?;
        }
    }
    visiting.remove(&issue);
    visited.insert(issue);
    Ok(())
}

fn parse_canonical_closing_line(line: &str) -> Option<u64> {
    let suffix = line.trim().strip_prefix("Closes #")?;
    if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    suffix.parse().ok()
}

fn valid_branch(branch: &str, issue_number: u64) -> bool {
    let basename = branch.rsplit('/').next().unwrap_or(branch);
    let prefix = format!("issue-{issue_number}-");
    let Some(slug) = basename.strip_prefix(&prefix) else {
        return false;
    };
    !slug.is_empty()
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn find_issue_references(text: &str) -> impl Iterator<Item = (String, u64)> + '_ {
    let tokens = text
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '#')
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let mut references = Vec::new();
    for window in tokens.windows(2) {
        let keyword = window[0].to_ascii_lowercase();
        if !CLOSING_KEYWORDS.contains(&keyword.as_str()) {
            continue;
        }
        let issue = window[1].strip_prefix('#').unwrap_or_default();
        if issue.is_empty() || !issue.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        if let Ok(number) = issue.parse() {
            references.push((keyword, number));
        }
    }
    references.into_iter()
}

fn required_string(value: &Value, path: &[&str], label: &str) -> Result<String, String> {
    walk(value, path)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{label}: required non-empty string is missing"))
}

fn optional_string(value: &Value, path: &[&str]) -> Option<String> {
    walk(value, path)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn required_u64(value: &Value, path: &[&str], label: &str) -> Result<u64, String> {
    walk(value, path)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{label}: required integer is missing"))
}

fn walk<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
}

fn repository_name(value: &Value) -> Option<String> {
    walk(value, &["repository", "full_name"])
        .and_then(Value::as_str)
        .or_else(|| value.get("repository").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .or_else(|| {
            value
                .get("repository_url")
                .and_then(Value::as_str)
                .and_then(|url| url.strip_prefix("https://api.github.com/repos/"))
                .map(ToOwned::to_owned)
        })
}

struct FixtureApi {
    issue: IssueSnapshot,
    issues: Vec<IssueSnapshot>,
    commit_pages: Vec<Vec<String>>,
}

impl FixtureApi {
    fn read(path: &Path) -> Result<Self, String> {
        let source = fs::read_to_string(path)
            .map_err(|error| format!("API fixture {}: {error}", path.display()))?;
        let value: Value = serde_json::from_str(&source)
            .map_err(|error| format!("API fixture: malformed JSON: {error}"))?;
        let issue = IssueSnapshot::parse(
            value
                .get("issue")
                .ok_or_else(|| "API fixture: issue object is missing".to_owned())?,
        )?;
        let issues = value
            .get("issues")
            .and_then(Value::as_array)
            .map(|issues| {
                issues
                    .iter()
                    .map(IssueSnapshot::parse)
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        let commit_pages = value
            .get("commit_pages")
            .and_then(Value::as_array)
            .ok_or_else(|| "API fixture: commit_pages array is missing".to_owned())?
            .iter()
            .map(|page| {
                page.as_array()
                    .ok_or_else(|| "API fixture: commit page must be an array".to_owned())?
                    .iter()
                    .map(|commit| {
                        commit
                            .as_str()
                            .or_else(|| commit.get("message").and_then(Value::as_str))
                            .map(ToOwned::to_owned)
                            .ok_or_else(|| "API fixture: commit message is missing".to_owned())
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            issue,
            issues,
            commit_pages,
        })
    }
}

impl ReadApi for FixtureApi {
    fn issue(&self, _repository: &str, _number: u64) -> Result<IssueSnapshot, String> {
        Ok(self.issue.clone())
    }

    fn issues(&self, _repository: &str) -> Result<Vec<IssueSnapshot>, String> {
        Ok(self.issues.clone())
    }

    fn commit_pages(&self, _repository: &str, _number: u64) -> Result<Vec<Vec<String>>, String> {
        Ok(self.commit_pages.clone())
    }
}

struct GitHubApi<'a> {
    runner: &'a ProcessRunner,
    base_url: String,
    token: String,
}

impl<'a> GitHubApi<'a> {
    fn from_environment(runner: &'a ProcessRunner) -> Result<Self, String> {
        let token = std::env::var("GH_TOKEN")
            .or_else(|_| std::env::var("GITHUB_TOKEN"))
            .map_err(|_| "GitHub API infrastructure failure: GH_TOKEN is missing".to_owned())?;
        let base_url =
            std::env::var("GITHUB_API_URL").unwrap_or_else(|_| "https://api.github.com".to_owned());
        Ok(Self {
            runner,
            base_url,
            token,
        })
    }

    fn get_json(&self, path: &str) -> Result<Value, String> {
        let url = format!("{}{}", self.base_url.trim_end_matches('/'), path);
        let authorization = format!("Authorization: Bearer {}", self.token);
        let request = ProcessRequest::new(
            "curl",
            [
                "--fail-with-body",
                "--silent",
                "--show-error",
                "--location",
                "--header",
                "Accept: application/vnd.github+json",
                "--header",
                authorization.as_str(),
                &url,
            ],
        )
        .limits(ProcessLimits {
            max_stdout_bytes: 512 * 1024,
            max_stderr_bytes: 16 * 1024,
            timeout: Duration::from_secs(20),
        });
        let result = self
            .runner
            .run(request)
            .map_err(|_| "GitHub API infrastructure failure: request could not start".to_owned())?;
        if !result.receipt.success() {
            return Err("GitHub API infrastructure failure: request failed".to_owned());
        }
        serde_json::from_slice(&result.stdout).map_err(|_| {
            "GitHub API infrastructure failure: response was not valid JSON".to_owned()
        })
    }
}

impl ReadApi for GitHubApi<'_> {
    fn issue(&self, repository: &str, number: u64) -> Result<IssueSnapshot, String> {
        let value = self.get_json(&format!("/repos/{repository}/issues/{number}"))?;
        IssueSnapshot::parse(&value)
    }

    fn issues(&self, repository: &str) -> Result<Vec<IssueSnapshot>, String> {
        let mut issues = Vec::new();
        for page in 1..=MAX_ISSUE_PAGES {
            let value = self.get_json(&format!(
                "/repos/{repository}/issues?state=all&per_page={ISSUE_PAGE_SIZE}&page={page}"
            ))?;
            let page_items = value.as_array().ok_or_else(|| {
                "GitHub API infrastructure failure: issues response was not an array".to_owned()
            })?;
            let page_len = page_items.len();
            issues.extend(
                page_items
                    .iter()
                    .map(IssueSnapshot::parse)
                    .collect::<Result<Vec<_>, _>>()?,
            );
            if page_len < ISSUE_PAGE_SIZE {
                return Ok(issues);
            }
        }
        Err("GitHub API infrastructure failure: issue pagination exceeded safety limit".to_owned())
    }

    fn commit_pages(&self, repository: &str, number: u64) -> Result<Vec<Vec<String>>, String> {
        let mut pages = Vec::new();
        for page in 1..=MAX_COMMIT_PAGES {
            let value = self.get_json(&format!(
                "/repos/{repository}/pulls/{number}/commits?per_page={COMMIT_PAGE_SIZE}&page={page}"
            ))?;
            let commits = value.as_array().ok_or_else(|| {
                "GitHub API infrastructure failure: commits response was not an array".to_owned()
            })?;
            let messages = commits
                .iter()
                .map(|commit| required_string(commit, &["commit", "message"], "commit.message"))
                .collect::<Result<Vec<_>, _>>()?;
            let page_len = messages.len();
            pages.push(messages);
            if page_len < COMMIT_PAGE_SIZE {
                return Ok(pages);
            }
        }
        Err("GitHub API infrastructure failure: commit pagination exceeded safety limit".to_owned())
    }
}

#[cfg(test)]
#[path = "github_tests.rs"]
mod tests;
