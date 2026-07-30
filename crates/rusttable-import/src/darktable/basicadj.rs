//! Operation-local Basic Adjustments history import leaf.
//!
//! This leaf intentionally stops at typed core plus an explicit pending-blend
//! status. The exhaustive `history.rs` dispatcher owns the aggregate conversion
//! and continues to preserve unsupported rows.

pub use rusttable_compat::basicadj::{
    BASIC_ADJUSTMENTS_COMPATIBILITY_NAME, BASIC_ADJUSTMENTS_V1_PARAMETER_BYTES,
    BASIC_ADJUSTMENTS_V2_PARAMETER_BYTES, BASICADJ_COMPATIBILITY_NAME, BASICADJ_V1_PARAMETER_BYTES,
    BASICADJ_V1_VERSION, BASICADJ_V2_PARAMETER_BYTES, BASICADJ_V2_VERSION, BasicAdjCodecError,
    BasicAdjHistory, BasicAdjHistoryDecodeFinding, BasicAdjHistoryDecodeFindingCode,
    BasicAdjHistoryStepDecode, BasicAdjParametersV1, BasicAdjParametersV2, BasicAdjPreserveColors,
    decode_basic_adjustments_history_step, decode_basicadj_history_step, migrate_v1,
    migrate_v1_to_v2,
};

/// Decodes one Basic Adjustments row at the operation-local import boundary.
///
/// The returned pending variant retains the complete source row, including
/// opaque blend/mask and multi-instance metadata. The shared dispatcher must
/// not emit an executable edit until those semantics are ported.
#[must_use]
pub fn decode_basicadj_import_history_step(
    step: &rusttable_compat::CompatHistoryStep,
) -> BasicAdjHistoryStepDecode {
    decode_basicadj_history_step(step)
}

/// Alias using the display-name spelling used by import callers.
#[must_use]
pub fn decode_basic_adjustments_import_history_step(
    step: &rusttable_compat::CompatHistoryStep,
) -> BasicAdjHistoryStepDecode {
    decode_basicadj_import_history_step(step)
}
