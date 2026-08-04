#![expect(
    clippy::suboptimal_flops,
    reason = "Native Colisa arithmetic order is preserved for IEEE-754 parity."
)]

//! Bounded CPU leaf for Darktable's deprecated `colisa` operation.
//!
//! Source lineage: `src/iop/colisa.c`, with exponential extrapolation from
//! `src/develop/imageop_math.h`, at Darktable commit
//! `d8628e8103989bc4ef06dbfb9fd01f3809f884bf`.
//!
//! This leaf owns the native v1 ABI, checked contrast/brightness LUTs, and
//! cancellation-safe Lab publication. Registry/history routing, shared
//! blending and masks, pixelpipe integration, GPU, GTK, and presets remain
//! explicitly deferred.

#![forbid(unsafe_code)]
#![allow(
    dead_code,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::missing_errors_doc,
    clippy::struct_excessive_bools,
    reason = "the unregistered source-shaped leaf keeps ABI, capability evidence, and arithmetic boundaries explicit"
)]

use std::fmt;
use std::mem::size_of;

pub const COLISA_COMPATIBILITY_ID: &str = "colisa";
pub const COLISA_RUST_ID: &str = "rusttable.colisa";
pub const COLISA_PARAMETER_VERSION: u16 = 1;
pub const COLISA_PARAMETER_BYTES: usize = 12;
pub const COLISA_TABLE_ENTRIES: usize = 0x1_0000;
pub const COLISA_TABLE_BYTES: usize = 2 * COLISA_TABLE_ENTRIES * size_of::<f32>();
pub const COLISA_CANCELLATION_POLL_PIXELS: usize = 256;
pub const COLISA_DEFAULT_CONTRAST: f32 = 0.0;
pub const COLISA_DEFAULT_BRIGHTNESS: f32 = 0.0;
pub const COLISA_DEFAULT_SATURATION: f32 = 0.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColisaMetadata {
    pub compatibility_id: &'static str,
    pub rust_id: &'static str,
    pub native_source: &'static str,
    pub parameter_version: u16,
    pub default_enabled: bool,
    pub deprecated: bool,
    pub default_groups: &'static [&'static str],
    pub default_colorspace: &'static str,
    pub allow_tiling: bool,
    pub supports_shared_blending_native: bool,
    pub shared_blending_integrated: bool,
    pub legacy_order: f32,
    pub v50_raw_order: f32,
    pub v50_jpeg_order: f32,
    pub generated_inventory_order: u32,
}

pub const COLISA_METADATA: ColisaMetadata = ColisaMetadata {
    compatibility_id: COLISA_COMPATIBILITY_ID,
    rust_id: COLISA_RUST_ID,
    native_source: "src/iop/colisa.c",
    parameter_version: COLISA_PARAMETER_VERSION,
    default_enabled: false,
    deprecated: true,
    default_groups: &["basic", "grading"],
    default_colorspace: "lab",
    allow_tiling: true,
    supports_shared_blending_native: true,
    shared_blending_integrated: false,
    legacy_order: 47.0,
    v50_raw_order: 47.0,
    v50_jpeg_order: 47.0,
    generated_inventory_order: 74,
};

/// Native v1 parameters in declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColisaParametersV1 {
    pub contrast: f32,
    pub brightness: f32,
    pub saturation: f32,
}

impl ColisaParametersV1 {
    #[must_use]
    pub const fn new(contrast: f32, brightness: f32, saturation: f32) -> Self {
        Self {
            contrast,
            brightness,
            saturation,
        }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(
            COLISA_DEFAULT_CONTRAST,
            COLISA_DEFAULT_BRIGHTNESS,
            COLISA_DEFAULT_SATURATION,
        )
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; COLISA_PARAMETER_BYTES] {
        let mut bytes = [0_u8; COLISA_PARAMETER_BYTES];
        put_f32(&mut bytes, 0, self.contrast);
        put_f32(&mut bytes, 4, self.brightness);
        put_f32(&mut bytes, 8, self.saturation);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColisaError> {
        require_length(bytes, COLISA_PARAMETER_BYTES)?;
        Ok(Self::new(
            read_f32(bytes, 0),
            read_f32(bytes, 4),
            read_f32(bytes, 8),
        ))
    }
}

/// Validated operation state used by the typed processing graph.
///
/// The native values remain exact `f32` bit patterns while the graph stores
/// them as bits so the operation kind can retain `Eq` and `Hash` identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColisaConfig {
    values: [u32; 3],
}

