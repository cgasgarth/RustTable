//! Bounded operation-local CPU leaf for retained Darktable `src/iop/colorcontrast.c`.
//!
//! This standalone leaf owns the native v1/v2 parameter ABI, the direct v1-to-v2
//! migration, an operation-local descriptor, immutable committed state, the
//! source-ordered Lab equations with a deterministic explicit-fused Rust adaptation,
//! and a Rust-only fail-closed execution boundary. Native C multiply-plus-add
//! contraction depends on compiler, target, and profile; no exact authoritative
//! tuple is established, so noncontracting native profiles remain deferred. Native
//! `commit_params` accepts non-finite floats, native `process` neither rejects
//! non-finite samples nor stages transactional publication, and native has no
//! cancellation callback; those safeguards are adaptations rather than ported
//! behavior. The typed API intentionally accepts only equal-size, four-lane
//! rasters. Native required-format ROI copy-through and the build-dependent
//! three-versus-four-lane `for_each_channel` policy remain explicitly deferred;
//! deterministic fourth-lane handling is another documented Rust adaptation. The
//! leaf records native alignment and store facts separately from its ordinary `Vec`
//! scalar-storage adaptation, and native tiling separately from Rust-only tile-edge
//! and staging-allocation policy. Registry and history materialization, pixelpipe
//! dispatch, GPU execution, masks and outer blending, application editing, and GTK
//! controls also remain deferred.

#![forbid(unsafe_code)]
#![allow(
    clippy::doc_markdown,
    clippy::float_cmp,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    dead_code,
    reason = "standalone source-shaped ABI leaf is intentionally not exported through shared hubs"
)]

use std::fmt;
use std::mem::{align_of, size_of};

use rusttable_color::ColorEncoding;
use rusttable_processing::RasterDimensions;
use rusttable_processing::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
};

#[path = "source_map.rs"]
pub mod source_map;

pub const COLOR_CONTRAST_COMPATIBILITY_ID: &str = "colorcontrast";
pub const COLOR_CONTRAST_RUST_ID: &str = "rusttable.colorcontrast";
pub const COLOR_CONTRAST_SCHEMA_VERSION: u16 = 2;
pub const COLOR_CONTRAST_V1_PARAMETER_BYTES: usize = 16;
pub const COLOR_CONTRAST_V2_PARAMETER_BYTES: usize = 20;
pub const COLOR_CONTRAST_MIGRATION_EDGES: &[(u16, u16)] = &[(1, 2)];

pub const COLOR_CONTRAST_DEFAULT_A_STEEPNESS: f32 = 1.0;
pub const COLOR_CONTRAST_DEFAULT_A_OFFSET: f32 = 0.0;
pub const COLOR_CONTRAST_DEFAULT_B_STEEPNESS: f32 = 1.0;
pub const COLOR_CONTRAST_DEFAULT_B_OFFSET: f32 = 0.0;
pub const COLOR_CONTRAST_DEFAULT_UNBOUND: i32 = 1;

pub const COLOR_CONTRAST_NATIVE_NAME: &str = "color contrast";
pub const COLOR_CONTRAST_NATIVE_ALIASES: &str = "saturation";
pub const COLOR_CONTRAST_NATIVE_COLORSPACE: &str = "Lab";
pub const COLOR_CONTRAST_NATIVE_GROUPS: [&str; 2] = ["color", "grading"];
pub const COLOR_CONTRAST_NATIVE_FLAGS: [&str; 3] =
    ["include-in-styles", "supports-blending", "allow-tiling"];
pub const COLOR_CONTRAST_GPU_PROGRAM: u32 = 8;
pub const COLOR_CONTRAST_GPU_KERNEL: &str = "colorcontrast";
pub const COLOR_CONTRAST_CANCELLATION_POLL_PIXELS: usize = 1024;
/// Native `dt_bauhaus_slider_from_params` precision for both visible steepness sliders.
pub const COLOR_CONTRAST_NATIVE_STEEPNESS_SLIDER_PRECISION: u8 = 2;

/// Source-derived equal-ROI host factor from native `default_tiling_callback`.
pub const COLOR_CONTRAST_NATIVE_TILING_FACTOR: f32 = 2.0;
/// Source-derived equal-ROI OpenCL factor from native `default_tiling_callback`.
pub const COLOR_CONTRAST_NATIVE_TILING_FACTOR_CL: f32 = 2.0;
/// Source-derived largest single host-buffer factor.
pub const COLOR_CONTRAST_NATIVE_TILING_MAXBUF: f32 = 1.0;
/// Source-derived largest single OpenCL-buffer factor.
pub const COLOR_CONTRAST_NATIVE_TILING_MAXBUF_CL: f32 = 1.0;
/// Source-derived fixed overhead in bytes.
pub const COLOR_CONTRAST_NATIVE_TILING_OVERHEAD_BYTES: usize = 0;
/// Source-derived tile overlap in pixels.
pub const COLOR_CONTRAST_NATIVE_TILING_OVERLAP_PIXELS: u32 = 0;
/// Source-derived x/y tile alignment in pixels.
pub const COLOR_CONTRAST_NATIVE_TILING_ALIGNMENT_PIXELS: u32 = 1;

