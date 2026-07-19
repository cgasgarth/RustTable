use std::collections::BTreeMap;
use std::env;
use std::time::Duration;

use super::MetadataContext;
use crate::commands::Result;
use crate::process::{ProcessLimits, ProcessRequest, ProcessRunner};
use crate::root::RepositoryRoot;

const METADATA_TIMEOUT: Duration = Duration::from_secs(4);
const METADATA_OUTPUT_LIMIT: usize = 16 * 1024 * 1024;
const ERROR_TEXT_LIMIT: usize = 4_096;

pub(super) fn run(
    root: &RepositoryRoot,
    runner: &ProcessRunner,
    name: &str,
    args: &[&str],
    package: Option<String>,
) -> Result<MetadataContext> {
    let environment = env::vars().collect::<BTreeMap<_, _>>();
    let result = runner
        .run(
            ProcessRequest::new("cargo", args.iter().copied())
                .current_dir(root.path())
                .environment(environment)
                .limits(ProcessLimits {
                    max_stdout_bytes: METADATA_OUTPUT_LIMIT,
                    max_stderr_bytes: 256 * 1024,
                    timeout: METADATA_TIMEOUT,
                }),
        )
        .map_err(|error| format!("workspace DAG {name}: cargo metadata failed to run: {error}"))?;
    if !result.receipt.success() {
        return Err(format!(
            "workspace DAG {name}: cargo metadata failed ({})
{}",
            result.receipt.status,
            bounded_text(&result.stderr)
        ));
    }
    let metadata = serde_json::from_slice(&result.stdout)
        .map_err(|error| format!("workspace DAG {name}: invalid cargo metadata JSON: {error}"))?;
    Ok(MetadataContext {
        name: name.to_owned(),
        package,
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        metadata,
    })
}

fn bounded_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes).trim().to_owned();
    if text.len() <= ERROR_TEXT_LIMIT {
        text
    } else {
        format!("{}…", &text[..text.floor_char_boundary(ERROR_TEXT_LIMIT)])
    }
}
