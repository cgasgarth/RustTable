#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const WATERMARK_COMPATIBILITY_ID: &str = "watermark";
pub const WATERMARK_RUST_ID: &str = "rusttable.watermark";
pub const WATERMARK_SCHEMA_VERSION: u16 = 1;
pub const WATERMARK_PARAMETER_VERSION: u16 = 7;
pub const WATERMARK_IMPLEMENTATION_VERSION: u16 = 1;
pub const WATERMARK_ALLOWED_FONT_SET_HASH: [u8; 32] = [0; 32];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WatermarkAnchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WatermarkScaleMode {
    Width,
    Height,
    Fit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatermarkParametersV7 {
    template_hash: [u8; 32],
    opacity_bits: u32,
    scale_bits: u32,
    scale_mode: WatermarkScaleMode,
    anchor: WatermarkAnchor,
    x_offset_bits: u32,
    y_offset_bits: u32,
    rotation_bits: u32,
    color_bits: [u32; 4],
    expand_variables: bool,
}

pub type WatermarkParametersV1 = WatermarkParametersV7;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatermarkCodecError {
    InvalidValue(&'static str),
    InvalidHistory,
    OpaqueSource(u16),
}

impl std::fmt::Display for WatermarkCodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidValue(name) => write!(f, "invalid watermark {name}"),
            Self::InvalidHistory => f.write_str("invalid watermark history"),
            Self::OpaqueSource(version) => write!(
                f,
                "watermark history version {version} is opaque and blocking"
            ),
        }
    }
}
impl std::error::Error for WatermarkCodecError {}

impl Default for WatermarkParametersV7 {
    fn default() -> Self {
        Self::new(
            [0; 32],
            1.0,
            0.25,
            WatermarkScaleMode::Width,
            WatermarkAnchor::BottomRight,
            0.0,
            0.0,
            0.0,
            [1.0; 4],
            true,
        )
        .expect("static watermark defaults")
    }
}

impl WatermarkParametersV7 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        template_hash: [u8; 32],
        opacity: f32,
        scale: f32,
        scale_mode: WatermarkScaleMode,
        anchor: WatermarkAnchor,
        x_offset: f32,
        y_offset: f32,
        rotation: f32,
        color: [f32; 4],
        expand_variables: bool,
    ) -> Result<Self, WatermarkCodecError> {
        finite_range(opacity, 0.0, 1.0, "opacity")?;
        finite_range(scale, 0.0, 8.0, "scale")?;
        finite_range(rotation, -3600.0, 3600.0, "rotation")?;
        finite_range(x_offset, -1_000_000.0, 1_000_000.0, "x offset")?;
        finite_range(y_offset, -1_000_000.0, 1_000_000.0, "y offset")?;
        if color
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err(WatermarkCodecError::InvalidValue("color"));
        }
        Ok(Self {
            template_hash,
            opacity_bits: opacity.to_bits(),
            scale_bits: scale.to_bits(),
            scale_mode,
            anchor,
            x_offset_bits: x_offset.to_bits(),
            y_offset_bits: y_offset.to_bits(),
            rotation_bits: rotation.to_bits(),
            color_bits: color.map(f32::to_bits),
            expand_variables,
        })
    }

    #[must_use]
    pub const fn template_hash(&self) -> [u8; 32] {
        self.template_hash
    }
    #[must_use]
    pub fn opacity(&self) -> f32 {
        f32::from_bits(self.opacity_bits)
    }
    #[must_use]
    pub fn scale(&self) -> f32 {
        f32::from_bits(self.scale_bits)
    }
    #[must_use]
    pub const fn scale_mode(&self) -> WatermarkScaleMode {
        self.scale_mode
    }
    #[must_use]
    pub const fn anchor(&self) -> WatermarkAnchor {
        self.anchor
    }
    #[must_use]
    pub fn x_offset(&self) -> f32 {
        f32::from_bits(self.x_offset_bits)
    }
    #[must_use]
    pub fn y_offset(&self) -> f32 {
        f32::from_bits(self.y_offset_bits)
    }
    #[must_use]
    pub fn rotation(&self) -> f32 {
        f32::from_bits(self.rotation_bits)
    }
    #[must_use]
    pub fn color(&self) -> [f32; 4] {
        self.color_bits.map(f32::from_bits)
    }
    #[must_use]
    pub const fn expand_variables(&self) -> bool {
        self.expand_variables
    }

    #[must_use]
    pub fn history_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("watermark DTO is serializable")
    }

    #[must_use]
    pub fn cache_identity(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable.watermark.parameters.v7");
        hasher.update(self.history_bytes());
        hasher.finalize().into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatermarkHistory {
    V7(WatermarkParametersV7),
    Opaque { version: u16, bytes: Vec<u8> },
}

pub fn decode_history(version: u16, bytes: &[u8]) -> Result<WatermarkHistory, WatermarkCodecError> {
    if version != WATERMARK_PARAMETER_VERSION {
        return Ok(WatermarkHistory::Opaque {
            version,
            bytes: bytes.to_vec(),
        });
    }
    postcard::from_bytes(bytes)
        .map(WatermarkHistory::V7)
        .map_err(|_| WatermarkCodecError::InvalidHistory)
}

pub fn migrate_history(
    history: WatermarkHistory,
) -> Result<WatermarkParametersV7, WatermarkCodecError> {
    match history {
        WatermarkHistory::V7(parameters) => Ok(parameters),
        WatermarkHistory::Opaque { version, .. } => Err(WatermarkCodecError::OpaqueSource(version)),
    }
}

fn finite_range(
    value: f32,
    minimum: f32,
    maximum: f32,
    name: &'static str,
) -> Result<(), WatermarkCodecError> {
    if !value.is_finite() || !(minimum..=maximum).contains(&value) {
        return Err(WatermarkCodecError::InvalidValue(name));
    }
    Ok(())
}
