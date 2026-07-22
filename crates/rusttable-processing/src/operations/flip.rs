//! Integer orientation, quarter-turn, and mirroring operation.
//!
//! The bit layout is the one used by Darktable's `dt_image_orientation_t`:
//! bit 0 flips Y, bit 1 flips X, and bit 2 swaps X/Y.  The plan resolves the
//! source orientation once; execution only applies the resulting bijection.

#![allow(dead_code, clippy::missing_errors_doc, clippy::must_use_candidate)]

use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};
use crate::{LinearRgb, RasterDimensions};
use rusttable_color::ColorEncoding;
use rusttable_image::{CfaDescriptor, ImageDimensions, Orientation, Roi};
use sha2::{Digest, Sha256};
use std::fmt;

pub const FLIP_COMPATIBILITY_ID: &str = "flip";
pub const FLIP_RUST_ID: &str = "rusttable.flip";
pub const FLIP_SCHEMA_VERSION: u16 = 2;
pub const FLIP_IMPLEMENTATION_VERSION: u16 = 1;

#[must_use]
///
/// # Panics
///
/// Panics only if the compile-time operation identifiers violate the descriptor key contract.
pub fn flip_descriptor() -> OperationDescriptor {
    let integer = |id: &str, default: i64| ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Integer {
            minimum: -1,
            maximum: 7,
        },
        default: ParameterDefault::Integer(default),
        required: true,
        introduced_version: 1,
        removed_version: None,
        unit: None,
        step: Some(1.0),
        precision: 0,
        role: ParameterRole::Geometry,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
        condition: None,
    };
    let image = ImagePredicate {
        channels: 3,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    };
    OperationDescriptor {
        id: DescriptorId::new(
            FLIP_COMPATIBILITY_ID,
            FLIP_RUST_ID,
            1,
            FLIP_SCHEMA_VERSION,
            FLIP_IMPLEMENTATION_VERSION,
        )
        .expect("static flip ID"),
        parameters: vec![integer("mode", 0), integer("orientation", 0)],
        flags: OperationFlags::DETERMINISTIC_CPU
            .insert(OperationFlags::GEOMETRY)
            .insert(OperationFlags::HISTORY_VISIBLE),
        stage: "geometry".to_owned(),
        roi: RoiKind::Distortion,
        tiling: TilingContract {
            overlap_pixels: 0,
            alignment_pixels: 1,
            minimum_tile_edge: 1,
            preferred_tile_edge: 256,
            temporary_multiplier_milli: 1000,
            input_multiplier_milli: 1000,
            output_multiplier_milli: 1000,
        },
        capability: CapabilityContract {
            cpu_supported: true,
            gpu_tier: None,
            required_features: Vec::new(),
            required_formats: Vec::new(),
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: true,
            precision: "integer".to_owned(),
            modes: vec!["preview".to_owned(), "export".to_owned()],
        },
        io: InputOutputContract {
            input: image.clone(),
            output: image,
            derives_output_encoding: false,
        },
        mask_blend: MaskBlendContract {
            consumes_mask: false,
            publishes_mask: false,
            blend_if: false,
            geometry: true,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![1, 2],
            target_version: FLIP_SCHEMA_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.flip".to_owned(),
            group_key: "group.basic".to_owned(),
            control: "orientation".to_owned(),
        }),
    }
}

/// Darktable's automatic-orientation sentinel.
pub const ORIENTATION_NULL: i32 = -1;
pub const ORIENTATION_NONE: u8 = 0;
pub const ORIENTATION_FLIP_Y: u8 = 1;
pub const ORIENTATION_FLIP_X: u8 = 1 << 1;
pub const ORIENTATION_SWAP_XY: u8 = 1 << 2;

/// The eight valid values of Darktable's orientation bit field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OrientationBits(u8);

impl OrientationBits {
    pub const fn new(bits: u8) -> Result<Self, FlipParameterError> {
        if bits <= 0b111 {
            Ok(Self(bits))
        } else {
            Err(FlipParameterError::UnknownOrientationBits(bits))
        }
    }

