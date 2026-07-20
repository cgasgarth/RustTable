use std::fmt;

use crate::MetadataPolicy;

pub const SETTINGS_SCHEMA_VERSION: u16 = 1;
pub const MAX_OUTPUT_BYTES: u64 = 1 << 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    J2k,
    Jp2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transform {
    Reversible53,
    Irreversible97,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressionOrder {
    Lrcp,
    Rlcp,
    Rpcl,
    Pcrl,
    Cprl,
}

impl ProgressionOrder {
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Lrcp => "LRCP",
            Self::Rlcp => "RLCP",
            Self::Rpcl => "RPCL",
            Self::Pcrl => "PCRL",
            Self::Cprl => "CPRL",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub container: Container,
    pub bit_depth: u8,
    pub lossless: bool,
    pub target_rate: Option<f32>,
    pub target_psnr: Option<f32>,
    pub quality_layers: u16,
    pub transform: Transform,
    pub resolution_levels: u8,
    pub code_block_width: u8,
    pub code_block_height: u8,
    pub precinct_width: Option<u8>,
    pub precinct_height: Option<u8>,
    pub progression: ProgressionOrder,
    pub tile_width: Option<u32>,
    pub tile_height: Option<u32>,
    pub sop: bool,
    pub eph: bool,
    pub threads: u16,
    pub max_output_bytes: u64,
    pub metadata: MetadataPolicy,
    pub allow_raw_alpha: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            container: Container::Jp2,
            bit_depth: 8,
            lossless: true,
            target_rate: None,
            target_psnr: None,
            quality_layers: 1,
            transform: Transform::Reversible53,
            resolution_levels: 5,
            code_block_width: 64,
            code_block_height: 64,
            precinct_width: None,
            precinct_height: None,
            progression: ProgressionOrder::Lrcp,
            tile_width: None,
            tile_height: None,
            sop: false,
            eph: false,
            threads: 1,
            max_output_bytes: MAX_OUTPUT_BYTES,
            metadata: MetadataPolicy::default(),
            allow_raw_alpha: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SettingsError {
    ZeroQualityLayers,
    LosslessRequiresReversible,
    LosslessHasQualityTarget,
    QualityTargetConflict,
    InvalidRate,
    InvalidPsnr,
    InvalidResolutionLevels,
    InvalidCodeBlock,
    InvalidPrecinct,
    InvalidTile,
    ZeroThreads,
    OutputLimit,
}

impl fmt::Display for SettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid JPEG 2000 settings: {self:?}")
    }
}

impl std::error::Error for SettingsError {}

impl Settings {
    /// Validates the versioned recipe settings without inspecting an image.
    ///
    /// # Errors
    ///
    /// Returns a typed error when a setting is contradictory or outside its bound.
    pub fn validate(self) -> Result<(), SettingsError> {
        if !matches!(self.bit_depth, 8 | 12 | 16) {
            return Err(SettingsError::InvalidResolutionLevels);
        }
        if self.quality_layers == 0 {
            return Err(SettingsError::ZeroQualityLayers);
        }
        if self.lossless && self.transform != Transform::Reversible53 {
            return Err(SettingsError::LosslessRequiresReversible);
        }
        if self.lossless && (self.target_rate.is_some() || self.target_psnr.is_some()) {
            return Err(SettingsError::LosslessHasQualityTarget);
        }
        if self.target_rate.is_some() && self.target_psnr.is_some() {
            return Err(SettingsError::QualityTargetConflict);
        }
        if self
            .target_rate
            .is_some_and(|value| !value.is_finite() || value <= 0.0)
        {
            return Err(SettingsError::InvalidRate);
        }
        if self
            .target_psnr
            .is_some_and(|value| !value.is_finite() || value <= 0.0)
        {
            return Err(SettingsError::InvalidPsnr);
        }
        if self.resolution_levels > 32 {
            return Err(SettingsError::InvalidResolutionLevels);
        }
        if !matches!(self.code_block_width, 4 | 8 | 16 | 32 | 64 | 128)
            || !matches!(self.code_block_height, 4 | 8 | 16 | 32 | 64 | 128)
        {
            return Err(SettingsError::InvalidCodeBlock);
        }
        if self
            .precinct_width
            .zip(self.precinct_height)
            .is_some_and(|(width, height)| width > 15 || height > 15)
            || self.precinct_width.is_some() != self.precinct_height.is_some()
        {
            return Err(SettingsError::InvalidPrecinct);
        }
        if self
            .tile_width
            .zip(self.tile_height)
            .is_some_and(|(width, height)| width == 0 || height == 0)
            || self.tile_width.is_some() != self.tile_height.is_some()
        {
            return Err(SettingsError::InvalidTile);
        }
        if self.threads == 0
            || self.max_output_bytes == 0
            || self.max_output_bytes > MAX_OUTPUT_BYTES
        {
            return Err(if self.threads == 0 {
                SettingsError::ZeroThreads
            } else {
                SettingsError::OutputLimit
            });
        }
        Ok(())
    }

    /// The pinned pure-Rust engine currently emits one LRCP, one-tile classic EBCOT stream.
    #[must_use]
    pub const fn engine_supported(self) -> bool {
        matches!(self.progression, ProgressionOrder::Lrcp)
            && self.tile_width.is_none()
            && self.precinct_width.is_none()
            && !self.sop
            && !self.eph
            && self.quality_layers == 1
    }
}
