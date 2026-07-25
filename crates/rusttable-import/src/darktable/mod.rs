//! Typed, non-executable decoding of Darktable compatibility records.

mod history;

pub use history::{
    DarktableHistoryDecodeFinding, DarktableHistoryDecodeFindingCode, DarktableHistoryStepDecode,
    DecodedColorContrastHistoryStep, DecodedVelviaHistoryStep, decode_history_step,
};