    pub const fn none() -> Self {
        Self(ORIENTATION_NONE)
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn flip_x(self) -> bool {
        self.0 & ORIENTATION_FLIP_X != 0
    }

    pub const fn flip_y(self) -> bool {
        self.0 & ORIENTATION_FLIP_Y != 0
    }

    pub const fn swap_xy(self) -> bool {
        self.0 & ORIENTATION_SWAP_XY != 0
    }

    pub const fn orientation(self) -> Orientation {
        if self.0 == 0 {
            return Orientation::Normal;
        }
        match self.0 {
            1 => Orientation::FlipVertical,
            2 => Orientation::FlipHorizontal,
            3 => Orientation::Rotate180,
            4 => Orientation::Transpose,
            5 => Orientation::Rotate270,
            6 => Orientation::Rotate90,
            7 => Orientation::Transverse,
            _ => Orientation::Normal,
        }
    }

    pub const fn from_orientation(orientation: Orientation) -> Self {
        Self(match orientation {
            Orientation::Normal => 0,
            Orientation::FlipVertical => 1,
            Orientation::FlipHorizontal => 2,
            Orientation::Rotate180 => 3,
            Orientation::Transpose => 4,
            Orientation::Rotate270 => 5,
            Orientation::Rotate90 => 6,
            Orientation::Transverse => 7,
        })
    }
}

impl TryFrom<i32> for OrientationBits {
    type Error = FlipParameterError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        let value =
            u8::try_from(value).map_err(|_| FlipParameterError::UnknownOrientationValue(value))?;
        Self::new(value)
    }
}

/// Whether the operation snapshots EXIF orientation or uses an explicit edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlipMode {
    Automatic,
    Explicit,
}

pub type OrientationMode = FlipMode;

/// Canonical persisted flip parameters.  `opaque_source` retains bytes from
/// an importer so a later serializer can preserve provenance exactly.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FlipConfig {
    mode: FlipMode,
    orientation: OrientationBits,
    opaque_source: Option<Vec<u8>>,
}

impl FlipConfig {
    pub const fn automatic() -> Self {
        Self {
            mode: FlipMode::Automatic,
            orientation: OrientationBits::none(),
            opaque_source: None,
        }
    }

    pub const fn explicit(orientation: OrientationBits) -> Self {
        Self {
            mode: FlipMode::Explicit,
            orientation,
            opaque_source: None,
        }
    }

    pub fn new(mode: FlipMode, orientation: OrientationBits) -> Result<Self, FlipParameterError> {
        if matches!(mode, FlipMode::Automatic) && orientation != OrientationBits::none() {
            return Err(FlipParameterError::AutomaticHasExplicitOrientation);
        }
        Ok(Self {
            mode,
            orientation,
            opaque_source: None,
        })
    }

    #[must_use]
    pub fn with_opaque_source(mut self, source: Vec<u8>) -> Self {
        self.opaque_source = Some(source);
        self
    }

    pub const fn mode(&self) -> FlipMode {
        self.mode
    }

    pub const fn orientation(&self) -> OrientationBits {
        self.orientation
    }

    pub fn opaque_source(&self) -> Option<&[u8]> {
        self.opaque_source.as_deref()
    }
}

impl Default for FlipConfig {
    fn default() -> Self {
        Self::automatic()
    }
}

/// Darktable v1 stored the bit field as a signed 32-bit value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlipParametersV1 {
    pub orientation: i32,
}

/// Darktable v2 retained the signed representation, but made automatic mode
/// an explicit persisted value instead of an importer-only convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlipParametersV2 {
    pub orientation: i32,
}

impl Default for FlipParametersV2 {
    fn default() -> Self {
        Self {
            orientation: ORIENTATION_NULL,
        }
    }
}

pub fn migrate_v1(value: FlipParametersV1) -> FlipParametersV2 {
    FlipParametersV2 {
        orientation: value.orientation,
    }
}

/// Migrates a legacy user transform while preserving Darktable's source-EXIF
/// composition rule. This is used only for imported v1 history; new edits
/// snapshot automatic orientation in [`FlipPlan`] instead.
pub fn migrate_v1_with_source(
    value: FlipParametersV1,
    source_orientation: Orientation,
) -> Result<FlipParametersV2, FlipParameterError> {
    if value.orientation == ORIENTATION_NULL {
        return Ok(migrate_v1(value));
    }
    let user = OrientationBits::try_from(value.orientation)?;
    let source = OrientationBits::from_orientation(source_orientation);
    Ok(FlipParametersV2 {
        orientation: i32::from(merge_two_orientations(source, user).bits()),
    })
}

