#![expect(
    clippy::suboptimal_flops,
    reason = "Native Monochrome arithmetic order is preserved for IEEE-754 parity."
)]

//! Bounded Monochrome CPU leaf ported from `src/iop/monochrome.c`.
//!
//! The leaf owns the native v2 ABI, v1 migration, defaults, Lab color-filter
//! math, bilateral contribution, cancellation/publication boundary, alpha
//! handling, and native tiling arithmetic. Registry, history materialization,
//! pixelpipe routing, outer blending, GPU/OpenCL execution, GTK/profile-panel
//! behavior, and presets remain explicitly unavailable rather than approximated.
//!
//! The retained `OpenCL` kernels are currently embedded in `data/kernels/basic.cl`;
//! no standalone `data/kernels/monochrome.cl` exists in the pinned tree. The CPU
//! leaf therefore does not claim an executable GPU capability.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::manual_saturating_arithmetic,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::similar_names,
    clippy::struct_excessive_bools,
    clippy::too_many_arguments,
    dead_code,
    reason = "native ABI and scalar raster expressions retain their source-shaped f32 boundaries"
)]

use std::fmt;
use std::mem::size_of;

use rusttable_processing::RasterDimensions;
use rusttable_processing::common::bilateral::{BilateralError, BilateralGeometry, BilateralGrid};

pub mod source_map;

pub const MONOCHROME_COMPATIBILITY_ID: &str = "monochrome";
pub const MONOCHROME_RUST_ID: &str = "rusttable.monochrome";
pub const MONOCHROME_SCHEMA_VERSION: u16 = 2;
pub const MONOCHROME_V1_PARAMETER_BYTES: usize = 12;
pub const MONOCHROME_V2_PARAMETER_BYTES: usize = 16;
pub const MONOCHROME_DEFAULT_A: f32 = 0.0;
pub const MONOCHROME_DEFAULT_B: f32 = 0.0;
pub const MONOCHROME_DEFAULT_SIZE: f32 = 2.0;
pub const MONOCHROME_DEFAULT_HIGHLIGHTS: f32 = 0.0;
pub const MONOCHROME_DEFAULT_GROUPS: [&str; 2] = ["color", "effects"];
pub const MONOCHROME_DEFAULT_COLORSPACE: &str = "Lab";
pub const MONOCHROME_DESCRIPTION: &str =
    "quickly convert an image to black & white\nusing a variable color filter";
pub const MONOCHROME_SUPPORTS_BLENDING: bool = true;
pub const MONOCHROME_ALLOW_TILING: bool = true;
pub const MONOCHROME_GPU_PROGRAM: u32 = 2;
pub const MONOCHROME_GPU_KERNELS: [&str; 2] = ["monochrome_filter", "monochrome"];
pub const MONOCHROME_GPU_EXECUTABLE: bool = false;
pub const MONOCHROME_MIGRATION_EDGES: &[(u16, u16)] = &[(1, 2)];

const COLOR_FILTER_SCALE: f32 = 128.0;
const SIGMA_S_BASE: f32 = 20.0;
const SIGMA_R: f32 = 250.0;
const BILATERAL_DETAIL: f32 = -1.0;
const LAB_LIGHTNESS_MAXIMUM: f32 = 100.0;

/// Native v1 `dt_iop_monochrome_params_t` before the highlights field existed.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct MonochromeParametersV1 {
    pub a: f32,
    pub b: f32,
    pub size: f32,
}

const _: () = assert!(size_of::<MonochromeParametersV1>() == MONOCHROME_V1_PARAMETER_BYTES);

impl MonochromeParametersV1 {
    #[must_use]
    pub const fn new(a: f32, b: f32, size: f32) -> Self {
        Self { a, b, size }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(
            MONOCHROME_DEFAULT_A,
            MONOCHROME_DEFAULT_B,
            MONOCHROME_DEFAULT_SIZE,
        )
    }

    /// Serializes native declaration order as little-endian history bytes.
    #[must_use]
    pub fn to_bytes(self) -> [u8; MONOCHROME_V1_PARAMETER_BYTES] {
        encode_f32s([self.a, self.b, self.size])
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, MonochromeCodecError> {
        let values = decode_f32s::<3>(bytes, MONOCHROME_V1_PARAMETER_BYTES)?;
        Ok(Self::new(values[0], values[1], values[2]))
    }
}

/// Current native v2 `dt_iop_monochrome_params_t` in declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct MonochromeParametersV2 {
    pub a: f32,
    pub b: f32,
    pub size: f32,
    pub highlights: f32,
}