/// Rust descriptor policy; native `default_tiling_callback` has no minimum edge.
pub const COLOR_CONTRAST_RUST_MINIMUM_TILE_EDGE: u32 = 1;
/// Rust scheduler preference; native `default_tiling_callback` has no preferred edge.
pub const COLOR_CONTRAST_RUST_PREFERRED_TILE_EDGE: u32 = 256;
/// Rust descriptor accounting for the caller-owned input raster.
pub const COLOR_CONTRAST_RUST_INPUT_MULTIPLIER_MILLI: u32 = 1000;
/// Rust descriptor accounting for the caller-owned destination raster.
pub const COLOR_CONTRAST_RUST_OUTPUT_MULTIPLIER_MILLI: u32 = 1000;
/// Rust-only full-raster staging allocation used for transactional publication.
pub const COLOR_CONTRAST_RUST_STAGING_MULTIPLIER_MILLI: u32 = 1000;
/// Rust `execute_into` resident budget: input, destination, and staging rasters.
pub const COLOR_CONTRAST_RUST_EXECUTE_INTO_RASTERS: usize = 3;
/// Rust descriptor-scale expression of the three-raster resident budget.
pub const COLOR_CONTRAST_RUST_EXECUTE_INTO_MULTIPLIER_MILLI: u32 =
    COLOR_CONTRAST_RUST_INPUT_MULTIPLIER_MILLI
        + COLOR_CONTRAST_RUST_OUTPUT_MULTIPLIER_MILLI
        + COLOR_CONTRAST_RUST_STAGING_MULTIPLIER_MILLI;

const _: () = assert!(COLOR_CONTRAST_RUST_EXECUTE_INTO_MULTIPLIER_MILLI == 3000);

/// Native `DT_CACHELINE_BYTES` on Apple AArch64 targets.
pub const COLOR_CONTRAST_NATIVE_APPLE_AARCH64_CACHELINE_BYTES: usize = 128;
/// Native `DT_CACHELINE_BYTES` on all other targets.
pub const COLOR_CONTRAST_NATIVE_OTHER_CACHELINE_BYTES: usize = 64;
/// Native alignment of the four-float `dt_aligned_pixel_t` stack type.
pub const COLOR_CONTRAST_NATIVE_ALIGNED_PIXEL_BYTES: usize = 16;

/// Native `for_each_channel` width when `DT_NO_VECTORIZATION` is defined.
pub const COLOR_CONTRAST_NATIVE_NO_VECTORIZATION_LANES: usize = 3;
/// Native `for_each_channel` width for vectorization-enabled builds.
pub const COLOR_CONTRAST_NATIVE_VECTORIZED_LANES: usize = 4;
/// Fixed lane width of this operation-local Rust adaptation.
pub const COLOR_CONTRAST_RUST_LANES: usize = 4;

const COLOR_CONTRAST_CHANNELS: usize = COLOR_CONTRAST_RUST_LANES;
const COLOR_CONTRAST_AB_MINIMUM: f32 = -128.0;
const COLOR_CONTRAST_AB_MAXIMUM: f32 = 128.0;

/// The leaf deliberately initializes all four lanes instead of exposing native's
/// build-dependent `DT_PIXEL_SIMD_CHANNELS` choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorContrastLanePolicy {
    DeterministicFourLaneRustAdaptation,
}

#[must_use]
pub const fn lane_policy() -> ColorContrastLanePolicy {
    ColorContrastLanePolicy::DeterministicFourLaneRustAdaptation
}

/// Native v1 payload in declaration and in-memory byte order.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct ColorContrastParametersV1 {
    pub a_steepness: f32,
    pub a_offset: f32,
    pub b_steepness: f32,
    pub b_offset: f32,
}

const _: () = assert!(size_of::<ColorContrastParametersV1>() == COLOR_CONTRAST_V1_PARAMETER_BYTES);

impl ColorContrastParametersV1 {
    #[must_use]
    pub const fn new(a_steepness: f32, a_offset: f32, b_steepness: f32, b_offset: f32) -> Self {
        Self {
            a_steepness,
            a_offset,
            b_steepness,
            b_offset,
        }
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; COLOR_CONTRAST_V1_PARAMETER_BYTES] {
        let mut bytes = [0_u8; COLOR_CONTRAST_V1_PARAMETER_BYTES];
        write_f32(&mut bytes, 0, self.a_steepness);
        write_f32(&mut bytes, 4, self.a_offset);
        write_f32(&mut bytes, 8, self.b_steepness);
        write_f32(&mut bytes, 12, self.b_offset);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColorContrastCodecError> {
        require_length(bytes, COLOR_CONTRAST_V1_PARAMETER_BYTES)?;
        Ok(Self::new(
            read_f32(bytes, 0),
            read_f32(bytes, 4),
            read_f32(bytes, 8),
            read_f32(bytes, 12),
        ))
    }
}

/// Current native v2 payload in declaration and in-memory byte order.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct ColorContrastParametersV2 {
    pub a_steepness: f32,
    pub a_offset: f32,
    pub b_steepness: f32,
    pub b_offset: f32,
    /// Exact native `int`; every nonzero value selects the unbounded branch.
    pub unbound: i32,
}

