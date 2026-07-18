use std::collections::BTreeSet;

use crate::operation_model::{OperationManifest, ParameterVersion};
use crate::scan::ScanError;

/// Parses an operation manifest from TOML.
///
/// # Errors
///
/// Returns an error when the TOML is malformed or cannot represent the schema.
pub fn parse_operation_manifest(contents: &str) -> Result<OperationManifest, ScanError> {
    toml::from_str(contents).map_err(|error| ScanError::InvalidManifest {
        message: error.to_string(),
    })
}

/// Validates and renders an operation manifest as deterministic TOML.
///
/// # Errors
///
/// Returns an error when a manifest invariant fails or serialization fails.
pub fn render_operation_manifest(manifest: &OperationManifest) -> Result<String, ScanError> {
    validate_operation_manifest(manifest)?;
    let mut rendered =
        toml::to_string_pretty(manifest).map_err(|error| ScanError::Serialization {
            message: error.to_string(),
        })?;
    rendered.insert_str(
        0,
        "# GENERATED FILE: rusttable-parity scan-operations; do not hand-edit.\n\n",
    );
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    Ok(rendered)
}

/// Validates operation identities, persisted versions, migrations, and kernel references.
///
/// # Errors
///
/// Returns an error when any compatibility invariant is violated.
pub fn validate_operation_manifest(manifest: &OperationManifest) -> Result<(), ScanError> {
    if manifest.schema_version != 1 {
        return Err(ScanError::InvalidManifest {
            message: format!(
                "unsupported operation schema version {}",
                manifest.schema_version
            ),
        });
    }
    if manifest.source_commit.trim().is_empty() || manifest.history.database_table.trim().is_empty()
    {
        return Err(ScanError::InvalidManifest {
            message: "source commit and history table are required".to_owned(),
        });
    }
    let mut names = BTreeSet::new();
    for operation in &manifest.operations {
        if !names.insert(operation.name.clone()) {
            return Err(ScanError::OperationValidation {
                operation: operation.name.clone(),
                message: "duplicate operation name".to_owned(),
            });
        }
        if operation.reference_path.trim().is_empty() || operation.owning_issue.len() != 4 {
            return Err(ScanError::OperationValidation {
                operation: operation.name.clone(),
                message: "reference path and four-digit owning issue are required".to_owned(),
            });
        }
        if ![
            "Exact",
            "Transfer",
            "Pointwise",
            "Neighborhood",
            "LegacyGpu",
        ]
        .contains(&operation.tolerance_class.as_str())
        {
            return Err(ScanError::OperationValidation {
                operation: operation.name.clone(),
                message: format!("unknown tolerance class {}", operation.tolerance_class),
            });
        }
        if operation.parameter_versions.is_empty() {
            return Err(ScanError::OperationValidation {
                operation: operation.name.clone(),
                message: "at least one parameter version is required".to_owned(),
            });
        }
        validate_versions(operation.name.as_str(), &operation.parameter_versions)?;
        let programs = operation.opencl_programs.iter().collect::<BTreeSet<_>>();
        if operation
            .opencl_kernels
            .iter()
            .any(|kernel| kernel.trim().is_empty())
        {
            return Err(ScanError::UnknownOpenclKernel {
                operation: operation.name.clone(),
                reference: "empty kernel".to_owned(),
            });
        }
        if !operation.opencl_kernels.is_empty() && programs.is_empty() {
            return Err(ScanError::UnknownOpenclKernel {
                operation: operation.name.clone(),
                reference: "kernel without program".to_owned(),
            });
        }
        for migration in &operation.migrations {
            if migration.from_version >= migration.to_version
                || migration.fixture_id.trim().is_empty()
            {
                return Err(ScanError::OperationValidation {
                    operation: operation.name.clone(),
                    message: "migration edges must increase and have fixtures".to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn validate_versions(name: &str, versions: &[ParameterVersion]) -> Result<(), ScanError> {
    let mut previous = 0;
    let mut ids = BTreeSet::new();
    for version in versions {
        if version.version != previous + 1
            || version.layout_hash.len() != 64
            || version.fixture_id.trim().is_empty()
        {
            return Err(ScanError::OperationValidation {
                operation: name.to_owned(),
                message: "parameter versions must be contiguous, hashed, and fixture-backed"
                    .to_owned(),
            });
        }
        if !ids.insert(version.fixture_id.clone()) {
            return Err(ScanError::OperationValidation {
                operation: name.to_owned(),
                message: "duplicate parameter fixture".to_owned(),
            });
        }
        if !version.opaque_blocking && version.decoder.trim().is_empty() {
            return Err(ScanError::OperationValidation {
                operation: name.to_owned(),
                message: "non-opaque version requires a decoder".to_owned(),
            });
        }
        previous = version.version;
    }
    Ok(())
}
