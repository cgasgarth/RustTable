//! Bounded RGB Levels CPU leaf ported from `src/iop/rgblevels.c`.
//!
//! The leaf owns the native v1 parameter ABI, opaque future history values,
//! `commit_params()`'s linked-channel expansion and LUT construction, the
//! source-shaped `dt_rgb_norm()` preservation modes, and the CPU pixel loop.
//! Registry, typed history import, evaluator, and pixelpipe CPU routing are
//! integrated. The retained OpenCL kernel, auto-levels GUI state, configured
//! profile transforms, masks, outer blending, presets, and GTK integration
//! remain explicitly deferred rather than approximated.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::float_cmp,
    clippy::if_not_else,
    clippy::manual_clamp,
    clippy::manual_range_contains,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::similar_names,
    clippy::struct_excessive_bools,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    dead_code,
    reason = "native ABI and scalar RGB expressions retain source-shaped f32 boundaries"
)]

use std::fmt;
use std::hash::{Hash, Hasher};
use std::mem::{size_of, size_of_val};

mod descriptor;
pub mod source_map;

pub use descriptor::rgblevels_descriptor;

pub const RGBLEVELS_COMPATIBILITY_ID: &str = "rgblevels";
pub const RGBLEVELS_RUST_ID: &str = "rusttable.rgblevels";
pub const RGBLEVELS_SCHEMA_VERSION: u16 = 1;
pub const RGBLEVELS_PARAMETER_BYTES: usize = 44;
pub const RGBLEVELS_LUT_ENTRIES: usize = 0x1_0000;
pub const RGBLEVELS_GPU_PROGRAM: u32 = 29;
pub const RGBLEVELS_GPU_KERNEL: &str = "rgblevels";
pub const RGBLEVELS_GPU_EXECUTABLE: bool = false;
pub const RGBLEVELS_DEFAULT_ENABLED: bool = false;
pub const RGBLEVELS_DEFAULT_VISIBLE: bool = true;
pub const RGBLEVELS_DEFAULT_ORDER: u32 = 126;
pub const RGBLEVELS_DEFAULT_GROUPS: [&str; 2] = ["tone", "grading"];
pub const RGBLEVELS_DEFAULT_COLORSPACE: &str = "RGB";
pub const RGBLEVELS_SUPPORTS_BLENDING: bool = true;
pub const RGBLEVELS_SUPPORTS_MASKS: bool = false;
pub const RGBLEVELS_TILING_SUPPORTED: bool = true;
pub const RGBLEVELS_MIGRATION_EDGES: &[(u16, u16)] = &[];

const RGBLEVELS_MIN: f32 = 0.0;
const RGBLEVELS_MID: f32 = 0.5;
const RGBLEVELS_MAX: f32 = 1.0;
const CAMERA_LUMINANCE_RED: f32 = 0.222_504_5;
const CAMERA_LUMINANCE_GREEN: f32 = 0.716_878_6;
const CAMERA_LUMINANCE_BLUE: f32 = 0.060_616_9;

/// Native `dt_iop_rgblevels_autoscale_t` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum RgbLevelsAutoscale {
    LinkedChannels = 0,
    IndependentChannels = 1,
}

impl TryFrom<i32> for RgbLevelsAutoscale {
    type Error = RgbLevelsCodecError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::LinkedChannels),
            1 => Ok(Self::IndependentChannels),
            other => Err(Self::Error::InvalidAutoscale(other)),
        }
    }
}

impl From<RgbLevelsAutoscale> for i32 {
    fn from(value: RgbLevelsAutoscale) -> Self {
        value as Self
    }
}

/// Native `dt_iop_rgb_norms_t` values from `src/common/rgb_norms.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum RgbLevelsPreserveColors {
    None = 0,
    Luminance = 1,
    Max = 2,
    Average = 3,
    Sum = 4,
    Norm = 5,
    Power = 6,
}

