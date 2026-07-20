//! Shared, checked raster and metadata handling for the pure-Rust document encoders.

use std::fmt;

use rusttable_image::{AlphaMode, ByteOrder, ChannelLayout, SampleType, StorageLayout};
use sha2::{Digest, Sha256};

use crate::CanonicalArtifact;

pub(crate) const MAX_METADATA_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_TEXT_BYTES: usize = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RasterShape {
    pub channels: usize,
    pub sample_bytes: usize,
    pub sample_type: SampleType,
}

pub(crate) fn shape(artifact: &CanonicalArtifact<'_>) -> Result<RasterShape, RasterError> {
    let descriptor = artifact.image().descriptor();
    let format = descriptor.format();
    if descriptor.orientation() != rusttable_image::Orientation::Normal {
        return Err(RasterError::Unsupported("non-normal orientation"));
    }
    if format.storage() != StorageLayout::Interleaved {
        return Err(RasterError::Unsupported("planar storage"));
    }
    if matches!(format.alpha(), AlphaMode::Premultiplied) {
        return Err(RasterError::Unsupported("premultiplied alpha"));
    }
    if !matches!(
        format.channels(),
        ChannelLayout::Gray | ChannelLayout::GrayA | ChannelLayout::Rgb | ChannelLayout::Rgba
    ) {
        return Err(RasterError::Unsupported("mosaic channels"));
    }
    Ok(RasterShape {
        channels: format.channels().channels(),
        sample_bytes: format.sample_type().bytes(),
        sample_type: format.sample_type(),
    })
}

pub(crate) fn metadata_len(artifact: &CanonicalArtifact<'_>) -> usize {
    let metadata = artifact.metadata();
    metadata.icc_profile().map_or(0, <[u8]>::len)
        + metadata.exif().map_or(0, <[u8]>::len)
        + metadata.xmp().map_or(0, <[u8]>::len)
        + metadata.iptc().map_or(0, <[u8]>::len)
        + metadata
            .text()
            .iter()
            .map(|field| field.keyword().len() + field.value().len())
            .sum::<usize>()
}

pub(crate) fn validate_metadata(
    artifact: &CanonicalArtifact<'_>,
    limit: usize,
) -> Result<(), RasterError> {
    if limit == 0 || limit > MAX_METADATA_BYTES {
        return Err(RasterError::InvalidLimit);
    }
    let actual = metadata_len(artifact);
    if actual > limit {
        return Err(RasterError::MetadataLimit { limit, actual });
    }
    if artifact
        .metadata()
        .icc_profile()
        .is_some_and(<[u8]>::is_empty)
    {
        return Err(RasterError::EmptyProfile);
    }
    if artifact.metadata().text().iter().any(|field| {
        field.keyword().is_empty()
            || field.keyword().len() > 255
            || field.keyword().contains('\0')
            || field.value().len() > MAX_TEXT_BYTES
            || field.value().contains('\0')
    }) {
        return Err(RasterError::InvalidText);
    }
    Ok(())
}

pub(crate) fn row<'a>(
    artifact: &'a CanonicalArtifact<'a>,
    y: u32,
) -> Result<&'a [u8], RasterError> {
    let view = artifact.view().map_err(|_| RasterError::InvalidImage)?;
    view.row(0, y).map_err(|_| RasterError::InvalidImage)
}

pub(crate) fn sample_u16(bytes: &[u8], order: ByteOrder) -> Result<u16, RasterError> {
    let pair = bytes.get(..2).ok_or(RasterError::InvalidImage)?;
    Ok(match order {
        ByteOrder::Big => u16::from_be_bytes([pair[0], pair[1]]),
        ByteOrder::Little => u16::from_le_bytes([pair[0], pair[1]]),
        ByteOrder::Native => u16::from_ne_bytes([pair[0], pair[1]]),
    })
}

pub(crate) fn sample_f32(bytes: &[u8], order: ByteOrder) -> Result<f32, RasterError> {
    let bytes = bytes.get(..4).ok_or(RasterError::InvalidImage)?;
    let bits = match order {
        ByteOrder::Big => {
            u32::from_be_bytes(bytes.try_into().map_err(|_| RasterError::InvalidImage)?)
        }
        ByteOrder::Little => {
            u32::from_le_bytes(bytes.try_into().map_err(|_| RasterError::InvalidImage)?)
        }
        ByteOrder::Native => {
            u32::from_ne_bytes(bytes.try_into().map_err(|_| RasterError::InvalidImage)?)
        }
    };
    let value = f32::from_bits(bits);
    if !value.is_finite() {
        return Err(RasterError::NonFiniteSample);
    }
    Ok(value)
}

pub(crate) fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RasterError {
    Unsupported(&'static str),
    InvalidImage,
    InvalidLimit,
    MetadataLimit { limit: usize, actual: usize },
    EmptyProfile,
    InvalidText,
    NonFiniteSample,
}

impl fmt::Display for RasterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(value) => write!(formatter, "unsupported raster: {value}"),
            Self::InvalidImage => formatter.write_str("invalid canonical image"),
            Self::InvalidLimit => formatter.write_str("metadata limit is outside the safe bound"),
            Self::MetadataLimit { limit, actual } => {
                write!(formatter, "metadata is {actual} bytes, limit is {limit}")
            }
            Self::EmptyProfile => formatter.write_str("ICC profile is empty"),
            Self::InvalidText => formatter.write_str("metadata text is invalid"),
            Self::NonFiniteSample => formatter.write_str("non-finite floating sample"),
        }
    }
}

impl std::error::Error for RasterError {}
