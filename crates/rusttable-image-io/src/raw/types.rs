use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

mod errors;
mod receipt;
#[cfg(test)]
mod tests;

pub use errors::{
    RawCapabilityError, RawCapabilityKind, RawDecodeError, RawFrameValidationError, RawSourceError,
};
pub use receipt::{RawDecodeReceipt, RawDecodeResult, RawDngReceipt};

/// Stable identity of the only initial camera-RAW backend.
pub const RAWLER_BACKEND_ID: &str = "rawler-0.7.2";

/// A proven camera-RAW container family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RawContainerKind {
    Dng,
    Raf,
    Cr2,
    Cr3,
    Crw,
    Nef,
    Nrw,
    Arw,
    Sr2,
    Srf,
    Orf,
    Rw2,
    Rwl,
    Pef,
    Srw,
    Erf,
    Iiq,
    ThreeFr,
    Fff,
    Mrw,
    X3f,
    TiffRaw,
}

/// Evidence for how sensor samples are compressed in the source container.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RawCompression {
    Uncompressed,
    LosslessJpeg,
    LossyJpeg,
    Deflate,
    JpegXl,
    FujiCompressed,
    CanonCrx,
    Vendor(u32),
    BackendDefined,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCompressionEvidence {
    pub compression: RawCompression,
    pub container_code: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCameraEvidence {
    pub maker: Option<String>,
    pub model: Option<String>,
}

/// Bounded structure proving that a generic container carries camera-RAW data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawProbeEvidence {
    pub signature: Vec<u8>,
    pub raw_tags: Vec<u16>,
    pub camera: RawCameraEvidence,
    pub compression: RawCompressionEvidence,
    pub bit_depth: Option<u8>,
}