impl TryFrom<i32> for RgbLevelsPreserveColors {
    type Error = RgbLevelsCodecError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::Luminance),
            2 => Ok(Self::Max),
            3 => Ok(Self::Average),
            4 => Ok(Self::Sum),
            5 => Ok(Self::Norm),
            6 => Ok(Self::Power),
            other => Err(Self::Error::InvalidPreserveColors(other)),
        }
    }
}

impl From<RgbLevelsPreserveColors> for i32 {
    fn from(value: RgbLevelsPreserveColors) -> Self {
        value as Self
    }
}

/// Current native declaration-order parameter payload.
///
/// The C ABI is two four-byte enum fields followed by nine contiguous f32
/// values: `autoscale`, `preserve_colors`, and `levels[3][3]`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct RgbLevelsParametersV1 {
    pub autoscale: RgbLevelsAutoscale,
    pub preserve_colors: RgbLevelsPreserveColors,
    pub levels: [[f32; 3]; 3],
}

const _: () = assert!(size_of::<RgbLevelsParametersV1>() == RGBLEVELS_PARAMETER_BYTES);

/// Integer-only ABI witness used to make enum representation explicit without
/// ever constructing an invalid Rust enum from untrusted history bytes.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct RgbLevelsAbiLayout {
    autoscale: i32,
    preserve_colors: i32,
    levels: [[f32; 3]; 3],
}

const _: () = assert!(size_of::<RgbLevelsAbiLayout>() == RGBLEVELS_PARAMETER_BYTES);

impl RgbLevelsParametersV1 {
    #[must_use]
    pub const fn new(
        autoscale: RgbLevelsAutoscale,
        preserve_colors: RgbLevelsPreserveColors,
        levels: [[f32; 3]; 3],
    ) -> Self {
        Self {
            autoscale,
            preserve_colors,
            levels,
        }
    }

    /// Exact native `init()` defaults after generated defaults are applied.
    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(
            RgbLevelsAutoscale::LinkedChannels,
            RgbLevelsPreserveColors::Luminance,
            [
                [RGBLEVELS_MIN, RGBLEVELS_MID, RGBLEVELS_MAX],
                [RGBLEVELS_MIN, RGBLEVELS_MID, RGBLEVELS_MAX],
                [RGBLEVELS_MIN, RGBLEVELS_MID, RGBLEVELS_MAX],
            ],
        )
    }

    /// Encodes the native little-endian history payload without relying on
    /// host enum layout or allowing invalid enum discriminants.
    #[must_use]
    pub fn to_bytes(self) -> [u8; RGBLEVELS_PARAMETER_BYTES] {
        let mut bytes = [0_u8; RGBLEVELS_PARAMETER_BYTES];
        write_i32(&mut bytes, 0, self.autoscale.into());
        write_i32(&mut bytes, 4, self.preserve_colors.into());
        for channel in 0..3 {
            for point in 0..3 {
                write_f32(
                    &mut bytes,
                    8 + (channel * 3 + point) * size_of::<f32>(),
                    self.levels[channel][point],
                );
            }
        }
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RgbLevelsCodecError> {
        if bytes.len() != RGBLEVELS_PARAMETER_BYTES {
            return Err(RgbLevelsCodecError::InvalidLength {
                expected: RGBLEVELS_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let levels = std::array::from_fn(|channel| {
            std::array::from_fn(|point| {
                read_f32(bytes, 8 + (channel * 3 + point) * size_of::<f32>())
            })
        });
        Ok(Self::new(
            RgbLevelsAutoscale::try_from(read_i32(bytes, 0))?,
            RgbLevelsPreserveColors::try_from(read_i32(bytes, 4))?,
            levels,
        ))
    }
}

/// Codec failures for known native payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgbLevelsCodecError {
    InvalidLength { expected: usize, actual: usize },
    InvalidAutoscale(i32),
    InvalidPreserveColors(i32),
    UnsupportedVersion(u16),
}

impl fmt::Display for RgbLevelsCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => write!(
                formatter,
                "RGB Levels payload has {actual} bytes; expected {expected}"
            ),
            Self::InvalidAutoscale(value) => {
                write!(formatter, "RGB Levels autoscale value {value} is unknown")
            }
            Self::InvalidPreserveColors(value) => write!(
                formatter,
                "RGB Levels preserve-colors value {value} is unknown"
            ),
            Self::UnsupportedVersion(version) => write!(
                formatter,
                "RGB Levels version {version} is opaque and unsupported"
            ),
        }
    }
}