const _: () = assert!(size_of::<ColorContrastParametersV2>() == COLOR_CONTRAST_V2_PARAMETER_BYTES);

impl ColorContrastParametersV2 {
    #[must_use]
    pub const fn new(
        a_steepness: f32,
        a_offset: f32,
        b_steepness: f32,
        b_offset: f32,
        unbound: i32,
    ) -> Self {
        Self {
            a_steepness,
            a_offset,
            b_steepness,
            b_offset,
            unbound,
        }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(
            COLOR_CONTRAST_DEFAULT_A_STEEPNESS,
            COLOR_CONTRAST_DEFAULT_A_OFFSET,
            COLOR_CONTRAST_DEFAULT_B_STEEPNESS,
            COLOR_CONTRAST_DEFAULT_B_OFFSET,
            COLOR_CONTRAST_DEFAULT_UNBOUND,
        )
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; COLOR_CONTRAST_V2_PARAMETER_BYTES] {
        let mut bytes = [0_u8; COLOR_CONTRAST_V2_PARAMETER_BYTES];
        write_f32(&mut bytes, 0, self.a_steepness);
        write_f32(&mut bytes, 4, self.a_offset);
        write_f32(&mut bytes, 8, self.b_steepness);
        write_f32(&mut bytes, 12, self.b_offset);
        write_i32(&mut bytes, 16, self.unbound);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColorContrastCodecError> {
        require_length(bytes, COLOR_CONTRAST_V2_PARAMETER_BYTES)?;
        Ok(Self::new(
            read_f32(bytes, 0),
            read_f32(bytes, 4),
            read_f32(bytes, 8),
            read_f32(bytes, 12),
            read_i32(bytes, 16),
        ))
    }
}

const fn require_length(bytes: &[u8], expected: usize) -> Result<(), ColorContrastCodecError> {
    if bytes.len() == expected {
        Ok(())
    } else {
        Err(ColorContrastCodecError::InvalidLength {
            expected,
            actual: bytes.len(),
        })
    }
}

fn write_f32<const LENGTH: usize>(bytes: &mut [u8; LENGTH], offset: usize, value: f32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_i32<const LENGTH: usize>(bytes: &mut [u8; LENGTH], offset: usize, value: i32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("payload length was checked"),
    )
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("payload length was checked"),
    )
}

/// Typed known history with byte-exact retention for unknown future versions.
#[derive(Debug, Clone, PartialEq)]
pub enum ColorContrastHistory {
    V1(ColorContrastParametersV1),
    V2(ColorContrastParametersV2),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl ColorContrastHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, ColorContrastCodecError> {
        match version {
            1 => Ok(Self::V1(ColorContrastParametersV1::from_bytes(bytes)?)),
            COLOR_CONTRAST_SCHEMA_VERSION => {
                Ok(Self::V2(ColorContrastParametersV2::from_bytes(bytes)?))
            }
            _ => Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            }),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => 1,
            Self::V2(_) => COLOR_CONTRAST_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(parameters) => parameters.to_bytes().to_vec(),
            Self::V2(parameters) => parameters.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    pub const fn current(&self) -> Result<ColorContrastParametersV2, ColorContrastCodecError> {
        match self {
            Self::V1(parameters) => Ok(migrate_v1_to_v2(*parameters)),
            Self::V2(parameters) => Ok(*parameters),
            Self::Opaque { version, .. } => {
                Err(ColorContrastCodecError::UnsupportedVersion(*version))
            }
        }
    }
}

/// Reproduces the only native `legacy_params` edge, including bounded mode.
#[must_use]
pub const fn migrate_v1_to_v2(parameters: ColorContrastParametersV1) -> ColorContrastParametersV2 {
    ColorContrastParametersV2::new(
        parameters.a_steepness,
        parameters.a_offset,
        parameters.b_steepness,
        parameters.b_offset,
        0,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorContrastCodecError {
    InvalidLength { expected: usize, actual: usize },
    UnsupportedVersion(u16),
}

impl fmt::Display for ColorContrastCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "Color Contrast payload has {actual} bytes; expected {expected}"
                )
            }
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "Color Contrast version {version} is opaque and unsupported"
                )
            }
        }
    }
}

impl std::error::Error for ColorContrastCodecError {}

/// Finite immutable state captured at the native `commit_params` synchronization point.
///
/// Native copies NaN and infinity into piece data without validation. Rejecting
/// those values is an explicit fail-closed Rust adaptation. Accepted finite values
/// retain their original IEEE-754 bits so persisted values such as `-0.0` remain
/// byte-exact through execution and reserialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorContrastConfig {
    a_steepness: PreservedFiniteF32,
    a_offset: PreservedFiniteF32,
    b_steepness: PreservedFiniteF32,
    b_offset: PreservedFiniteF32,
    unbound: i32,
}

