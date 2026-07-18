use super::report;
use crate::cli::RepoCommand;
use crate::root::RepositoryRoot;

pub(super) fn run(root: &RepositoryRoot, command: &RepoCommand) -> crate::output::Report {
    let name = match command {
        RepoCommand::Dag => "repo.verify-dag",
        RepoCommand::Files => "repo.verify-files",
        RepoCommand::Workflows => "repo.verify-workflows",
    };
    report(
        root,
        name,
        serde_json::json!({ "placeholder": true, "message": "typed repository API pending its issue" }),
    )
}