impl std::error::Error for RgbLevelsCodecError {}

/// Known native history and byte-preserved future values.
#[derive(Debug, Clone, PartialEq)]
pub enum RgbLevelsHistory {
    V1(RgbLevelsParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl RgbLevelsHistory {
    /// Native v1 is the only known version; every other version remains opaque.
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, RgbLevelsCodecError> {
        match version {
            RGBLEVELS_SCHEMA_VERSION => Ok(Self::V1(RgbLevelsParametersV1::from_bytes(bytes)?)),
            _ => Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            }),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => RGBLEVELS_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(parameters) => parameters.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    /// Materializes v1 and rejects a future opaque value.
    pub fn current(&self) -> Result<RgbLevelsParametersV1, RgbLevelsCodecError> {
        match self {
            Self::V1(parameters) => Ok(*parameters),
            Self::Opaque { version, .. } => Err(RgbLevelsCodecError::UnsupportedVersion(*version)),
        }
    }
}

/// Strict parameter validation at the Rust execution boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RgbLevelsParameterError {
    NonFiniteLevel {
        channel: usize,
        point: usize,
    },
    NonIncreasingRange {
        channel: usize,
        minimum: f32,
        maximum: f32,
    },
}

impl fmt::Display for RgbLevelsParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteLevel { channel, point } => write!(
                formatter,
                "RGB Levels channel {channel} point {point} is non-finite"
            ),
            Self::NonIncreasingRange {
                channel,
                minimum,
                maximum,
            } => write!(
                formatter,
                "RGB Levels channel {channel} range {minimum}..{maximum} is not increasing"
            ),
        }
    }
}

impl std::error::Error for RgbLevelsParameterError {}

/// Validated current RGB Levels settings. Native GUI bounds are not execution
/// clamps; finite values outside [0, 1] remain representable for extrapolation.
#[derive(Debug, Clone, Copy)]
pub struct RgbLevelsConfig {
    parameters: RgbLevelsParametersV1,
}

impl PartialEq for RgbLevelsConfig {
    fn eq(&self, other: &Self) -> bool {
        self.parameters.to_bytes() == other.parameters.to_bytes()
    }
}

impl Eq for RgbLevelsConfig {}

impl Hash for RgbLevelsConfig {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.parameters.to_bytes().hash(state);
    }
}

impl RgbLevelsConfig {
    pub fn new(parameters: RgbLevelsParametersV1) -> Result<Self, RgbLevelsParameterError> {
        for (channel, levels) in parameters.levels.iter().enumerate() {
            for (point, value) in levels.iter().copied().enumerate() {
                if !value.is_finite() {
                    return Err(RgbLevelsParameterError::NonFiniteLevel { channel, point });
                }
            }
            if levels[0] >= levels[2] {
                return Err(RgbLevelsParameterError::NonIncreasingRange {
                    channel,
                    minimum: levels[0],
                    maximum: levels[2],
                });
            }
        }
        Ok(Self { parameters })
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::new(RgbLevelsParametersV1::defaults()).expect("native RGB Levels defaults are valid")
    }

    #[must_use]
    pub const fn parameters(self) -> RgbLevelsParametersV1 {
        self.parameters
    }
}

/// Explicit working-profile evidence used by native RGB luminance mode.
///
/// A missing profile intentionally follows `dt_camera_rgb_luminance`. When
/// evidence is present, this reproduces `dt_ioppr_get_rgb_matrix_luminance`:
/// optional TRC application followed by row 1 of `matrix_in`.
#[derive(Debug, Clone, PartialEq)]
pub struct RgbLevelsProfileEvidence {
    matrix_in: [[f32; 3]; 3],
    lut_in: [Vec<f32>; 3],
    unbounded_coeffs_in: [[f32; 3]; 3],
    lut_size: usize,
    nonlinearlut: bool,
}