impl ColorContrastConfig {
    pub fn new(
        a_steepness: f32,
        a_offset: f32,
        b_steepness: f32,
        b_offset: f32,
        unbound: i32,
    ) -> Result<Self, ColorContrastParameterError> {
        Self::try_from(ColorContrastParametersV2::new(
            a_steepness,
            a_offset,
            b_steepness,
            b_offset,
            unbound,
        ))
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::new(
            COLOR_CONTRAST_DEFAULT_A_STEEPNESS,
            COLOR_CONTRAST_DEFAULT_A_OFFSET,
            COLOR_CONTRAST_DEFAULT_B_STEEPNESS,
            COLOR_CONTRAST_DEFAULT_B_OFFSET,
            COLOR_CONTRAST_DEFAULT_UNBOUND,
        )
        .expect("source defaults are finite")
    }

    #[must_use]
    pub const fn a_steepness(self) -> f32 {
        self.a_steepness.get()
    }

    #[must_use]
    pub const fn a_offset(self) -> f32 {
        self.a_offset.get()
    }

    #[must_use]
    pub const fn b_steepness(self) -> f32 {
        self.b_steepness.get()
    }

    #[must_use]
    pub const fn b_offset(self) -> f32 {
        self.b_offset.get()
    }

    #[must_use]
    pub const fn unbound(self) -> i32 {
        self.unbound
    }

    #[must_use]
    pub const fn is_unbound(self) -> bool {
        self.unbound != 0
    }

    #[must_use]
    pub const fn parameters(self) -> ColorContrastParametersV2 {
        ColorContrastParametersV2::new(
            self.a_steepness.get(),
            self.a_offset.get(),
            self.b_steepness.get(),
            self.b_offset.get(),
            self.unbound,
        )
    }
}

impl TryFrom<ColorContrastParametersV2> for ColorContrastConfig {
    type Error = ColorContrastParameterError;

