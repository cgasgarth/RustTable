use std::collections::BTreeSet;

use crate::operation_model::{Evidence, Operation, OperationManifest, ParameterVersion};
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
    if manifest.schema_version != 2 {
        return Err(ScanError::InvalidManifest {
            message: format!(
                "unsupported operation schema version {}",
                manifest.schema_version
            ),
        });
    }
    validate_reference_identity(manifest)?;
    if manifest.history.database_table.trim().is_empty() {
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
        validate_operation(manifest, operation)?;
    }
    Ok(())
}

fn validate_operation(
    manifest: &OperationManifest,
    operation: &Operation,
) -> Result<(), ScanError> {
    if operation.reference_path.trim().is_empty() || operation.owning_issue_number == 0 {
        return Err(ScanError::OperationValidation {
            operation: operation.name.clone(),
            message: "reference path and positive owning GitHub issue are required".to_owned(),
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
    validate_operation_evidence(manifest, operation)?;
    let parameterized = operation.parameter_size > 0 || !operation.parameter_versions.is_empty();
    if parameterized {
        validate_current_layout(operation)?;
    } else if operation.parameter_size != 0
        || !operation.parameter_layout_hash.is_empty()
        || !operation.parameter_versions.is_empty()
    {
        return Err(ScanError::OperationValidation {
            operation: operation.name.clone(),
            message: "zero-parameter operation cannot carry parameter layout data".to_owned(),
        });
    }
    validate_versions(operation.name.as_str(), &operation.parameter_versions)?;
    validate_opencl(operation)?;
    for migration in &operation.migrations {
        if migration.from_version >= migration.to_version || migration.fixture_id.trim().is_empty()
        {
            return Err(ScanError::OperationValidation {
                operation: operation.name.clone(),
                message: "migration edges must increase and have fixtures".to_owned(),
            });
        }
        validate_evidence(
            manifest,
            operation.name.as_str(),
            &migration.evidence,
            "migration",
        )?;
    }
    Ok(())
}

fn validate_current_layout(operation: &Operation) -> Result<(), ScanError> {
    if operation.module_version == 0
        || operation.parameter_size == 0
        || operation.parameter_layout_hash.len() != 64
        || operation.parameter_versions.is_empty()
    {
        return Err(ScanError::OperationValidation {
            operation: operation.name.clone(),
            message: "parameterized operation requires an exact nonzero current layout".to_owned(),
        });
    }
    if operation
        .parameter_versions
        .last()
        .map(|version| version.version)
        != Some(operation.module_version)
        || operation
            .parameter_versions
            .last()
            .map(|version| version.byte_size)
            != Some(operation.parameter_size)
    {
        return Err(ScanError::OperationValidation {
            operation: operation.name.clone(),
            message: "current operation layout does not match its version record".to_owned(),
        });
    }
    Ok(())
}

fn validate_opencl(operation: &Operation) -> Result<(), ScanError> {
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
    Ok(())
}

fn validate_versions(name: &str, versions: &[ParameterVersion]) -> Result<(), ScanError> {
    let mut ids = BTreeSet::new();
    for version in versions {
        if version.version == 0
            || version.layout_hash.len() != 64
            || version.byte_size == 0
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
        if !ids.insert(format!("version:{}", version.version)) {
            return Err(ScanError::OperationValidation {
                operation: name.to_owned(),
                message: "duplicate parameter version".to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_reference_identity(manifest: &OperationManifest) -> Result<(), ScanError> {
    let identity = &manifest.reference;
    if [
        identity.source_commit.as_str(),
        identity.build_version.as_str(),
        identity.executable_hash.as_str(),
        identity.data_bundle_hash.as_str(),
        identity.target_triple.as_str(),
        identity.c_abi_model.as_str(),
        identity.build_option_hash.as_str(),
    ]
    .iter()
    .any(|value| value.trim().is_empty())
    {
        return Err(ScanError::InvalidManifest {
            message: "complete reference identity is required".to_owned(),
        });
    }
    Ok(())
}

fn validate_operation_evidence(
    manifest: &OperationManifest,
    operation: &Operation,
) -> Result<(), ScanError> {
    let mut fields = BTreeSet::new();
    for evidence in &operation.evidence {
        if !fields.insert(evidence.field.clone()) {
            return Err(ScanError::OperationValidation {
                operation: operation.name.clone(),
                message: format!("duplicate evidence field {}", evidence.field),
            });
        }
        validate_evidence(
            manifest,
            operation.name.as_str(),
            &evidence.evidence,
            evidence.field.as_str(),
        )?;
    }
    for field in ["registration", "layout", "contract", "ownership"] {
        if !fields.contains(field) {
            return Err(ScanError::OperationValidation {
                operation: operation.name.clone(),
                message: format!("missing {field} evidence"),
            });
        }
    }
    if !operation.opencl_programs.is_empty() && !fields.contains("gpu") {
        return Err(ScanError::OperationValidation {
            operation: operation.name.clone(),
            message: "OpenCL references require gpu evidence".to_owned(),
        });
    }
    if !operation.preset_sources.is_empty() && !fields.contains("presets") {
        return Err(ScanError::OperationValidation {
            operation: operation.name.clone(),
            message: "preset records require presets evidence".to_owned(),
        });
    }
    Ok(())
}

fn validate_evidence(
    manifest: &OperationManifest,
    operation: &str,
    evidence: &Evidence,
    field: &str,
) -> Result<(), ScanError> {
    let valid_hash = evidence.evidence_hash.len() == 64
        && evidence
            .evidence_hash
            .chars()
            .all(|character| character.is_ascii_hexdigit());
    let source_lines = evidence.source_path.is_some()
        && evidence.line_start.is_some()
        && evidence.line_end.is_some();
    let fixture = evidence
        .fixture_id
        .as_ref()
        .is_some_and(|id| !id.trim().is_empty());
    if evidence.source_commit != manifest.reference.source_commit
        || (!source_lines && !fixture)
        || (source_lines && fixture)
        || evidence.reason.trim().is_empty()
        || evidence.reviewer.trim().is_empty()
        || !valid_hash
        || evidence
            .line_start
            .zip(evidence.line_end)
            .is_some_and(|(start, end)| start > end)
    {
        return Err(ScanError::OperationValidation {
            operation: operation.to_owned(),
            message: format!("invalid {field} evidence or mismatched reference identity"),
        });
    }
    Ok(())
}
