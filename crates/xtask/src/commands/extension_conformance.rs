use crate::{
    cli::ExtensionConformanceArgs,
    commands::{Result, report},
    root::RepositoryRoot,
};

pub(crate) fn run(root: &RepositoryRoot, arguments: &ExtensionConformanceArgs) -> Result {
    if !arguments.all_fixtures {
        return Err("extension conformance requires --all-fixtures".to_owned());
    }
    let receipt = rusttable_scripting::component::conformance::run_all(
        arguments.verify_isolation,
        arguments.verify_limits,
    )
    .map_err(|error| error.to_string())?;
    Ok(report(
        root,
        "extension-conformance",
        serde_json::to_value(receipt).map_err(|error| error.to_string())?,
    ))
}