impl RgbLevelsProfileEvidence {
    #[must_use]
    pub fn new_linear(matrix_in: [[f32; 3]; 3]) -> Self {
        Self {
            matrix_in,
            lut_in: std::array::from_fn(|_| vec![0.0, 1.0]),
            unbounded_coeffs_in: [[-1.0, 0.0, 1.0]; 3],
            lut_size: 2,
            nonlinearlut: false,
        }
    }

    pub fn new_with_trc(
        matrix_in: [[f32; 3]; 3],
        lut_in: [Vec<f32>; 3],
        unbounded_coeffs_in: [[f32; 3]; 3],
        lut_size: usize,
        nonlinearlut: bool,
    ) -> Result<Self, RgbLevelsProfileError> {
        let evidence = Self {
            matrix_in,
            lut_in,
            unbounded_coeffs_in,
            lut_size,
            nonlinearlut,
        };
        evidence.validate()?;
        Ok(evidence)
    }

    fn validate(&self) -> Result<(), RgbLevelsProfileError> {
        if self
            .matrix_in
            .iter()
            .flatten()
            .any(|value| !value.is_finite())
        {
            return Err(RgbLevelsProfileError::NonFiniteMatrix);
        }
        if self.nonlinearlut {
            if self.lut_size < 2
                || self.lut_in.iter().any(|lut| {
                    lut.len() != self.lut_size || lut.iter().any(|value| !value.is_finite())
                })
            {
                return Err(RgbLevelsProfileError::InvalidLut);
            }
            if self
                .unbounded_coeffs_in
                .iter()
                .flatten()
                .any(|value| !value.is_finite())
            {
                return Err(RgbLevelsProfileError::NonFiniteCoefficients);
            }
        }
        Ok(())
    }

    /// Source `dt_ioppr_get_rgb_matrix_luminance` with f32 operation order.
    #[must_use]
    pub fn luminance(&self, rgb: [f32; 3]) -> f32 {
        let rgb = if self.nonlinearlut {
            [
                self.apply_trc(rgb[0], 0),
                self.apply_trc(rgb[1], 1),
                self.apply_trc(rgb[2], 2),
            ]
        } else {
            rgb
        };
        self.matrix_in[1][0] * rgb[0]
            + self.matrix_in[1][1] * rgb[1]
            + self.matrix_in[1][2] * rgb[2]
    }

    fn apply_trc(&self, value: f32, channel: usize) -> f32 {
        let lut = &self.lut_in[channel];
        if lut[0] >= 0.0 {
            if value < 1.0 {
                extrapolate_lut(lut, value, self.lut_size)
            } else {
                eval_exp(self.unbounded_coeffs_in[channel], value)
            }
        } else {
            value
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgbLevelsProfileError {
    NonFiniteMatrix,
    InvalidLut,
    NonFiniteCoefficients,
}

impl fmt::Display for RgbLevelsProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFiniteMatrix => "RGB Levels profile matrix is non-finite",
            Self::InvalidLut => "RGB Levels profile LUT size or samples are invalid",
            Self::NonFiniteCoefficients => "RGB Levels profile coefficients are non-finite",
        })
    }
}

impl std::error::Error for RgbLevelsProfileError {}

/// Four-float native full-color pixel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RgbLevelsPixel {
    channels: [f32; 4],
}