const _: () = assert!(size_of::<MonochromeParametersV2>() == MONOCHROME_V2_PARAMETER_BYTES);

impl MonochromeParametersV2 {
    #[must_use]
    pub const fn new(a: f32, b: f32, size: f32, highlights: f32) -> Self {
        Self {
            a,
            b,
            size,
            highlights,
        }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(
            MONOCHROME_DEFAULT_A,
            MONOCHROME_DEFAULT_B,
            MONOCHROME_DEFAULT_SIZE,
            MONOCHROME_DEFAULT_HIGHLIGHTS,
        )
    }

    /// Serializes all four native fields in their ABI order.
    #[must_use]
    pub fn to_bytes(self) -> [u8; MONOCHROME_V2_PARAMETER_BYTES] {
        encode_f32s([self.a, self.b, self.size, self.highlights])
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, MonochromeCodecError> {
        let values = decode_f32s::<4>(bytes, MONOCHROME_V2_PARAMETER_BYTES)?;
        Ok(Self::new(values[0], values[1], values[2], values[3]))
    }
}

fn encode_f32s<const FIELDS: usize, const BYTES: usize>(values: [f32; FIELDS]) -> [u8; BYTES] {
    debug_assert_eq!(FIELDS * size_of::<f32>(), BYTES);
    let mut bytes = [0_u8; BYTES];
    for (index, value) in values.into_iter().enumerate() {
        let start = index * size_of::<f32>();
        bytes[start..start + size_of::<f32>()].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn decode_f32s<const FIELDS: usize>(
    bytes: &[u8],
    expected: usize,
) -> Result<[f32; FIELDS], MonochromeCodecError> {
    if bytes.len() != expected {
        return Err(MonochromeCodecError::InvalidLength {
            expected,
            actual: bytes.len(),
        });
    }
    Ok(std::array::from_fn(|index| {
        let start = index * size_of::<f32>();
        f32::from_le_bytes(
            bytes[start..start + size_of::<f32>()]
                .try_into()
                .expect("payload length was checked"),
        )
    }))
}

/// Known history values and byte-preserved future values.
#[derive(Debug, Clone, PartialEq)]
pub enum MonochromeHistory {
    V1(MonochromeParametersV1),
    V2(MonochromeParametersV2),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl MonochromeHistory {
    /// Native v1 migrates directly to v2; unknown versions remain opaque.
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, MonochromeCodecError> {
        match version {
            1 => Ok(Self::V1(MonochromeParametersV1::from_bytes(bytes)?)),
            MONOCHROME_SCHEMA_VERSION => Ok(Self::V2(MonochromeParametersV2::from_bytes(bytes)?)),
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
            Self::V2(_) => MONOCHROME_SCHEMA_VERSION,
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

    /// Materializes a current v2 value, rejecting a future opaque value.
    pub const fn current(&self) -> Result<MonochromeParametersV2, MonochromeCodecError> {
        match self {
            Self::V1(parameters) => Ok(MonochromeParametersV2::new(
                parameters.a,
                parameters.b,
                parameters.size,
                MONOCHROME_DEFAULT_HIGHLIGHTS,
            )),
            Self::V2(parameters) => Ok(*parameters),
            Self::Opaque { version, .. } => Err(MonochromeCodecError::UnsupportedVersion(*version)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonochromeCodecError {
    InvalidLength { expected: usize, actual: usize },
    UnsupportedVersion(u16),
}

impl fmt::Display for MonochromeCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "monochrome payload has {actual} bytes; expected {expected}"
                )
            }
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "monochrome version {version} is opaque and unsupported"
                )
            }
        }
    }
}

impl std::error::Error for MonochromeCodecError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonochromeParameterError {
    NonFinite(&'static str),
}

impl fmt::Display for MonochromeParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "monochrome {name} is non-finite"),
        }
    }
}

impl std::error::Error for MonochromeParameterError {}

/// Finite committed data. Native UI ranges are not execution clamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MonochromeConfig {
    a: rusttable_processing::FiniteF32,
    b: rusttable_processing::FiniteF32,
    size: rusttable_processing::FiniteF32,
    highlights: rusttable_processing::FiniteF32,
}

