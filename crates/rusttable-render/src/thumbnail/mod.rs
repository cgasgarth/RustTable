#![allow(
    clippy::cast_possible_truncation,
    clippy::format_collect,
    clippy::manual_div_ceil,
    clippy::missing_errors_doc
)]

use std::fmt;

use rusttable_core::{
    AssetId, ContentHash, EditId, PhotoId, RenderSizeError, RenderSizeRequest, Revision,
};
use rusttable_image::{
    CancellationToken, ColorEncoding, DecodedImage, ImageDimensions, Orientation,
};
use sha2::{Digest, Sha256};

const KEY_MAGIC: &[u8; 4] = b"TMK1";

/// A power-of-two mipmap level. Level zero is the decoded source size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MipmapLevel(u8);

impl MipmapLevel {
    /// Creates a level whose shift is representable by a 32-bit image axis.
    pub const fn new(value: u8) -> Result<Self, ThumbnailKeyError> {
        if value > 31 {
            Err(ThumbnailKeyError::LevelTooLarge)
        } else {
            Ok(Self(value))
        }
    }

    #[must_use]
    pub const fn zero() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// A requested thumbnail shape. Fit preserves the oriented source aspect ratio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ThumbnailSize {
    Fit { max_width: u32, max_height: u32 },
    Exact { width: u32, height: u32 },
}

impl ThumbnailSize {
    pub fn fit(width: u32, height: u32) -> Result<Self, RenderSizeError> {
        RenderSizeRequest::fit(width, height).map(|_| Self::Fit {
            max_width: width,
            max_height: height,
        })
    }

    pub fn exact(width: u32, height: u32) -> Result<Self, RenderSizeError> {
        RenderSizeRequest::exact(width, height).map(|_| Self::Exact { width, height })
    }

