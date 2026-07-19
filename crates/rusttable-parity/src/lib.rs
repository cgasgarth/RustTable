#![forbid(unsafe_code)]
#![doc = "Deterministic darktable feature-parity discovery and manifest validation."]

mod mapping;
mod model;
mod operation_model;
mod operation_reference;
mod operation_scan;
mod operation_validate;
mod parameter_codec;
mod reconciliation;
mod scan;
mod validate;

pub use model::{Capability, CapabilityReceipt, IssueOwnership, Manifest, Override, SummaryGroup};
pub use operation_model::{
    AbiLayout, CallbackResult, CapabilityContract, CodecField, ColorContract, EnumValue, Evidence,
    FieldLayout, HistoryCompatibility, OpenclProgramResolution, Operation, OperationEvidence,
    OperationManifest, OperationOverride, PaddingInterval, ParameterCodec, ParameterMigration,
    ParameterVersion, PresetRecord, ReferenceIdentity, RoiContract, TargetCodec, TilingContract,
};
pub use operation_scan::{
    scan_operations, scan_operations_with_identity, scan_operations_with_overrides,
};
pub use operation_validate::{
    canonical_layout_hash, parse_operation_manifest, render_operation_manifest,
    validate_operation_manifest,
};
pub use parameter_codec::{DecodedParameter, ParameterValue, decode_parameter, encode_parameter};
pub use reconciliation::{
    CapabilityCandidate, CapabilityDeclaration, IssueAudit, IssueInput, IssueSpecification,
    PlannedClosure, PlannedCreation, PlannedLabelChange, PlannedMilestoneChange, PlannedUpdate,
    ReconciliationPlan, build_reconciliation_plan, parse_capability_declarations,
};
pub use scan::{
    ScanError, scan_darktable, scan_darktable_with_issue_index, scan_darktable_with_overrides,
};
pub use validate::{parse_manifest, render_manifest, render_receipt, validate_manifest};

/// Scans the source tree selected by the single resolved reference identity.
///
/// # Errors
///
/// Returns an error when the override file cannot be read, scanning fails, or
/// the scan commit differs from the resolved identity.
pub fn scan_darktable_with_identity(
    identity: &rusttable_testkit::reference::ReferenceIdentity,
    overrides: &std::path::Path,
) -> Result<Manifest, ScanError> {
    let overrides = std::fs::read_to_string(overrides).map_err(|error| ScanError::Io {
        path: overrides.display().to_string(),
        message: error.to_string(),
    })?;
    scan_darktable_with_overrides(&identity.source_dir, &overrides).and_then(|manifest| {
        if manifest.source_commit == identity.commit {
            Ok(manifest)
        } else {
            Err(ScanError::ReferenceIdentityMismatch {
                expected: identity.commit.clone(),
                actual: manifest.source_commit,
            })
        }
    })
}
