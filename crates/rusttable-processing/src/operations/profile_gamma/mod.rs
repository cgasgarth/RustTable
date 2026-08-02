//! Bounded CPU leaf for Darktable's `profile_gamma` operation.
//!
//! Source lineage: `src/iop/profile_gamma.c`, with exponential extrapolation
//! from `src/develop/imageop_math.h` and `fastlog2` from `src/common/math.h` at
//! Darktable commit `d8628e8103989bc4ef06dbfb9fd01f3809f884bf`.
//!
//! This leaf owns native v1/v2 little-endian history, the v1-to-v2 migration,
//! checked LUT construction, and cancellation-safe CPU publication. Registry,
//! history import, shared blending/masks, pixelpipe routing, GPU, GTK, presets,
//! and image-picker auto-tuning remain explicitly deferred.

#![forbid(unsafe_code)]
#![allow(
    dead_code,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::float_cmp,
    clippy::missing_errors_doc,
    clippy::struct_excessive_bools,
    reason = "the unregistered source-shaped leaf keeps native ABI, exact float branches, and arithmetic boundaries explicit"
)]

use std::fmt;
use std::mem::size_of;

pub const PROFILE_GAMMA_COMPATIBILITY_ID: &str = "profile_gamma";
pub const PROFILE_GAMMA_RUST_ID: &str = "rusttable.profile_gamma";
pub const PROFILE_GAMMA_PARAMETER_VERSION: u16 = 2;
pub const PROFILE_GAMMA_V1_PARAMETER_BYTES: usize = 8;
pub const PROFILE_GAMMA_V2_PARAMETER_BYTES: usize = 28;
pub const PROFILE_GAMMA_TABLE_ENTRIES: usize = 0x1_0000;
pub const PROFILE_GAMMA_CANCELLATION_POLL_PIXELS: usize = 256;
pub const PROFILE_GAMMA_NOISE_FLOOR: f32 = 1.525_878_9e-5;

pub const PROFILE_GAMMA_DEFAULT_MODE: i32 = ProfileGammaMode::Log as i32;
pub const PROFILE_GAMMA_DEFAULT_LINEAR: f32 = 0.1;
pub const PROFILE_GAMMA_DEFAULT_GAMMA: f32 = 0.45;
pub const PROFILE_GAMMA_DEFAULT_DYNAMIC_RANGE: f32 = 10.0;
pub const PROFILE_GAMMA_DEFAULT_GREY_POINT: f32 = 18.0;
pub const PROFILE_GAMMA_DEFAULT_SHADOWS_RANGE: f32 = -5.0;
pub const PROFILE_GAMMA_DEFAULT_SECURITY_FACTOR: f32 = 0.0;

/// Operation-local registration, metadata, and native order evidence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfileGammaMetadata {
    pub compatibility_id: &'static str,
    pub rust_id: &'static str,
    pub native_source: &'static str,
    pub parameter_version: u16,
    pub default_enabled: bool,
    pub default_groups: &'static [&'static str],
    pub default_colorspace: &'static str,
    pub allow_tiling: bool,
    pub one_instance: bool,
    pub supports_shared_blending_native: bool,
    pub shared_blending_integrated: bool,
    pub legacy_order: f32,
    pub v50_raw_order: f32,
    pub v50_jpeg_order: f32,
    pub generated_inventory_order: u32,
}

pub const PROFILE_GAMMA_METADATA: ProfileGammaMetadata = ProfileGammaMetadata {
    compatibility_id: PROFILE_GAMMA_COMPATIBILITY_ID,
    rust_id: PROFILE_GAMMA_RUST_ID,
    native_source: "src/iop/profile_gamma.c",
    parameter_version: PROFILE_GAMMA_PARAMETER_VERSION,
    default_enabled: false,
    default_groups: &["color", "technical"],
    default_colorspace: "rgb",
    allow_tiling: true,
    one_instance: true,
    supports_shared_blending_native: true,
    shared_blending_integrated: false,
    legacy_order: 25.0,
    v50_raw_order: 26.0,
    v50_jpeg_order: 28.0,
    generated_inventory_order: 103,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ProfileGammaMode {
    Log = 0,
    Gamma = 1,
}

impl TryFrom<i32> for ProfileGammaMode {
    type Error = ProfileGammaError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Log),
            1 => Ok(Self::Gamma),
            _ => Err(ProfileGammaError::UnsupportedMode(value)),
        }
    }
}

