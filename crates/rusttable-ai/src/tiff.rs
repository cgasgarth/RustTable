use std::fmt;

use sha2::{Digest, Sha256};

use crate::{ColorProfile, ImageDimensions};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TiffBitDepth {
    Eight,
    Sixteen,
    ThirtyTwo,
}

impl TiffBitDepth {
    #[must_use]
    pub const fn bits(self) -> u16 {
        match self {
            Self::Eight => 8,
            Self::Sixteen => 16,
            Self::ThirtyTwo => 32,
        }
    }

    #[must_use]
    pub const fn bytes(self) -> usize {
        match self {
            Self::Eight => 1,
            Self::Sixteen => 2,
            Self::ThirtyTwo => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TiffSampleFormat {
    UnsignedInteger,
    IeeeFloat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TiffSettings {
    pub(crate) bit_depth: TiffBitDepth,
    pub(crate) alpha: bool,
    pub(crate) max_bytes: u64,
}

impl Default for TiffSettings {
    fn default() -> Self {
        Self {
            bit_depth: TiffBitDepth::Sixteen,
            alpha: true,
            max_bytes: 512 * 1024 * 1024,
        }
    }
}

impl TiffSettings {
    pub fn new(
        bit_depth: TiffBitDepth,
        alpha: bool,
        max_bytes: u64,
    ) -> Result<Self, TiffSettingsError> {
        if max_bytes == 0 {
            return Err(TiffSettingsError::ZeroByteLimit);
        }
        Ok(Self {
            bit_depth,
            alpha,
            max_bytes,
        })
    }

    #[must_use]
    pub const fn bit_depth(&self) -> TiffBitDepth {
        self.bit_depth
    }

    #[must_use]
    pub const fn alpha(&self) -> bool {
        self.alpha
    }

    #[must_use]
    pub const fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    #[must_use]
    pub const fn sample_format(&self) -> TiffSampleFormat {
        match self.bit_depth {
            TiffBitDepth::ThirtyTwo => TiffSampleFormat::IeeeFloat,
            _ => TiffSampleFormat::UnsignedInteger,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TiffProbe {
    dimensions: ImageDimensions,
    bit_depth: TiffBitDepth,
    sample_format: TiffSampleFormat,
    channels: u16,
    icc_sha256: String,
}

impl TiffProbe {
    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn bit_depth(&self) -> TiffBitDepth {
        self.bit_depth
    }
    #[must_use]
    pub const fn sample_format(&self) -> TiffSampleFormat {
        self.sample_format
    }
    #[must_use]
    pub const fn channels(&self) -> u16 {
        self.channels
    }
    #[must_use]
    pub fn icc_sha256(&self) -> &str {
        &self.icc_sha256
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TiffSettingsError {
    ZeroByteLimit,
}

impl fmt::Display for TiffSettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TIFF byte limit must be nonzero")
    }
}
impl std::error::Error for TiffSettingsError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TiffError {
    InvalidHeader,
    MissingTag(u16),
    InvalidTag,
    ProfileMismatch,
    DimensionOverflow,
    ByteLimit,
    NonFinite,
    UnsupportedOffset,
}

impl fmt::Display for TiffError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid color-managed TIFF: {self:?}")
    }
}
impl std::error::Error for TiffError {}

pub(crate) fn encode_tiff(
    dimensions: ImageDimensions,
    pixels: &[[f32; 4]],
    profile: &ColorProfile,
    settings: &TiffSettings,
) -> Result<Vec<u8>, TiffError> {
    let expected = dimensions.pixels().ok_or(TiffError::DimensionOverflow)?;
    if pixels.len() != expected {
        return Err(TiffError::DimensionOverflow);
    }
    let channels = if settings.alpha { 4usize } else { 3usize };
    let sample_bytes = settings.bit_depth.bytes();
    let pixel_bytes = expected
        .checked_mul(channels)
        .and_then(|value| value.checked_mul(sample_bytes))
        .ok_or(TiffError::DimensionOverflow)?;
    let ifd_count: u16 = if settings.alpha { 13 } else { 12 };
    let ifd_bytes = 2usize
        .checked_add(
            usize::from(ifd_count)
                .checked_mul(12)
                .ok_or(TiffError::DimensionOverflow)?,
        )
        .and_then(|value| value.checked_add(4))
        .ok_or(TiffError::DimensionOverflow)?;
    let extra_bytes = 6usize
        .checked_add(profile.icc().len())
        .ok_or(TiffError::DimensionOverflow)?;
    let pixel_offset = 8usize
        .checked_add(ifd_bytes)
        .and_then(|value| value.checked_add(extra_bytes))
        .ok_or(TiffError::DimensionOverflow)?;
    let total = pixel_offset
        .checked_add(pixel_bytes)
        .ok_or(TiffError::DimensionOverflow)?;
    if u32::try_from(pixel_offset).is_err()
        || u32::try_from(pixel_bytes).is_err()
        || total as u64 > settings.max_bytes
    {
        return Err(TiffError::ByteLimit);
    }

    let mut output = Vec::with_capacity(total);
    output.extend_from_slice(b"II");
    output.extend_from_slice(&42u16.to_le_bytes());
    output.extend_from_slice(&8u32.to_le_bytes());
    output.extend_from_slice(&ifd_count.to_le_bytes());
    let bits_offset = 8usize + ifd_bytes;
    let profile_offset = bits_offset + 6;
    let mut entries = Vec::with_capacity(usize::from(ifd_count));
    entries.push((256, 4, 1, u32::from(dimensions.width())));
    entries.push((257, 4, 1, u32::from(dimensions.height())));
    entries.push((
        258,
        3,
        3,
        u32::try_from(bits_offset).map_err(|_| TiffError::UnsupportedOffset)?,
    ));
    entries.push((259, 3, 1, 1));
    entries.push((262, 3, 1, 2));
    entries.push((
        273,
        4,
        1,
        u32::try_from(pixel_offset).map_err(|_| TiffError::UnsupportedOffset)?,
    ));
    entries.push((
        277,
        3,
        1,
        u32::try_from(channels).map_err(|_| TiffError::UnsupportedOffset)?,
    ));
    entries.push((278, 4, 1, u32::from(dimensions.height())));
    entries.push((
        279,
        4,
        1,
        u32::try_from(pixel_bytes).map_err(|_| TiffError::UnsupportedOffset)?,
    ));
    entries.push((284, 3, 1, 1));
    if settings.alpha {
        entries.push((338, 3, 1, 1));
    }
    entries.push((
        339,
        3,
        1,
        if settings.sample_format() == TiffSampleFormat::IeeeFloat {
            3
        } else {
            1
        },
    ));
    entries.push((
        34675,
        7,
        u32::try_from(profile.icc().len()).map_err(|_| TiffError::UnsupportedOffset)?,
        u32::try_from(profile_offset).map_err(|_| TiffError::UnsupportedOffset)?,
    ));
    for (tag, kind, count, value) in entries {
        write_entry(&mut output, tag, kind, count, value);
    }
    output.extend_from_slice(&0u32.to_le_bytes());
    for bit in [settings.bit_depth.bits(); 3] {
        output.extend_from_slice(&bit.to_le_bytes());
    }
    output.extend_from_slice(profile.icc());
    while output.len() < pixel_offset {
        output.push(0);
    }
    for pixel in pixels {
        write_pixel(&mut output, *pixel, settings);
    }
    Ok(output)
}

fn write_entry(output: &mut Vec<u8>, tag: u16, kind: u16, count: u32, value: u32) {
    output.extend_from_slice(&tag.to_le_bytes());
    output.extend_from_slice(&kind.to_le_bytes());
    output.extend_from_slice(&count.to_le_bytes());
    output.extend_from_slice(&value.to_le_bytes());
}

fn write_pixel(output: &mut Vec<u8>, pixel: [f32; 4], settings: &TiffSettings) {
    let channels = if settings.alpha { 4 } else { 3 };
    for value in pixel.into_iter().take(channels) {
        match settings.bit_depth {
            TiffBitDepth::Eight => output.push((value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8),
            TiffBitDepth::Sixteen => output
                .extend_from_slice(&((value.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16).to_le_bytes()),
            TiffBitDepth::ThirtyTwo => output.extend_from_slice(&value.to_le_bytes()),
        }
    }
}

/// Independently checks the bounded classic-TIFF structure produced here.
pub fn verify_tiff_artifact(bytes: &[u8]) -> Result<TiffProbe, TiffError> {
    if bytes.len() < 10 || &bytes[0..2] != b"II" || u16::from_le_bytes([bytes[2], bytes[3]]) != 42 {
        return Err(TiffError::InvalidHeader);
    }
    let ifd = usize::try_from(u32::from_le_bytes(
        bytes[4..8]
            .try_into()
            .map_err(|_| TiffError::InvalidHeader)?,
    ))
    .map_err(|_| TiffError::InvalidHeader)?;
    if ifd + 2 > bytes.len() {
        return Err(TiffError::InvalidHeader);
    }
    let count = usize::from(u16::from_le_bytes(
        bytes[ifd..ifd + 2]
            .try_into()
            .map_err(|_| TiffError::InvalidHeader)?,
    ));
    let entries_end = ifd
        .checked_add(2 + count * 12)
        .ok_or(TiffError::InvalidHeader)?;
    if entries_end + 4 > bytes.len() {
        return Err(TiffError::InvalidHeader);
    }
    let mut tags = std::collections::BTreeMap::new();
    for index in 0..count {
        let start = ifd + 2 + index * 12;
        let tag = u16::from_le_bytes(
            bytes[start..start + 2]
                .try_into()
                .map_err(|_| TiffError::InvalidTag)?,
        );
        let kind = u16::from_le_bytes(
            bytes[start + 2..start + 4]
                .try_into()
                .map_err(|_| TiffError::InvalidTag)?,
        );
        let count = u32::from_le_bytes(
            bytes[start + 4..start + 8]
                .try_into()
                .map_err(|_| TiffError::InvalidTag)?,
        );
        let value = u32::from_le_bytes(
            bytes[start + 8..start + 12]
                .try_into()
                .map_err(|_| TiffError::InvalidTag)?,
        );
        tags.insert(tag, (kind, count, value));
    }
    let dimension = |tag: u16| -> Result<u32, TiffError> {
        let (_, count, value) = tags.get(&tag).copied().ok_or(TiffError::MissingTag(tag))?;
        (count == 1).then_some(value).ok_or(TiffError::InvalidTag)
    };
    let dimensions = ImageDimensions::new(dimension(256)?, dimension(257)?)
        .map_err(|_| TiffError::DimensionOverflow)?;
    let channels = u16::try_from(dimension(277)?).map_err(|_| TiffError::InvalidTag)?;
    let bits_tag = tags.get(&258).copied().ok_or(TiffError::MissingTag(258))?;
    if bits_tag.0 != 3 || bits_tag.1 != 3 {
        return Err(TiffError::InvalidTag);
    }
    let bits_offset = usize::try_from(bits_tag.2).map_err(|_| TiffError::InvalidHeader)?;
    let bits_end = bits_offset.checked_add(6).ok_or(TiffError::InvalidHeader)?;
    if bits_end > bytes.len() {
        return Err(TiffError::InvalidHeader);
    }
    let bits = u16::from_le_bytes(
        bytes[bits_offset..bits_offset + 2]
            .try_into()
            .map_err(|_| TiffError::InvalidTag)?,
    );
    if [
        bits,
        u16::from_le_bytes(
            bytes[bits_offset + 2..bits_offset + 4]
                .try_into()
                .map_err(|_| TiffError::InvalidTag)?,
        ),
        u16::from_le_bytes(
            bytes[bits_offset + 4..bits_offset + 6]
                .try_into()
                .map_err(|_| TiffError::InvalidTag)?,
        ),
    ] != [bits; 3]
    {
        return Err(TiffError::InvalidTag);
    }
    let bit_depth = match bits {
        8 => TiffBitDepth::Eight,
        16 => TiffBitDepth::Sixteen,
        32 => TiffBitDepth::ThirtyTwo,
        _ => return Err(TiffError::InvalidTag),
    };
    let sample_format = match dimension(339)? {
        1 => TiffSampleFormat::UnsignedInteger,
        3 => TiffSampleFormat::IeeeFloat,
        _ => return Err(TiffError::InvalidTag),
    };
    let (kind, profile_count, profile_offset) = tags
        .get(&34675)
        .copied()
        .ok_or(TiffError::MissingTag(34675))?;
    if kind != 7 {
        return Err(TiffError::InvalidTag);
    }
    let start = usize::try_from(profile_offset).map_err(|_| TiffError::InvalidHeader)?;
    let end = start
        .checked_add(usize::try_from(profile_count).map_err(|_| TiffError::InvalidHeader)?)
        .ok_or(TiffError::InvalidHeader)?;
    if end > bytes.len() {
        return Err(TiffError::InvalidHeader);
    }
    let icc_sha256 = Sha256::digest(&bytes[start..end])
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(TiffProbe {
        dimensions,
        bit_depth,
        sample_format,
        channels,
        icc_sha256,
    })
}