impl TryFrom<MonochromeParametersV2> for MonochromeConfig {
    type Error = MonochromeParameterError;

    fn try_from(parameters: MonochromeParametersV2) -> Result<Self, Self::Error> {
        Ok(Self {
            a: finite_parameter("a", parameters.a)?,
            b: finite_parameter("b", parameters.b)?,
            size: finite_parameter("size", parameters.size)?,
            highlights: finite_parameter("highlights", parameters.highlights)?,
        })
    }
}

impl MonochromeConfig {
    pub fn new(
        a: f32,
        b: f32,
        size: f32,
        highlights: f32,
    ) -> Result<Self, MonochromeParameterError> {
        Self::try_from(MonochromeParametersV2::new(a, b, size, highlights))
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::try_from(MonochromeParametersV2::defaults()).expect("monochrome defaults are finite")
    }

    #[must_use]
    pub const fn parameters(self) -> MonochromeParametersV2 {
        MonochromeParametersV2::new(
            self.a.get(),
            self.b.get(),
            self.size.get(),
            self.highlights.get(),
        )
    }

    #[must_use]
    pub const fn a(self) -> f32 {
        self.a.get()
    }

    #[must_use]
    pub const fn b(self) -> f32 {
        self.b.get()
    }

    #[must_use]
    pub const fn size(self) -> f32 {
        self.size.get()
    }

    #[must_use]
    pub const fn highlights(self) -> f32 {
        self.highlights.get()
    }
}

fn finite_parameter(
    name: &'static str,
    value: f32,
) -> Result<rusttable_processing::FiniteF32, MonochromeParameterError> {
    rusttable_processing::FiniteF32::new(value)
        .map_err(|_| MonochromeParameterError::NonFinite(name))
}

/// Native four-channel Lab sample: L, a, b, and the alpha/spare channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonochromePixel {
    channels: [f32; 4],
}

impl MonochromePixel {
    #[must_use]
    pub const fn new(lightness: f32, a: f32, b: f32, alpha: f32) -> Self {
        Self {
            channels: [lightness, a, b, alpha],
        }
    }

    #[must_use]
    pub const fn from_channels(channels: [f32; 4]) -> Self {
        Self { channels }
    }

    #[must_use]
    pub const fn channels(self) -> [f32; 4] {
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
pub enum MonochromeChannel {
    Lightness,
    A,
    B,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonochromeExecutionError {
    DimensionsMismatch {
        expected: usize,
        actual: usize,
    },
    NonFiniteInput {
        pixel: usize,
        channel: MonochromeChannel,
    },
    NonFiniteOutput {
        pixel: usize,
    },
    AllocationFailed {
        required_bytes: usize,
    },
    SizeOverflow,
    Bilateral(BilateralError),
    Cancelled,
}

impl fmt::Display for MonochromeExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DimensionsMismatch { expected, actual } => {
                write!(
                    formatter,
                    "monochrome expected {expected} pixels, got {actual}"
                )
            }
            Self::NonFiniteInput { pixel, channel } => {
                write!(
                    formatter,
                    "monochrome input pixel {pixel} has non-finite {channel:?}"
                )
            }
            Self::NonFiniteOutput { pixel } => {
                write!(
                    formatter,
                    "monochrome produced non-finite lightness at pixel {pixel}"
                )
            }
            Self::AllocationFailed { required_bytes } => {
                write!(
                    formatter,
                    "monochrome allocation failed for {required_bytes} bytes"
                )
            }
            Self::SizeOverflow => formatter.write_str("monochrome execution size overflowed"),
            Self::Bilateral(error) => {
                write!(formatter, "monochrome bilateral stage failed: {error}")
            }
            Self::Cancelled => formatter.write_str("monochrome execution was cancelled"),
        }
    }
}

impl std::error::Error for MonochromeExecutionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonochromeCapabilityError {
    GpuUnavailable,
    GtkUnavailable,
    ProductionRoutingDeferred,
}

impl fmt::Display for MonochromeCapabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GpuUnavailable => formatter.write_str("monochrome GPU execution is unavailable"),
            Self::GtkUnavailable => formatter.write_str("monochrome GTK controls are unavailable"),
            Self::ProductionRoutingDeferred => {
                formatter.write_str("monochrome production routing is deferred")
            }
        }
    }
}

