//! Typed, non-executable decoding of Darktable compatibility records.

mod basicadj;
mod history;

pub use basicadj::*;
pub use history::{
    DarktableHistoryDecodeFinding, DarktableHistoryDecodeFindingCode, DarktableHistoryStepDecode,
    DecodedChannelMixerHistoryStep, DecodedColorContrastHistoryStep,
    DecodedColorCorrectionHistoryStep, DecodedColorZonesHistoryStep, DecodedSharpenHistoryStep,
    DecodedSoftenHistoryStep, DecodedVelviaHistoryStep, DecodedVibranceHistoryStep,
    decode_history_step,
};
