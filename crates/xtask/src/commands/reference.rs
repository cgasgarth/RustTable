use super::{Result, report};
use crate::cli::ReferenceCommand;
use crate::process::{ProcessRequest, ProcessRunner};
use crate::root::RepositoryRoot;

pub(super) fn run(
    root: &RepositoryRoot,
    command: &ReferenceCommand,
    runner: &ProcessRunner,
) -> Result {
    let (name, arguments) = match command {
        ReferenceCommand::Probe(arguments) => ("reference.probe", arguments),
        ReferenceCommand::Render(arguments) => ("reference.render", arguments),
    };
    let Some(executable) = &arguments.executable else {
        return Ok(report(
            root,
            name,
            serde_json::json!({ "placeholder": true, "message": "reference API pending its issue" }),
        ));
    };
    let result = runner
        .run(
            ProcessRequest::new(executable.display().to_string(), ["--version"])
                .current_dir(root.path()),
        )
        .map_err(|error| error.to_string())?;
    Ok(report(
        root,
        name,
        serde_json::json!({ "receipt": result.receipt }),
    ))
}