impl std::error::Error for MonochromeCapabilityError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonochromeCapabilities {
    pub cpu_supported: bool,
    pub gpu_supported: bool,
    pub gtk_supported: bool,
    pub masks_consumed: bool,
    pub outer_blending_deferred: bool,
    pub production_routing_deferred: bool,
    pub alpha_preserved: bool,
}

impl MonochromeCapabilities {
    #[must_use]
    pub const fn bounded_cpu_leaf() -> Self {
        Self {
            cpu_supported: true,
            gpu_supported: MONOCHROME_GPU_EXECUTABLE,
            gtk_supported: false,
            masks_consumed: false,
            outer_blending_deferred: true,
            production_routing_deferred: true,
            alpha_preserved: true,
        }
    }

    pub const fn require_gpu(self) -> Result<(), MonochromeCapabilityError> {
        if self.gpu_supported {
            Ok(())
        } else {
            Err(MonochromeCapabilityError::GpuUnavailable)
        }
    }

    pub const fn require_gtk(self) -> Result<(), MonochromeCapabilityError> {
        if self.gtk_supported {
            Ok(())
        } else {
            Err(MonochromeCapabilityError::GtkUnavailable)
        }
    }

    pub const fn require_production_routing(self) -> Result<(), MonochromeCapabilityError> {
        if self.production_routing_deferred {
            Err(MonochromeCapabilityError::ProductionRoutingDeferred)
        } else {
            Ok(())
        }
    }
}

#[must_use]
pub const fn capabilities() -> MonochromeCapabilities {
    MonochromeCapabilities::bounded_cpu_leaf()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonochromeTiling {
    pub factor: f32,
    pub factor_cl: f32,
    pub maxbuf: f32,
    pub maxbuf_cl: f32,
    pub overhead: usize,
    pub overlap: u32,
    pub align: usize,
}

/// Immutable CPU plan. `scale` is native `fmaxf(piece->iscale / roi_in->scale, 1.f)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonochromePlan {
    config: MonochromeConfig,
    dimensions: RasterDimensions,
    sigma_s: f32,
    tiling_sigma_s: f32,
}

impl MonochromePlan {
    pub fn new(
        config: MonochromeConfig,
        dimensions: RasterDimensions,
    ) -> Result<Self, MonochromeExecutionError> {
        Self::new_with_scale(config, dimensions, 1.0, 1.0)
    }

    pub fn new_with_scale(
        config: MonochromeConfig,
        dimensions: RasterDimensions,
        piece_iscale: f32,
        roi_in_scale: f32,
    ) -> Result<Self, MonochromeExecutionError> {
        let ratio = piece_iscale / roi_in_scale;
        // This comparison has the same useful fmaxf behavior for finite values,
        // infinities, and NaN: a non-greater ratio resolves to the lower bound.
        let scale = if ratio > 1.0 { ratio } else { 1.0 };
        let sigma_s = SIGMA_S_BASE / scale;
        let tiling_sigma_s = SIGMA_S_BASE / ratio;
        BilateralGeometry::new(
            usize::try_from(dimensions.width()).expect("u32 dimensions fit usize"),
            usize::try_from(dimensions.height()).expect("u32 dimensions fit usize"),
            sigma_s,
            SIGMA_R,
        )
        .map_err(map_bilateral_error)?;
        Ok(Self {
            config,
            dimensions,
            sigma_s,
            tiling_sigma_s,
        })
    }

    #[must_use]
    pub const fn config(self) -> MonochromeConfig {
        self.config
    }

    #[must_use]
    pub const fn dimensions(self) -> RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn sigma_s(self) -> f32 {
        self.sigma_s
    }

