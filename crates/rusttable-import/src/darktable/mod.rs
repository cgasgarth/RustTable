//! Typed, non-executable decoding of Darktable compatibility records.

mod history;

pub use history::{
    DarktableHistoryDecodeFinding, DarktableHistoryDecodeFindingCode, DarktableHistoryStepDecode,
    DecodedVelviaHistoryStep, decode_history_step,
};
