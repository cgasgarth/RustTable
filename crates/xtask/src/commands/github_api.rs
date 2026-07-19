use std::fs;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use super::{
    COMMIT_PAGE_SIZE, ISSUE_PAGE_SIZE, IssueSnapshot, MAX_COMMIT_PAGES, MAX_ISSUE_PAGES, ReadApi,
    Result, TARGET_REPOSITORY, WriteApi,
};
use crate::process::{
    EnvironmentProfile, NetworkPolicy, ProcessLimits, ProcessRequest, ProcessRunner,
};

pub(crate) struct FixtureApi {
    issue: IssueSnapshot,
    issues: Vec<IssueSnapshot>,
    commit_pages: Vec<Vec<String>>,
}

impl FixtureApi {
    pub(crate) fn read(path: &Path) -> Result<Self, String> {
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

pub(crate) struct GitHubApi<'a> {
    runner: &'a ProcessRunner,
    base_url: String,
    token: String,
}

impl<'a> GitHubApi<'a> {
    pub(crate) fn from_environment(runner: &'a ProcessRunner) -> Result<Self, String> {
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
        .profile(EnvironmentProfile::GitHubApi)
        .network(NetworkPolicy::Read)
        // Keep the complete header as the effective argument while marking
        // the argument secret for receipts; replacing it with the bare token
        // would make curl interpret the token as a malformed header.
        .secret_arg(7, authorization)
        .limits(ProcessLimits {
            // Child-issue pages include full review bodies; keep the bound
            // finite but large enough that a valid page cannot be truncated
            // before JSON parsing.
            max_stdout_bytes: 4 * 1024 * 1024,
            max_stderr_bytes: 16 * 1024,
            timeout: Some(Duration::from_secs(20)),
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

    fn mutate_json(&self, method: &str, path: &str, payload: &Value) -> Result<Value, String> {
        let url = format!("{}{}", self.base_url.trim_end_matches('/'), path);
        let authorization = format!("Authorization: Bearer {}", self.token);
        let body = serde_json::to_string(&payload)
            .map_err(|error| format!("GitHub API request serialization failed: {error}"))?;
        let request = ProcessRequest::new(
            "curl",
            [
                "--fail-with-body",
                "--silent",
                "--show-error",
                "--location",
                "--request",
                method,
                "--header",
                "Accept: application/vnd.github+json",
                "--header",
                authorization.as_str(),
                "--header",
                "Content-Type: application/json",
                "--data-raw",
                body.as_str(),
                url.as_str(),
            ],
        )
        .profile(EnvironmentProfile::GitHubApi)
        .network(NetworkPolicy::Write)
        .secret_arg(9, authorization)
        .limits(ProcessLimits {
            max_stdout_bytes: 128 * 1024,
            max_stderr_bytes: 16 * 1024,
            timeout: Some(Duration::from_secs(20)),
        });
        let result = self
            .runner
            .run(request)
            .map_err(|_| "GitHub API infrastructure failure: request could not start".to_owned())?;
        if !result.receipt.success() {
            return Err("GitHub API infrastructure failure: mutation request failed".to_owned());
        }
        serde_json::from_slice(&result.stdout).map_err(|_| {
            "GitHub API infrastructure failure: mutation response was not valid JSON".to_owned()
        })
    }
}

impl WriteApi for GitHubApi<'_> {
    fn update_issue(&self, repository: &str, number: u64, payload: Value) -> Result<(), String> {
        self.mutate_json(
            "PATCH",
            &format!("/repos/{repository}/issues/{number}"),
            &payload,
        )
        .map(|_| ())
    }

    fn create_issue(&self, repository: &str, payload: Value) -> Result<(), String> {
        self.mutate_json("POST", &format!("/repos/{repository}/issues"), &payload)
            .map(|_| ())
    }
}

pub(crate) fn apply_issue_spec_updates(
    runner: &ProcessRunner,
    updates: &[(u64, String)],
) -> Result<(), String> {
    let api = GitHubApi::from_environment(runner)?;
    for (issue, body) in updates {
        api.update_issue(TARGET_REPOSITORY, *issue, serde_json::json!({"body": body}))?;
    }
    Ok(())
}

pub(crate) fn issue_spec_values(runner: &ProcessRunner) -> Result<Vec<Value>, String> {
    let api = GitHubApi::from_environment(runner)?;
    let mut issues = Vec::new();
    for page in 1..=MAX_ISSUE_PAGES {
        let value = api.get_json(&format!(
            "/repos/{TARGET_REPOSITORY}/issues?state=all&per_page={ISSUE_PAGE_SIZE}&page={page}"
        ))?;
        let page_items = value.as_array().ok_or_else(|| {
            "GitHub API infrastructure failure: issues response was not an array".to_owned()
        })?;
        let page_len = page_items.len();
        issues.extend(page_items.iter().cloned());
        if page_len < ISSUE_PAGE_SIZE {
            return Ok(issues);
        }
    }
    Err("GitHub API infrastructure failure: issue pagination exceeded safety limit".to_owned())
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
                .map(|commit| {
                    super::required_string(commit, &["commit", "message"], "commit.message")
                })
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
