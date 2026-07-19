use std::fs;

use super::{Result, report};
use crate::cli::ParityCommand;
use crate::process::ProcessRunner;
use crate::root::RepositoryRoot;
use rusttable_parity::scan_darktable_with_identity;
use rusttable_testkit::reference::{ReferenceIdentityOverrides, resolve_reference};

pub(super) fn run(
    root: &RepositoryRoot,
    command: &ParityCommand,
    runner: &ProcessRunner,
) -> Result {
    match command {
        ParityCommand::ScanDarktable(arguments) => {
            let overrides = root.join(&arguments.overrides);
            let output = root.join(&arguments.output);
            let receipt = root.join(&arguments.receipt);
            let identity = resolve_reference(
                root.join(&arguments.identity),
                &ReferenceIdentityOverrides {
                    source_path: arguments.source.as_ref().map(|path| root.join(path)),
                    executable_path: arguments.executable.as_ref().map(|path| root.join(path)),
                    data_dir: arguments.data_dir.as_ref().map(|path| root.join(path)),
                },
            )
            .map_err(|error| error.to_string())?;
            let manifest = scan_darktable_with_identity(&identity, &overrides)
                .map_err(|error| error.to_string())?;
            let rendered =
                rusttable_parity::render_manifest(&manifest).map_err(|error| error.to_string())?;
            fs::write(&output, rendered)
                .map_err(|error| format!("write {}: {error}", output.display()))?;
            let rendered_receipt =
                rusttable_parity::render_receipt(&manifest).map_err(|error| error.to_string())?;
            fs::write(&receipt, rendered_receipt)
                .map_err(|error| format!("write {}: {error}", receipt.display()))?;
            Ok(report(
                root,
                "parity.scan-darktable",
                serde_json::json!({ "output": output, "receipt": receipt, "capabilities": manifest.capabilities.len() }),
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
            let receipt_path = root.join("architecture/darktable-capabilities.receipt.toml");
            let expected_receipt = rusttable_parity::render_receipt(&capabilities)
                .map_err(|error| error.to_string())?;
            let actual_receipt = fs::read_to_string(&receipt_path)
                .map_err(|error| format!("read {}: {error}", receipt_path.display()))?;
            if actual_receipt != expected_receipt {
                return Err(format!("{} is stale", receipt_path.display()));
            }
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
        ParityCommand::PlanIssueReconciliation(arguments) => {
            super::github_reconciliation::plan_issue_reconciliation(root, arguments, runner)
        }
        ParityCommand::ApplyIssueReconciliation(arguments) => {
            super::github_reconciliation::apply_issue_reconciliation(root, arguments, runner)
        }
    }
}