pub fn migrate_v2(value: FlipParametersV2) -> Result<FlipConfig, FlipParameterError> {
    if value.orientation == ORIENTATION_NULL {
        return Ok(FlipConfig::automatic());
    }
    OrientationBits::try_from(value.orientation).map(FlipConfig::explicit)
}

pub fn migrate(version: u16, value: FlipParametersV1) -> Result<FlipConfig, FlipParameterError> {
    match version {
        1 => migrate_v2(migrate_v1(value)),
        2 => migrate_v2(FlipParametersV2 {
            orientation: value.orientation,
        }),
        _ => Err(FlipParameterError::UnsupportedVersion(version)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlipParameterError {
    UnsupportedVersion(u16),
    UnknownOrientationValue(i32),
    UnknownOrientationBits(u8),
    AutomaticHasExplicitOrientation,
}

impl fmt::Display for FlipParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported flip version {version}")
            }
            Self::UnknownOrientationValue(value) => {
                write!(formatter, "unknown flip orientation value {value}")
            }
            Self::UnknownOrientationBits(value) => {
                write!(formatter, "unknown flip orientation bits {value:#x}")
            }
            Self::AutomaticHasExplicitOrientation => {
                formatter.write_str("automatic flip mode cannot carry explicit orientation bits")
            }
        }
    }
}

impl std::error::Error for FlipParameterError {}

/// Composes stored source orientation with a subsequent user transform using
/// Darktable `flip.c::merge_two_orientations` axis-bit semantics.
#[must_use]
pub const fn merge_two_orientations(
    source: OrientationBits,
    user: OrientationBits,
) -> OrientationBits {
    OrientationBits(merge_orientation_bits(source.bits(), user.bits()))
}

const fn merge_orientation_bits(raw: u8, user: u8) -> u8 {
    let mut corrected = raw;
    if user & ORIENTATION_SWAP_XY != 0 {
        if raw & ORIENTATION_FLIP_Y != 0 {
            corrected |= ORIENTATION_FLIP_X;
        } else {
            corrected &= !ORIENTATION_FLIP_X;
        }
        if raw & ORIENTATION_FLIP_X != 0 {
            corrected |= ORIENTATION_FLIP_Y;
        } else {
            corrected &= !ORIENTATION_FLIP_Y;
        }
        if raw & ORIENTATION_SWAP_XY != 0 {
            corrected |= ORIENTATION_SWAP_XY;
        }
    }
    corrected ^ user
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlipPlanError {
    Parameter(FlipParameterError),
    InvalidDimensions,
    ArithmeticOverflow,
}

impl fmt::Display for FlipPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parameter(error) => write!(formatter, "flip parameter error: {error}"),
            Self::InvalidDimensions => formatter.write_str("flip dimensions are invalid"),
            Self::ArithmeticOverflow => {
                formatter.write_str("flip coordinate arithmetic overflowed")
            }
        }
    }
}

impl std::error::Error for FlipPlanError {}

impl From<FlipParameterError> for FlipPlanError {
    fn from(error: FlipParameterError) -> Self {
        Self::Parameter(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlipExecutionError {
    Cancelled,
    InvalidShape { expected: usize, actual: usize },
    InvalidStride { minimum: usize, actual: usize },
    ArithmeticOverflow,
}

impl fmt::Display for FlipExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("flip execution was cancelled"),
            Self::InvalidShape { expected, actual } => {
                write!(formatter, "flip expected {expected} pixels, got {actual}")
            }
            Self::InvalidStride { minimum, actual } => {
                write!(formatter, "flip stride {actual} is smaller than {minimum}")
            }
            Self::ArithmeticOverflow => formatter.write_str("flip pixel arithmetic overflowed"),
        }
    }
}

impl std::error::Error for FlipExecutionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlipCoordinateError {
    OutOfBounds,
    ArithmeticOverflow,
}

impl fmt::Display for FlipCoordinateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::OutOfBounds => "flip coordinate is out of bounds",
            Self::ArithmeticOverflow => "flip coordinate arithmetic overflowed",
        })
    }
}

impl std::error::Error for FlipCoordinateError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlipPlan {
    source_dimensions: RasterDimensions,
    output_dimensions: RasterDimensions,
    source_orientation: Orientation,
    resolved_orientation: Orientation,
    output_metadata_orientation: Orientation,
    config: FlipConfig,
    identity: [u8; 32],
}

