// Darktable-compatible crop operation.
//
// Crop is a geometry operation rather than a pointwise color transform.  The
// module therefore owns its normalized parameter DTO, history codec, checked
// ROI plan, coordinate callbacks, and scalar CPU copy.  The processing
// orchestrator binds these pieces to its operation and pixelpipe registries.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::chunks_exact_to_as_chunks,
    clippy::manual_is_multiple_of,
    clippy::cast_possible_truncation,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate
)]

use super::common::OperationExecutionError;
use crate::{FiniteF32, LinearRgb, RasterDimensions};
use sha2::{Digest, Sha256};
use std::fmt;

mod crop_descriptor {
    include!("crop_descriptor.rs");
}

pub use crop_descriptor::crop_descriptor;

pub const CROP_COMPATIBILITY_ID: &str = "crop";
pub const CROP_RUST_ID: &str = "rusttable.crop";
pub const CROP_SCHEMA_VERSION: u16 = 3;
pub const CROP_PARAMETER_BYTES: usize = 32;
pub const CROP_LEGACY_V1_BYTES: usize = 184;
pub const CROP_LEGACY_V2_BYTES: usize = 208;
pub const MIN_CROP_SIZE: f32 = 0.01;
pub const MIN_OUTPUT_EDGE: u32 = 4;

/// Current normalized crop parameters. `cw` and `ch` are right and bottom,
/// matching darktable's persisted DTO rather than width and height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CropConfig {
    cx: FiniteF32,
    cy: FiniteF32,
    cw: FiniteF32,
    ch: FiniteF32,
    ratio_n: i32,
    ratio_d: i32,
}

impl CropConfig {
    /// Builds a finite configuration. Normalized bounds are clamped by the
    /// commit/plan path, as they are in darktable's `commit_params` callback.
    pub fn new(
        cx: f32,
        cy: f32,
        cw: f32,
        ch: f32,
        ratio_n: i32,
        ratio_d: i32,
    ) -> Result<Self, CropConfigError> {
        Ok(Self {
            cx: finite(cx)?,
            cy: finite(cy)?,
            cw: finite(cw)?,
            ch: finite(ch)?,
            ratio_n,
            ratio_d,
        })
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::new(0.0, 0.0, 1.0, 1.0, -1, -1).expect("crop defaults are finite")
    }

    #[must_use]
    pub const fn cx(self) -> FiniteF32 {
        self.cx
    }
    #[must_use]
    pub const fn cy(self) -> FiniteF32 {
        self.cy
    }
    #[must_use]
    pub const fn cw(self) -> FiniteF32 {
        self.cw
    }
    #[must_use]
    pub const fn ch(self) -> FiniteF32 {
        self.ch
    }
    #[must_use]
    pub const fn ratio_n(self) -> i32 {
        self.ratio_n
    }
    #[must_use]
    pub const fn ratio_d(self) -> i32 {
        self.ratio_d
    }

    #[must_use]
    pub const fn ratio_is_freehand(self) -> bool {
        self.ratio_n == 0 && self.ratio_d == 0
    }

    #[must_use]
    pub const fn ratio_is_original_image(self) -> bool {
        self.ratio_n == 0 && (self.ratio_d == 1 || self.ratio_d == -1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CropConfigError {
    NonFinite,
}

impl fmt::Display for CropConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("crop parameters must be finite")
    }
}

impl std::error::Error for CropConfigError {}

/// The modern v3 DTO, including the eight bytes of ABI padding retained in
/// imported history. The old C payloads are intentionally opaque because
/// their layouts are platform-dependent and are not safe portable decoders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CropParametersV3 {
    config: CropConfig,
    padding: [u8; 8],
}

impl CropParametersV3 {
    #[must_use]
    pub const fn new(config: CropConfig) -> Self {
        Self {
            config,
            padding: [0; 8],
        }
    }

    #[must_use]
    pub const fn with_padding(config: CropConfig, padding: [u8; 8]) -> Self {
        Self { config, padding }
    }

    #[must_use]
    pub const fn config(self) -> CropConfig {
        self.config
    }

    #[must_use]
    pub const fn padding(self) -> [u8; 8] {
        self.padding
    }