    #[must_use]
    pub const fn dimensions(self) -> (u32, u32) {
        match self {
            Self::Fit {
                max_width,
                max_height,
            } => (max_width, max_height),
            Self::Exact { width, height } => (width, height),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ThumbnailProvenance {
    EmbeddedPreview,
    RawFallback,
    PipelineRender,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResamplingQuality {
    Nearest,
    Box,
}

/// All output-affecting settings for a thumbnail request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThumbnailRequest {
    level: MipmapLevel,
    size: ThumbnailSize,
    orientation: Orientation,
    quality: ResamplingQuality,
    provenance: ThumbnailProvenance,
}

impl ThumbnailRequest {
    #[must_use]
    pub const fn new(level: MipmapLevel, size: ThumbnailSize) -> Self {
        Self {
            level,
            size,
            orientation: Orientation::Normal,
            quality: ResamplingQuality::Box,
            provenance: ThumbnailProvenance::PipelineRender,
        }
    }

    #[must_use]
    pub const fn with_orientation(mut self, value: Orientation) -> Self {
        self.orientation = value;
        self
    }

    #[must_use]
    pub const fn with_provenance(mut self, value: ThumbnailProvenance) -> Self {
        self.provenance = value;
        self
    }

    #[must_use]
    pub const fn with_quality(mut self, value: ResamplingQuality) -> Self {
        self.quality = value;
        self
    }

    #[must_use]
    pub const fn level(self) -> MipmapLevel {
        self.level
    }
    #[must_use]
    pub const fn size(self) -> ThumbnailSize {
        self.size
    }
    #[must_use]
    pub const fn orientation(self) -> Orientation {
        self.orientation
    }
    #[must_use]
    pub const fn quality(self) -> ResamplingQuality {
        self.quality
    }
    #[must_use]
    pub const fn provenance(self) -> ThumbnailProvenance {
        self.provenance
    }
}

/// Stable identity for the source/edit/render/profile inputs of one entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThumbnailKey {
    source_content: ContentHash,
    photo_id: PhotoId,
    asset_id: AssetId,
    edit_id: EditId,
    base_photo_revision: Revision,
    edit_revision: Revision,
    decoder_version: u32,
    renderer_version: u32,
    profile_identity: [u8; 32],
    profile_version: u32,
    configuration_identity: [u8; 32],
    request: ThumbnailRequest,
}

impl ThumbnailKey {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        source_content: ContentHash,
        photo_id: PhotoId,
        asset_id: AssetId,
        edit_id: EditId,
        base_photo_revision: Revision,
        edit_revision: Revision,
        decoder_version: u32,
        renderer_version: u32,
        profile_identity: [u8; 32],
        profile_version: u32,
        configuration_identity: [u8; 32],
        request: ThumbnailRequest,
    ) -> Self {
        Self {
            source_content,
            photo_id,
            asset_id,
            edit_id,
            base_photo_revision,
            edit_revision,
            decoder_version,
            renderer_version,
            profile_identity,
            profile_version,
            configuration_identity,
            request,
        }
    }

    #[must_use]
    pub const fn request(self) -> ThumbnailRequest {
        self.request
    }
    #[must_use]
    pub const fn source_content(self) -> ContentHash {
        self.source_content
    }
    #[must_use]
    pub const fn photo_id(self) -> PhotoId {
        self.photo_id
    }
    #[must_use]
    pub const fn edit_id(self) -> EditId {
        self.edit_id
    }
    #[must_use]
    pub const fn edit_revision(self) -> Revision {
        self.edit_revision
    }
    #[must_use]
    pub const fn profile_identity(self) -> [u8; 32] {
        self.profile_identity
    }
    #[must_use]
    pub const fn profile_version(self) -> u32 {
        self.profile_version
    }
    #[must_use]
    pub const fn decoder_version(self) -> u32 {
        self.decoder_version
    }
    #[must_use]
    pub const fn renderer_version(self) -> u32 {
        self.renderer_version
    }
    #[must_use]
    pub const fn configuration_identity(self) -> [u8; 32] {
        self.configuration_identity
    }

    /// Canonical bytes deliberately avoid debug formatting and platform-sized integers.
    #[must_use]
    pub fn canonical_bytes(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(160);
        bytes.extend_from_slice(KEY_MAGIC);
        bytes.extend_from_slice(self.source_content.bytes());
        bytes.extend_from_slice(&self.photo_id.get().to_be_bytes());
        bytes.extend_from_slice(&self.asset_id.get().to_be_bytes());
        bytes.extend_from_slice(&self.edit_id.get().to_be_bytes());
        bytes.extend_from_slice(&self.base_photo_revision.get().to_be_bytes());
        bytes.extend_from_slice(&self.edit_revision.get().to_be_bytes());
        bytes.extend_from_slice(&self.decoder_version.to_be_bytes());
        bytes.extend_from_slice(&self.renderer_version.to_be_bytes());
        bytes.extend_from_slice(&self.profile_identity);
        bytes.extend_from_slice(&self.profile_version.to_be_bytes());
        bytes.extend_from_slice(&self.configuration_identity);
        bytes.push(self.request.level.get());
        match self.request.size {
            ThumbnailSize::Fit {
                max_width,
                max_height,
            } => {
                bytes.push(0);
                bytes.extend_from_slice(&max_width.to_be_bytes());
                bytes.extend_from_slice(&max_height.to_be_bytes());
            }
            ThumbnailSize::Exact { width, height } => {
                bytes.push(1);
                bytes.extend_from_slice(&width.to_be_bytes());
                bytes.extend_from_slice(&height.to_be_bytes());
            }
        }
        bytes.push(self.request.quality as u8);
        bytes.push(self.request.orientation as u8);
        bytes.push(self.request.provenance as u8);
        bytes
    }

    #[must_use]
    pub fn digest(self) -> [u8; 32] {
        Sha256::digest(self.canonical_bytes()).into()
    }

    #[must_use]
    pub fn digest_hex(self) -> String {
        self.digest()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    pub(crate) fn from_canonical_bytes(bytes: &[u8]) -> Result<Self, ThumbnailKeyError> {
        let mut reader = KeyReader { bytes, offset: 0 };
        if reader.take(4)? != KEY_MAGIC {
            return Err(ThumbnailKeyError::InvalidEncoding);
        }
        let source_content = ContentHash::Sha256(reader.array::<32>()?);
        let photo_id = PhotoId::new(reader.u128()?).ok_or(ThumbnailKeyError::ZeroId)?;
        let asset_id = AssetId::new(reader.u128()?).ok_or(ThumbnailKeyError::ZeroId)?;
        let edit_id = EditId::new(reader.u128()?).ok_or(ThumbnailKeyError::ZeroId)?;
        let base_photo_revision = Revision::from_u64(reader.u64()?);
        let edit_revision = Revision::from_u64(reader.u64()?);
        let decoder_version = reader.u32()?;
        let renderer_version = reader.u32()?;
        let profile_identity = reader.array::<32>()?;
        let profile_version = reader.u32()?;
        let configuration_identity = reader.array::<32>()?;
        let level = MipmapLevel::new(reader.byte()?)?;
        let size = match reader.byte()? {
            0 => ThumbnailSize::fit(reader.u32()?, reader.u32()?)
                .map_err(|_| ThumbnailKeyError::InvalidEncoding)?,
            1 => ThumbnailSize::exact(reader.u32()?, reader.u32()?)
                .map_err(|_| ThumbnailKeyError::InvalidEncoding)?,
            _ => return Err(ThumbnailKeyError::InvalidEncoding),
        };
        let quality = match reader.byte()? {
            0 => ResamplingQuality::Nearest,
            1 => ResamplingQuality::Box,
            _ => return Err(ThumbnailKeyError::InvalidEncoding),
        };
        let orientation = match reader.byte()? {
            1 => Orientation::Normal,
            2 => Orientation::FlipHorizontal,
            3 => Orientation::Rotate180,
            4 => Orientation::FlipVertical,
            5 => Orientation::Transpose,
            6 => Orientation::Rotate90,
            7 => Orientation::Transverse,
            8 => Orientation::Rotate270,
            _ => return Err(ThumbnailKeyError::InvalidEncoding),
        };
        let provenance = match reader.byte()? {
            0 => ThumbnailProvenance::EmbeddedPreview,
            1 => ThumbnailProvenance::RawFallback,
            2 => ThumbnailProvenance::PipelineRender,
            _ => return Err(ThumbnailKeyError::InvalidEncoding),
        };
        if reader.offset != bytes.len() {
            return Err(ThumbnailKeyError::InvalidEncoding);
        }
        Ok(Self::new(
            source_content,
            photo_id,
            asset_id,
            edit_id,
            base_photo_revision,
            edit_revision,
            decoder_version,
            renderer_version,
            profile_identity,
            profile_version,
            configuration_identity,
            ThumbnailRequest::new(level, size)
                .with_quality(quality)
                .with_orientation(orientation)
                .with_provenance(provenance),
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThumbnailError {
    OutputTooLarge { bytes: u64, limit: u64 },
    ArithmeticOverflow,
    Cancelled,
    InvalidImage,
}

impl fmt::Display for ThumbnailError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "thumbnail generation failed: {self:?}")
    }
}
impl std::error::Error for ThumbnailError {}

/// Deterministic, bounded RGBA8 thumbnail generation over the image contract.
pub struct ThumbnailGenerator;

impl ThumbnailGenerator {
    /// Generates a complete image in memory. Cancellation is checked per row
    /// and per resampling sample, so no partial image can be published.
    pub fn generate(
        source: &DecodedImage,
        request: ThumbnailRequest,
        max_output_bytes: u64,
        cancellation: &CancellationToken,
    ) -> Result<DecodedImage, ThumbnailError> {
        let level_dimensions = mipmap_dimensions(source.dimensions(), request.level);
        let oriented_dimensions = request.orientation.output_dimensions(level_dimensions);
        let dimensions = resolve_size(request.size, oriented_dimensions)?;
        let bytes = dimensions
            .decoded_byte_count()
            .map_err(|_| ThumbnailError::ArithmeticOverflow)?;
        if bytes > max_output_bytes {
            return Err(ThumbnailError::OutputTooLarge {
                bytes,
                limit: max_output_bytes,
            });
        }
        let capacity = usize::try_from(bytes).map_err(|_| ThumbnailError::ArithmeticOverflow)?;
        let mut output = Vec::with_capacity(capacity);
        for y in 0..dimensions.height() {
            if cancellation.is_cancelled() {
                return Err(ThumbnailError::Cancelled);
            }
            for x in 0..dimensions.width() {
                output.extend_from_slice(&sample_pixel(
                    source,
                    request,
                    level_dimensions,
                    oriented_dimensions,
                    dimensions,
                    x,
                    y,
                    cancellation,
                )?);
            }
        }
        DecodedImage::new_with_color_encoding(dimensions, output, ColorEncoding::Srgb)
            .map_err(|_| ThumbnailError::InvalidImage)
    }
}

fn resolve_size(
    size: ThumbnailSize,
    source: ImageDimensions,
) -> Result<ImageDimensions, ThumbnailError> {
    let request = match size {
        ThumbnailSize::Fit {
            max_width,
            max_height,
        } => RenderSizeRequest::fit(max_width, max_height),
        ThumbnailSize::Exact { width, height } => RenderSizeRequest::exact(width, height),
    }
    .map_err(|_| ThumbnailError::InvalidImage)?;
    let (width, height) = request
        .resolve(source.width(), source.height())
        .map_err(|_| ThumbnailError::InvalidImage)?;
    ImageDimensions::new(width, height).map_err(|_| ThumbnailError::InvalidImage)
}

fn mipmap_dimensions(source: ImageDimensions, level: MipmapLevel) -> ImageDimensions {
    let factor = 1_u64 << level.get();
    ImageDimensions::new(
        (u64::from(source.width()) / factor).max(1) as u32,
        (u64::from(source.height()) / factor).max(1) as u32,
    )
    .expect("source dimensions are nonzero")
}

#[allow(clippy::too_many_arguments)]
fn sample_pixel(
    source: &DecodedImage,
    request: ThumbnailRequest,
    level_dimensions: ImageDimensions,
    oriented_dimensions: ImageDimensions,
    output_dimensions: ImageDimensions,
    x: u32,
    y: u32,
    cancellation: &CancellationToken,
) -> Result<[u8; 4], ThumbnailError> {
    let (x_start, x_end, y_start, y_end) = match request.quality {
        ResamplingQuality::Nearest => {
            let sx = center_index(x, output_dimensions.width(), oriented_dimensions.width());
            let sy = center_index(y, output_dimensions.height(), oriented_dimensions.height());
            (sx, sx + 1, sy, sy + 1)
        }
        ResamplingQuality::Box => (
            x * oriented_dimensions.width() / output_dimensions.width(),
            (((u64::from(x) + 1) * u64::from(oriented_dimensions.width())
                + u64::from(output_dimensions.width())
                - 1)
                / u64::from(output_dimensions.width())) as u32,
            y * oriented_dimensions.height() / output_dimensions.height(),
            (((u64::from(y) + 1) * u64::from(oriented_dimensions.height())
                + u64::from(output_dimensions.height())
                - 1)
                / u64::from(output_dimensions.height())) as u32,
        ),
    };
    let mut sum = [0_u64; 4];
    let mut count = 0_u64;
    for oriented_y in y_start..y_end.max(y_start + 1).min(oriented_dimensions.height()) {
        for oriented_x in x_start..x_end.max(x_start + 1).min(oriented_dimensions.width()) {
            if cancellation.is_cancelled() {
                return Err(ThumbnailError::Cancelled);
            }
            let (level_x, level_y) = request.orientation.inverse().map_source_to_output(
                oriented_dimensions,
                oriented_x,
                oriented_y,
            );
            let factor = 1_u64 << request.level.get();
            let source_x = (u64::from(level_x) * factor + factor / 2)
                .min(u64::from(source.dimensions().width() - 1)) as u32;
            let source_y = (u64::from(level_y) * factor + factor / 2)
                .min(u64::from(source.dimensions().height() - 1)) as u32;
            let offset = source
                .descriptor()
                .pixel_offset(source_x, source_y)
                .map_err(|_| ThumbnailError::InvalidImage)?;
            for (channel, value) in source.pixels()[offset..offset + 4].iter().enumerate() {
                sum[channel] += u64::from(*value);
            }
            count += 1;
        }
    }
    if count == 0 || level_dimensions.width() == 0 {
        return Err(ThumbnailError::InvalidImage);
    }
    let mut result = [0_u8; 4];
    for (channel, value) in result.iter_mut().enumerate() {
        *value = u8::try_from((sum[channel] + count / 2) / count)
            .map_err(|_| ThumbnailError::ArithmeticOverflow)?;
    }
    Ok(result)
}

fn center_index(index: u32, output: u32, source: u32) -> u32 {
    (((u64::from(index) * 2 + 1) * u64::from(source)) / (2 * u64::from(output)))
        .min(u64::from(source - 1)) as u32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThumbnailKeyError {
    LevelTooLarge,
    ZeroId,
    InvalidEncoding,
}

impl fmt::Display for ThumbnailKeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::LevelTooLarge => "mipmap level exceeds the checked 32-bit envelope",
            Self::ZeroId => "thumbnail key contains a zero identifier",
            Self::InvalidEncoding => "thumbnail key encoding is invalid",
        })
    }
}
impl std::error::Error for ThumbnailKeyError {}

struct KeyReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> KeyReader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], ThumbnailKeyError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or(ThumbnailKeyError::InvalidEncoding)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(ThumbnailKeyError::InvalidEncoding)?;
        self.offset = end;
        Ok(value)
    }

    fn byte(&mut self) -> Result<u8, ThumbnailKeyError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, ThumbnailKeyError> {
        Ok(u32::from_be_bytes(
            self.take(4)?.try_into().expect("checked length"),
        ))
    }
    fn u64(&mut self) -> Result<u64, ThumbnailKeyError> {
        Ok(u64::from_be_bytes(
            self.take(8)?.try_into().expect("checked length"),
        ))
    }
    fn u128(&mut self) -> Result<u128, ThumbnailKeyError> {
        Ok(u128::from_be_bytes(
            self.take(16)?.try_into().expect("checked length"),
        ))
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], ThumbnailKeyError> {
        Ok(self.take(N)?.try_into().expect("checked length"))
    }
}
pub(crate) mod cache;
pub(crate) mod lifecycle;
pub(crate) mod scheduler;
