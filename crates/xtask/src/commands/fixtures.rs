use std::fs;

use super::{Result, report};
use crate::cli::FixturesCommand;
use crate::root::RepositoryRoot;

pub(super) fn run(root: &RepositoryRoot, command: &FixturesCommand) -> Result {
    let arguments = match command {
        FixturesCommand::Verify(arguments)
        | FixturesCommand::List(arguments)
        | FixturesCommand::ScrubReport(arguments) => arguments,
    };
    let manifest_path = root.join(&arguments.manifest);
    let source = fs::read_to_string(&manifest_path).map_err(|error| error.to_string())?;
    let manifest = rusttable_testkit::fixtures::FixtureManifest::parse(&source)
        .map_err(|error| error.to_string())?;
    let repository = rusttable_testkit::fixtures::FixtureRepository::new(root.path(), manifest)
        .map_err(|error| error.to_string())?;
    match command {
        FixturesCommand::Verify(_) => {
            let verified = repository.verify().map_err(|error| error.to_string())?;
            Ok(report(
                root,
                "fixtures.verify",
                serde_json::json!({ "fixtures": verified.fixtures().len(), "total_bytes": verified.total_bytes() }),
            ))
        }
        FixturesCommand::List(_) => Ok(report(
            root,
            "fixtures.list",
            serde_json::json!({
                "fixtures": repository.list().into_iter().map(|entry| serde_json::json!({
                    "id": entry.id,
                    "path": entry.path,
                    "size": entry.size,
                })).collect::<Vec<_>>(),
            }),
        )),
        FixturesCommand::ScrubReport(_) => {
            let scrub = repository
                .scrub_report()
                .map_err(|error| error.to_string())?;
            Ok(report(
                root,
                "fixtures.scrub-report",
                serde_json::json!({
                    "fixtures": scrub.fixtures.into_iter().map(|fixture| serde_json::json!({
                        "id": fixture.id,
                        "finding_count": fixture.report.findings().len(),
                    })).collect::<Vec<_>>(),
                }),
            ))
        }
    }
}