/// Bounded evidence explaining why a backend result was not accepted by the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCapabilityEvidence {
    pub signature: Vec<u8>,
    pub raw_tags: Vec<u16>,
    pub backend_format: String,
    pub compression: RawCompressionEvidence,
    pub bit_depth: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawContainerProbe {
    pub container: RawContainerKind,
    pub evidence: RawProbeEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawProbeOutcome {
    NoMatch,
    Match(RawContainerProbe),
    MalformedRecognized {
        container: RawContainerKind,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawDimensions {
    pub width: u32,
    pub height: u32,
}

impl RawDimensions {
    /// Creates non-zero RAW dimensions.
    ///
    /// # Errors
    ///
    /// Returns [`RawFrameValidationError::ZeroDimensions`] for a zero axis.
    pub fn new(width: u32, height: u32) -> Result<Self, RawFrameValidationError> {
        if width == 0 || height == 0 {
            return Err(RawFrameValidationError::ZeroDimensions);
        }
        Ok(Self { width, height })
    }

    fn sample_count(self, channels: u8) -> Result<u64, RawFrameValidationError> {
        u64::from(self.width)
            .checked_mul(u64::from(self.height))
            .and_then(|pixels| pixels.checked_mul(u64::from(channels)))
            .ok_or(RawFrameValidationError::ArithmeticOverflow)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl RawRect {
    /// Creates a non-empty rectangle whose edges do not overflow.
    ///
    /// # Errors
    ///
    /// Returns an area or arithmetic validation error for invalid coordinates.
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Result<Self, RawFrameValidationError> {
        if width == 0 || height == 0 {
            return Err(RawFrameValidationError::EmptyArea);
        }
        x.checked_add(width)
            .and_then(|_| y.checked_add(height))
            .ok_or(RawFrameValidationError::ArithmeticOverflow)?;
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    /// Validates that this rectangle fits inside `dimensions`.
    ///
    /// # Errors
    ///
    /// Returns an area or arithmetic validation error when it does not fit.
    pub fn validate_within(self, dimensions: RawDimensions) -> Result<(), RawFrameValidationError> {
        let right = self
            .x
            .checked_add(self.width)
            .ok_or(RawFrameValidationError::ArithmeticOverflow)?;
        let bottom = self
            .y
            .checked_add(self.height)
            .ok_or(RawFrameValidationError::ArithmeticOverflow)?;
        if right > dimensions.width || bottom > dimensions.height {
            return Err(RawFrameValidationError::AreaOutOfBounds);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RawChannel {
    Red,
    Green,
    Blue,
    Cyan,
    Magenta,
    Yellow,
    White,
    FujiGreen,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RawCfa {
    pub width: u8,
    pub height: u8,
    pub phase_x: u8,
    pub phase_y: u8,
    pub pattern: Vec<RawChannel>,
}

impl RawCfa {
    /// Validates CFA dimensions, phase, and bounded pattern length.
    ///
    /// # Errors
    ///
    /// Returns a CFA or arithmetic validation error for inconsistent metadata.
    pub fn validate(&self, max_cells: u16) -> Result<(), RawFrameValidationError> {
        if self.width == 0 || self.height == 0 {
            return Err(RawFrameValidationError::InvalidCfa);
        }
        let cells = u16::from(self.width)
            .checked_mul(u16::from(self.height))
            .ok_or(RawFrameValidationError::ArithmeticOverflow)?;
        if cells > max_cells || usize::from(cells) != self.pattern.len() {
            return Err(RawFrameValidationError::InvalidCfa);
        }
        if self.phase_x >= self.width || self.phase_y >= self.height {
            return Err(RawFrameValidationError::InvalidCfaPhase);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawPlaneLayout {
    Mosaic(RawCfa),
    Linear { channels: Vec<RawChannel> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPlane {
    pub dimensions: RawDimensions,
    pub channels_per_pixel: u8,
    pub layout: RawPlaneLayout,
    pub samples: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawLevelPattern {
    pub repeat_width: u8,
    pub repeat_height: u8,
    pub channels: u8,
    pub values: Vec<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawOrientation {
    Normal,
    HorizontalFlip,
    Rotate180,
    VerticalFlip,
    Transpose,
    Rotate90,
    Transverse,
    Rotate270,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RawCameraIdentity {
    pub maker: String,
    pub model: String,
    pub normalized_maker: String,
    pub normalized_model: String,
    pub mode: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RawIlluminant {
    Unknown,
    Daylight,
    Fluorescent,
    Tungsten,
    Flash,
    FineWeather,
    CloudyWeather,
    Shade,
    DaylightFluorescent,
    DaylightWhiteFluorescent,
    CoolWhiteFluorescent,
    WhiteFluorescent,
    A,
    B,
    C,
    D50,
    D55,
    D65,
    D75,
    IsoStudioTungsten,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawColorMatrix {
    pub illuminant: RawIlluminant,
    pub rows: u8,
    pub columns: u8,
    pub coefficients: Vec<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawOpcodeStage {
    BeforeDemosaic,
    AfterDemosaic,
    AfterGainMap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawOpcodeDescriptor {
    pub stage: RawOpcodeStage,
    pub byte_length: u32,
    pub sha256: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawPreviewKind {
    Thumbnail,
    Preview,
    FullSize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawPreviewFormat {
    Jpeg,
    Tiff,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPreviewDescriptor {
    pub kind: RawPreviewKind,
    pub format: RawPreviewFormat,
    pub offset: u64,
    pub byte_length: u64,
    pub dimensions: Option<RawDimensions>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawFrameParts {
    pub planes: Vec<RawPlane>,
    pub active_area: RawRect,
    pub crop_area: RawRect,
    pub masked_areas: Vec<RawRect>,
    pub black_levels: RawLevelPattern,
    pub white_levels: Vec<f32>,
    pub orientation: RawOrientation,
    pub camera: RawCameraIdentity,
    pub color_matrices: Vec<RawColorMatrix>,
    /// RGBE white-balance samples; `None` preserves a backend-declared missing channel.
    pub white_balance: Vec<Option<f32>>,
    pub compression: RawCompressionEvidence,
    pub bit_depth: u8,
    pub opcodes: Vec<RawOpcodeDescriptor>,
    pub previews: Vec<RawPreviewDescriptor>,
}

/// Validated, owned sensor/linear output. Backend types never cross this boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct RawFrame {
    parts: RawFrameParts,
}

impl RawFrame {
    /// Constructs a frame only after all owned data satisfies `limits` and the RAW contract.
    ///
    /// # Errors
    ///
    /// Returns [`RawFrameValidationError`] for inconsistent or over-limit frame data.
    pub fn try_new(
        parts: RawFrameParts,
        limits: RawDecodeLimits,
    ) -> Result<Self, RawFrameValidationError> {
        validate_frame(&parts, limits)?;
        Ok(Self { parts })
    }

    #[must_use]
    pub const fn parts(&self) -> &RawFrameParts {
        &self.parts
    }

    #[must_use]
    pub fn into_parts(self) -> RawFrameParts {
        self.parts
    }
}

fn validate_frame(
    parts: &RawFrameParts,
    limits: RawDecodeLimits,
) -> Result<(), RawFrameValidationError> {
    if parts.planes.is_empty() || parts.planes.len() > usize::from(limits.max_planes) {
        return Err(RawFrameValidationError::PlaneCount);
    }
    let dimensions = parts.planes[0].dimensions;
    validate_dimensions(dimensions, limits)?;
    let mut total_samples = 0_u64;
    for plane in &parts.planes {
        if plane.dimensions != dimensions || plane.channels_per_pixel == 0 {
            return Err(RawFrameValidationError::PlaneLayout);
        }
        let expected = plane.dimensions.sample_count(plane.channels_per_pixel)?;
        if usize::try_from(expected).ok() != Some(plane.samples.len()) {
            return Err(RawFrameValidationError::PlaneSize {
                expected,
                actual: u64::try_from(plane.samples.len()).unwrap_or(u64::MAX),
            });
        }
        total_samples = total_samples
            .checked_add(expected)
            .ok_or(RawFrameValidationError::ArithmeticOverflow)?;
        match &plane.layout {
            RawPlaneLayout::Mosaic(cfa) => {
                cfa.validate(limits.max_cfa_cells)?;
                if plane.channels_per_pixel != 1 {
                    return Err(RawFrameValidationError::PlaneLayout);
                }
            }
            RawPlaneLayout::Linear { channels } => {
                if channels.len() != usize::from(plane.channels_per_pixel) {
                    return Err(RawFrameValidationError::PlaneLayout);
                }
            }
        }
    }
    if total_samples > limits.max_samples {
        return Err(RawFrameValidationError::SampleLimit {
            actual: total_samples,
            limit: limits.max_samples,
        });
    }
    let decoded_bytes = total_samples
        .checked_mul(2)
        .ok_or(RawFrameValidationError::ArithmeticOverflow)?;
    if decoded_bytes > limits.max_decoded_bytes {
        return Err(RawFrameValidationError::DecodedByteLimit {
            actual: decoded_bytes,
            limit: limits.max_decoded_bytes,
        });
    }
    parts.active_area.validate_within(dimensions)?;
    parts.crop_area.validate_within(dimensions)?;
    for area in &parts.masked_areas {
        area.validate_within(dimensions)?;
    }
    if parts.masked_areas.len() > usize::from(limits.max_metadata_items) {
        return Err(RawFrameValidationError::MetadataLimit);
    }
    validate_levels(&parts.black_levels, &parts.white_levels, limits)?;
    if parts.bit_depth == 0 || parts.bit_depth > 16 {
        return Err(RawFrameValidationError::BitDepth);
    }
    if parts.white_balance.len() > 4
        || parts
            .white_balance
            .iter()
            .flatten()
            .any(|value| !value.is_finite() || *value <= 0.0)
    {
        return Err(RawFrameValidationError::NonFiniteMetadata);
    }
    validate_color_metadata(parts, limits)?;
    validate_identity(&parts.camera, limits.max_string_bytes)
}

fn validate_color_metadata(
    parts: &RawFrameParts,
    limits: RawDecodeLimits,
) -> Result<(), RawFrameValidationError> {
    if parts.color_matrices.len() > usize::from(limits.max_metadata_items)
        || parts.opcodes.len() > usize::from(limits.max_metadata_items)
        || parts.previews.len() > usize::from(limits.max_previews)
    {
        return Err(RawFrameValidationError::MetadataLimit);
    }
    for matrix in &parts.color_matrices {
        let expected = usize::from(matrix.rows)
            .checked_mul(usize::from(matrix.columns))
            .ok_or(RawFrameValidationError::ArithmeticOverflow)?;
        if matrix.rows == 0
            || matrix.columns == 0
            || expected != matrix.coefficients.len()
            || matrix.coefficients.iter().any(|value| !value.is_finite())
        {
            return Err(RawFrameValidationError::InvalidMatrix);
        }
    }
    let metadata_bytes = parts
        .opcodes
        .iter()
        .try_fold(0_u64, |total, opcode| {
            total.checked_add(u64::from(opcode.byte_length))
        })
        .ok_or(RawFrameValidationError::ArithmeticOverflow)?;
    if metadata_bytes > limits.max_metadata_bytes {
        return Err(RawFrameValidationError::MetadataByteLimit);
    }
    for preview in &parts.previews {
        preview
            .offset
            .checked_add(preview.byte_length)
            .ok_or(RawFrameValidationError::InvalidPreviewRange)?;
        if preview.byte_length == 0 {
            return Err(RawFrameValidationError::InvalidPreviewRange);
        }
    }
    Ok(())
}

fn validate_dimensions(
    dimensions: RawDimensions,
    limits: RawDecodeLimits,
) -> Result<(), RawFrameValidationError> {
    if dimensions.width == 0 || dimensions.height == 0 {
        return Err(RawFrameValidationError::ZeroDimensions);
    }
    if dimensions.width > limits.max_width || dimensions.height > limits.max_height {
        return Err(RawFrameValidationError::DimensionLimit {
            width: dimensions.width,
            height: dimensions.height,
        });
    }
    let pixels = u64::from(dimensions.width)
        .checked_mul(u64::from(dimensions.height))
        .ok_or(RawFrameValidationError::ArithmeticOverflow)?;
    if pixels > limits.max_pixels {
        return Err(RawFrameValidationError::PixelLimit {
            actual: pixels,
            limit: limits.max_pixels,
        });
    }
    Ok(())
}

fn validate_levels(
    black: &RawLevelPattern,
    white: &[f32],
    limits: RawDecodeLimits,
) -> Result<(), RawFrameValidationError> {
    if black.repeat_width == 0 || black.repeat_height == 0 || black.channels == 0 {
        return Err(RawFrameValidationError::InvalidLevels);
    }
    let expected = usize::from(black.repeat_width)
        .checked_mul(usize::from(black.repeat_height))
        .and_then(|cells| cells.checked_mul(usize::from(black.channels)))
        .ok_or(RawFrameValidationError::ArithmeticOverflow)?;
    if expected != black.values.len()
        || expected > usize::from(limits.max_level_values)
        || white.is_empty()
        || white.len() > usize::from(limits.max_level_values)
        || black
            .values
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        || white
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
    {
        return Err(RawFrameValidationError::InvalidLevels);
    }
    let minimum_white = white.iter().copied().fold(f32::INFINITY, f32::min);
    if black.values.iter().any(|value| *value >= minimum_white) {
        return Err(RawFrameValidationError::InvalidLevels);
    }
    Ok(())
}

fn validate_identity(
    identity: &RawCameraIdentity,
    max_string_bytes: u16,
) -> Result<(), RawFrameValidationError> {
    let max = usize::from(max_string_bytes);
    for value in [
        &identity.maker,
        &identity.model,
        &identity.normalized_maker,
        &identity.normalized_model,
        &identity.mode,
    ] {
        if value.len() > max || value.chars().any(char::is_control) {
            return Err(RawFrameValidationError::UnsafeCameraIdentity);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawDecodeLimits {
    pub max_source_bytes: u64,
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u64,
    pub max_samples: u64,
    pub max_decoded_bytes: u64,
    pub max_metadata_bytes: u64,
    pub max_metadata_items: u16,
    pub max_string_bytes: u16,
    pub max_cfa_cells: u16,
    pub max_level_values: u16,
    pub max_planes: u8,
    pub max_previews: u8,
}

impl RawDecodeLimits {
    /// Creates internally consistent limits and derives the remaining bounds.
    ///
    /// # Errors
    ///
    /// Returns [`RawDecodeLimitsError`] when a limit is zero, overflows, or is inconsistent.
    pub fn new(
        max_source_bytes: u64,
        max_width: u32,
        max_height: u32,
        max_pixels: u64,
        max_decoded_bytes: u64,
    ) -> Result<Self, RawDecodeLimitsError> {
        if max_source_bytes == 0
            || max_width == 0
            || max_height == 0
            || max_pixels == 0
            || max_decoded_bytes == 0
        {
            return Err(RawDecodeLimitsError::ZeroLimit);
        }
        let dimension_pixels = u64::from(max_width)
            .checked_mul(u64::from(max_height))
            .ok_or(RawDecodeLimitsError::ArithmeticOverflow)?;
        if max_pixels > dimension_pixels {
            return Err(RawDecodeLimitsError::InconsistentPixelCount);
        }
        let max_samples = max_pixels
            .checked_mul(4)
            .ok_or(RawDecodeLimitsError::ArithmeticOverflow)?;
        if max_decoded_bytes > max_samples.saturating_mul(2) {
            return Err(RawDecodeLimitsError::InconsistentDecodedBytes);
        }
        Ok(Self {
            max_source_bytes,
            max_width,
            max_height,
            max_pixels,
            max_samples,
            max_decoded_bytes,
            max_metadata_bytes: 8 * 1024 * 1024,
            max_metadata_items: 64,
            max_string_bytes: 256,
            max_cfa_cells: 144,
            max_level_values: 256,
            max_planes: 4,
            max_previews: 16,
        })
    }

    #[must_use]
    pub const fn standard() -> Self {
        Self {
            max_source_bytes: 4 * 1024 * 1024 * 1024,
            max_width: 65_535,
            max_height: 65_535,
            max_pixels: 250_000_000,
            max_samples: 1_000_000_000,
            max_decoded_bytes: 2_000_000_000,
            max_metadata_bytes: 8 * 1024 * 1024,
            max_metadata_items: 64,
            max_string_bytes: 256,
            max_cfa_cells: 144,
            max_level_values: 256,
            max_planes: 4,
            max_previews: 16,
        }
    }
}

impl Default for RawDecodeLimits {
    fn default() -> Self {
        Self::standard()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawDecodeLimitsError {
    ZeroLimit,
    InconsistentPixelCount,
    InconsistentDecodedBytes,
    ArithmeticOverflow,
}

#[derive(Debug, Clone, Default)]
pub struct RawCancellationToken(Arc<AtomicBool>);

impl RawCancellationToken {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone)]
pub struct RawDecodeRequest {
    pub image_index: usize,
    pub limits: RawDecodeLimits,
    pub cancellation: RawCancellationToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawHeader {
    pub container: RawContainerKind,
    pub dimensions: RawDimensions,
    pub active_area: RawRect,
    pub crop_area: RawRect,
    pub orientation: RawOrientation,
    pub camera: RawCameraIdentity,
    pub compression: RawCompressionEvidence,
    pub bit_depth: u8,
}

impl RawDecodeRequest {
    #[must_use]
    pub fn new(limits: RawDecodeLimits) -> Self {
        Self {
            image_index: 0,
            limits,
            cancellation: RawCancellationToken::new(),
        }
    }

    #[must_use]
    pub const fn with_image_index(mut self, image_index: usize) -> Self {
        self.image_index = image_index;
        self
    }

    #[must_use]
    pub fn with_cancellation(mut self, cancellation: RawCancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSourceReceipt {
    pub source_bytes: u64,
    pub source_sha256: [u8; 32],
    pub stable_copy_used: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RawCapabilityKey {
    pub maker: String,
    pub model: String,
    pub mode: String,
    pub container: RawContainerKind,
    pub compression: RawCompression,
    pub cfa: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCapabilityDescriptor {
    pub key: RawCapabilityKey,
    pub family: RawVendorFamily,
    pub profile_id: String,
    pub backend_format: String,
    pub decoder_path: String,
    pub layout: RawCapabilityLayout,
    pub quirk_ids: Vec<String>,
    pub reference_evidence: Vec<String>,
    pub reviewed: bool,
    pub normalized_maker: String,
    pub normalized_model: String,
    pub bit_depth: Option<u8>,
    pub corpus_fixtures: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RawVendorFamily {
    Canon,
    Nikon,
    Sony,
    Fujifilm,
    Olympus,
    PanasonicLeica,
    Pentax,
    Samsung,
    Hasselblad,
    PhaseOne,
    Epson,
    Standardized,
    UnreviewedBackend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RawCapabilityLayout {
    BayerOrLinear,
    BayerOrXTrans,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCapabilityManifest {
    pub backend: String,
    pub sha256: [u8; 32],
    entries: Vec<RawCapabilityDescriptor>,
}

impl RawCapabilityManifest {
    pub(crate) fn generated(entries: Vec<RawCapabilityDescriptor>, sha256: [u8; 32]) -> Self {
        Self {
            backend: RAWLER_BACKEND_ID.to_owned(),
            sha256,
            entries,
        }
    }

    #[must_use]
    pub fn entries(&self) -> &[RawCapabilityDescriptor] {
        &self.entries
    }

    /// Resolves one exact or normalized camera capability without guessing across aliases.
    ///
    /// # Errors
    ///
    /// Returns [`RawCapabilityResolveError`] when no unique capability matches.
    pub fn resolve(
        &self,
        maker: &str,
        model: &str,
        container: RawContainerKind,
    ) -> Result<&RawCapabilityDescriptor, RawCapabilityResolveError> {
        let exact: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| {
                entry.key.container == container
                    && entry.key.maker == maker
                    && entry.key.model == model
            })
            .collect();
        if exact.len() == 1 {
            return Ok(exact[0]);
        }
        if exact.len() > 1 {
            return Err(RawCapabilityResolveError::Ambiguous {
                candidates: exact.len(),
            });
        }
        let folded_maker = maker.trim().to_lowercase();
        let folded_model = model.trim().to_lowercase();
        let aliases: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| {
                entry.key.container == container
                    && entry.normalized_maker.to_lowercase() == folded_maker
                    && entry.normalized_model.to_lowercase() == folded_model
            })
            .collect();
        match aliases.as_slice() {
            [entry] => Ok(entry),
            [] => Err(RawCapabilityResolveError::Unsupported),
            values => Err(RawCapabilityResolveError::Ambiguous {
                candidates: values.len(),
            }),
        }
    }

    /// Selects a unique backend camera profile using exact identity first, then reviewed aliases.
    pub fn resolve_backend(
        &self,
        maker: &str,
        model: &str,
        mode: &str,
        container: RawContainerKind,
    ) -> Result<(&RawCapabilityDescriptor, bool), RawCapabilityResolveError> {
        let exact: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| {
                entry.key.container == container
                    && entry.key.maker == maker
                    && entry.key.model == model
                    && entry.key.mode == mode
            })
            .collect();
        if let [entry] = exact.as_slice() {
            return Ok((entry, false));
        }
        if exact.len() > 1 {
            return Err(RawCapabilityResolveError::Ambiguous {
                candidates: exact.len(),
            });
        }
        let folded_maker = maker.trim().to_lowercase();
        let folded_model = model.trim().to_lowercase();
        let aliases: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| {
                entry.key.container == container
                    && entry.normalized_maker.to_lowercase() == folded_maker
                    && entry.normalized_model.to_lowercase() == folded_model
            })
            .collect();
        match aliases.as_slice() {
            [entry] => Ok((entry, true)),
            [] => Err(RawCapabilityResolveError::Unsupported),
            values => Err(RawCapabilityResolveError::Ambiguous {
                candidates: values.len(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawCapabilityResolveError {
    Unsupported,
    Ambiguous { candidates: usize },
}