    fn try_from(parameters: ColorContrastParametersV2) -> Result<Self, Self::Error> {
        Ok(Self {
            a_steepness: PreservedFiniteF32::new("a_steepness", parameters.a_steepness)?,
            a_offset: PreservedFiniteF32::new("a_offset", parameters.a_offset)?,
            b_steepness: PreservedFiniteF32::new("b_steepness", parameters.b_steepness)?,
            b_offset: PreservedFiniteF32::new("b_offset", parameters.b_offset)?,
            unbound: parameters.unbound,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorContrastParameterError {
    NonFinite(&'static str),
}

impl fmt::Display for ColorContrastParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(field) => write!(formatter, "Color Contrast {field} is non-finite"),
        }
    }
}

impl std::error::Error for ColorContrastParameterError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PreservedFiniteF32(u32);

impl PreservedFiniteF32 {
    const fn new(field: &'static str, value: f32) -> Result<Self, ColorContrastParameterError> {
        if value.is_finite() {
            Ok(Self(value.to_bits()))
        } else {
            Err(ColorContrastParameterError::NonFinite(field))
        }
    }

    const fn get(self) -> f32 {
        f32::from_bits(self.0)
    }
}

/// Rust leaf sample in `L`, `a`, `b`, spare/alpha order.
///
/// The fixed fourth lane is a deterministic Rust adaptation. Native
/// `for_each_channel` processes either three or four lanes depending on its
/// vectorization build, and native explicitly tells later code not to rely on
/// the spare/alpha value unless another boundary sets it. Native `process` also
/// asserts cache-line-aligned input/output pointers, uses 16-byte-aligned
/// `dt_aligned_pixel_t` locals, and publishes through architecture-dependent
/// `copy_pixel_nontemporal` stores. This leaf instead uses ordinary `Vec` storage,
/// natural Rust element alignment, source-level scalar evaluation, and
/// `copy_from_slice`; that storage policy is an adaptation independent of tile
/// alignment and allocation-budget facts.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(transparent)]
pub struct ColorContrastPixel {
    channels: [f32; COLOR_CONTRAST_CHANNELS],
}

const _: () = assert!(size_of::<ColorContrastPixel>() == 4 * size_of::<f32>());
/// Required element alignment of ordinary `Vec<ColorContrastPixel>` storage.
pub const COLOR_CONTRAST_RUST_PIXEL_TYPE_ALIGNMENT_BYTES: usize = align_of::<ColorContrastPixel>();

impl ColorContrastPixel {
    #[must_use]
    pub const fn new(lightness: f32, a: f32, b: f32, alpha: f32) -> Self {
        Self {
            channels: [lightness, a, b, alpha],
        }
    }

    #[must_use]
    pub const fn from_channels(channels: [f32; COLOR_CONTRAST_CHANNELS]) -> Self {
        Self { channels }
    }

    #[must_use]
    pub const fn channels(self) -> [f32; COLOR_CONTRAST_CHANNELS] {
        self.channels
    }

    #[must_use]
    pub const fn lightness(self) -> f32 {
        self.channels[0]
    }

    #[must_use]
    pub const fn a(self) -> f32 {
        self.channels[1]
    }

    #[must_use]
    pub const fn b(self) -> f32 {
        self.channels[2]
    }

    #[must_use]
    pub const fn alpha(self) -> f32 {
        self.channels[3]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorContrastChannel {
    Lightness,
    A,
    B,
    Alpha,
}

const CHANNEL_ORDER: [ColorContrastChannel; COLOR_CONTRAST_CHANNELS] = [
    ColorContrastChannel::Lightness,
    ColorContrastChannel::A,
    ColorContrastChannel::B,
    ColorContrastChannel::Alpha,
];

/// Byte accounting for the transactional `execute_into` boundary.
///
/// Input and destination storage are caller-owned. The leaf requests
/// `staging_bytes` of element payload before publication; allocator metadata and
/// rounding are outside this operation contract. `resident_bytes` accounts for
/// the three simultaneously live raster payloads and no fixed operation overhead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorContrastAllocationBudget {
    pixel_count: usize,
    raster_bytes: usize,
    input_bytes: usize,
    output_bytes: usize,
    staging_bytes: usize,
    resident_bytes: usize,
}

impl ColorContrastAllocationBudget {
    fn for_pixel_count(pixel_count: usize) -> Result<Self, ColorContrastExecutionError> {
        let raster_bytes = pixel_count
            .checked_mul(size_of::<ColorContrastPixel>())
            .ok_or(ColorContrastExecutionError::SizeOverflow)?;
        let resident_bytes = raster_bytes
            .checked_mul(COLOR_CONTRAST_RUST_EXECUTE_INTO_RASTERS)
            .ok_or(ColorContrastExecutionError::SizeOverflow)?;
        Ok(Self {
            pixel_count,
            raster_bytes,
            input_bytes: raster_bytes,
            output_bytes: raster_bytes,
            staging_bytes: raster_bytes,
            resident_bytes,
        })
    }

    #[must_use]
    pub const fn pixel_count(self) -> usize {
        self.pixel_count
    }

    #[must_use]
    pub const fn raster_bytes(self) -> usize {
        self.raster_bytes
    }

    #[must_use]
    pub const fn input_bytes(self) -> usize {
        self.input_bytes
    }

    #[must_use]
    pub const fn output_bytes(self) -> usize {
        self.output_bytes
    }

    /// Operation-owned `Vec` element-payload budget requested before execution.
    #[must_use]
    pub const fn staging_bytes(self) -> usize {
        self.staging_bytes
    }

    #[must_use]
    pub const fn resident_bytes(self) -> usize {
        self.resident_bytes
    }
}

/// Immutable point-operation plan corresponding to a committed pixelpipe piece.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorContrastPlan {
    config: ColorContrastConfig,
    dimensions: RasterDimensions,
}

impl ColorContrastPlan {
    #[must_use]
    pub const fn new(config: ColorContrastConfig, dimensions: RasterDimensions) -> Self {
        Self { config, dimensions }
    }

    #[must_use]
    pub const fn config(self) -> ColorContrastConfig {
        self.config
    }

    #[must_use]
    pub const fn dimensions(self) -> RasterDimensions {
        self.dimensions
    }

    /// Returns the byte-accurate transactional `execute_into` resident budget.
    pub fn allocation_budget(
        self,
    ) -> Result<ColorContrastAllocationBudget, ColorContrastExecutionError> {
        ColorContrastAllocationBudget::for_pixel_count(self.expected_pixels()?)
    }

    /// Executes the source CPU equation and returns only a complete raster.
    ///
    /// This narrowed leaf boundary accepts an already validated four-lane input
    /// whose ROI exactly matches `dimensions`. Actual-channel fallback,
    /// differently positioned input/output ROIs, cropping, zero-padding, and
    /// native trouble publication belong to the deferred shared image-buffer
    /// boundary.
    pub fn execute(
        &self,
        input: &[ColorContrastPixel],
    ) -> Result<Vec<ColorContrastPixel>, ColorContrastExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    /// Polls at row boundaries and at most every 1024 pixels, then again before publication.
    pub fn execute_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[ColorContrastPixel],
        mut cancelled: F,
    ) -> Result<Vec<ColorContrastPixel>, ColorContrastExecutionError> {
        let expected = self.expected_pixels()?;
        if input.len() != expected {
            return Err(ColorContrastExecutionError::InputDimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        if cancelled() {
            return Err(ColorContrastExecutionError::Cancelled);
        }

        let allocation_budget = ColorContrastAllocationBudget::for_pixel_count(expected)?;
        let mut output = Vec::new();
        output.try_reserve_exact(expected).map_err(|_| {
            ColorContrastExecutionError::AllocationFailed {
                required_bytes: allocation_budget.staging_bytes(),
            }
        })?;

        let width = usize::try_from(self.dimensions.width())
            .map_err(|_| ColorContrastExecutionError::SizeOverflow)?;
        for (pixel_index, pixel) in input.iter().copied().enumerate() {
            if pixel_index != 0
                && (pixel_index % width == 0
                    || pixel_index % COLOR_CONTRAST_CANCELLATION_POLL_PIXELS == 0)
                && cancelled()
            {
                return Err(ColorContrastExecutionError::Cancelled);
            }
            output.push(self.evaluate_pixel(pixel, pixel_index)?);
        }

        if cancelled() {
            return Err(ColorContrastExecutionError::Cancelled);
        }
        Ok(output)
    }

    /// Executes into a caller-owned destination and publishes only after full success.
    pub fn execute_into(
        &self,
        input: &[ColorContrastPixel],
        output: &mut [ColorContrastPixel],
    ) -> Result<(), ColorContrastExecutionError> {
        self.execute_into_with_cancel(input, output, || false)
    }

    /// Leaves `output` byte-for-byte unchanged on validation, execution, or cancellation failure.
    pub fn execute_into_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[ColorContrastPixel],
        output: &mut [ColorContrastPixel],
        cancelled: F,
    ) -> Result<(), ColorContrastExecutionError> {
        let expected = self.expected_pixels()?;
        if output.len() != expected {
            return Err(ColorContrastExecutionError::OutputDimensionsMismatch {
                expected,
                actual: output.len(),
            });
        }
        let staged = self.execute_with_cancel(input, cancelled)?;
        output.copy_from_slice(&staged);
        Ok(())
    }

    fn expected_pixels(self) -> Result<usize, ColorContrastExecutionError> {
        usize::try_from(self.dimensions.pixel_count())
            .map_err(|_| ColorContrastExecutionError::SizeOverflow)
    }

    fn evaluate_pixel(
        self,
        pixel: ColorContrastPixel,
        pixel_index: usize,
    ) -> Result<ColorContrastPixel, ColorContrastExecutionError> {
        let input = pixel.channels();
        for (channel_index, value) in input.into_iter().enumerate() {
            if !value.is_finite() {
                return Err(ColorContrastExecutionError::NonFiniteInput {
                    pixel: pixel_index,
                    channel: CHANNEL_ORDER[channel_index],
                });
            }
        }

        // Preserve the native declaration and evaluation order from `process`.
        let slope = [
            1.0_f32,
            self.config.a_steepness(),
            self.config.b_steepness(),
            1.0_f32,
        ];
        let offset = [
            0.0_f32,
            self.config.a_offset(),
            self.config.b_offset(),
            0.0_f32,
        ];
        let lower = [
            -f32::MAX,
            COLOR_CONTRAST_AB_MINIMUM,
            COLOR_CONTRAST_AB_MINIMUM,
            -f32::MAX,
        ];
        let upper = [
            f32::MAX,
            COLOR_CONTRAST_AB_MAXIMUM,
            COLOR_CONTRAST_AB_MAXIMUM,
            f32::MAX,
        ];

        // Native selects three or four iterations through `for_each_channel` at
        // build time. This standalone leaf instead initializes all four lanes so
        // its typed output is deterministic; that is a Rust adaptation, not a
        // claim that native guarantees fourth-lane preservation.
        let mut output = [0.0_f32; COLOR_CONTRAST_CHANNELS];
        for channel_index in 0..COLOR_CONTRAST_CHANNELS {
            // Native spells this as multiply followed by addition. Contraction is
            // compiler/target/profile dependent, and no exact authoritative native
            // tuple is established. Explicit `mul_add` is therefore a deterministic
            // Rust adaptation; noncontracting native profiles remain deferred.
            let scaled = input[channel_index].mul_add(slope[channel_index], offset[channel_index]);
            output[channel_index] = if self.config.is_unbound() {
                scaled
            } else {
                darktable_clamps(scaled, lower[channel_index], upper[channel_index])
            };
            if !output[channel_index].is_finite() {
                return Err(ColorContrastExecutionError::NonFiniteOutput {
                    pixel: pixel_index,
                    channel: CHANNEL_ORDER[channel_index],
                });
            }
        }
        Ok(ColorContrastPixel::from_channels(output))
    }
}

/// Exact branch order of native `CLAMPS(A, L, H)`.
fn darktable_clamps(value: f32, lower: f32, upper: f32) -> f32 {
    if value > lower {
        if value < upper { value } else { upper }
    } else {
        lower
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorContrastExecutionError {
    InputDimensionsMismatch {
        expected: usize,
        actual: usize,
    },
    OutputDimensionsMismatch {
        expected: usize,
        actual: usize,
    },
    NonFiniteInput {
        pixel: usize,
        channel: ColorContrastChannel,
    },
    NonFiniteOutput {
        pixel: usize,
        channel: ColorContrastChannel,
    },
    AllocationFailed {
        required_bytes: usize,
    },
    SizeOverflow,
    Cancelled,
}

impl fmt::Display for ColorContrastExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputDimensionsMismatch { expected, actual } => write!(
                formatter,
                "Color Contrast expected {expected} input pixels, got {actual}"
            ),
            Self::OutputDimensionsMismatch { expected, actual } => write!(
                formatter,
                "Color Contrast expected {expected} output pixels, got {actual}"
            ),
            Self::NonFiniteInput { pixel, channel } => write!(
                formatter,
                "Color Contrast input pixel {pixel} has non-finite {channel:?}"
            ),
            Self::NonFiniteOutput { pixel, channel } => write!(
                formatter,
                "Color Contrast output pixel {pixel} has non-finite {channel:?}"
            ),
            Self::AllocationFailed { required_bytes } => write!(
                formatter,
                "Color Contrast allocation failed for {required_bytes} bytes"
            ),
            Self::SizeOverflow => formatter.write_str("Color Contrast execution size overflowed"),
            Self::Cancelled => formatter.write_str("Color Contrast execution was cancelled"),
        }
    }
}

impl std::error::Error for ColorContrastExecutionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorContrastCapabilityError {
    GpuUnavailable,
    GtkUnavailable,
    MasksAndBlendingUnavailable,
    RequiredFormatImageBufferBoundaryDeferred,
    NativeLanePolicyDeferred,
    ProductionRoutingDeferred,
}

impl fmt::Display for ColorContrastCapabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::GpuUnavailable => "Color Contrast GPU execution is unavailable in this leaf",
            Self::GtkUnavailable => "Color Contrast GTK controls are unavailable in this leaf",
            Self::MasksAndBlendingUnavailable => {
                "Color Contrast masks and outer blending are unavailable in this leaf"
            }
            Self::RequiredFormatImageBufferBoundaryDeferred => {
                "Color Contrast native required-format image-buffer handling is deferred"
            }
            Self::NativeLanePolicyDeferred => {
                "Color Contrast native three-versus-four-lane policy is deferred"
            }
            Self::ProductionRoutingDeferred => "Color Contrast production routing is deferred",
        })
    }
}

