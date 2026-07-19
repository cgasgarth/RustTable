use std::collections::BTreeSet;
use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use crate::operation_model::{
    AbiLayout, CallbackResult, Evidence, Operation, OperationManifest, ParameterCodec,
    ParameterVersion,
};
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
    if manifest.schema_version != 3 {
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
    validate_semantics(operation)?;
    validate_opencl(operation)?;
    validate_layout_matrix(operation)?;
    validate_current_codec(operation)?;
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
    if operation
        .parameter_versions
        .last()
        .is_some_and(|version| version.abi_layouts.len() != 3)
    {
        return Err(ScanError::OperationValidation {
            operation: operation.name.clone(),
            message: "current parameter version requires all supported ABI layouts".to_owned(),
        });
    }
    Ok(())
}

const SUPPORTED_TARGETS: [&str; 3] = [
    "x86_64-unknown-linux-gnu",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
];

fn validate_layout_matrix(operation: &Operation) -> Result<(), ScanError> {
    let Some(version) = operation.parameter_versions.last() else {
        return Ok(());
    };
    if version.abi_layouts.len() != SUPPORTED_TARGETS.len() {
        return Err(ScanError::OperationValidation {
            operation: operation.name.clone(),
            message: "current layout matrix is incomplete".to_owned(),
        });
    }
    for target in SUPPORTED_TARGETS {
        let Some(layout) = version
            .abi_layouts
            .iter()
            .find(|layout| layout.target == target)
        else {
            return Err(ScanError::OperationValidation {
                operation: operation.name.clone(),
                message: format!("missing current ABI layout for {target}"),
            });
        };
        validate_layout(operation.name.as_str(), layout)?;
    }
    let baseline = version
        .abi_layouts
        .iter()
        .find(|layout| layout.target == SUPPORTED_TARGETS[0])
        .expect("baseline layout was checked above");
    let differs = version
        .abi_layouts
        .iter()
        .skip(1)
        .any(|layout| layout_signature(layout) != layout_signature(baseline));
    if differs && version.target_codecs.len() != SUPPORTED_TARGETS.len() {
        return Err(ScanError::OperationValidation {
            operation: operation.name.clone(),
            message: "ABI differences require one decoder/encoder per target".to_owned(),
        });
    }
    if !version.target_codecs.is_empty() {
        for target in SUPPORTED_TARGETS {
            if !version
                .target_codecs
                .iter()
                .any(|codec| codec.target == target)
            {
                return Err(ScanError::OperationValidation {
                    operation: operation.name.clone(),
                    message: format!("missing target-specific codec for {target}"),
                });
            }
        }
    }
    if operation.abi_layouts.is_empty() {
        return Err(ScanError::OperationValidation {
            operation: operation.name.clone(),
            message: "operation is missing its current ABI layout projection".to_owned(),
        });
    }
    Ok(())
}

fn validate_layout(operation: &str, layout: &AbiLayout) -> Result<(), ScanError> {
    if layout.c_abi_model.trim().is_empty()
        || layout.endianness != "little"
        || layout.pointer_width != 64
        || layout.total_size == 0
        || layout.alignment == 0
        || !valid_hash(&layout.layout_hash)
        || layout.fields.is_empty()
    {
        return Err(ScanError::OperationValidation {
            operation: operation.to_owned(),
            message: format!("incomplete ABI layout for {}", layout.target),
        });
    }
    let mut names = BTreeSet::new();
    let mut ranges = Vec::new();
    for field in &layout.fields {
        let extent = field.array_extent.unwrap_or(1);
        if field.name.trim().is_empty()
            || field.type_name.trim().is_empty()
            || field.size == 0
            || field.alignment == 0
            || extent == 0
            || field.offset + field.size > layout.total_size
            || !names.insert(field.name.clone())
        {
            return Err(ScanError::OperationValidation {
                operation: operation.to_owned(),
                message: format!("invalid field layout in {}", layout.target),
            });
        }
        let end = field.offset + field.size;
        if ranges
            .iter()
            .any(|(start, stop): &(usize, usize)| field.offset < *stop && end > *start)
        {
            return Err(ScanError::OperationValidation {
                operation: operation.to_owned(),
                message: format!("overlapping field layout in {}", layout.target),
            });
        }
        ranges.push((field.offset, end));
    }
    for padding in &layout.padding {
        if padding.size == 0
            || padding.offset + padding.size > layout.total_size
            || !matches!(padding.kind.as_str(), "explicit" | "implicit")
            || ranges.iter().any(|(start, stop)| {
                padding.offset < *stop && padding.offset + padding.size > *start
            })
        {
            return Err(ScanError::OperationValidation {
                operation: operation.to_owned(),
                message: format!("invalid padding interval in {}", layout.target),
            });
        }
    }
    Ok(())
}

