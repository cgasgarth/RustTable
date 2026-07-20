use std::fmt;

use rusttable_image::{ChannelLayout, SampleType};

pub const SETTINGS_SCHEMA_VERSION: u16 = 1;
pub const MAX_TILE_EDGE: u32 = 4096;
pub const DEFAULT_MAX_OUTPUT_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Rle,
    Zip1,
    Zip16,
    Piz,
    Pxr24,
    B44,
    B44A,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Storage {
    ScanLines,
    Tiles { width: u32, height: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineOrder {
    Increasing,
    Decreasing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonFinitePolicy {
    Reject,
    PreserveNonFinite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub sample_type: SampleType,
    pub channels: ChannelLayout,
    pub compression: Compression,
    pub storage: Storage,
    pub line_order: LineOrder,
    pub non_finite: NonFinitePolicy,
    pub max_output_bytes: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sample_type: SampleType::F32,
            channels: ChannelLayout::Rgba,
            compression: Compression::Zip16,
            storage: Storage::ScanLines,
            line_order: LineOrder::Increasing,
            non_finite: NonFinitePolicy::Reject,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsError {
    UnsupportedChannels(ChannelLayout),
    UnsupportedSample(SampleType),
    ZeroOutputLimit,
    TileTooLarge,
    ZeroTile,
    LossyCompressionForF32,
    LossyCompressionForAlpha,
}

impl fmt::Display for SettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedChannels(layout) => {
                write!(formatter, "unsupported OpenEXR channels: {layout:?}")
            }
            Self::UnsupportedSample(sample) => {
                write!(formatter, "unsupported OpenEXR sample type: {sample:?}")
            }
            Self::ZeroOutputLimit => formatter.write_str("OpenEXR output limit must be nonzero"),
            Self::TileTooLarge => formatter.write_str("OpenEXR tile edge exceeds the safe bound"),
            Self::ZeroTile => formatter.write_str("OpenEXR tile dimensions must be nonzero"),
            Self::LossyCompressionForF32 => {
                formatter.write_str("lossy OpenEXR compression is not valid for f32")
            }
            Self::LossyCompressionForAlpha => {
                formatter.write_str("B44 OpenEXR compression is not valid for alpha")
            }
        }
    }
}

impl std::error::Error for SettingsError {}

impl Settings {
    /// Validates the settings against the supported bounded encoder contract.
    ///
    /// # Errors
    ///
    /// Returns the first unsupported channel, sample, tile, limit, or
    /// compression combination.
    pub fn validate(self) -> Result<(), SettingsError> {
        if !matches!(
            self.channels,
            ChannelLayout::Gray | ChannelLayout::GrayA | ChannelLayout::Rgb | ChannelLayout::Rgba
        ) {
            return Err(SettingsError::UnsupportedChannels(self.channels));
        }
        if !matches!(self.sample_type, SampleType::F16 | SampleType::F32) {
            return Err(SettingsError::UnsupportedSample(self.sample_type));
        }
        if self.max_output_bytes == 0 {
            return Err(SettingsError::ZeroOutputLimit);
        }
        if let Storage::Tiles { width, height } = self.storage {
            if width == 0 || height == 0 {
                return Err(SettingsError::ZeroTile);
            }
            if width > MAX_TILE_EDGE || height > MAX_TILE_EDGE {
                return Err(SettingsError::TileTooLarge);
            }
        }
        if matches!(self.compression, Compression::Pxr24) && self.sample_type == SampleType::F32 {
            return Err(SettingsError::LossyCompressionForF32);
        }
        if matches!(self.compression, Compression::B44 | Compression::B44A)
            && matches!(self.channels, ChannelLayout::GrayA | ChannelLayout::Rgba)
        {
            return Err(SettingsError::LossyCompressionForAlpha);
        }
        Ok(())
    }

    #[must_use]
    pub fn canonical_bytes(self) -> Vec<u8> {
        format!(
            "schema={SETTINGS_SCHEMA_VERSION}\nsample={:?}\nchannels={:?}\ncompression={:?}\nstorage={:?}\nline_order={:?}\nnon_finite={:?}\nmax_output_bytes={}\n",
            self.sample_type,
            self.channels,
            self.compression,
            self.storage,
            self.line_order,
            self.non_finite,
            self.max_output_bytes
        )
        .into_bytes()
    }
}