    /// Mirrors `tiling_callback`; `cpu_threads` is the source helper's thread count.
    pub fn tiling(
        &self,
        channels: usize,
        cpu_threads: usize,
    ) -> Result<MonochromeTiling, MonochromeExecutionError> {
        if channels == 0 || cpu_threads == 0 {
            return Err(MonochromeExecutionError::SizeOverflow);
        }
        let width = usize::try_from(self.dimensions.width()).expect("u32 dimensions fit usize");
        let height = usize::try_from(self.dimensions.height()).expect("u32 dimensions fit usize");
        let geometry = BilateralGeometry::new(width, height, self.tiling_sigma_s, SIGMA_R)
            .map_err(map_bilateral_error)?;
        let [size_x, _, size_z] = geometry.grid_dimensions();
        let grid_bytes = geometry
            .grid_values()
            .checked_mul(size_of::<f32>())
            .ok_or(MonochromeExecutionError::SizeOverflow)?;
        let per_thread_scratch = size_x
            .checked_mul(size_z)
            .and_then(|value| value.checked_mul(3))
            .and_then(|value| value.checked_mul(size_of::<f32>()))
            .ok_or(MonochromeExecutionError::SizeOverflow)?;
        let bilat_mem = grid_bytes
            .checked_add(
                per_thread_scratch
                    .checked_mul(cpu_threads)
                    .ok_or(MonochromeExecutionError::SizeOverflow)?,
            )
            .ok_or(MonochromeExecutionError::SizeOverflow)?;
        let basebuffer = size_of::<f32>()
            .checked_mul(channels)
            .and_then(|value| value.checked_mul(width))
            .and_then(|value| value.checked_mul(height))
            .ok_or(MonochromeExecutionError::SizeOverflow)?;
        if basebuffer == 0 {
            return Err(MonochromeExecutionError::SizeOverflow);
        }
        let factor_extra = bilat_mem as f32 / basebuffer as f32;
        let maxbuf = (bilat_mem as f32 / basebuffer as f32).max(1.0);
        Ok(MonochromeTiling {
            factor: 2.0 + factor_extra,
            factor_cl: 3.0 + factor_extra,
            maxbuf,
            maxbuf_cl: maxbuf,
            overhead: 0,
            overlap: (4.0 * self.tiling_sigma_s).ceil() as u32,
            align: 1,
        })
    }

