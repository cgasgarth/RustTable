use rusttable_color::ColorEncoding;
use sha2::{Digest, Sha256};

use crate::{ImageDimensions, Provider};

use super::detail::DetailReceipt;
use super::policy::{MemoryReceipt, TileReceipt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbColorReceipt {
    pub input_plan_hash: [u8; 32],
    pub output_plan_hash: [u8; 32],
    pub source: ColorEncoding,
    pub model: ColorEncoding,
    pub output: ColorEncoding,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RgbAiReceipt {
    pub plan_hash: [u8; 32],
    pub source_identity: [u8; 32],
    pub model_hash: [u8; 32],
    pub provider: Provider,
    pub output_identity: [u8; 32],
    pub tiles: TileReceipt,
    pub memory: MemoryReceipt,
    pub color: RgbColorReceipt,
    pub detail: Option<DetailReceipt>,
}

pub(crate) fn image_identity(
    dimensions: ImageDimensions,
    profile: ColorEncoding,
    pixels: &[[f32; 4]],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable-ai-rgb-image-v1");
    hasher.update(dimensions.width().to_le_bytes());
    hasher.update(dimensions.height().to_le_bytes());
    hasher.update(format!("{profile:?}").as_bytes());
    for pixel in pixels {
        for value in pixel {
            hasher.update(value.to_bits().to_le_bytes());
        }
    }
    hasher.finalize().into()
}