    /// Encodes the stable little-endian modern payload and preserves padding.
    #[must_use]
    pub fn to_bytes(self) -> [u8; CROP_PARAMETER_BYTES] {
        let mut bytes = [0; CROP_PARAMETER_BYTES];
        write_f32(&mut bytes[0..4], self.config.cx().get());
        write_f32(&mut bytes[4..8], self.config.cy().get());
        write_f32(&mut bytes[8..12], self.config.cw().get());
        write_f32(&mut bytes[12..16], self.config.ch().get());
        bytes[16..20].copy_from_slice(&self.config.ratio_n().to_le_bytes());
        bytes[20..24].copy_from_slice(&self.config.ratio_d().to_le_bytes());
        bytes[24..32].copy_from_slice(&self.padding);
        bytes
    }

    /// Decodes exactly the modern v3 payload.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CropCodecError> {
        if bytes.len() != CROP_PARAMETER_BYTES {
            return Err(CropCodecError::InvalidLength {
                expected: CROP_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let config = CropConfig::new(
            read_f32(&bytes[0..4]),
            read_f32(&bytes[4..8]),
            read_f32(&bytes[8..12]),
            read_f32(&bytes[12..16]),
            i32::from_le_bytes(bytes[16..20].try_into().expect("checked slice")),
            i32::from_le_bytes(bytes[20..24].try_into().expect("checked slice")),
        )
        .map_err(CropCodecError::Config)?;
        Ok(Self::with_padding(
            config,
            bytes[24..32].try_into().expect("checked slice"),
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CropCodecError {
    InvalidLength { expected: usize, actual: usize },
    Config(CropConfigError),
    LegacyPayloadOpaque { version: u16, expected: usize },
}

impl fmt::Display for CropCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "crop payload has {actual} bytes; expected {expected}"
                )
            }
            Self::Config(error) => write!(formatter, "invalid crop payload: {error}"),
            Self::LegacyPayloadOpaque { version, expected } => write!(
                formatter,
                "crop parameter version {version} is opaque; preserve its {expected}-byte payload"
            ),
        }
    }
}

impl std::error::Error for CropCodecError {}

/// Semantic representation of darktable's v1 payload for migration tests and
/// audited adapters. Raw v1 bytes remain opaque at the persistence boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropLegacyParametersV1 {
    pub cx: f32,
    pub cy: f32,
    pub cw: f32,
    pub ch: f32,
    pub ratio_n: i32,
    pub ratio_d: i32,
}

/// Semantic representation of darktable's v2 payload. `aligned` was removed
/// in v3 after being folded into the crop geometry behavior.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropLegacyParametersV2 {
    pub cx: f32,
    pub cy: f32,
    pub cw: f32,
    pub ch: f32,
    pub ratio_n: i32,
    pub ratio_d: i32,
    pub aligned: bool,
}

/// Dimensions/orientation needed to reproduce darktable's v2→v3 recovery for
/// the historical original-image square-crop bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CropMigrationContext {
    image_width: u32,
    image_height: u32,
    swaps_xy: bool,
}

impl CropMigrationContext {
    pub const fn new(image_width: u32, image_height: u32, swaps_xy: bool) -> Self {
        Self {
            image_width,
            image_height,
            swaps_xy,
        }
    }

    #[must_use]
    pub const fn image_width(self) -> u32 {
        self.image_width
    }
    #[must_use]
    pub const fn image_height(self) -> u32 {
        self.image_height
    }
    #[must_use]
    pub const fn swaps_xy(self) -> bool {
        self.swaps_xy
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CropMigrationError {
    Config(CropConfigError),
    InvalidContext,
}

impl fmt::Display for CropMigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => write!(formatter, "crop migration config error: {error}"),
            Self::InvalidContext => formatter.write_str("crop migration context is invalid"),
        }
    }
}

impl std::error::Error for CropMigrationError {}

impl From<CropConfigError> for CropMigrationError {
    fn from(error: CropConfigError) -> Self {
        Self::Config(error)
    }
}

/// Converts the v1 semantic DTO to v2, adding darktable's historical false
/// alignment flag.
pub fn migrate_v1(
    value: CropLegacyParametersV1,
) -> Result<CropLegacyParametersV2, CropMigrationError> {
    ensure_finite([value.cx, value.cy, value.cw, value.ch])?;
    Ok(CropLegacyParametersV2 {
        cx: value.cx,
        cy: value.cy,
        cw: value.cw,
        ch: value.ch,
        ratio_n: value.ratio_n,
        ratio_d: value.ratio_d,
        aligned: false,
    })
}