    /// Executes the complete CPU two-pass operation without publishing partial output.
    pub fn execute(
        &self,
        input: &[MonochromePixel],
    ) -> Result<Vec<MonochromePixel>, MonochromeExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    /// Cancellation is polled at raster-row and bilateral-grid boundaries.
    pub fn execute_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[MonochromePixel],
        mut cancelled: F,
    ) -> Result<Vec<MonochromePixel>, MonochromeExecutionError> {
        let expected = usize::try_from(self.dimensions.pixel_count())
            .map_err(|_| MonochromeExecutionError::SizeOverflow)?;
        if input.len() != expected {
            return Err(MonochromeExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        if cancelled() {
            return Err(MonochromeExecutionError::Cancelled);
        }
        let width = usize::try_from(self.dimensions.width()).expect("u32 dimensions fit usize");
        let height = usize::try_from(self.dimensions.height()).expect("u32 dimensions fit usize");
        let mut filtered = Vec::new();
        filtered.try_reserve_exact(expected).map_err(|_| {
            MonochromeExecutionError::AllocationFailed {
                required_bytes: expected.saturating_mul(size_of::<[f32; 4]>()),
            }
        })?;

        let sigma2 = 2.0_f32
            * (self.config.size() * COLOR_FILTER_SCALE)
            * (self.config.size() * COLOR_FILTER_SCALE);
        for (index, pixel) in input.iter().copied().enumerate() {
            if index % width == 0 && cancelled() {
                return Err(MonochromeExecutionError::Cancelled);
            }
            validate_input(pixel, index)?;
            let filter = color_filter(
                pixel.a(),
                pixel.b(),
                self.config.a(),
                self.config.b(),
                sigma2,
            );
            filtered.push([LAB_LIGHTNESS_MAXIMUM * filter, 0.0, 0.0, pixel.alpha()]);
        }

        let mut bilateral = BilateralGrid::new(width, height, self.sigma_s, SIGMA_R)
            .map_err(map_bilateral_error)?;
        bilateral
            .splat_with_cancel(&filtered, &mut cancelled)
            .map_err(map_bilateral_error)?;
        bilateral
            .blur_with_cancel(&mut cancelled)
            .map_err(map_bilateral_error)?;
        bilateral
            .slice_to_output_in_place_with_cancel(&mut filtered, BILATERAL_DETAIL, &mut cancelled)
            .map_err(map_bilateral_error)?;

        if cancelled() {
            return Err(MonochromeExecutionError::Cancelled);
        }
        let mut output = Vec::new();
        output.try_reserve_exact(expected).map_err(|_| {
            MonochromeExecutionError::AllocationFailed {
                required_bytes: expected.saturating_mul(size_of::<MonochromePixel>()),
            }
        })?;
        let highlights = self.config.highlights();
        for index in 0..expected {
            if index % width == 0 && cancelled() {
                return Err(MonochromeExecutionError::Cancelled);
            }
            let source = input[index];
            let tt = envelope(source.lightness());
            let t = tt + (1.0 - tt) * (1.0 - highlights);
            let lightness = (1.0 - t) * source.lightness()
                + t * filtered[index][0] * (1.0 / LAB_LIGHTNESS_MAXIMUM) * source.lightness();
            if !lightness.is_finite() {
                return Err(MonochromeExecutionError::NonFiniteOutput { pixel: index });
            }
            // Native OpenCL and the Lab operation contract preserve alpha. The
            // retained CPU first pass leaves that fourth slot to the caller's
            // pre-existing output, so this leaf makes the preservation explicit.
            output.push(MonochromePixel::new(lightness, 0.0, 0.0, source.alpha()));
        }
        Ok(output)
    }
}

const fn validate_input(
    pixel: MonochromePixel,
    index: usize,
) -> Result<(), MonochromeExecutionError> {
    if !pixel.lightness().is_finite() {
        return Err(MonochromeExecutionError::NonFiniteInput {
            pixel: index,
            channel: MonochromeChannel::Lightness,
        });
    }
    if !pixel.a().is_finite() {
        return Err(MonochromeExecutionError::NonFiniteInput {
            pixel: index,
            channel: MonochromeChannel::A,
        });
    }
    if !pixel.b().is_finite() {
        return Err(MonochromeExecutionError::NonFiniteInput {
            pixel: index,
            channel: MonochromeChannel::B,
        });
    }
    Ok(())
}

fn color_filter(ai: f32, bi: f32, a: f32, b: f32, double_size: f32) -> f32 {
    let a_delta = (ai - a) * (ai - a);
    let b_delta = (bi - b) * (bi - b);
    fast_expf(-darktable_clamps(
        (a_delta + b_delta) / double_size,
        0.0,
        1.0,
    ))
}

/// Native envelope, including the source's branch and multiplication order.
#[must_use]
pub fn envelope(lightness: f32) -> f32 {
    let x = darktable_clamps(lightness / LAB_LIGHTNESS_MAXIMUM, 0.0, 1.0);
    let beta = 0.6_f32;
    if x < beta {
        let tmp = x / beta - 1.0;
        1.0 - tmp * tmp
    } else {
        let tmp1 = (1.0 - x) / (1.0 - beta);
        let tmp2 = tmp1 * tmp1;
        let tmp3 = tmp2 * tmp1;
        3.0 * tmp2 - 2.0 * tmp3
    }
}

/// Native `dt_fast_expf`, shared with `src/common/math.h` and `basic.cl`.
#[must_use]
pub fn fast_expf(x: f32) -> f32 {
    const I1: u32 = 0x3f80_0000;
    const I2: u32 = 0x402d_f854;
    let k0 = (I1 as f32 + x * (I2 - I1) as f32) as i32;
    f32::from_bits(if k0 > 0 { k0 as u32 } else { 0 })
}

fn darktable_clamps(value: f32, lower: f32, upper: f32) -> f32 {
    if value > lower {
        if value < upper { value } else { upper }
    } else {
        lower
    }
}

const fn map_bilateral_error(error: BilateralError) -> MonochromeExecutionError {
    match error {
        BilateralError::BufferShape { expected, actual } => {
            MonochromeExecutionError::DimensionsMismatch { expected, actual }
        }
        BilateralError::NonFiniteLightness { pixel } => MonochromeExecutionError::NonFiniteInput {
            pixel,
            channel: MonochromeChannel::Lightness,
        },
        BilateralError::NonFiniteOutput { pixel } => {
            MonochromeExecutionError::NonFiniteOutput { pixel }
        }
        BilateralError::AllocationFailed { required_bytes } => {
            MonochromeExecutionError::AllocationFailed { required_bytes }
        }
        BilateralError::SizeOverflow => MonochromeExecutionError::SizeOverflow,
        BilateralError::Cancelled => MonochromeExecutionError::Cancelled,
        other => MonochromeExecutionError::Bilateral(other),
    }
}
