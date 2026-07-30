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
    scan_operations, scan_operations_with_identity, scan_operations_with_overrides,
};
pub use operation::validate::{
    canonical_layout_hash, parse_operation_manifest, render_operation_manifest,
    validate_operation_manifest,
};
pub use parameter_codec::{DecodedParameter, ParameterValue, decode_parameter, encode_parameter};