impl RgbLevelsPixel {
    #[must_use]
    pub const fn new(red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        Self {
            channels: [red, green, blue, alpha],
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
}

/// Native process result with the required-four-channel format boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct RgbLevelsExecution {
    pub pixels: Vec<RgbLevelsPixel>,
    pub input_format_problem: bool,
}

/// Immutable compiled `commit_params()` state and CPU plan.
#[derive(Debug, Clone, PartialEq)]
pub struct RgbLevelsPlan {
    config: RgbLevelsConfig,
    effective_levels: [[f32; 3]; 3],
    multipliers: [f32; 3],
    inv_gamma: [f32; 3],
    lut: [Vec<f32>; 3],
    profile: Option<RgbLevelsProfileEvidence>,
}

impl RgbLevelsPlan {
    pub fn new(
        config: RgbLevelsConfig,
        profile: Option<RgbLevelsProfileEvidence>,
    ) -> Result<Self, RgbLevelsPlanError> {
        let parameters = config.parameters();
        let effective_levels = match parameters.autoscale {
            RgbLevelsAutoscale::LinkedChannels => [parameters.levels[0]; 3],
            RgbLevelsAutoscale::IndependentChannels => parameters.levels,
        };
        let mut multipliers = [0.0_f32; 3];
        let mut inv_gamma = [0.0_f32; 3];
        let mut lut = [Vec::new(), Vec::new(), Vec::new()];
        for channel in 0..3 {
            let levels = effective_levels[channel];
            let multiplier = 1.0_f32 / (levels[2] - levels[0]);
            if !multiplier.is_finite() {
                return Err(RgbLevelsPlanError::NonFiniteDerived {
                    channel,
                    field: "multiplier",
                });
            }
            multipliers[channel] = multiplier;
            let delta = (levels[2] - levels[0]) / 2.0;
            let mid = levels[0] + delta;
            let tmp = (levels[1] - mid) / delta;
            // `_compute_lut` calls double-precision `pow`, not `powf`, then
            // stores the result in a float. Keep that promotion explicit.
            let gamma = (10.0_f64).powf(f64::from(tmp)) as f32;
            if !gamma.is_finite() {
                return Err(RgbLevelsPlanError::NonFiniteDerived {
                    channel,
                    field: "inv_gamma",
                });
            }
            inv_gamma[channel] = gamma;

            let mut table = Vec::new();
            table
                .try_reserve_exact(RGBLEVELS_LUT_ENTRIES)
                .map_err(|_| RgbLevelsPlanError::AllocationFailed {
                    required_bytes: RGBLEVELS_LUT_ENTRIES * size_of::<f32>(),
                })?;
            for index in 0..RGBLEVELS_LUT_ENTRIES {
                let percentage = index as f32 / RGBLEVELS_LUT_ENTRIES as f32;
                // `_compute_lut` also calls double-precision `pow` on promoted
                // float operands before narrowing to the LUT's f32 element.
                let value = (f64::from(percentage)).powf(f64::from(gamma)) as f32;
                if !value.is_finite() {
                    return Err(RgbLevelsPlanError::NonFiniteDerived {
                        channel,
                        field: "lut",
                    });
                }
                table.push(value);
            }
            lut[channel] = table;
        }
        Ok(Self {
            config,
            effective_levels,
            multipliers,
            inv_gamma,
            lut,
            profile,
        })
    }

    #[must_use]
    pub const fn config(&self) -> RgbLevelsConfig {
        self.config
    }

    #[must_use]
    pub const fn effective_levels(&self) -> [[f32; 3]; 3] {
        self.effective_levels
    }

    #[must_use]
    pub const fn multipliers(&self) -> [f32; 3] {
        self.multipliers
    }

    #[must_use]
    pub const fn inv_gamma(&self) -> [f32; 3] {
        self.inv_gamma
    }

    #[must_use]
    pub fn lut(&self, channel: usize) -> Option<&[f32]> {
        self.lut.get(channel).map(Vec::as_slice)
    }

    #[must_use]
    pub const fn profile(&self) -> Option<&RgbLevelsProfileEvidence> {
        self.profile.as_ref()
    }

    /// Executes the native four-channel CPU path without partial publication.
    pub fn execute(
        &self,
        input: &[RgbLevelsPixel],
    ) -> Result<Vec<RgbLevelsPixel>, RgbLevelsExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    pub fn execute_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[RgbLevelsPixel],
        mut cancelled: F,
    ) -> Result<Vec<RgbLevelsPixel>, RgbLevelsExecutionError> {
        if cancelled() {
            return Err(RgbLevelsExecutionError::Cancelled);
        }
        let required_bytes = size_of_val(input);
        let mut output = Vec::new();
        output
            .try_reserve_exact(input.len())
            .map_err(|_| RgbLevelsExecutionError::AllocationFailed { required_bytes })?;
        for (pixel_index, pixel) in input.iter().copied().enumerate() {
            if cancelled() {
                return Err(RgbLevelsExecutionError::Cancelled);
            }
            output.push(self.evaluate_pixel(pixel, pixel_index)?);
        }
        Ok(output)
    }