/// Converts v2 to modern v3 and reproduces darktable's guarded recovery of
/// malformed original-image square crops. The `aligned` bit is intentionally
/// discarded because v3 has no such field.
pub fn migrate_v2(
    value: CropLegacyParametersV2,
    context: CropMigrationContext,
) -> Result<CropParametersV3, CropMigrationError> {
    ensure_finite([value.cx, value.cy, value.cw, value.ch])?;
    if context.image_width == 0 || context.image_height == 0 {
        return Err(CropMigrationError::InvalidContext);
    }
    let mut current = CropConfig::new(
        value.cx,
        value.cy,
        value.cw,
        value.ch,
        value.ratio_n,
        value.ratio_d,
    )?;
    if value.ratio_n == 0 && value.ratio_d.unsigned_abs() == 1 {
        recover_original_square(&mut current, context);
    }
    Ok(CropParametersV3::new(current))
}

/// Returns the expected legacy payload size without attempting a nonportable
/// decode. This lets an importer preserve unknown/legacy history verbatim.
pub const fn legacy_payload_size(version: u16) -> Option<usize> {
    match version {
        1 => Some(CROP_LEGACY_V1_BYTES),
        2 => Some(CROP_LEGACY_V2_BYTES),
        _ => None,
    }
}

pub fn decode_legacy(version: u16, bytes: &[u8]) -> Result<(), CropCodecError> {
    let expected = legacy_payload_size(version).ok_or(CropCodecError::LegacyPayloadOpaque {
        version,
        expected: 0,
    })?;
    if bytes.len() != expected {
        return Err(CropCodecError::InvalidLength {
            expected,
            actual: bytes.len(),
        });
    }
    Err(CropCodecError::LegacyPayloadOpaque { version, expected })
}

/// Crop execution mode controls darktable's export-only ratio alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CropPlanMode {
    Preview,
    Export,
}

/// Checked integer half-open crop rectangle in source coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CropRoi {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl CropRoi {
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Result<Self, CropGeometryError> {
        if width == 0 || height == 0 {
            return Err(CropGeometryError::EmptyRoi);
        }
        if x.checked_add(width).is_none() || y.checked_add(height).is_none() {
            return Err(CropGeometryError::ArithmeticOverflow);
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    #[must_use]
    pub const fn x(self) -> u32 {
        self.x
    }
    #[must_use]
    pub const fn y(self) -> u32 {
        self.y
    }
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }
    #[must_use]
    pub const fn right(self) -> u32 {
        self.x + self.width
    }
    #[must_use]
    pub const fn bottom(self) -> u32 {
        self.y + self.height
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CropGeometryError {
    EmptyRoi,
    ArithmeticOverflow,
    OutsideSource,
    InvalidDimensions,
    InvalidPointBuffer,
    NonFinitePoint,
    NonFiniteAspect,
}

impl fmt::Display for CropGeometryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyRoi => "crop ROI is empty",
            Self::ArithmeticOverflow => "crop ROI arithmetic overflowed",
            Self::OutsideSource => "crop ROI is outside the source image",
            Self::InvalidDimensions => "crop source dimensions are invalid",
            Self::InvalidPointBuffer => "crop point buffer must contain x/y pairs",
            Self::NonFinitePoint => "crop transform received a non-finite point",
            Self::NonFiniteAspect => "crop aspect ratio is non-finite",
        })
    }
}

impl std::error::Error for CropGeometryError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CropAspect {
    Freehand,
    OriginalImage {
        flipped: bool,
    },
    Fixed {
        numerator: u32,
        denominator: u32,
        flipped: bool,
    },
}

/// Immutable crop ROI plan and its deterministic identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CropPlan {
    config: CropConfig,
    source_dimensions: RasterDimensions,
    output_dimensions: RasterDimensions,
    input_roi: CropRoi,
    mode: CropPlanMode,
    identity: [u8; 32],
}

impl CropPlan {
    pub fn new(
        config: CropConfig,
        source_dimensions: RasterDimensions,
    ) -> Result<Self, CropPlanError> {
        Self::new_with_mode(config, source_dimensions, CropPlanMode::Preview)
    }