impl FlipPlan {
    pub fn new(
        source_dimensions: RasterDimensions,
        config: FlipConfig,
        source_orientation: Orientation,
    ) -> Result<Self, FlipPlanError> {
        if source_dimensions.width() == 0 || source_dimensions.height() == 0 {
            return Err(FlipPlanError::InvalidDimensions);
        }
        let resolved_orientation = match config.mode() {
            FlipMode::Automatic => source_orientation,
            FlipMode::Explicit => config.orientation().orientation(),
        };
        let output_dimensions = if config.orientation().swap_xy()
            || resolved_orientation
                .output_dimensions(to_image_dimensions(source_dimensions))
                .width()
                != source_dimensions.width()
        {
            RasterDimensions::new(source_dimensions.height(), source_dimensions.width())
                .map_err(|_| FlipPlanError::InvalidDimensions)?
        } else {
            source_dimensions
        };
        let identity = plan_identity(
            source_dimensions,
            &config,
            source_orientation,
            resolved_orientation,
        )?;
        Ok(Self {
            source_dimensions,
            output_dimensions,
            source_orientation,
            resolved_orientation,
            output_metadata_orientation: Orientation::Normal,
            config,
            identity,
        })
    }

    pub fn from_parameters(
        source_dimensions: RasterDimensions,
        value: FlipParametersV2,
        source_orientation: Orientation,
    ) -> Result<Self, FlipPlanError> {
        Self::new(source_dimensions, migrate_v2(value)?, source_orientation)
    }

    pub const fn config(&self) -> &FlipConfig {
        &self.config
    }

    pub const fn source_dimensions(&self) -> RasterDimensions {
        self.source_dimensions
    }

    pub const fn output_dimensions(&self) -> RasterDimensions {
        self.output_dimensions
    }

    pub const fn source_orientation(&self) -> Orientation {
        self.source_orientation
    }

    pub const fn resolved_orientation(&self) -> Orientation {
        self.resolved_orientation
    }