    /// Models `dt_iop_have_required_input_format`: copy-through occurs before
    /// processing and before the Rust cancellation poll when four channels are
    /// unavailable.
    pub fn execute_required_format_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[RgbLevelsPixel],
        required_format_available: bool,
        mut cancelled: F,
    ) -> Result<RgbLevelsExecution, RgbLevelsExecutionError> {
        if !required_format_available {
            return Ok(RgbLevelsExecution {
                pixels: copy_pixels_fallibly(input)?,
                input_format_problem: true,
            });
        }
        if cancelled() {
            return Err(RgbLevelsExecutionError::Cancelled);
        }
        Ok(RgbLevelsExecution {
            pixels: self.execute_with_cancel(input, cancelled)?,
            input_format_problem: false,
        })
    }

    fn evaluate_pixel(
        &self,
        pixel: RgbLevelsPixel,
        pixel_index: usize,
    ) -> Result<RgbLevelsPixel, RgbLevelsExecutionError> {
        let input = pixel.channels;
        for (channel, value) in input[..3].iter().copied().enumerate() {
            if !value.is_finite() {
                return Err(RgbLevelsExecutionError::NonFiniteInput {
                    pixel: pixel_index,
                    channel,
                });
            }
        }

        let parameters = self.config.parameters();
        let mut output = [0.0_f32; 4];
        if parameters.autoscale == RgbLevelsAutoscale::IndependentChannels
            || parameters.preserve_colors == RgbLevelsPreserveColors::None
        {
            for channel in 0..3 {
                output[channel] = self.map_channel(input[channel], channel);
            }
            // The native CPU independent branch writes only RGB. The bounded
            // Rust destination is zero-initialized to make that unwritten
            // alpha lane deterministic instead of exposing uninitialized data.
        } else {
            let luminance = rgb_norm(
                [input[0], input[1], input[2]],
                parameters.preserve_colors,
                self.profile.as_ref(),
            );
            let minimum = self.effective_levels[0][0];
            if luminance > minimum {
                let percentage = (luminance - minimum) * self.multipliers[0];
                let curve_luminance = if luminance >= self.effective_levels[0][2] {
                    percentage.powf(self.inv_gamma[0])
                } else {
                    self.lut[0][lut_index(percentage)]
                };
                let ratio = curve_luminance / luminance;
                // Native `for_each_channel` may execute a fourth SIMD-padding
                // iteration, but `copy_pixel_nontemporal` defines only RGB as
                // semantic unless alpha is explicitly set afterward. Keep the
                // bounded fourth lane deterministic instead of treating that
                // optimizer-only spare-lane write as an alpha transform.
                for channel in 0..3 {
                    output[channel] = ratio * input[channel];
                }
            }
        }

        if output.iter().copied().all(f32::is_finite) {
            Ok(RgbLevelsPixel::from_channels(output))
        } else {
            Err(RgbLevelsExecutionError::NonFiniteOutput { pixel: pixel_index })
        }
    }

    fn map_channel(&self, input: f32, channel: usize) -> f32 {
        let levels = self.effective_levels[channel];
        if input <= levels[0] {
            0.0
        } else if input >= levels[2] {
            let percentage = (input - levels[0]) * self.multipliers[channel];
            percentage.powf(self.inv_gamma[channel])
        } else {
            let percentage = (input - levels[0]) * self.multipliers[channel];
            self.lut[channel][lut_index(percentage)]
        }
    }
}

fn copy_pixels_fallibly(
    input: &[RgbLevelsPixel],
) -> Result<Vec<RgbLevelsPixel>, RgbLevelsExecutionError> {
    let required_bytes = size_of_val(input);
    let mut output = Vec::new();
    reserve_pixels(&mut output, input.len(), required_bytes)?;
    output.extend_from_slice(input);
    Ok(output)
}

