use std::fs;

use super::{Result, report};
use crate::cli::ParityCommand;
use crate::root::RepositoryRoot;

pub(super) fn run(root: &RepositoryRoot, command: &ParityCommand) -> Result {
    match command {
        ParityCommand::ScanDarktable(arguments) => {
            let source = root.join(&arguments.source);
            let overrides = root.join(&arguments.overrides);
            let output = root.join(&arguments.output);
            let manifest = rusttable_parity::scan_darktable(&source, &overrides)
                .map_err(|error| error.to_string())?;
            let rendered =
                rusttable_parity::render_manifest(&manifest).map_err(|error| error.to_string())?;
            fs::write(&output, rendered)
                .map_err(|error| format!("write {}: {error}", output.display()))?;
            Ok(report(
                root,
                "parity.scan-darktable",
                serde_json::json!({ "output": output, "capabilities": manifest.capabilities.len() }),
            ))
        }
        ParityCommand::Verify => {
            let capability_path = root.join("architecture/darktable-capabilities.toml");
            let operation_path = root.join("architecture/darktable-operations.toml");
            let capabilities = rusttable_parity::parse_manifest(
                &fs::read_to_string(&capability_path).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            rusttable_parity::validate_manifest(&capabilities)
                .map_err(|error| error.to_string())?;
            let operations = rusttable_parity::parse_operation_manifest(
                &fs::read_to_string(&operation_path).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            rusttable_parity::validate_operation_manifest(&operations)
                .map_err(|error| error.to_string())?;
            Ok(report(
                root,
                "parity.verify",
                serde_json::json!({
                    "capabilities": capabilities.capabilities.len(),
                    "operations": operations.operations.len(),
                }),
            ))
        }
    }
}