fn layout_signature(
    layout: &AbiLayout,
) -> Vec<(String, String, Option<usize>, usize, usize, usize)> {
    layout
        .fields
        .iter()
        .map(|field| {
            (
                field.name.clone(),
                field.type_name.clone(),
                field.array_extent,
                field.offset,
                field.size,
                field.alignment,
            )
        })
        .collect()
}

fn validate_current_codec(operation: &Operation) -> Result<(), ScanError> {
    let Some(version) = operation.parameter_versions.last() else {
        return Ok(());
    };
    let Some(codec) = version.codec.as_ref().or(operation.codec.as_ref()) else {
        return Err(ScanError::OperationValidation {
            operation: operation.name.clone(),
            message: "current parameter version has no executable decoder/encoder".to_owned(),
        });
    };
    if version.decoder == "opaque" || version.opaque_blocking || codec.decoder == "opaque" {
        return Err(ScanError::OperationValidation {
            operation: operation.name.clone(),
            message: "current parameter version cannot use an opaque blocking decoder".to_owned(),
        });
    }
    validate_codec(
        operation.name.as_str(),
        codec,
        version.byte_size,
        &version.abi_layouts,
    )?;
    Ok(())
}

fn validate_codec(
    operation: &str,
    codec: &ParameterCodec,
    byte_size: usize,
    layouts: &[AbiLayout],
) -> Result<(), ScanError> {
    if codec.byte_size != byte_size
        || codec.decoder.trim().is_empty()
        || codec.encoder.trim().is_empty()
        || codec.byte_order != "little"
        || codec.format.trim().is_empty()
        || !codec.preserves_padding
        || codec.fields.is_empty()
    {
        return Err(ScanError::OperationValidation {
            operation: operation.to_owned(),
            message: "current decoder/encoder is incomplete".to_owned(),
        });
    }
    for layout in layouts {
        for field in &codec.fields {
            if !layout.fields.iter().any(|candidate| {
                candidate.name == field.name
                    && candidate.offset == field.offset
                    && candidate.size == field.size
            }) {
                return Err(ScanError::OperationValidation {
                    operation: operation.to_owned(),
                    message: format!(
                        "codec field {} is absent from {}",
                        field.name, layout.target
                    ),
                });
            }
        }
    }
    Ok(())
}

