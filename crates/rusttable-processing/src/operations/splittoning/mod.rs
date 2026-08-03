//! Bounded CPU leaf for Darktable's `splittoning` operation.
//!
//! Source lineage: `src/iop/splittoning.c` and the `rgb2hsl`/`hsl2rgb`
//! helpers in `src/common/colorspaces.h` at Darktable commit
//! `d8628e8103989bc4ef06dbfb9fd01f3809f884bf`.
//!
//! This leaf owns native v1 history, source-order HSL arithmetic, required
//! four-channel format checks, and cancellation-safe publication. Registry and
//! history routing, shared blending/masks, pixelpipe, GPU, GTK, color pickers,
//! and preset installation remain explicitly deferred.

#![forbid(unsafe_code)]
#![allow(
    dead_code,
    clippy::cast_possible_truncation,
    clippy::doc_markdown,
    clippy::float_cmp,
    clippy::missing_errors_doc,
    clippy::struct_excessive_bools,
    reason = "the unregistered source-shaped leaf keeps exact source comparisons, ABI, and mixed f32/f64 boundaries explicit"
)]

use std::fmt;
use std::mem::size_of;

pub const SPLIT_TONING_COMPATIBILITY_ID: &str = "splittoning";
pub const SPLIT_TONING_RUST_ID: &str = "rusttable.splittoning";
pub const SPLIT_TONING_PARAMETER_VERSION: u16 = 1;
pub const SPLIT_TONING_PARAMETER_BYTES: usize = 24;
pub const SPLIT_TONING_CANCELLATION_POLL_PIXELS: usize = 256;
pub const SPLIT_TONING_DEFAULT_SHADOW_HUE: f32 = 0.0;
pub const SPLIT_TONING_DEFAULT_SHADOW_SATURATION: f32 = 0.5;
pub const SPLIT_TONING_DEFAULT_HIGHLIGHT_HUE: f32 = 0.2;
pub const SPLIT_TONING_DEFAULT_HIGHLIGHT_SATURATION: f32 = 0.5;
pub const SPLIT_TONING_DEFAULT_BALANCE: f32 = 0.5;
pub const SPLIT_TONING_DEFAULT_COMPRESS: f32 = 33.0;
/// Display bounds from the source introspection annotation.
pub const SPLIT_TONING_BALANCE_UI_MIN: f32 = 0.0;
pub const SPLIT_TONING_BALANCE_UI_MAX: f32 = 1.0;
/// Finite history admission follows executable native presets, including 100.
pub const SPLIT_TONING_BALANCE_HISTORY_MIN: f32 = 0.0;
pub const SPLIT_TONING_BALANCE_HISTORY_MAX: f32 = 100.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitToningMetadata {
    pub compatibility_id: &'static str,
    pub rust_id: &'static str,
    pub native_source: &'static str,
    pub parameter_version: u16,
    pub default_enabled: bool,
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

pub const SPLIT_TONING_METADATA: SplitToningMetadata = SplitToningMetadata {
    compatibility_id: SPLIT_TONING_COMPATIBILITY_ID,
    rust_id: SPLIT_TONING_RUST_ID,
    native_source: "src/iop/splittoning.c",
    parameter_version: SPLIT_TONING_PARAMETER_VERSION,
    default_enabled: false,
    default_groups: &["effect", "grading"],
    default_colorspace: "rgb",
    allow_tiling: true,
    supports_shared_blending_native: true,
    shared_blending_integrated: false,
    legacy_order: 62.0,
    v50_raw_order: 67.0,
    v50_jpeg_order: 67.0,
    generated_inventory_order: 99,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitToningParametersV1 {
    pub shadow_hue: f32,
    pub shadow_saturation: f32,
    pub highlight_hue: f32,
    pub highlight_saturation: f32,
    pub balance: f32,
    pub compress: f32,
}

impl SplitToningParametersV1 {
    #[must_use]
    pub const fn new(
        shadow_hue: f32,
        shadow_saturation: f32,
        highlight_hue: f32,
        highlight_saturation: f32,
        balance: f32,
        compress: f32,
    ) -> Self {
        Self {
            shadow_hue,
            shadow_saturation,
            highlight_hue,
            highlight_saturation,
            balance,
            compress,
        }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(
            SPLIT_TONING_DEFAULT_SHADOW_HUE,
            SPLIT_TONING_DEFAULT_SHADOW_SATURATION,
            SPLIT_TONING_DEFAULT_HIGHLIGHT_HUE,
            SPLIT_TONING_DEFAULT_HIGHLIGHT_SATURATION,
            SPLIT_TONING_DEFAULT_BALANCE,
            SPLIT_TONING_DEFAULT_COMPRESS,
        )
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; SPLIT_TONING_PARAMETER_BYTES] {
        let mut bytes = [0_u8; SPLIT_TONING_PARAMETER_BYTES];
        for (index, value) in [
            self.shadow_hue,
            self.shadow_saturation,
            self.highlight_hue,
            self.highlight_saturation,
            self.balance,
            self.compress,
        ]
        .into_iter()
        .enumerate()
        {
            put_f32(&mut bytes, index * 4, value);
        }
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SplitToningError> {
        require_length(bytes, SPLIT_TONING_PARAMETER_BYTES)?;
        Ok(Self::new(
            read_f32(bytes, 0),
            read_f32(bytes, 4),
            read_f32(bytes, 8),
            read_f32(bytes, 12),
            read_f32(bytes, 16),
            read_f32(bytes, 20),
        ))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SplitToningHistory {
    V1(SplitToningParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl SplitToningHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, SplitToningError> {
        if version == SPLIT_TONING_PARAMETER_VERSION {
            Ok(Self::V1(SplitToningParametersV1::from_bytes(bytes)?))
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
            Self::V1(_) => SPLIT_TONING_PARAMETER_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    pub fn payload(&self) -> Result<Vec<u8>, SplitToningError> {
        match self {
            Self::V1(parameters) => fallible_copy(&parameters.to_bytes()),
            Self::Opaque { bytes, .. } => fallible_copy(bytes),
        }
    }

    pub const fn current(&self) -> Result<SplitToningParametersV1, SplitToningError> {
        match self {
            Self::V1(parameters) => Ok(*parameters),
            Self::Opaque { version, .. } => Err(SplitToningError::OpaqueVersion(*version)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitToningFormat {
    RgbaF32x4,
    RgbF32x3,
}

#[derive(Debug, Clone, Copy)]
pub struct SplitToningRaster<'a> {
    pub samples: &'a [f32],
    pub width: u32,
    pub height: u32,
    pub format: SplitToningFormat,
}

impl<'a> SplitToningRaster<'a> {
    #[must_use]
    pub const fn new(
        samples: &'a [f32],
        width: u32,
        height: u32,
        format: SplitToningFormat,
    ) -> Self {
        Self {
            samples,
            width,
            height,
            format,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitToningPlan {
    parameters: SplitToningParametersV1,
    compress: f32,
}

impl SplitToningPlan {
    pub fn compile(parameters: SplitToningParametersV1) -> Result<Self, SplitToningError> {
        validate_parameters(parameters)?;
        // Native source uses unsuffixed double literals for both divisions.
        let compress = ((f64::from(parameters.compress) / 110.0_f64) / 2.0_f64) as f32;
        Ok(Self {
            parameters,
            compress,
        })
    }

    #[must_use]
    pub const fn parameters(self) -> SplitToningParametersV1 {
        self.parameters
    }

    pub fn execute<F: Fn() -> bool>(
        &self,
        raster: SplitToningRaster<'_>,
        maximum_output_bytes: usize,
        cancelled: F,
    ) -> Result<Vec<f32>, SplitToningError> {
        let (pixels, samples, required) = validate_raster(raster, maximum_output_bytes)?;
        if cancelled() {
            return Err(SplitToningError::Cancelled);
        }
        let mut output = allocate_f32(samples, required)?;

        for pixel in 0..pixels {
            if pixel % SPLIT_TONING_CANCELLATION_POLL_PIXELS == 0 && cancelled() {
                return Err(SplitToningError::Cancelled);
            }
            let offset = pixel
                .checked_mul(4)
                .ok_or(SplitToningError::ShapeOverflow)?;
            let input = &raster.samples[offset..offset + 4];
            if let Some(channel) = input.iter().position(|value| !value.is_finite()) {
                return Err(SplitToningError::NonFiniteInput {
                    index: offset + channel,
                });
            }

            let (_, _, lightness) = rgb_to_hsl([input[0], input[1], input[2]]);
            let lower = self.parameters.balance - self.compress;
            let upper = self.parameters.balance + self.compress;
            if lightness < lower {
                let mix = hsl_to_rgba(
                    self.parameters.shadow_hue,
                    self.parameters.shadow_saturation,
                    lightness,
                );
                let remote = clip((lower - lightness) * 2.0_f32);
                let local = 1.0_f32 - remote;
                for channel in 0..4 {
                    // The C translation unit contracts one multiply-add under
                    // fp-contract=fast. Lane four intentionally mixes toward the
                    // zero written by `hsl2rgb`, matching native CPU semantics.
                    output[offset + channel] =
                        clip(input[channel].mul_add(local, mix[channel] * remote));
                }
            } else if lightness > upper {
                let mix = hsl_to_rgba(
                    self.parameters.highlight_hue,
                    self.parameters.highlight_saturation,
                    lightness,
                );
                let remote = clip((lightness - upper) * 2.0_f32);
                let local = 1.0_f32 - remote;
                for channel in 0..4 {
                    output[offset + channel] =
                        clip(input[channel].mul_add(local, mix[channel] * remote));
                }
            } else {
                output[offset..offset + 4].copy_from_slice(input);
            }

            if let Some(channel) = output[offset..offset + 4]
                .iter()
                .position(|value| !value.is_finite())
            {
                return Err(SplitToningError::NonFiniteOutput {
                    index: offset + channel,
                });
            }
        }
        Ok(output)
    }

    pub fn execute_and_publish<F: Fn() -> bool>(
        &self,
        raster: SplitToningRaster<'_>,
        destination: &mut Vec<f32>,
        maximum_output_bytes: usize,
        cancelled: F,
    ) -> Result<(), SplitToningError> {
        let candidate = self.execute(raster, maximum_output_bytes, cancelled)?;
        *destination = candidate;
        Ok(())
    }
}

fn validate_parameters(parameters: SplitToningParametersV1) -> Result<(), SplitToningError> {
    for (name, value, minimum, maximum) in [
        ("shadow_hue", parameters.shadow_hue, 0.0_f32, 1.0_f32),
        (
            "shadow_saturation",
            parameters.shadow_saturation,
            0.0_f32,
            1.0_f32,
        ),
        ("highlight_hue", parameters.highlight_hue, 0.0_f32, 1.0_f32),
        (
            "highlight_saturation",
            parameters.highlight_saturation,
            0.0_f32,
            1.0_f32,
        ),
        ("compress", parameters.compress, 0.0_f32, 100.0_f32),
    ] {
        if !value.is_finite() {
            return Err(SplitToningError::NonFiniteParameter(name));
        }
        if value < minimum || value > maximum {
            return Err(SplitToningError::ParameterOutOfRange(name));
        }
    }

    // The slider annotation exposes balance as 0..=1, but native history
    // admission must also accept the authentic platinotype preset's 100.0f.
    if !parameters.balance.is_finite() {
        return Err(SplitToningError::NonFiniteParameter("balance"));
    }
    if parameters.balance < SPLIT_TONING_BALANCE_HISTORY_MIN
        || parameters.balance > SPLIT_TONING_BALANCE_HISTORY_MAX
    {
        return Err(SplitToningError::ParameterOutOfRange("balance"));
    }
    Ok(())
}

fn validate_raster(
    raster: SplitToningRaster<'_>,
    maximum_output_bytes: usize,
) -> Result<(usize, usize, usize), SplitToningError> {
    if raster.format != SplitToningFormat::RgbaF32x4 {
        return Err(SplitToningError::UnsupportedFormat);
    }
    let width = usize::try_from(raster.width).map_err(|_| SplitToningError::ShapeOverflow)?;
    let height = usize::try_from(raster.height).map_err(|_| SplitToningError::ShapeOverflow)?;
    let pixels = width
        .checked_mul(height)
        .ok_or(SplitToningError::ShapeOverflow)?;
    let samples = pixels
        .checked_mul(4)
        .ok_or(SplitToningError::ShapeOverflow)?;
    if raster.samples.len() != samples {
        return Err(SplitToningError::InputLengthMismatch {
            expected: samples,
            actual: raster.samples.len(),
        });
    }
    let required = samples
        .checked_mul(size_of::<f32>())
        .ok_or(SplitToningError::ShapeOverflow)?;
    if required > maximum_output_bytes {
        return Err(SplitToningError::OutputMemoryBudgetExceeded {
            required,
            budget: maximum_output_bytes,
        });
    }
    Ok((pixels, samples, required))
}

fn rgb_to_hsl(rgb: [f32; 3]) -> (f32, f32, f32) {
    let red = rgb[0];
    let green = rgb[1];
    let blue = rgb[2];
    let maximum = red.max(green).max(blue);
    let minimum = red.min(green).min(blue);
    let delta = maximum - minimum;

    // Native `(pmin + pmax) / 2.0` promotes only at the unsuffixed divisor.
    let sum = minimum + maximum;
    let lightness = (f64::from(sum) / 2.0_f64) as f32;
    let mut hue = 0.0_f32;
    let mut saturation = 0.0_f32;

    if delta != 0.0_f32 {
        saturation = if lightness < 0.5_f32 {
            delta / (maximum + minimum).max(1.525_878_9e-5_f32)
        } else {
            let denominator = (2.0_f64 - f64::from(maximum) - f64::from(minimum)) as f32;
            delta / denominator.max(1.525_878_9e-5_f32)
        };

        hue = if maximum == red {
            (green - blue) / delta
        } else if maximum == green {
            (2.0_f64 + f64::from((blue - red) / delta)) as f32
        } else {
            (4.0_f64 + f64::from((red - green) / delta)) as f32
        };
        hue = (f64::from(hue) / 6.0_f64) as f32;
        if hue < 0.0_f32 {
            hue = (f64::from(hue) + 1.0_f64) as f32;
        } else if hue > 1.0_f32 {
            hue = (f64::from(hue) - 1.0_f64) as f32;
        }
    }
    (hue, saturation, lightness)
}

fn hsl_to_rgba(mut hue: f32, saturation: f32, lightness: f32) -> [f32; 4] {
    if saturation == 0.0_f32 {
        return [lightness, lightness, lightness, 0.0_f32];
    }
    let second = if lightness < 0.5_f32 {
        (f64::from(lightness) * (1.0_f64 + f64::from(saturation))) as f32
    } else {
        (-lightness).mul_add(saturation, lightness + saturation)
    };
    #[expect(
        clippy::suboptimal_flops,
        reason = "preserve the native hsl2rgb subtraction order and rounding"
    )]
    let first = (2.0_f64 * f64::from(lightness) - f64::from(second)) as f32;
    hue *= 6.0_f32;
    [
        hue_to_rgb(
            first,
            second,
            if hue < 4.0_f32 {
                hue + 2.0_f32
            } else {
                hue - 4.0_f32
            },
        ),
        hue_to_rgb(first, second, hue),
        hue_to_rgb(
            first,
            second,
            if hue > 2.0_f32 {
                hue - 2.0_f32
            } else {
                hue + 4.0_f32
            },
        ),
        0.0_f32,
    ]
}

fn hue_to_rgb(first: f32, second: f32, hue: f32) -> f32 {
    if hue < 1.0_f32 {
        (second - first).mul_add(hue, first)
    } else if hue < 3.0_f32 {
        second
    } else if hue < 4.0_f32 {
        (second - first).mul_add(4.0_f32 - hue, first)
    } else {
        first
    }
}

fn clip(value: f32) -> f32 {
    if value >= 0.0_f32 {
        if value <= 1.0_f32 { value } else { 1.0_f32 }
    } else {
        0.0_f32
    }
}

fn allocate_f32(count: usize, required: usize) -> Result<Vec<f32>, SplitToningError> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(count)
        .map_err(|_| SplitToningError::AllocationFailed { required })?;
    output.resize(count, 0.0_f32);
    Ok(output)
}

fn fallible_copy(bytes: &[u8]) -> Result<Vec<u8>, SplitToningError> {
    let mut copied = Vec::new();
    copied
        .try_reserve_exact(bytes.len())
        .map_err(|_| SplitToningError::AllocationFailed {
            required: bytes.len(),
        })?;
    copied.extend_from_slice(bytes);
    Ok(copied)
}

const fn require_length(bytes: &[u8], expected: usize) -> Result<(), SplitToningError> {
    if bytes.len() == expected {
        Ok(())
    } else {
        Err(SplitToningError::InvalidPayloadLength {
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
pub enum SplitToningError {
    InvalidPayloadLength { expected: usize, actual: usize },
    OpaqueVersion(u16),
    NonFiniteParameter(&'static str),
    ParameterOutOfRange(&'static str),
    UnsupportedFormat,
    ShapeOverflow,
    InputLengthMismatch { expected: usize, actual: usize },
    OutputMemoryBudgetExceeded { required: usize, budget: usize },
    AllocationFailed { required: usize },
    NonFiniteInput { index: usize },
    NonFiniteOutput { index: usize },
    Cancelled,
}

impl fmt::Display for SplitToningError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPayloadLength { expected, actual } => write!(
                formatter,
                "splittoning v1 payload has {actual} bytes; expected {expected}"
            ),
            Self::OpaqueVersion(version) => {
                write!(formatter, "splittoning history v{version} is opaque")
            }
            Self::NonFiniteParameter(name) => {
                write!(formatter, "splittoning parameter {name} is non-finite")
            }
            Self::ParameterOutOfRange(name) => {
                write!(
                    formatter,
                    "splittoning parameter {name} is outside its native range"
                )
            }
            Self::UnsupportedFormat => {
                formatter.write_str("splittoning requires four-channel f32 RGB")
            }
            Self::ShapeOverflow => formatter.write_str("splittoning raster shape overflowed"),
            Self::InputLengthMismatch { expected, actual } => write!(
                formatter,
                "splittoning input has {actual} samples; expected {expected}"
            ),
            Self::OutputMemoryBudgetExceeded { required, budget } => write!(
                formatter,
                "splittoning needs {required} output bytes; budget is {budget}"
            ),
            Self::AllocationFailed { required } => {
                write!(formatter, "splittoning could not allocate {required} bytes")
            }
            Self::NonFiniteInput { index } => {
                write!(formatter, "splittoning input sample {index} is non-finite")
            }
            Self::NonFiniteOutput { index } => {
                write!(formatter, "splittoning output sample {index} is non-finite")
            }
            Self::Cancelled => formatter.write_str("splittoning execution was cancelled"),
        }
    }
}

impl std::error::Error for SplitToningError {}

pub const DEFAULT_V1_FIXTURE_HEX: &str = include_str!("fixtures/default_v1.hex");
