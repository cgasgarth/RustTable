//! Typed, fail-closed decoding of Darktable compatibility records.

mod basicadj;
mod history;

pub use basicadj::*;
pub use history::{
    DarktableHistoryDecodeFinding, DarktableHistoryDecodeFindingCode, DarktableHistoryStepDecode,
    DecodedAgxHistoryStep, DecodedBasecurveHistoryStep, DecodedBasicAdjHistoryStep,
    DecodedChannelMixerHistoryStep, DecodedColorContrastHistoryStep,
    DecodedColorCorrectionHistoryStep, DecodedColorMappingHistoryStep,
    DecodedColorTransferHistoryStep, DecodedColorZonesHistoryStep, DecodedHighpassHistoryStep,
    DecodedLevelsHistoryStep, DecodedRgbLevelsHistoryStep, DecodedSharpenHistoryStep,
    DecodedSoftenHistoryStep, DecodedToneCurveHistoryStep, DecodedVelviaHistoryStep,
    DecodedVibranceHistoryStep, decode_history_step, decode_history_steps,
};