impl std::error::Error for ColorContrastCapabilityError {}

/// Capability facts kept separate from native metadata and shared production routing.
#[expect(
    clippy::struct_excessive_bools,
    reason = "Independent leaf capabilities must remain independently fail-closed."
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorContrastCapabilities {
    pub cpu_supported: bool,
    pub gpu_supported: bool,
    pub gtk_supported: bool,
    pub masks_and_outer_blending_supported: bool,
    pub native_required_format_image_buffer_supported: bool,
    pub native_for_each_channel_policy_supported: bool,
    pub deterministic_four_lane_rust_adaptation: bool,
    pub production_routing_supported: bool,
}

impl ColorContrastCapabilities {
    #[must_use]
    pub const fn bounded_cpu_leaf() -> Self {
        Self {
            cpu_supported: true,
            gpu_supported: false,
            gtk_supported: false,
            masks_and_outer_blending_supported: false,
            native_required_format_image_buffer_supported: false,
            native_for_each_channel_policy_supported: false,
            deterministic_four_lane_rust_adaptation: true,
            production_routing_supported: false,
        }
    }

    pub const fn require_gpu(self) -> Result<(), ColorContrastCapabilityError> {
        if self.gpu_supported {
            Ok(())
        } else {
            Err(ColorContrastCapabilityError::GpuUnavailable)
        }
    }

