use super::{Result, report};
use crate::cli::LuaConformanceArgs;
use crate::root::RepositoryRoot;

pub(super) fn run(root: &RepositoryRoot, arguments: &LuaConformanceArgs) -> Result {
    let receipts = rusttable_scripting::conformance::run_fixtures(
        root.path(),
        rusttable_scripting::conformance::ConformanceOptions {
            all_fixtures: arguments.all_fixtures,
            verify_isolation: arguments.verify_isolation,
            verify_limits: arguments.verify_limits,
            verify_events: arguments.verify_events,
        },
    )
    .map_err(|error| error.to_string())?;
    Ok(report(
        root,
        "lua-conformance",
        serde_json::json!({
            "schema_version": 1,
            "receipts": receipts,
            "verified": {
                "isolation": arguments.verify_isolation,
                "limits": arguments.verify_limits,
                "events": arguments.verify_events,
            },
        }),
    ))
}