    pub const fn output_metadata_orientation(&self) -> Orientation {
        self.output_metadata_orientation
    }

    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }

    pub const fn is_identity(&self) -> bool {
        matches!(self.resolved_orientation, Orientation::Normal)
    }

    pub fn forward(&self, x: u32, y: u32) -> Result<(u32, u32), FlipCoordinateError> {
        if x >= self.source_dimensions.width() || y >= self.source_dimensions.height() {
            return Err(FlipCoordinateError::OutOfBounds);
        }
        Ok(self.resolved_orientation.map_source_to_output(
            to_image_dimensions(self.source_dimensions),
            x,
            y,
        ))
    }

    pub fn inverse(&self, x: u32, y: u32) -> Result<(u32, u32), FlipCoordinateError> {
        if x >= self.output_dimensions.width() || y >= self.output_dimensions.height() {
            return Err(FlipCoordinateError::OutOfBounds);
        }
        Ok(self.resolved_orientation.inverse().map_source_to_output(
            to_image_dimensions(self.output_dimensions),
            x,
            y,
        ))
    }

    pub fn output_roi(&self, input: Roi) -> Result<Roi, FlipCoordinateError> {
        map_roi(
            input,
            to_image_dimensions(self.source_dimensions),
            to_image_dimensions(self.output_dimensions),
            |x, y| self.forward(x, y),
        )
    }

    pub fn input_roi(&self, output: Roi) -> Result<Roi, FlipCoordinateError> {
        map_roi(
            output,
            to_image_dimensions(self.output_dimensions),
            to_image_dimensions(self.source_dimensions),
            |x, y| self.inverse(x, y),
        )
    }

    pub fn output_cfa(&self, source: ImageDimensions, cfa: CfaDescriptor) -> CfaDescriptor {
        cfa.after_orientation(source, self.resolved_orientation)
    }

    pub fn execute(&self, input: &[LinearRgb]) -> Result<FlipExecution, FlipExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<FlipExecution, FlipExecutionError> {
        let expected = pixel_count(self.source_dimensions)?;
        if input.len() != expected {
            return Err(FlipExecutionError::InvalidShape {
                expected,
                actual: input.len(),
            });
        }
        if cancelled() {
            return Err(FlipExecutionError::Cancelled);
        }
        let output_count = pixel_count(self.output_dimensions)?;
        let mut output = vec![input[0]; output_count];
        for (source_index, pixel) in input.iter().copied().enumerate() {
            if cancelled() {
                return Err(FlipExecutionError::Cancelled);
            }
            let x = u32::try_from(
                source_index
                    % usize::try_from(self.source_dimensions.width())
                        .map_err(|_| FlipExecutionError::ArithmeticOverflow)?,
            )
            .map_err(|_| FlipExecutionError::ArithmeticOverflow)?;
            let y = u32::try_from(
                source_index
                    / usize::try_from(self.source_dimensions.width())
                        .map_err(|_| FlipExecutionError::ArithmeticOverflow)?,
            )
            .map_err(|_| FlipExecutionError::ArithmeticOverflow)?;
            let (output_x, output_y) = self
                .forward(x, y)
                .map_err(|_| FlipExecutionError::ArithmeticOverflow)?;
            let output_index = checked_index(output_x, output_y, self.output_dimensions)?;
            output[output_index] = pixel;
        }
        let receipt = FlipReceipt::new(self, input, &output);
        Ok(FlipExecution {
            pixels: output,
            dimensions: self.output_dimensions,
            receipt,
        })
    }

    /// Routes a single-plane mask or raster buffer, accepting padded input
    /// rows and returning a tightly packed output plane.
    pub fn execute_plane<T: Copy>(
        &self,
        input: &[T],
        input_stride: usize,
    ) -> Result<Vec<T>, FlipExecutionError> {
        let source_width = usize::try_from(self.source_dimensions.width())
            .map_err(|_| FlipExecutionError::ArithmeticOverflow)?;
        let source_height = usize::try_from(self.source_dimensions.height())
            .map_err(|_| FlipExecutionError::ArithmeticOverflow)?;
        if input_stride < source_width {
            return Err(FlipExecutionError::InvalidStride {
                minimum: source_width,
                actual: input_stride,
            });
        }
        let required = input_stride
            .checked_mul(source_height)
            .ok_or(FlipExecutionError::ArithmeticOverflow)?;
        if input.len() < required {
            return Err(FlipExecutionError::InvalidShape {
                expected: required,
                actual: input.len(),
            });
        }
        let output_count = pixel_count(self.output_dimensions)?;
        let mut output = vec![input[0]; output_count];
        for y in 0..source_height {
            for x in 0..source_width {
                let source_index = y
                    .checked_mul(input_stride)
                    .and_then(|row| row.checked_add(x))
                    .ok_or(FlipExecutionError::ArithmeticOverflow)?;
                let (output_x, output_y) = self
                    .forward(
                        u32::try_from(x).map_err(|_| FlipExecutionError::ArithmeticOverflow)?,
                        u32::try_from(y).map_err(|_| FlipExecutionError::ArithmeticOverflow)?,
                    )
                    .map_err(|_| FlipExecutionError::ArithmeticOverflow)?;
                let output_index = checked_index(output_x, output_y, self.output_dimensions)?;
                output[output_index] = input[source_index];
            }
        }
        Ok(output)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlipExecution {
    pixels: Vec<LinearRgb>,
    dimensions: RasterDimensions,
    receipt: FlipReceipt,
}

impl FlipExecution {
    pub fn pixels(&self) -> &[LinearRgb] {
        &self.pixels
    }

    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }

    pub const fn receipt(&self) -> &FlipReceipt {
        &self.receipt
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlipReceipt {
    plan_identity: [u8; 32],
    source_orientation: Orientation,
    resolved_orientation: Orientation,
    output_orientation: Orientation,
    input_digest: [u8; 32],
    output_digest: [u8; 32],
}

impl FlipReceipt {
    fn new(plan: &FlipPlan, input: &[LinearRgb], output: &[LinearRgb]) -> Self {
        Self {
            plan_identity: plan.identity,
            source_orientation: plan.source_orientation,
            resolved_orientation: plan.resolved_orientation,
            output_orientation: plan.output_metadata_orientation,
            input_digest: digest_pixels(input),
            output_digest: digest_pixels(output),
        }
    }

    pub const fn plan_identity(&self) -> [u8; 32] {
        self.plan_identity
    }

    pub const fn source_orientation(&self) -> Orientation {
        self.source_orientation
    }

    pub const fn resolved_orientation(&self) -> Orientation {
        self.resolved_orientation
    }

    pub const fn output_orientation(&self) -> Orientation {
        self.output_orientation
    }

    pub const fn input_digest(&self) -> [u8; 32] {
        self.input_digest
    }

    pub const fn output_digest(&self) -> [u8; 32] {
        self.output_digest
    }
}

fn to_image_dimensions(dimensions: RasterDimensions) -> ImageDimensions {
    ImageDimensions::new(dimensions.width(), dimensions.height())
        .expect("validated raster dimensions are nonzero")
}

fn pixel_count(dimensions: RasterDimensions) -> Result<usize, FlipExecutionError> {
    usize::try_from(dimensions.pixel_count()).map_err(|_| FlipExecutionError::ArithmeticOverflow)
}

fn checked_index(
    x: u32,
    y: u32,
    dimensions: RasterDimensions,
) -> Result<usize, FlipExecutionError> {
    let width =
        usize::try_from(dimensions.width()).map_err(|_| FlipExecutionError::ArithmeticOverflow)?;
    usize::try_from(y)
        .map_err(|_| FlipExecutionError::ArithmeticOverflow)?
        .checked_mul(width)
        .and_then(|index| usize::try_from(x).ok().and_then(|x| index.checked_add(x)))
        .ok_or(FlipExecutionError::ArithmeticOverflow)
}

fn map_roi<F>(
    roi: Roi,
    source: ImageDimensions,
    target: ImageDimensions,
    map: F,
) -> Result<Roi, FlipCoordinateError>
where
    F: Fn(u32, u32) -> Result<(u32, u32), FlipCoordinateError>,
{
    roi.within(source)
        .map_err(|_| FlipCoordinateError::OutOfBounds)?;
    if roi.is_empty() {
        return Roi::new(0, 0, 0, 0).map_err(|_| FlipCoordinateError::ArithmeticOverflow);
    }
    let right = roi
        .right()
        .checked_sub(1)
        .ok_or(FlipCoordinateError::ArithmeticOverflow)?;
    let bottom = roi
        .bottom()
        .checked_sub(1)
        .ok_or(FlipCoordinateError::ArithmeticOverflow)?;
    let corners = [
        (roi.x(), roi.y()),
        (right, roi.y()),
        (roi.x(), bottom),
        (right, bottom),
    ];
    let mut min_x = target.width();
    let mut min_y = target.height();
    let mut max_x = 0;
    let mut max_y = 0;
    for (x, y) in corners {
        let (mapped_x, mapped_y) = map(x, y)?;
        min_x = min_x.min(mapped_x);
        min_y = min_y.min(mapped_y);
        max_x = max_x.max(mapped_x);
        max_y = max_y.max(mapped_y);
    }
    Roi::new(
        min_x,
        min_y,
        max_x
            .checked_sub(min_x)
            .and_then(|width| width.checked_add(1))
            .ok_or(FlipCoordinateError::ArithmeticOverflow)?,
        max_y
            .checked_sub(min_y)
            .and_then(|height| height.checked_add(1))
            .ok_or(FlipCoordinateError::ArithmeticOverflow)?,
    )
    .map_err(|_| FlipCoordinateError::ArithmeticOverflow)
}

fn plan_identity(
    dimensions: RasterDimensions,
    config: &FlipConfig,
    source_orientation: Orientation,
    resolved_orientation: Orientation,
) -> Result<[u8; 32], FlipPlanError> {
    let mut hasher = Sha256::new();
    hasher.update(FLIP_COMPATIBILITY_ID.as_bytes());
    hasher.update(FLIP_SCHEMA_VERSION.to_le_bytes());
    hasher.update(FLIP_IMPLEMENTATION_VERSION.to_le_bytes());
    hasher.update(dimensions.width().to_le_bytes());
    hasher.update(dimensions.height().to_le_bytes());
    hasher.update([u8::from(matches!(config.mode(), FlipMode::Automatic))]);
    hasher.update([config.orientation().bits()]);
    hasher.update([source_orientation as u8, resolved_orientation as u8]);
    if let Some(source) = config.opaque_source() {
        let length = u64::try_from(source.len()).map_err(|_| FlipPlanError::ArithmeticOverflow)?;
        hasher.update(length.to_le_bytes());
        hasher.update(source);
    }
    Ok(hasher.finalize().into())
}

fn digest_pixels(pixels: &[LinearRgb]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(FLIP_COMPATIBILITY_ID.as_bytes());
    for pixel in pixels {
        hasher.update(pixel.red().get().to_bits().to_le_bytes());
        hasher.update(pixel.green().get().to_bits().to_le_bytes());
        hasher.update(pixel.blue().get().to_bits().to_le_bytes());
    }
    hasher.finalize().into()
}