fn validate_semantics(operation: &Operation) -> Result<(), ScanError> {
    validate_callback(operation, "input color", &operation.color_contract.input)?;
    validate_callback(operation, "output color", &operation.color_contract.output)?;
    if operation.color_contract.input.value == "unknown"
        || operation.color_contract.output.value == "unknown"
    {
        return Err(ScanError::OperationValidation {
            operation: operation.name.clone(),
            message: "executable operation has an unknown color-space contract".to_owned(),
        });
    }
    if operation
        .capability_contract
        .flags
        .iter()
        .any(|flag| flag == "IOP_FLAGS_SUPPORTS_BLENDING")
        && !operation.capability_contract.supports_shared_blending
    {
        return Err(ScanError::OperationValidation {
            operation: operation.name.clone(),
            message: "blending flag contradicts capability contract".to_owned(),
        });
    }
    if operation.supports_blend_masks
        != (operation.capability_contract.supports_drawn_masks
            || operation.capability_contract.consumes_raster_mask)
    {
        return Err(ScanError::OperationValidation {
            operation: operation.name.clone(),
            message: "legacy blend-mask field contradicts split mask capabilities".to_owned(),
        });
    }
    if ![
        "identity",
        "expands",
        "reduces",
        "expands-and-reduces",
        "conditional",
    ]
    .contains(&operation.roi_contract.behavior.as_str())
        || operation.roi_contract.behavior == "unresolved"
        || operation.tiling_contract.class == "unresolved"
        || operation.tiling_requirement == "scanline"
    {
        return Err(ScanError::OperationValidation {
            operation: operation.name.clone(),
            message: "ROI and tiling contracts must be explicit".to_owned(),
        });
    }
    Ok(())
}

fn validate_callback(
    operation: &Operation,
    label: &str,
    callback: &CallbackResult,
) -> Result<(), ScanError> {
    if !matches!(callback.mode.as_str(), "unconditional" | "conditional")
        || callback.value.trim().is_empty()
        || (callback.mode == "conditional"
            && callback.predicate.as_deref().is_none_or(str::is_empty))
        || callback.evidence.is_empty()
    {
        return Err(ScanError::OperationValidation {
            operation: operation.name.clone(),
            message: format!("{label} callback result is unresolved"),
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
    if operation.opencl_programs.len() != operation.opencl_resolution.len() {
        return Err(ScanError::UnknownOpenclProgram {
            operation: operation.name.clone(),
            reference: "missing resolved program registry record".to_owned(),
        });
    }
    for resolution in &operation.opencl_resolution {
        if !programs.contains(&resolution.program)
            || std::path::Path::new(&resolution.source_path)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("c"))
            || resolution.source_path.trim().is_empty()
            || resolution.kernels.is_empty()
            || operation
                .opencl_kernels
                .iter()
                .any(|kernel| !resolution.kernels.contains(kernel))
        {
            return Err(ScanError::UnknownOpenclProgram {
                operation: operation.name.clone(),
                reference: resolution.program.clone(),
            });
        }
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
    if manifest.reference.canonical_identity != "fixture"
        && manifest.reference.canonical_identity != "fixtures/reference/darktable.toml"
    {
        return Err(ScanError::InvalidManifest {
            message: "operation manifest must reference the canonical #449 identity".to_owned(),
        });
    }
    if manifest.reference.canonical_identity == "fixtures/reference/darktable.toml"
        && manifest.reference.source_commit != "cfe57f3bbf5269bfacf31e832267279caa6938ad"
    {
        return Err(ScanError::InvalidManifest {
            message: "operation manifest source commit differs from #449/#494 identity".to_owned(),
        });
    }
    if manifest.reference.canonical_identity != "fixture" {
        for (label, value) in [
            ("executable", identity.executable_hash.as_str()),
            ("data", identity.data_bundle_hash.as_str()),
            ("build", identity.build_option_hash.as_str()),
        ] {
            if value.starts_with("not-built") || value == "unprovisioned-local-build" {
                return Err(ScanError::InvalidManifest {
                    message: format!("{label} identity is not qualified"),
                });
            }
        }
    }
    if manifest.reference.canonical_identity == "fixtures/reference/darktable.toml"
        && !valid_hash(&manifest.reference.identity_hash)
    {
        return Err(ScanError::InvalidManifest {
            message: "canonical reference identity hash is required".to_owned(),
        });
    }
    Ok(())
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Computes the stable hash used for a field-level ABI layout receipt.
#[must_use]
pub fn canonical_layout_hash(layout: &AbiLayout) -> String {
    let mut normalized = layout.clone();
    normalized.layout_hash.clear();
    let bytes = serde_json::to_vec(&normalized).unwrap_or_default();
    let mut hash = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(&mut hash, "{byte:02x}").expect("writing to String cannot fail");
    }
    hash
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
