#![forbid(unsafe_code)]
#![doc = "Pinned darktable operation and history compatibility data."]

mod error;
mod operation;
mod parameter_codec;

pub use error::ScanError;
pub use operation::model::{
    AbiLayout, CallbackResult, CapabilityContract, CodecField, ColorContract, EnumValue, Evidence,
    FieldLayout, HistoryCompatibility, OpenclProgramResolution, Operation, OperationEvidence,
    OperationManifest, OperationOverride, PaddingInterval, ParameterCodec, ParameterMigration,
    ParameterVersion, PresetRecord, ReferenceIdentity, RoiContract, TargetCodec, TilingContract,
};
pub use operation::scan::{
    parse_operation_overrides, scan_operations, scan_operations_with_identity,
    scan_operations_with_overrides,
};
pub use operation::trust_anchor::{
    TrustedRegistryEntry, generated_compatibility_name, is_independently_trusted_manifest_name,
    validate_architecture_provenance, validate_manifest_capability_accounting,
    validate_trusted_registry_entries,
};
pub use operation::validate::{
    canonical_layout_hash, parse_operation_manifest, render_operation_manifest,
    validate_operation_manifest,
};
pub use parameter_codec::{DecodedParameter, ParameterValue, decode_parameter, encode_parameter};