    pub const fn require_gtk(self) -> Result<(), ColorContrastCapabilityError> {
        if self.gtk_supported {
            Ok(())
        } else {
            Err(ColorContrastCapabilityError::GtkUnavailable)
        }
    }

    pub const fn require_masks_and_outer_blending(
        self,
    ) -> Result<(), ColorContrastCapabilityError> {
        if self.masks_and_outer_blending_supported {
            Ok(())
        } else {
            Err(ColorContrastCapabilityError::MasksAndBlendingUnavailable)
        }
    }

    pub const fn require_native_required_format_image_buffer(
        self,
    ) -> Result<(), ColorContrastCapabilityError> {
        if self.native_required_format_image_buffer_supported {
            Ok(())
        } else {
            Err(ColorContrastCapabilityError::RequiredFormatImageBufferBoundaryDeferred)
        }
    }

    pub const fn require_native_for_each_channel_policy(
        self,
    ) -> Result<(), ColorContrastCapabilityError> {
        if self.native_for_each_channel_policy_supported {
            Ok(())
        } else {
            Err(ColorContrastCapabilityError::NativeLanePolicyDeferred)
        }
    }

    pub const fn require_production_routing(self) -> Result<(), ColorContrastCapabilityError> {
        if self.production_routing_supported {
            Ok(())
        } else {
            Err(ColorContrastCapabilityError::ProductionRoutingDeferred)
        }
    }
}

#[must_use]
pub const fn capabilities() -> ColorContrastCapabilities {
    ColorContrastCapabilities::bounded_cpu_leaf()
}

