//! Typed, non-executable decoding of Darktable compatibility records.

mod basicadj;
mod history;

pub use basicadj::*;
pub use history::{
    DarktableHistoryDecodeFinding, DarktableHistoryDecodeFindingCode, DarktableHistoryStepDecode,
    DecodedAgxHistoryStep, DecodedBasicAdjHistoryStep, DecodedChannelMixerHistoryStep,
    DecodedColorContrastHistoryStep, DecodedColorCorrectionHistoryStep,
    DecodedColorMappingHistoryStep, DecodedColorTransferHistoryStep, DecodedColorZonesHistoryStep,
    DecodedLevelsHistoryStep, DecodedRgbLevelsHistoryStep, DecodedSharpenHistoryStep,
    DecodedSoftenHistoryStep, DecodedVelviaHistoryStep, DecodedVibranceHistoryStep,
    decode_history_step,
};
