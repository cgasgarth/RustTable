#![forbid(unsafe_code)]
#![doc = "Deterministic darktable feature-parity discovery and manifest validation."]

mod mapping;
mod model;
mod operation_model;
mod operation_scan;
mod operation_validate;
mod scan;
mod validate;

pub use model::{Capability, CapabilityReceipt, IssueOwnership, Manifest, Override, SummaryGroup};
pub use operation_model::{
    HistoryCompatibility, Operation, OperationManifest, OperationOverride, ParameterMigration,
    ParameterVersion,
};
pub use operation_scan::{scan_operations, scan_operations_with_overrides};
pub use operation_validate::{
    parse_operation_manifest, render_operation_manifest, validate_operation_manifest,
};
pub use scan::{
    ScanError, scan_darktable, scan_darktable_with_issue_index, scan_darktable_with_overrides,
};
pub use validate::{parse_manifest, render_manifest, render_receipt, validate_manifest};