    pub fn new_with_mode(
        config: CropConfig,
        source_dimensions: RasterDimensions,
        mode: CropPlanMode,
    ) -> Result<Self, CropPlanError> {
        let input_roi =
            normalized_roi(config, source_dimensions).map_err(CropPlanError::Geometry)?;
        let input_roi = if mode == CropPlanMode::Export {
            align_export_roi(input_roi, config)?
        } else {
            input_roi
        };
        let output_dimensions = RasterDimensions::new(input_roi.width(), input_roi.height())
            .map_err(|_| CropPlanError::Geometry(CropGeometryError::InvalidDimensions))?;
        let identity = plan_identity(
            config,
            source_dimensions,
            output_dimensions,
            input_roi,
            mode,
        );
        Ok(Self {
            config,
            source_dimensions,
            output_dimensions,
            input_roi,
            mode,
            identity,
        })
    }

    #[must_use]
    pub const fn config(&self) -> CropConfig {
        self.config
    }
    #[must_use]
    pub const fn source_dimensions(&self) -> RasterDimensions {
        self.source_dimensions
    }
    #[must_use]
    pub const fn output_dimensions(&self) -> RasterDimensions {
        self.output_dimensions
    }
    #[must_use]
    pub const fn input_roi(&self) -> CropRoi {
        self.input_roi
    }
    #[must_use]
    pub const fn mode(&self) -> CropPlanMode {
        self.mode
    }
    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }

    #[must_use]
    pub const fn output_roi(&self) -> CropRoi {
        CropRoi {
            x: 0,
            y: 0,
            width: self.input_roi.width,
            height: self.input_roi.height,
        }
    }

    pub fn aspect(&self) -> Result<CropAspect, CropGeometryError> {
        crop_aspect(self.config, self.source_dimensions)
    }

    /// Translates source-space points into cropped output space using the
    /// exact integer offset used by ROI planning.
    pub fn forward_transform(&self, points: &mut [f32]) -> Result<(), CropGeometryError> {
        transform_points(points, self.input_roi.x, self.input_roi.y, false)
    }

    /// Translates cropped output points back into source space.
    pub fn back_transform(&self, points: &mut [f32]) -> Result<(), CropGeometryError> {
        transform_points(points, self.input_roi.x, self.input_roi.y, true)
    }

    pub fn execute(&self, input: &[LinearRgb]) -> Result<CropExecution, OperationExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<CropExecution, OperationExecutionError> {
        let expected = usize::try_from(self.source_dimensions.pixel_count()).map_err(|_| {
            OperationExecutionError::DimensionsMismatch {
                expected: usize::MAX,
                actual: input.len(),
            }
        })?;
        if input.len() != expected {
            return Err(OperationExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        let source_width = usize::try_from(self.source_dimensions.width()).map_err(|_| {
            OperationExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            }
        })?;
        let width = usize::try_from(self.input_roi.width()).map_err(|_| {
            OperationExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            }
        })?;
        let height = usize::try_from(self.input_roi.height()).map_err(|_| {
            OperationExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            }
        })?;
        let x = usize::try_from(self.input_roi.x()).expect("u32 fits usize on supported targets");
        let y = usize::try_from(self.input_roi.y()).expect("u32 fits usize on supported targets");
        let mut pixels = Vec::with_capacity(width.checked_mul(height).unwrap_or(0));
        for row in 0..height {
            if cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            let start = (y + row) * source_width + x;
            pixels.extend_from_slice(&input[start..start + width]);
        }
        Ok(CropExecution {
            pixels,
            dimensions: self.output_dimensions,
            roi: self.input_roi,
            receipt: CropReceipt {
                identity: self.identity,
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CropExecution {
    pixels: Vec<LinearRgb>,
    dimensions: RasterDimensions,
    roi: CropRoi,
    receipt: CropReceipt,
}

impl CropExecution {
    #[must_use]
    pub fn pixels(&self) -> &[LinearRgb] {
        &self.pixels
    }
    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn input_roi(&self) -> CropRoi {
        self.roi
    }
    #[must_use]
    pub const fn receipt(&self) -> CropReceipt {
        self.receipt
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CropReceipt {
    identity: [u8; 32],
}

impl CropReceipt {
    #[must_use]
    pub const fn identity(self) -> [u8; 32] {
        self.identity
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CropPlanError {
    Geometry(CropGeometryError),
    Config(CropConfigError),
}

impl fmt::Display for CropPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Geometry(error) => write!(formatter, "crop geometry error: {error}"),
            Self::Config(error) => write!(formatter, "crop config error: {error}"),
        }
    }
}

impl std::error::Error for CropPlanError {}

impl From<CropConfigError> for CropPlanError {
    fn from(error: CropConfigError) -> Self {
        Self::Config(error)
    }
}

fn normalized_roi(
    config: CropConfig,
    dimensions: RasterDimensions,
) -> Result<CropRoi, CropGeometryError> {
    let width = dimensions.width();
    let height = dimensions.height();
    if width == 0 || height == 0 {
        return Err(CropGeometryError::InvalidDimensions);
    }
    let cx = config.cx().get().clamp(0.0, 1.0 - MIN_CROP_SIZE);
    let cy = config.cy().get().clamp(0.0, 1.0 - MIN_CROP_SIZE);
    let cw = config.cw().get().clamp(MIN_CROP_SIZE, 1.0);
    let ch = config.ch().get().clamp(MIN_CROP_SIZE, 1.0);
    if cw <= cx || ch <= cy {
        return Err(CropGeometryError::EmptyRoi);
    }
    let x = (cx * width as f32).floor() as u32;
    let y = (cy * height as f32).floor() as u32;
    let crop_width = MIN_OUTPUT_EDGE.max(((cw - cx) * width as f32).floor() as u32);
    let crop_height = MIN_OUTPUT_EDGE.max(((ch - cy) * height as f32).floor() as u32);
    let roi = CropRoi::new(x, y, crop_width, crop_height)?;
    if roi.right() > width || roi.bottom() > height {
        return Err(CropGeometryError::OutsideSource);
    }
    Ok(roi)
}

fn align_export_roi(mut roi: CropRoi, config: CropConfig) -> Result<CropRoi, CropPlanError> {
    if config.ratio_n() == 0 || config.ratio_d() == 0 {
        return Ok(roi);
    }
    let mut align_w = config.ratio_d().unsigned_abs().max(1);
    let mut align_h = config.ratio_n().unsigned_abs().max(1);
    for divisor in (2..=7).rev() {
        while align_w % divisor == 0 && align_h % divisor == 0 {
            align_w /= divisor;
            align_h /= divisor;
        }
    }
    if align_w > 16 || align_h > 16 || (align_w == 1 && align_h == 1) {
        return Ok(roi);
    }
    let (align_width, align_height) = if roi.width() >= roi.height() {
        (align_w, align_h)
    } else {
        (align_h, align_w)
    };
    let drop_width = roi.width() % align_width;
    let drop_height = roi.height() % align_height;
    let width = roi.width().saturating_sub(drop_width).max(MIN_OUTPUT_EDGE);
    let height = roi
        .height()
        .saturating_sub(drop_height)
        .max(MIN_OUTPUT_EDGE);
    let x = roi
        .x()
        .checked_add(drop_width / 2)
        .ok_or(CropPlanError::Geometry(
            CropGeometryError::ArithmeticOverflow,
        ))?;
    let y = roi
        .y()
        .checked_add(drop_height / 2)
        .ok_or(CropPlanError::Geometry(
            CropGeometryError::ArithmeticOverflow,
        ))?;
    roi = CropRoi::new(x, y, width, height).map_err(CropPlanError::Geometry)?;
    Ok(roi)
}

fn crop_aspect(
    config: CropConfig,
    dimensions: RasterDimensions,
) -> Result<CropAspect, CropGeometryError> {
    if config.ratio_is_freehand() {
        return Ok(CropAspect::Freehand);
    }
    if config.ratio_is_original_image() {
        return Ok(CropAspect::OriginalImage {
            flipped: config.ratio_d() < 0,
        });
    }
    if config.ratio_n() == 0 || config.ratio_d() == 0 {
        return Ok(CropAspect::Freehand);
    }
    let numerator = config.ratio_d().unsigned_abs();
    let denominator = config.ratio_n().unsigned_abs();
    if numerator == 0
        || denominator == 0
        || !((numerator as f32) / (denominator as f32)).is_finite()
    {
        return Err(CropGeometryError::NonFiniteAspect);
    }
    let _ = dimensions;
    Ok(CropAspect::Fixed {
        numerator,
        denominator,
        flipped: config.ratio_d() < 0,
    })
}

fn transform_points(
    points: &mut [f32],
    x: u32,
    y: u32,
    inverse: bool,
) -> Result<(), CropGeometryError> {
    if points.len() % 2 != 0 {
        return Err(CropGeometryError::InvalidPointBuffer);
    }
    let x = x as f32;
    let y = y as f32;
    for pair in points.chunks_exact_mut(2) {
        if pair.iter().any(|value| !value.is_finite()) {
            return Err(CropGeometryError::NonFinitePoint);
        }
        if inverse {
            pair[0] += x;
            pair[1] += y;
        } else {
            pair[0] -= x;
            pair[1] -= y;
        }
    }
    Ok(())
}

fn recover_original_square(config: &mut CropConfig, context: CropMigrationContext) {
    let width = context.image_width.max(1) as f32;
    let height = context.image_height.max(1) as f32;
    if context.image_width <= 4 || context.image_height <= 4 {
        return;
    }
    let landscape = if context.swaps_xy {
        height > width
    } else {
        width >= height
    };
    let (wd, ht) = if landscape {
        (width, height)
    } else {
        (height, width)
    };
    let ratio = if wd >= ht { wd / ht } else { ht / wd };
    let px = config.cx().get() * wd;
    let py = config.cy().get() * ht;
    let dx = (config.cw().get() - config.cx().get()) * wd;
    let dy = (config.ch().get() - config.cy().get()) * ht;
    let correct =
        approximately_equal(ratio, dx / dy, 0.01) || approximately_equal(ratio, dy / dx, 0.01);
    if correct {
        return;
    }
    let flipped = config.ratio_d() < 0;
    let new_width = if landscape {
        if flipped { dy / ratio } else { dx }
    } else if flipped {
        dy * ratio
    } else {
        dy / ratio
    };
    let new_height = if landscape && !flipped {
        dx / ratio
    } else {
        dy
    };
    if new_width.is_finite() && new_height.is_finite() && new_width > 0.0 && new_height > 0.0 {
        let cw = if landscape && !flipped {
            config.cw().get()
        } else {
            (new_width + px) / wd
        };
        let ch = if landscape && !flipped {
            (new_height + py) / ht
        } else {
            config.ch().get()
        };
        *config = CropConfig::new(
            config.cx().get(),
            config.cy().get(),
            cw,
            ch,
            config.ratio_n(),
            config.ratio_d(),
        )
        .expect("finite recovered crop");
    }
}

fn approximately_equal(left: f32, right: f32, tolerance: f32) -> bool {
    left.is_finite()
        && right.is_finite()
        && (left - right).abs() <= tolerance * left.abs().max(right.abs()).max(1.0)
}

fn finite(value: f32) -> Result<FiniteF32, CropConfigError> {
    FiniteF32::new(value).map_err(|_| CropConfigError::NonFinite)
}

fn ensure_finite(values: [f32; 4]) -> Result<(), CropMigrationError> {
    if values.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(CropMigrationError::Config(CropConfigError::NonFinite))
    }
}

fn write_f32(target: &mut [u8], value: f32) {
    target.copy_from_slice(&value.to_bits().to_le_bytes());
}
fn read_f32(source: &[u8]) -> f32 {
    f32::from_bits(u32::from_le_bytes(
        source.try_into().expect("checked slice"),
    ))
}

fn plan_identity(
    config: CropConfig,
    source: RasterDimensions,
    output: RasterDimensions,
    roi: CropRoi,
    mode: CropPlanMode,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(CROP_COMPATIBILITY_ID.as_bytes());
    hasher.update(CROP_SCHEMA_VERSION.to_le_bytes());
    for value in [config.cx(), config.cy(), config.cw(), config.ch()] {
        hasher.update(value.get().to_bits().to_le_bytes());
    }
    hasher.update(config.ratio_n().to_le_bytes());
    hasher.update(config.ratio_d().to_le_bytes());
    hasher.update(source.width().to_le_bytes());
    hasher.update(source.height().to_le_bytes());
    hasher.update(output.width().to_le_bytes());
    hasher.update(output.height().to_le_bytes());
    hasher.update(roi.x().to_le_bytes());
    hasher.update(roi.y().to_le_bytes());
    hasher.update(roi.width().to_le_bytes());
    hasher.update(roi.height().to_le_bytes());
    hasher.update([match mode {
        CropPlanMode::Preview => 0,
        CropPlanMode::Export => 1,
    }]);
    hasher.finalize().into()
}
