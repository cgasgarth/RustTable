//! Typed, non-executable decoding of Darktable compatibility records.

mod history;

pub use history::{
    DarktableHistoryDecodeFinding, DarktableHistoryDecodeFindingCode, DarktableHistoryStepDecode,
    DecodedColorContrastHistoryStep, DecodedColorCorrectionHistoryStep,
    DecodedColorZonesHistoryStep, DecodedVelviaHistoryStep, DecodedVibranceHistoryStep,
    decode_history_step,
};