impl ColisaConfig {
    pub fn new(parameters: ColisaParametersV1) -> Result<Self, ColisaError> {
        validate_parameters(parameters)?;
        Ok(Self {
            values: [
                parameters.contrast.to_bits(),
                parameters.brightness.to_bits(),
                parameters.saturation.to_bits(),
            ],
        })
    }

    #[must_use]
    pub const fn parameters(self) -> ColisaParametersV1 {
        ColisaParametersV1::new(
            f32::from_bits(self.values[0]),
            f32::from_bits(self.values[1]),
            f32::from_bits(self.values[2]),
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ColisaHistory {
    V1(ColisaParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl ColisaHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, ColisaError> {
        if version == COLISA_PARAMETER_VERSION {
            Ok(Self::V1(ColisaParametersV1::from_bytes(bytes)?))
        } else {
            Ok(Self::Opaque {
                version,
                bytes: fallible_copy(bytes)?,
            })
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => COLISA_PARAMETER_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    pub fn payload(&self) -> Result<Vec<u8>, ColisaError> {
        match self {
            Self::V1(parameters) => fallible_copy(&parameters.to_bytes()),
            Self::Opaque { bytes, .. } => fallible_copy(bytes),
        }
    }

    pub const fn current(&self) -> Result<ColisaParametersV1, ColisaError> {
        match self {
            Self::V1(parameters) => Ok(*parameters),
            Self::Opaque { version, .. } => Err(ColisaError::OpaqueVersion(*version)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColisaFormat {
    LabF32x4,
    RgbaF32x4,
}

#[derive(Debug, Clone, Copy)]
pub struct ColisaRaster<'a> {
    pub samples: &'a [f32],
    pub width: u32,
    pub height: u32,
    pub format: ColisaFormat,
}

impl<'a> ColisaRaster<'a> {
    #[must_use]
    pub const fn new(samples: &'a [f32], width: u32, height: u32, format: ColisaFormat) -> Self {
        Self {
            samples,
            width,
            height,
            format,
        }
    }
}

/// Immutable native lookup state.
#[derive(Debug, Clone, PartialEq)]
pub struct ColisaPlan {
    parameters: ColisaParametersV1,
    saturation: f32,
    contrast_table: Box<[f32]>,
    contrast_unbounded: [f32; 3],
    brightness_table: Box<[f32]>,
    brightness_unbounded: [f32; 3],
}

impl ColisaPlan {
    pub fn compile(
        parameters: ColisaParametersV1,
        maximum_working_bytes: usize,
    ) -> Result<Self, ColisaError> {
        Self::compile_with_cancellation(parameters, maximum_working_bytes, || false)
    }

    pub fn compile_with_cancellation<F: Fn() -> bool>(
        parameters: ColisaParametersV1,
        maximum_working_bytes: usize,
        cancelled: F,
    ) -> Result<Self, ColisaError> {
        validate_parameters(parameters)?;
        let one_table_bytes = COLISA_TABLE_ENTRIES
            .checked_mul(size_of::<f32>())
            .ok_or(ColisaError::ShapeOverflow)?;
        let required = one_table_bytes
            .checked_mul(2)
            .ok_or(ColisaError::ShapeOverflow)?;
        if required > maximum_working_bytes {
            return Err(ColisaError::WorkingMemoryBudgetExceeded {
                required,
                budget: maximum_working_bytes,
            });
        }
        if cancelled() {
            return Err(ColisaError::Cancelled);
        }

        let contrast = parameters.contrast + 1.0_f32;
        let brightness = parameters.brightness * 2.0_f32;
        let saturation = parameters.saturation + 1.0_f32;
        let mut contrast_table = allocate_f32(COLISA_TABLE_ENTRIES, one_table_bytes)?;
        let mut brightness_table = allocate_f32(COLISA_TABLE_ENTRIES, one_table_bytes)?;

        if contrast <= 1.0_f32 {
            for (index, value) in contrast_table.iter_mut().enumerate() {
                if index % 1024 == 0 && cancelled() {
                    return Err(ColisaError::Cancelled);
                }
                let position = 100.0_f32 * index as f32 / 65_536.0_f32 - 50.0_f32;
                // Native compilation enables fp-contract=fast for this source expression.
                *value = contrast.mul_add(position, 50.0_f32);
            }
        } else {
            let boost = 20.0_f32;
            let contrast_minus_one = contrast - 1.0_f32;
            let contrast_minus_one_squared = boost * contrast_minus_one * contrast_minus_one;
            let contrast_scale = (1.0_f32 + contrast_minus_one_squared).sqrt();
            for (index, value) in contrast_table.iter_mut().enumerate() {
                if index % 1024 == 0 && cancelled() {
                    return Err(ColisaError::Cancelled);
                }
                let position = 2.0_f32 * index as f32 / 65_536.0_f32 - 1.0_f32;
                let square = position * position;
                let denominator = contrast_minus_one_squared.mul_add(square, 1.0_f32).sqrt();
                *value = 50.0_f32 * (contrast_scale * position / denominator + 1.0_f32);
            }
        }

        let gamma = if brightness >= 0.0_f32 {
            1.0_f32 / (1.0_f32 + brightness)
        } else {
            1.0_f32 - brightness
        };
        for (index, value) in brightness_table.iter_mut().enumerate() {
            if index % 1024 == 0 && cancelled() {
                return Err(ColisaError::Cancelled);
            }
            *value = 100.0_f32 * (index as f32 / 65_536.0_f32).powf(gamma);
        }

        let x = [0.7_f32, 0.8_f32, 0.9_f32, 1.0_f32];
        let contrast_y = x.map(|sample| contrast_table[lookup_index(sample)]);
        let brightness_y = x.map(|sample| brightness_table[lookup_index(sample)]);
        let contrast_unbounded = estimate_exp(x, contrast_y);
        let brightness_unbounded = estimate_exp(x, brightness_y);
        if contrast_table.iter().any(|value| !value.is_finite())
            || brightness_table.iter().any(|value| !value.is_finite())
            || contrast_unbounded.iter().any(|value| !value.is_finite())
            || brightness_unbounded.iter().any(|value| !value.is_finite())
        {
            return Err(ColisaError::NonFinitePlan);
        }

        Ok(Self {
            parameters,
            saturation,
            contrast_table: contrast_table.into_boxed_slice(),
            contrast_unbounded,
            brightness_table: brightness_table.into_boxed_slice(),
            brightness_unbounded,
        })
    }

    #[must_use]
    pub const fn parameters(&self) -> ColisaParametersV1 {
        self.parameters
    }

    pub fn execute<F: Fn() -> bool>(
        &self,
        raster: ColisaRaster<'_>,
        maximum_output_bytes: usize,
        cancelled: F,
    ) -> Result<Vec<f32>, ColisaError> {
        let (pixels, samples, required) = validate_raster(raster, maximum_output_bytes)?;
        if cancelled() {
            return Err(ColisaError::Cancelled);
        }
        let mut output = allocate_f32(samples, required)?;

        for pixel in 0..pixels {
            if pixel % COLISA_CANCELLATION_POLL_PIXELS == 0 && cancelled() {
                return Err(ColisaError::Cancelled);
            }
            let offset = pixel.checked_mul(4).ok_or(ColisaError::ShapeOverflow)?;
            let input = &raster.samples[offset..offset + 4];
            if let Some(channel) = input.iter().position(|value| !value.is_finite()) {
                return Err(ColisaError::NonFiniteInput {
                    index: offset + channel,
                });
            }

            let contrasted = if input[0] < 100.0_f32 {
                self.contrast_table[lookup_index(input[0] / 100.0_f32)]
            } else {
                eval_exp(self.contrast_unbounded, input[0] / 100.0_f32)
            };
            output[offset] = if contrasted < 100.0_f32 {
                self.brightness_table[lookup_index(contrasted / 100.0_f32)]
            } else {
                eval_exp(self.brightness_unbounded, contrasted / 100.0_f32)
            };
            output[offset + 1] = input[1] * self.saturation;
            output[offset + 2] = input[2] * self.saturation;
            output[offset + 3] = input[3];

            if let Some(channel) = output[offset..offset + 4]
                .iter()
                .position(|value| !value.is_finite())
            {
                return Err(ColisaError::NonFiniteOutput {
                    index: offset + channel,
                });
            }
        }
        Ok(output)
    }

    pub fn execute_and_publish<F: Fn() -> bool>(
        &self,
        raster: ColisaRaster<'_>,
        destination: &mut Vec<f32>,
        maximum_output_bytes: usize,
        cancelled: F,
    ) -> Result<(), ColisaError> {
        let candidate = self.execute(raster, maximum_output_bytes, cancelled)?;
        *destination = candidate;
        Ok(())
    }
}

fn validate_parameters(parameters: ColisaParametersV1) -> Result<(), ColisaError> {
    for (name, value) in [
        ("contrast", parameters.contrast),
        ("brightness", parameters.brightness),
        ("saturation", parameters.saturation),
    ] {
        if !value.is_finite() {
            return Err(ColisaError::NonFiniteParameter(name));
        }
        if !(-1.0_f32..=1.0_f32).contains(&value) {
            return Err(ColisaError::ParameterOutOfRange(name));
        }
    }
    Ok(())
}

fn validate_raster(
    raster: ColisaRaster<'_>,
    maximum_output_bytes: usize,
) -> Result<(usize, usize, usize), ColisaError> {
    if raster.format != ColisaFormat::LabF32x4 {
        return Err(ColisaError::UnsupportedFormat);
    }
    let width = usize::try_from(raster.width).map_err(|_| ColisaError::ShapeOverflow)?;
    let height = usize::try_from(raster.height).map_err(|_| ColisaError::ShapeOverflow)?;
    let pixels = width
        .checked_mul(height)
        .ok_or(ColisaError::ShapeOverflow)?;
    let samples = pixels.checked_mul(4).ok_or(ColisaError::ShapeOverflow)?;
    if raster.samples.len() != samples {
        return Err(ColisaError::InputLengthMismatch {
            expected: samples,
            actual: raster.samples.len(),
        });
    }
    let required = samples
        .checked_mul(size_of::<f32>())
        .ok_or(ColisaError::ShapeOverflow)?;
    if required > maximum_output_bytes {
        return Err(ColisaError::OutputMemoryBudgetExceeded {
            required,
            budget: maximum_output_bytes,
        });
    }
    Ok((pixels, samples, required))
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
        COLISA_TABLE_ENTRIES - 1
    } else {
        (value * 65_536.0_f32) as usize
    }
}

fn allocate_f32(count: usize, required: usize) -> Result<Vec<f32>, ColisaError> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(count)
        .map_err(|_| ColisaError::AllocationFailed { required })?;
    output.resize(count, 0.0_f32);
    Ok(output)
}

fn fallible_copy(bytes: &[u8]) -> Result<Vec<u8>, ColisaError> {
    let mut copied = Vec::new();
    copied
        .try_reserve_exact(bytes.len())
        .map_err(|_| ColisaError::AllocationFailed {
            required: bytes.len(),
        })?;
    copied.extend_from_slice(bytes);
    Ok(copied)
}

const fn require_length(bytes: &[u8], expected: usize) -> Result<(), ColisaError> {
    if bytes.len() == expected {
        Ok(())
    } else {
        Err(ColisaError::InvalidPayloadLength {
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
pub enum ColisaError {
    InvalidPayloadLength { expected: usize, actual: usize },
    OpaqueVersion(u16),
    NonFiniteParameter(&'static str),
    ParameterOutOfRange(&'static str),
    NonFinitePlan,
    UnsupportedFormat,
    ShapeOverflow,
    InputLengthMismatch { expected: usize, actual: usize },
    WorkingMemoryBudgetExceeded { required: usize, budget: usize },
    OutputMemoryBudgetExceeded { required: usize, budget: usize },
    AllocationFailed { required: usize },
    NonFiniteInput { index: usize },
    NonFiniteOutput { index: usize },
    Cancelled,
}

impl fmt::Display for ColisaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPayloadLength { expected, actual } => write!(
                formatter,
                "colisa v1 payload has {actual} bytes; expected {expected}"
            ),
            Self::OpaqueVersion(version) => {
                write!(formatter, "colisa history v{version} is opaque")
            }
            Self::NonFiniteParameter(name) => {
                write!(formatter, "colisa parameter {name} is non-finite")
            }
            Self::ParameterOutOfRange(name) => {
                write!(
                    formatter,
                    "colisa parameter {name} is outside its native range"
                )
            }
            Self::NonFinitePlan => formatter.write_str("colisa compiled a non-finite plan"),
            Self::UnsupportedFormat => formatter.write_str("colisa requires four-channel f32 Lab"),
            Self::ShapeOverflow => formatter.write_str("colisa raster shape overflowed"),
            Self::InputLengthMismatch { expected, actual } => {
                write!(
                    formatter,
                    "colisa input has {actual} samples; expected {expected}"
                )
            }
            Self::WorkingMemoryBudgetExceeded { required, budget } => write!(
                formatter,
                "colisa needs {required} working bytes; budget is {budget}"
            ),
            Self::OutputMemoryBudgetExceeded { required, budget } => write!(
                formatter,
                "colisa needs {required} output bytes; budget is {budget}"
            ),
            Self::AllocationFailed { required } => {
                write!(formatter, "colisa could not allocate {required} bytes")
            }
            Self::NonFiniteInput { index } => {
                write!(formatter, "colisa input sample {index} is non-finite")
            }
            Self::NonFiniteOutput { index } => {
                write!(formatter, "colisa output sample {index} is non-finite")
            }
            Self::Cancelled => formatter.write_str("colisa execution was cancelled"),
        }
    }
}

impl std::error::Error for ColisaError {}

pub const DEFAULT_V1_FIXTURE_HEX: &str = include_str!("fixtures/default_v1.hex");