fn reserve_pixels(
    output: &mut Vec<RgbLevelsPixel>,
    additional: usize,
    required_bytes: usize,
) -> Result<(), RgbLevelsExecutionError> {
    output
        .try_reserve_exact(additional)
        .map_err(|_| RgbLevelsExecutionError::AllocationFailed { required_bytes })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgbLevelsPlanError {
    NonFiniteDerived { channel: usize, field: &'static str },
    AllocationFailed { required_bytes: usize },
}

impl fmt::Display for RgbLevelsPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteDerived { channel, field } => {
                write!(
                    formatter,
                    "RGB Levels channel {channel} derived {field} is non-finite"
                )
            }
            Self::AllocationFailed { required_bytes } => {
                write!(
                    formatter,
                    "RGB Levels allocation failed for {required_bytes} bytes"
                )
            }
        }
    }
}

impl std::error::Error for RgbLevelsPlanError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgbLevelsExecutionError {
    Cancelled,
    AllocationFailed { required_bytes: usize },
    NonFiniteInput { pixel: usize, channel: usize },
    NonFiniteOutput { pixel: usize },
}

impl fmt::Display for RgbLevelsExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("RGB Levels execution was cancelled"),
            Self::AllocationFailed { required_bytes } => write!(
                formatter,
                "RGB Levels allocation failed for {required_bytes} bytes"
            ),
            Self::NonFiniteInput { pixel, channel } => write!(
                formatter,
                "RGB Levels input pixel {pixel} channel {channel} is non-finite"
            ),
            Self::NonFiniteOutput { pixel } => {
                write!(formatter, "RGB Levels output pixel {pixel} is non-finite")
            }
        }
    }
}

impl std::error::Error for RgbLevelsExecutionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgbLevelsCapabilityError {
    GpuUnavailable,
    GtkUnavailable,
    MasksUnavailable,
    ProductionRoutingDeferred,
}

impl fmt::Display for RgbLevelsCapabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::GpuUnavailable => "RGB Levels GPU execution is unavailable",
            Self::GtkUnavailable => "RGB Levels GTK controls are unavailable",
            Self::MasksUnavailable => "RGB Levels mask consumption is unavailable",
            Self::ProductionRoutingDeferred => "RGB Levels production routing is deferred",
        })
    }
}

impl std::error::Error for RgbLevelsCapabilityError {}

/// Independent capability surfaces for this bounded leaf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbLevelsCapabilities {
    pub cpu_supported: bool,
    pub gpu_supported: bool,
    pub gtk_supported: bool,
    pub masks_consumed: bool,
    pub outer_blending_deferred: bool,
    pub profile_luminance_supported: bool,
    pub production_routing_deferred: bool,
    pub alpha_semantics_source_shaped: bool,
}

impl RgbLevelsCapabilities {
    #[must_use]
    pub const fn bounded_cpu_leaf() -> Self {
        Self {
            cpu_supported: true,
            gpu_supported: RGBLEVELS_GPU_EXECUTABLE,
            gtk_supported: false,
            masks_consumed: RGBLEVELS_SUPPORTS_MASKS,
            outer_blending_deferred: true,
            profile_luminance_supported: true,
            production_routing_deferred: false,
            alpha_semantics_source_shaped: true,
        }
    }

    pub fn require_gpu(self) -> Result<(), RgbLevelsCapabilityError> {
        if self.gpu_supported {
            Ok(())
        } else {
            Err(RgbLevelsCapabilityError::GpuUnavailable)
        }
    }

    pub fn require_gtk(self) -> Result<(), RgbLevelsCapabilityError> {
        if self.gtk_supported {
            Ok(())
        } else {
            Err(RgbLevelsCapabilityError::GtkUnavailable)
        }
    }

    pub fn require_masks(self) -> Result<(), RgbLevelsCapabilityError> {
        if self.masks_consumed {
            Ok(())
        } else {
            Err(RgbLevelsCapabilityError::MasksUnavailable)
        }
    }