/// Operation-local descriptor evidence; this function does not register the leaf.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "The descriptor keeps the source operation identity, capability, IO, and tiling contract together."
)]
pub fn colorcontrast_descriptor() -> OperationDescriptor {
    OperationDescriptor {
        id: DescriptorId::new(
            COLOR_CONTRAST_COMPATIBILITY_ID,
            COLOR_CONTRAST_RUST_ID,
            COLOR_CONTRAST_SCHEMA_VERSION,
            COLOR_CONTRAST_SCHEMA_VERSION,
            1,
        )
        .expect("static Color Contrast descriptor identity"),
        parameters: vec![
            scalar_parameter(
                "a_steepness",
                0.0,
                5.0,
                f64::from(COLOR_CONTRAST_DEFAULT_A_STEEPNESS),
                1,
                true,
            ),
            scalar_parameter(
                "a_offset",
                f64::from(-f32::MAX),
                f64::from(f32::MAX),
                f64::from(COLOR_CONTRAST_DEFAULT_A_OFFSET),
                1,
                false,
            ),
            scalar_parameter(
                "b_steepness",
                0.0,
                5.0,
                f64::from(COLOR_CONTRAST_DEFAULT_B_STEEPNESS),
                1,
                true,
            ),
            scalar_parameter(
                "b_offset",
                f64::from(-f32::MAX),
                f64::from(f32::MAX),
                f64::from(COLOR_CONTRAST_DEFAULT_B_OFFSET),
                1,
                false,
            ),
            ParameterDescriptor {
                id: "unbound".to_owned(),
                kind: ParameterKind::Integer {
                    minimum: i64::from(i32::MIN),
                    maximum: i64::from(i32::MAX),
                },
                default: ParameterDefault::Integer(i64::from(COLOR_CONTRAST_DEFAULT_UNBOUND)),
                required: false,
                introduced_version: 2,
                removed_version: None,
                unit: None,
                step: None,
                precision: 0,
                role: ParameterRole::Processing,
                cache_affecting: true,
                animatable: false,
                ui_hint: None,
                condition: None,
            },
        ],
        flags: OperationFlags::MULTI_INSTANCE
            .insert(OperationFlags::STYLE_ELIGIBLE)
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::COLOR),
        stage: "display-referred-lab-d50".to_owned(),
        roi: RoiKind::Identity,
        tiling: TilingContract {
            overlap_pixels: COLOR_CONTRAST_NATIVE_TILING_OVERLAP_PIXELS,
            alignment_pixels: COLOR_CONTRAST_NATIVE_TILING_ALIGNMENT_PIXELS,
            minimum_tile_edge: COLOR_CONTRAST_RUST_MINIMUM_TILE_EDGE,
            preferred_tile_edge: COLOR_CONTRAST_RUST_PREFERRED_TILE_EDGE,
            temporary_multiplier_milli: COLOR_CONTRAST_RUST_STAGING_MULTIPLIER_MILLI,
            input_multiplier_milli: COLOR_CONTRAST_RUST_INPUT_MULTIPLIER_MILLI,
            output_multiplier_milli: COLOR_CONTRAST_RUST_OUTPUT_MULTIPLIER_MILLI,
        },
        capability: CapabilityContract {
            cpu_supported: true,
            gpu_tier: None,
            required_features: vec![
                "deterministic-row-major".to_owned(),
                "explicit-fused-f32-rust-adaptation".to_owned(),
                "four-lane-rust-adaptation".to_owned(),
                "ordinary-vec-scalar-storage-rust-adaptation".to_owned(),
            ],
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: false,
            precision: "explicit fused f32 Lab multiply-add as a deterministic Rust adaptation because native C contraction is compiler/target/profile dependent, with native CLAMPS and deterministic four-lane Rust handling"
                .to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: lab_io(),
        mask_blend: MaskBlendContract {
            consumes_mask: false,
            publishes_mask: false,
            blend_if: false,
            geometry: false,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![1, 2],
            target_version: COLOR_CONTRAST_SCHEMA_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}

fn scalar_parameter(
    id: &str,
    minimum: f64,
    maximum: f64,
    default: f64,
    introduced_version: u16,
    visible_slider: bool,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: false,
        introduced_version,
        removed_version: None,
        unit: None,
        step: None,
        // Native introspection supplies the 0..5 range to
        // `dt_bauhaus_slider_from_params`, whose precision formula resolves to two
        // digits for both visible steepness controls. Hidden offsets have no slider.
        precision: if visible_slider {
            COLOR_CONTRAST_NATIVE_STEEPNESS_SLIDER_PRECISION
        } else {
            0
        },
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: false,
        ui_hint: visible_slider.then(|| "slider".to_owned()),
        condition: None,
    }
}

fn lab_io() -> InputOutputContract {
    // `Preserve` describes this narrowed Rust leaf's deterministic fourth-lane
    // adaptation, not native's build-dependent `for_each_channel` guarantee.
    // `Reject` likewise records the Rust fail-closed boundary; native process
    // performs no finite sample validation.
    let image = ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LabD50],
        nonfinite: NonFinitePolicy::Reject,
    };
    InputOutputContract {
        input: image.clone(),
        output: image,
        derives_output_encoding: false,
    }
}