/// Native v1 payload declared by `dt_iop_profilegamma_params_v1_t`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfileGammaParametersV1 {
    pub linear: f32,
    pub gamma: f32,
}

impl ProfileGammaParametersV1 {
    #[must_use]
    pub const fn new(linear: f32, gamma: f32) -> Self {
        Self { linear, gamma }
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; PROFILE_GAMMA_V1_PARAMETER_BYTES] {
        let mut bytes = [0_u8; PROFILE_GAMMA_V1_PARAMETER_BYTES];
        put_f32(&mut bytes, 0, self.linear);
        put_f32(&mut bytes, 4, self.gamma);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProfileGammaError> {
        require_length(1, bytes, PROFILE_GAMMA_V1_PARAMETER_BYTES)?;
        Ok(Self::new(read_f32(bytes, 0), read_f32(bytes, 4)))
    }
}

/// Current native v2 payload in declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfileGammaParametersV2 {
    /// Raw C enum representation. Compilation rejects values other than 0/1.
    pub mode: i32,
    pub linear: f32,
    pub gamma: f32,
    pub dynamic_range: f32,
    pub grey_point: f32,
    pub shadows_range: f32,
    pub security_factor: f32,
}

impl ProfileGammaParametersV2 {
    #[must_use]
    pub const fn new(
        mode: i32,
        linear: f32,
        gamma: f32,
        dynamic_range: f32,
        grey_point: f32,
        shadows_range: f32,
        security_factor: f32,
    ) -> Self {
        Self {
            mode,
            linear,
            gamma,
            dynamic_range,
            grey_point,
            shadows_range,
            security_factor,
        }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(
            PROFILE_GAMMA_DEFAULT_MODE,
            PROFILE_GAMMA_DEFAULT_LINEAR,
            PROFILE_GAMMA_DEFAULT_GAMMA,
            PROFILE_GAMMA_DEFAULT_DYNAMIC_RANGE,
            PROFILE_GAMMA_DEFAULT_GREY_POINT,
            PROFILE_GAMMA_DEFAULT_SHADOWS_RANGE,
            PROFILE_GAMMA_DEFAULT_SECURITY_FACTOR,
        )
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; PROFILE_GAMMA_V2_PARAMETER_BYTES] {
        let mut bytes = [0_u8; PROFILE_GAMMA_V2_PARAMETER_BYTES];
        bytes[0..4].copy_from_slice(&self.mode.to_le_bytes());
        put_f32(&mut bytes, 4, self.linear);
        put_f32(&mut bytes, 8, self.gamma);
        put_f32(&mut bytes, 12, self.dynamic_range);
        put_f32(&mut bytes, 16, self.grey_point);
        put_f32(&mut bytes, 20, self.shadows_range);
        put_f32(&mut bytes, 24, self.security_factor);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProfileGammaError> {
        require_length(2, bytes, PROFILE_GAMMA_V2_PARAMETER_BYTES)?;
        Ok(Self::new(
            i32::from_le_bytes(bytes[0..4].try_into().expect("checked v2 payload length")),
            read_f32(bytes, 4),
            read_f32(bytes, 8),
            read_f32(bytes, 12),
            read_f32(bytes, 16),
            read_f32(bytes, 20),
            read_f32(bytes, 24),
        ))
    }
}

/// Known native history plus byte-exact retention for future versions.
#[derive(Debug, Clone, PartialEq)]
pub enum ProfileGammaHistory {
    V1(ProfileGammaParametersV1),
    V2(ProfileGammaParametersV2),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl ProfileGammaHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, ProfileGammaError> {
        match version {
            1 => Ok(Self::V1(ProfileGammaParametersV1::from_bytes(bytes)?)),
            PROFILE_GAMMA_PARAMETER_VERSION => {
                Ok(Self::V2(ProfileGammaParametersV2::from_bytes(bytes)?))
            }
            _ => Ok(Self::Opaque {
                version,
                bytes: fallible_copy(bytes)?,
            }),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => 1,
            Self::V2(_) => PROFILE_GAMMA_PARAMETER_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    pub fn payload(&self) -> Result<Vec<u8>, ProfileGammaError> {
        match self {
            Self::V1(parameters) => fallible_copy(&parameters.to_bytes()),
            Self::V2(parameters) => fallible_copy(&parameters.to_bytes()),
            Self::Opaque { bytes, .. } => fallible_copy(bytes),
        }
    }

    pub fn current(&self) -> Result<ProfileGammaParametersV2, ProfileGammaError> {
        match self {
            Self::V1(parameters) => Ok(migrate_v1_to_v2(*parameters)),
            Self::V2(parameters) => Ok(*parameters),
            Self::Opaque { version, .. } => Err(ProfileGammaError::OpaqueVersion(*version)),
        }
    }
}

/// Exact `legacy_params` v1-to-v2 migration.
#[must_use]
pub const fn migrate_v1_to_v2(parameters: ProfileGammaParametersV1) -> ProfileGammaParametersV2 {
    ProfileGammaParametersV2::new(
        ProfileGammaMode::Gamma as i32,
        parameters.linear,
        parameters.gamma,
        PROFILE_GAMMA_DEFAULT_DYNAMIC_RANGE,
        PROFILE_GAMMA_DEFAULT_GREY_POINT,
        PROFILE_GAMMA_DEFAULT_SHADOWS_RANGE,
        PROFILE_GAMMA_DEFAULT_SECURITY_FACTOR,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileGammaFormat {
    RgbF32x3,
    RgbaF32x4,
    LabF32x4,
}

impl ProfileGammaFormat {
    const fn channels(self) -> usize {
        match self {
            Self::RgbF32x3 => 3,
            Self::RgbaF32x4 | Self::LabF32x4 => 4,
        }
    }

    const fn supported(self) -> bool {
        matches!(self, Self::RgbF32x3 | Self::RgbaF32x4)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ProfileGammaRaster<'a> {
    pub samples: &'a [f32],
    pub width: u32,
    pub height: u32,
    pub format: ProfileGammaFormat,
}

impl<'a> ProfileGammaRaster<'a> {
    #[must_use]
    pub const fn new(
        samples: &'a [f32],
        width: u32,
        height: u32,
        format: ProfileGammaFormat,
    ) -> Self {
        Self {
            samples,
            width,
            height,
            format,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum CompiledProfileGamma {
    Log {
        grey: f32,
        dynamic_range: f32,
        shadows_range: f32,
    },
    Gamma {
        table: Box<[f32]>,
        unbounded_coefficients: [f32; 3],
    },
}

/// Immutable checked CPU plan.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileGammaPlan {
    parameters: ProfileGammaParametersV2,
    compiled: CompiledProfileGamma,
}

impl ProfileGammaPlan {
    pub fn compile(
        parameters: ProfileGammaParametersV2,
        maximum_working_bytes: usize,
    ) -> Result<Self, ProfileGammaError> {
        Self::compile_with_cancellation(parameters, maximum_working_bytes, || false)
    }

    pub fn compile_with_cancellation<F: Fn() -> bool>(
        parameters: ProfileGammaParametersV2,
        maximum_working_bytes: usize,
        cancelled: F,
    ) -> Result<Self, ProfileGammaError> {
        validate_parameters(parameters)?;
        let mode = ProfileGammaMode::try_from(parameters.mode)?;
        if cancelled() {
            return Err(ProfileGammaError::Cancelled);
        }

        let compiled = match mode {
            ProfileGammaMode::Log => CompiledProfileGamma::Log {
                grey: parameters.grey_point / 100.0_f32,
                dynamic_range: parameters.dynamic_range,
                shadows_range: parameters.shadows_range,
            },
            ProfileGammaMode::Gamma => {
                let required = PROFILE_GAMMA_TABLE_ENTRIES
                    .checked_mul(size_of::<f32>())
                    .ok_or(ProfileGammaError::ShapeOverflow)?;
                if required > maximum_working_bytes {
                    return Err(ProfileGammaError::WorkingMemoryBudgetExceeded {
                        required,
                        budget: maximum_working_bytes,
                    });
                }
                let mut table = allocate_f32(PROFILE_GAMMA_TABLE_ENTRIES, required)?;
                fill_gamma_table(&mut table, parameters.linear, parameters.gamma, &cancelled)?;
                let x = [0.7_f32, 0.8_f32, 0.9_f32, 1.0_f32];
                let y = x.map(|sample| table[lookup_index(sample)]);
                let unbounded_coefficients = estimate_exp(x, y);
                if table.iter().any(|value| !value.is_finite())
                    || unbounded_coefficients
                        .iter()
                        .any(|value| !value.is_finite())
                {
                    return Err(ProfileGammaError::NonFinitePlan);
                }
                CompiledProfileGamma::Gamma {
                    table: table.into_boxed_slice(),
                    unbounded_coefficients,
                }
            }
        };
        Ok(Self {
            parameters,
            compiled,
        })
    }

    #[must_use]
    pub const fn parameters(&self) -> ProfileGammaParametersV2 {
        self.parameters
    }

    pub fn execute<F: Fn() -> bool>(
        &self,
        raster: ProfileGammaRaster<'_>,
        maximum_output_bytes: usize,
        cancelled: F,
    ) -> Result<Vec<f32>, ProfileGammaError> {
        let (pixels, samples, required) = validate_raster(raster, maximum_output_bytes)?;
        if cancelled() {
            return Err(ProfileGammaError::Cancelled);
        }
        let mut output = allocate_f32(samples, required)?;
        let channels = raster.format.channels();

        for pixel in 0..pixels {
            if pixel % PROFILE_GAMMA_CANCELLATION_POLL_PIXELS == 0 && cancelled() {
                return Err(ProfileGammaError::Cancelled);
            }
            let offset = pixel
                .checked_mul(channels)
                .ok_or(ProfileGammaError::ShapeOverflow)?;
            let input = &raster.samples[offset..offset + channels];
            if let Some(channel) = input.iter().position(|value| !value.is_finite()) {
                return Err(ProfileGammaError::NonFiniteInput {
                    index: offset + channel,
                });
            }

            match &self.compiled {
                CompiledProfileGamma::Log {
                    grey,
                    dynamic_range,
                    shadows_range,
                } => {
                    // Native CPU `process` applies the log branch to all `piece->colors` lanes.
                    for channel in 0..channels {
                        let mut relative = input[channel] / *grey;
                        if relative < PROFILE_GAMMA_NOISE_FLOOR {
                            relative = PROFILE_GAMMA_NOISE_FLOOR;
                        }
                        let mapped = (fast_log2(relative) - *shadows_range) / *dynamic_range;
                        output[offset + channel] = if mapped < PROFILE_GAMMA_NOISE_FLOOR {
                            PROFILE_GAMMA_NOISE_FLOOR
                        } else {
                            mapped
                        };
                    }
                }
                CompiledProfileGamma::Gamma {
                    table,
                    unbounded_coefficients,
                } => {
                    for channel in 0..3 {
                        let value = input[channel];
                        output[offset + channel] = if value < 1.0_f32 {
                            table[lookup_index(value)]
                        } else {
                            eval_exp(*unbounded_coefficients, value)
                        };
                    }
                    if channels == 4 {
                        // Native CPU leaves lane four unwritten except during mask display. Safe
                        // publication cannot expose uninitialized storage, so the leaf adopts the
                        // native OpenCL/mask-display behavior and preserves the source lane. The
                        // production pixelpipe owner must choose the final alpha/spare-lane policy.
                        output[offset + 3] = input[3];
                    }
                }
            }

            if let Some(channel) = output[offset..offset + channels]
                .iter()
                .position(|value| !value.is_finite())
            {
                return Err(ProfileGammaError::NonFiniteOutput {
                    index: offset + channel,
                });
            }
        }
        Ok(output)
    }

    /// Publishes only after format, shape, finite, allocation, and cancellation checks succeed.
    pub fn execute_and_publish<F: Fn() -> bool>(
        &self,
        raster: ProfileGammaRaster<'_>,
        destination: &mut Vec<f32>,
        maximum_output_bytes: usize,
        cancelled: F,
    ) -> Result<(), ProfileGammaError> {
        let candidate = self.execute(raster, maximum_output_bytes, cancelled)?;
        *destination = candidate;
        Ok(())
    }
}

fn validate_parameters(parameters: ProfileGammaParametersV2) -> Result<(), ProfileGammaError> {
    for (name, value) in [
        ("linear", parameters.linear),
        ("gamma", parameters.gamma),
        ("dynamic_range", parameters.dynamic_range),
        ("grey_point", parameters.grey_point),
        ("shadows_range", parameters.shadows_range),
        ("security_factor", parameters.security_factor),
    ] {
        if !value.is_finite() {
            return Err(ProfileGammaError::NonFiniteParameter(name));
        }
    }
    ProfileGammaMode::try_from(parameters.mode)?;
    require_range("linear", parameters.linear, 0.0, 1.0)?;
    require_range("gamma", parameters.gamma, 0.0, 1.0)?;
    require_range("dynamic_range", parameters.dynamic_range, 0.01, 32.0)?;
    require_range("grey_point", parameters.grey_point, 0.1, 100.0)?;
    require_range("shadows_range", parameters.shadows_range, -16.0, 16.0)?;
    require_range("security_factor", parameters.security_factor, -100.0, 100.0)
}

fn require_range(
    name: &'static str,
    value: f32,
    minimum: f32,
    maximum: f32,
) -> Result<(), ProfileGammaError> {
    if value < minimum || value > maximum {
        Err(ProfileGammaError::ParameterOutOfRange(name))
    } else {
        Ok(())
    }
}

fn fill_gamma_table<F: Fn() -> bool>(
    table: &mut [f32],
    linear: f32,
    gamma: f32,
    cancelled: &F,
) -> Result<(), ProfileGammaError> {
    if gamma == 1.0_f32 {
        for (index, value) in table.iter_mut().enumerate() {
            if index % 1024 == 0 && cancelled() {
                return Err(ProfileGammaError::Cancelled);
            }
            // Native `1.0 * k / 0x10000` is evaluated as double, then narrowed.
            *value = ((index as f64) / 65_536.0_f64) as f32;
        }
        return Ok(());
    }

    if linear == 0.0_f32 {
        for (index, value) in table.iter_mut().enumerate() {
            if index % 1024 == 0 && cancelled() {
                return Err(ProfileGammaError::Cancelled);
            }
            // Native `1.00 * k / 0x10000` has the same explicit double boundary.
            let base = ((index as f64) / 65_536.0_f64) as f32;
            *value = base.powf(gamma);
        }
        return Ok(());
    }

    let (a, b, c, g) = if linear < 1.0_f32 {
        // C's unsuffixed `1.0` operands promote the surrounding operations to
        // double, but `gamma * linear` and `linear * (g - 1)` are completed as
        // float subexpressions before their enclosing double operation. Each
        // assigned `float` therefore narrows only after that mixed evaluation.
        let gamma_times_linear = gamma * linear;
        let g = (f64::from(gamma) * (1.0_f64 - f64::from(linear))
            / (1.0_f64 - f64::from(gamma_times_linear))) as f32;
        let linear_times_g_minus_one = linear * (g - 1.0_f32);
        let a = (1.0_f64 / (1.0_f64 + f64::from(linear_times_g_minus_one))) as f32;
        let b = linear * (g - 1.0_f32) * a;
        // The native translation unit enables fp-contract=fast; make the only
        // multiply-add in this boundary explicit instead of target-dependent.
        let c = a.mul_add(linear, b).powf(g) / linear;
        (a, b, c, g)
    } else {
        (0.0_f32, 0.0_f32, 1.0_f32, 0.0_f32)
    };

    let transition = 65_536.0_f32 * linear;
    for (index, value) in table.iter_mut().enumerate() {
        if index % 1024 == 0 && cancelled() {
            return Err(ProfileGammaError::Cancelled);
        }
        let index_f32 = index as f32;
        let position = index_f32 / 65_536.0_f32;
        *value = if index_f32 < transition {
            c * position
        } else {
            a.mul_add(position, b).powf(g)
        };
    }
    Ok(())
}

fn validate_raster(
    raster: ProfileGammaRaster<'_>,
    maximum_output_bytes: usize,
) -> Result<(usize, usize, usize), ProfileGammaError> {
    if !raster.format.supported() {
        return Err(ProfileGammaError::UnsupportedFormat);
    }
    let width = usize::try_from(raster.width).map_err(|_| ProfileGammaError::ShapeOverflow)?;
    let height = usize::try_from(raster.height).map_err(|_| ProfileGammaError::ShapeOverflow)?;
    let pixels = width
        .checked_mul(height)
        .ok_or(ProfileGammaError::ShapeOverflow)?;
    let samples = pixels
        .checked_mul(raster.format.channels())
        .ok_or(ProfileGammaError::ShapeOverflow)?;
    if raster.samples.len() != samples {
        return Err(ProfileGammaError::InputLengthMismatch {
            expected: samples,
            actual: raster.samples.len(),
        });
    }
    let required = samples
        .checked_mul(size_of::<f32>())
        .ok_or(ProfileGammaError::ShapeOverflow)?;
    if required > maximum_output_bytes {
        return Err(ProfileGammaError::OutputMemoryBudgetExceeded {
            required,
            budget: maximum_output_bytes,
        });
    }
    Ok((pixels, samples, required))
}

fn fast_log2(value: f32) -> f32 {
    let bits = value.to_bits();
    let mantissa = f32::from_bits((bits & 0x007f_ffff) | 0x3f00_0000);
    let scaled_exponent = (bits as f32) * 1.192_092_9e-7_f32;
    let first = scaled_exponent - 124.225_52_f32;
    let contracted = (-1.498_030_3_f32).mul_add(mantissa, first);
    contracted - 1.725_88_f32 / (0.352_088_72_f32 + mantissa)
}

fn estimate_exp(x: [f32; 4], y: [f32; 4]) -> [f32; 3] {
    let x0 = x[3];
    let y0 = y[3];
    let mut exponent = 0.0_f32;
    let mut count = 0_u32;
    for index in 0..3 {
        let yy = y[index] / y0;
        let xx = x[index] / x0;
        if yy > 0.0_f32 && xx > 0.0_f32 {
            exponent += (y[index] / y0).ln() / (x[index] / x0).ln();
            count += 1;
        }
    }
    if count == 0 {
        exponent = 1.0_f32;
    } else {
        exponent *= 1.0_f32 / count as f32;
    }
    [1.0_f32 / x0, y0, exponent]
}

fn eval_exp(coefficients: [f32; 3], value: f32) -> f32 {
    coefficients[1] * (value * coefficients[0]).powf(coefficients[2])
}

fn lookup_index(value: f32) -> usize {
    if value <= 0.0_f32 {
        0
    } else if value >= 1.0_f32 {
        PROFILE_GAMMA_TABLE_ENTRIES - 1
    } else {
        (value * 65_536.0_f32) as usize
    }
}

fn allocate_f32(count: usize, required: usize) -> Result<Vec<f32>, ProfileGammaError> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(count)
        .map_err(|_| ProfileGammaError::AllocationFailed { required })?;
    output.resize(count, 0.0_f32);
    Ok(output)
}

fn fallible_copy(bytes: &[u8]) -> Result<Vec<u8>, ProfileGammaError> {
    let mut copied = Vec::new();
    copied
        .try_reserve_exact(bytes.len())
        .map_err(|_| ProfileGammaError::AllocationFailed {
            required: bytes.len(),
        })?;
    copied.extend_from_slice(bytes);
    Ok(copied)
}

fn require_length(version: u16, bytes: &[u8], expected: usize) -> Result<(), ProfileGammaError> {
    if bytes.len() == expected {
        Ok(())
    } else {
        Err(ProfileGammaError::InvalidPayloadLength {
            version,
            expected,
            actual: bytes.len(),
        })
    }
}

fn put_f32(bytes: &mut [u8], offset: usize, value: f32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("payload length checked before field decoding"),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileGammaError {
    InvalidPayloadLength {
        version: u16,
        expected: usize,
        actual: usize,
    },
    OpaqueVersion(u16),
    UnsupportedMode(i32),
    NonFiniteParameter(&'static str),
    ParameterOutOfRange(&'static str),
    NonFinitePlan,
    UnsupportedFormat,
    ShapeOverflow,
    InputLengthMismatch {
        expected: usize,
        actual: usize,
    },
    WorkingMemoryBudgetExceeded {
        required: usize,
        budget: usize,
    },
    OutputMemoryBudgetExceeded {
        required: usize,
        budget: usize,
    },
    AllocationFailed {
        required: usize,
    },
    NonFiniteInput {
        index: usize,
    },
    NonFiniteOutput {
        index: usize,
    },
    Cancelled,
}

impl fmt::Display for ProfileGammaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPayloadLength {
                version,
                expected,
                actual,
            } => write!(
                formatter,
                "profile_gamma v{version} payload has {actual} bytes; expected {expected}"
            ),
            Self::OpaqueVersion(version) => {
                write!(formatter, "profile_gamma history v{version} is opaque")
            }
            Self::UnsupportedMode(mode) => {
                write!(formatter, "profile_gamma mode {mode} is unsupported")
            }
            Self::NonFiniteParameter(name) => {
                write!(formatter, "profile_gamma parameter {name} is non-finite")
            }
            Self::ParameterOutOfRange(name) => {
                write!(
                    formatter,
                    "profile_gamma parameter {name} is outside its native range"
                )
            }
            Self::NonFinitePlan => formatter.write_str("profile_gamma compiled a non-finite plan"),
            Self::UnsupportedFormat => {
                formatter.write_str("profile_gamma requires three- or four-channel f32 RGB")
            }
            Self::ShapeOverflow => formatter.write_str("profile_gamma raster shape overflowed"),
            Self::InputLengthMismatch { expected, actual } => write!(
                formatter,
                "profile_gamma input has {actual} samples; expected {expected}"
            ),
            Self::WorkingMemoryBudgetExceeded { required, budget } => write!(
                formatter,
                "profile_gamma needs {required} working bytes; budget is {budget}"
            ),
            Self::OutputMemoryBudgetExceeded { required, budget } => write!(
                formatter,
                "profile_gamma needs {required} output bytes; budget is {budget}"
            ),
            Self::AllocationFailed { required } => {
                write!(
                    formatter,
                    "profile_gamma could not allocate {required} bytes"
                )
            }
            Self::NonFiniteInput { index } => {
                write!(
                    formatter,
                    "profile_gamma input sample {index} is non-finite"
                )
            }
            Self::NonFiniteOutput { index } => {
                write!(
                    formatter,
                    "profile_gamma output sample {index} is non-finite"
                )
            }
            Self::Cancelled => formatter.write_str("profile_gamma execution was cancelled"),
        }
    }
}

impl std::error::Error for ProfileGammaError {}

/// Source-derived default v1 payload bytes.
pub const DEFAULT_V1_FIXTURE_HEX: &str = include_str!("fixtures/default_v1.hex");
/// Source-derived default v2 payload bytes.
pub const DEFAULT_V2_FIXTURE_HEX: &str = include_str!("fixtures/default_v2.hex");