    pub fn require_production_routing(self) -> Result<(), RgbLevelsCapabilityError> {
        if self.production_routing_deferred {
            Err(RgbLevelsCapabilityError::ProductionRoutingDeferred)
        } else {
            Ok(())
        }
    }
}

#[must_use]
pub const fn capabilities() -> RgbLevelsCapabilities {
    RgbLevelsCapabilities::bounded_cpu_leaf()
}

fn rgb_norm(
    rgb: [f32; 3],
    mode: RgbLevelsPreserveColors,
    profile: Option<&RgbLevelsProfileEvidence>,
) -> f32 {
    match mode {
        RgbLevelsPreserveColors::None => unreachable!("None is handled before norm selection"),
        RgbLevelsPreserveColors::Luminance => profile.map_or_else(
            || {
                rgb[0] * CAMERA_LUMINANCE_RED
                    + rgb[1] * CAMERA_LUMINANCE_GREEN
                    + rgb[2] * CAMERA_LUMINANCE_BLUE
            },
            |profile| profile.luminance(rgb),
        ),
        RgbLevelsPreserveColors::Max => rgb[0].max(rgb[1]).max(rgb[2]),
        RgbLevelsPreserveColors::Average => (rgb[0] + rgb[1] + rgb[2]) / 3.0,
        RgbLevelsPreserveColors::Sum => rgb[0] + rgb[1] + rgb[2],
        RgbLevelsPreserveColors::Norm => {
            (rgb[0] * rgb[0] + rgb[1] * rgb[1] + rgb[2] * rgb[2]).sqrt()
        }
        RgbLevelsPreserveColors::Power => {
            let red = rgb[0] * rgb[0];
            let green = rgb[1] * rgb[1];
            let blue = rgb[2] * rgb[2];
            (rgb[0] * red + rgb[1] * green + rgb[2] * blue) / (red + green + blue)
        }
    }
}

fn lut_index(percentage: f32) -> usize {
    let scaled = percentage * RGBLEVELS_LUT_ENTRIES as f32;
    if scaled <= 0.0 {
        0
    } else if scaled >= (RGBLEVELS_LUT_ENTRIES - 1) as f32 {
        RGBLEVELS_LUT_ENTRIES - 1
    } else {
        scaled as usize
    }
}

fn extrapolate_lut(lut: &[f32], value: f32, lut_size: usize) -> f32 {
    let upper = (lut_size - 1) as f32;
    let ft = (value * upper).clamp(0.0, upper);
    let t = if ft < (lut_size - 2) as f32 {
        ft as usize
    } else {
        lut_size - 2
    };
    let fraction = ft - t as f32;
    let l1 = lut[t];
    let l2 = lut[t + 1];
    l1 * (1.0 - fraction) + l2 * fraction
}

fn eval_exp(coefficients: [f32; 3], value: f32) -> f32 {
    coefficients[1] * (value * coefficients[0]).powf(coefficients[2])
}

fn write_i32(bytes: &mut [u8], offset: usize, value: i32) {
    bytes[offset..offset + size_of::<i32>()].copy_from_slice(&value.to_le_bytes());
}

fn write_f32(bytes: &mut [u8], offset: usize, value: f32) {
    bytes[offset..offset + size_of::<f32>()].copy_from_slice(&value.to_le_bytes());
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    let mut raw = [0_u8; size_of::<i32>()];
    raw.copy_from_slice(&bytes[offset..offset + size_of::<i32>()]);
    i32::from_le_bytes(raw)
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    let mut raw = [0_u8; size_of::<f32>()];
    raw.copy_from_slice(&bytes[offset..offset + size_of::<f32>()]);
    f32::from_le_bytes(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_through_reservation_failure_is_fail_closed() {
        let mut output = Vec::new();
        assert_eq!(
            reserve_pixels(&mut output, usize::MAX, usize::MAX),
            Err(RgbLevelsExecutionError::AllocationFailed {
                required_bytes: usize::MAX,
            })
        );
        assert!(output.is_empty());
    }
}
