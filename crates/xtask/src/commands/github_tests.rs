use super::*;

struct MockApi {
    issue: Result<IssueSnapshot, String>,
    pages: Result<Vec<Vec<String>>, String>,
}

impl ReadApi for MockApi {
    fn issue(&self, _repository: &str, _number: u64) -> Result<IssueSnapshot, String> {
        self.issue.clone()
    }

    fn issues(&self, _repository: &str) -> Result<Vec<IssueSnapshot>, String> {
        Ok(vec![self.issue.clone()?])
    }

    fn commit_pages(&self, _repository: &str, _number: u64) -> Result<Vec<Vec<String>>, String> {
        self.pages.clone()
    }
}

fn event(body: &str) -> PullRequestEvent {
    PullRequestEvent {
        repository: TARGET_REPOSITORY.to_owned(),
        number: 501,
        title: "Enforce one open issue per pull request".to_owned(),
        body: body.to_owned(),
        branch: "issue-171-pr-contract".to_owned(),
        base_repository: TARGET_REPOSITORY.to_owned(),
    }
}

fn issue() -> IssueSnapshot {
    IssueSnapshot {
        number: 171,
        state: "open".to_owned(),
        state_reason: None,
        title: "Enforce one open issue per pull request".to_owned(),
        body: "Parent: #158\nDetails".to_owned(),
        repository: TARGET_REPOSITORY.to_owned(),
        milestone: Some(1),
        is_pull_request: false,
        labels: vec!["enhancement".to_owned(), "priority: P0".to_owned()],
    }
}

fn valid_body() -> &'static str {
    "## Why\nA focused contract.\n\n## Implementation\nPure validation.\n\n## Validation\nTests.\n\n## Out of scope\nAPI writes.\n\nCloses #171"
}

#[test]
fn accepts_a_conforming_fixture() {
    let api = MockApi {
        issue: Ok(issue()),
        pages: Ok(vec![vec!["feat: contract".to_owned()]]),
    };
    let receipt = verify_contract(&event(valid_body()), &api).expect("valid contract");
    assert_eq!(receipt.issue_number, 171);
    assert_eq!(receipt.title, "Enforce one open issue per pull request");
}

#[test]
fn accepts_paginated_commit_results() {
    let api = MockApi {
        issue: Ok(issue()),
        pages: Ok(vec![
            vec!["feat: first".to_owned()],
            vec!["test: second".to_owned()],
        ]),
    };
    let receipt = verify_contract(&event(valid_body()), &api).expect("valid pagination");
    assert_eq!(receipt.commit_pages, 2);
}

#[test]
fn rejects_missing_issue_as_an_infrastructure_failure() {
    let api = MockApi {
        issue: Err("GitHub API infrastructure failure: issue lookup failed".to_owned()),
        pages: Ok(Vec::new()),
    };
    let error = verify_contract(&event(valid_body()), &api).expect_err("missing issue");
    assert!(error.contains("infrastructure failure"));
}

#[test]
fn rejects_additional_commit_reference() {
    let api = MockApi {
        issue: Ok(issue()),
        pages: Ok(vec![vec!["fixes #99".to_owned()]]),
    };
    let error = verify_contract(&event(valid_body()), &api).expect_err("extra reference");
    assert!(error.contains("additional"));
}

#[test]
fn rejects_malformed_body_and_closed_issue() {
    let api = MockApi {
        issue: Ok(IssueSnapshot {
            state: "closed".to_owned(),
            ..issue()
        }),
        pages: Ok(Vec::new()),
    };
    let error = verify_contract(&event("Closes #171"), &api).expect_err("invalid body");
    assert!(error.contains("state") || error.contains("required section"));
}
