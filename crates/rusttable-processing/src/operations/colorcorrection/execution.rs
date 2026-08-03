//! Checked scalar Color Correction CPU execution from `process` and `commit_params`.

#![forbid(unsafe_code)]

use std::fmt;
use std::hash::{Hash, Hasher};
use std::mem::size_of;

use rusttable_processing::RasterDimensions;

use super::codec::ColorCorrectionParametersV1;

const CANCELLATION_INTERVAL_PIXELS: usize = 1024;
const DEFAULT_MEMORY_BUDGET_BYTES: usize = usize::MAX;

/// Bounded-leaf process arithmetic selected under `RustTable`'s numerics policy.
///
/// Retained build metadata defaults an unspecified native build to
/// `RelWithDebInfo`; `src/CMakeLists.txt` appends `-O2 -ftree-vectorize` there,
/// appends `-O0` for non-custom Debug and `-D_DEBUG` for every Debug build, and
/// appends `-O3`, the contiguous `-ffast` + `-math` flag, and
/// `-fno-finite-math-only` for non-custom Release. GNU adds profile-specific
/// debug symbols and `-fexpensive-optimizations` in Release. Toolchain defaults,
/// target selection, and Release fast-math leave native contraction and
/// reassociation compiler/profile dependent; no one native profile supplies a
/// portable intermediate-bit contract.
///
/// `RustTable` forbids global fast-math and registers every deliberate
/// `.mul_add`. This leaf therefore makes the explicit Rust adaptation of one f32
/// rounding after each written multiply and add. It does not claim the Debug,
/// default `RelWithDebInfo`, or Release bits of any particular native binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorCorrectionCpuArithmeticProfile {
    SeparateRoundings,
}

pub const COLORCORRECTION_CPU_ARITHMETIC_PROFILE: ColorCorrectionCpuArithmeticProfile =
    ColorCorrectionCpuArithmeticProfile::SeparateRoundings;

/// Finite persisted parameters in native declaration order.
///
/// Native `commit_params` does not clamp history to the GUI ranges, so every
/// finite f32 is accepted here. Parameters stay as raw f32 bits, and no stage
/// canonicalizes signed zero. Parameter/config bits, committed coefficient
/// bits, lightness, and channel four are preserved in their stated boundaries;
/// computed opponent-channel zero signs are the IEEE results of the selected
/// separate-rounding stages.
#[derive(Debug, Clone, Copy)]
pub struct ColorCorrectionConfig {
    hia: f32,
    hib: f32,
    loa: f32,
    lob: f32,
    saturation: f32,
}

impl PartialEq for ColorCorrectionConfig {
    fn eq(&self, other: &Self) -> bool {
        self.parameters().to_bytes() == other.parameters().to_bytes()
    }
}

impl Eq for ColorCorrectionConfig {}

impl Hash for ColorCorrectionConfig {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.parameters().to_bytes().hash(state);
    }
}

impl ColorCorrectionConfig {
    pub fn new(
        hia: f32,
        hib: f32,
        loa: f32,
        lob: f32,
        saturation: f32,
    ) -> Result<Self, ColorCorrectionParameterError> {
        Self::try_from(ColorCorrectionParametersV1::new(
            hia, hib, loa, lob, saturation,
        ))
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::try_from(ColorCorrectionParametersV1::defaults())
            .expect("static Color Correction defaults are finite")
    }

    #[must_use]
    pub const fn hia(self) -> f32 {
        self.hia
    }

    #[must_use]
    pub const fn hib(self) -> f32 {
        self.hib
    }

    #[must_use]
    pub const fn loa(self) -> f32 {
        self.loa
    }

    #[must_use]
    pub const fn lob(self) -> f32 {
        self.lob
    }

    #[must_use]
    pub const fn saturation(self) -> f32 {
        self.saturation
    }

    #[must_use]
    pub const fn parameters(self) -> ColorCorrectionParametersV1 {
        ColorCorrectionParametersV1::new(
            self.hia(),
            self.hib(),
            self.loa(),
            self.lob(),
            self.saturation(),
        )
    }
}

impl Default for ColorCorrectionConfig {
    fn default() -> Self {
        Self::defaults()
    }
}

impl TryFrom<ColorCorrectionParametersV1> for ColorCorrectionConfig {
    type Error = ColorCorrectionParameterError;

    fn try_from(parameters: ColorCorrectionParametersV1) -> Result<Self, Self::Error> {
        Ok(Self {
            hia: finite_parameter("hia", parameters.hia)?,
            hib: finite_parameter("hib", parameters.hib)?,
            loa: finite_parameter("loa", parameters.loa)?,
            lob: finite_parameter("lob", parameters.lob)?,
            saturation: finite_parameter("saturation", parameters.saturation)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorCorrectionParameterError {
    NonFinite(&'static str),
}

impl fmt::Display for ColorCorrectionParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(field) => {
                write!(formatter, "Color Correction {field} is non-finite")
            }
        }
    }
}

impl std::error::Error for ColorCorrectionParameterError {}

const fn finite_parameter(
    field: &'static str,
    value: f32,
) -> Result<f32, ColorCorrectionParameterError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(ColorCorrectionParameterError::NonFinite(field))
    }
}

/// Immutable values produced by native `commit_params`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorCorrectionCoefficients {
    a_scale: f32,
    a_base: f32,
    b_scale: f32,
    b_base: f32,
    saturation: f32,
}

impl ColorCorrectionCoefficients {
    #[must_use]
    pub const fn a_scale(self) -> f32 {
        self.a_scale
    }

    #[must_use]
    pub const fn a_base(self) -> f32 {
        self.a_base
    }

    #[must_use]
    pub const fn b_scale(self) -> f32 {
        self.b_scale
    }

    #[must_use]
    pub const fn b_base(self) -> f32 {
        self.b_base
    }

    #[must_use]
    pub const fn saturation(self) -> f32 {
        self.saturation
    }

    /// Argument order of `data/kernels/basic.cl::colorcorrection` after size.
    #[must_use]
    pub const fn as_kernel_arguments(self) -> [f32; 5] {
        [
            self.saturation,
            self.a_scale,
            self.a_base,
            self.b_scale,
            self.b_base,
        ]
    }
}

/// Four-channel native Lab sample. Channel four is preserved bit-for-bit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorCorrectionPixel {
    channels: [f32; 4],
}

impl ColorCorrectionPixel {
    #[must_use]
    pub const fn new(lightness: f32, a: f32, b: f32, alpha_or_spare: f32) -> Self {
        Self {
            channels: [lightness, a, b, alpha_or_spare],
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
    pub const fn alpha_or_spare(self) -> f32 {
        self.channels[3]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorCorrectionChannel {
    Lightness,
    A,
    B,
    AlphaOrSpare,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorCorrectionPlanError {
    NonFiniteDerived(&'static str),
}

impl fmt::Display for ColorCorrectionPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteDerived(field) => {
                write!(formatter, "Color Correction derived {field} is non-finite")
            }
        }
    }
}

impl std::error::Error for ColorCorrectionPlanError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorCorrectionExecutionError {
    DimensionsMismatch {
        expected: usize,
        actual: usize,
    },
    DestinationLengthMismatch {
        expected: usize,
        actual: usize,
    },
    NonFiniteInput {
        pixel: usize,
        channel: ColorCorrectionChannel,
    },
    NonFiniteOutput {
        pixel: usize,
        channel: ColorCorrectionChannel,
    },
    AllocationFailed {
        required_bytes: usize,
    },
    SizeOverflow,
    Cancelled {
        processed: usize,
    },
}

impl fmt::Display for ColorCorrectionExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DimensionsMismatch { expected, actual } => write!(
                formatter,
                "Color Correction expected {expected} input pixels, got {actual}"
            ),
            Self::DestinationLengthMismatch { expected, actual } => write!(
                formatter,
                "Color Correction expected {expected} destination pixels, got {actual}"
            ),
            Self::NonFiniteInput { pixel, channel } => write!(
                formatter,
                "Color Correction input pixel {pixel} has non-finite {channel:?}"
            ),
            Self::NonFiniteOutput { pixel, channel } => write!(
                formatter,
                "Color Correction output pixel {pixel} has non-finite {channel:?}"
            ),
            Self::AllocationFailed { required_bytes } => write!(
                formatter,
                "Color Correction allocation failed for {required_bytes} bytes"
            ),
            Self::SizeOverflow => formatter.write_str("Color Correction execution size overflowed"),
            Self::Cancelled { processed } => write!(
                formatter,
                "Color Correction execution cancelled after {processed} pixels"
            ),
        }
    }
}

impl std::error::Error for ColorCorrectionExecutionError {}

/// Immutable source-shaped CPU plan.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorCorrectionPlan {
    config: ColorCorrectionConfig,
    dimensions: RasterDimensions,
    coefficients: ColorCorrectionCoefficients,
    memory_budget_bytes: usize,
}

impl ColorCorrectionPlan {
    pub fn new(
        config: ColorCorrectionConfig,
        dimensions: RasterDimensions,
    ) -> Result<Self, ColorCorrectionPlanError> {
        // C evaluates each parenthesized endpoint subtraction as f32, promotes
        // that rounded f32 to double for division by unsuffixed `100.0`, then
        // narrows the assigned result back to f32. Preserve all three stages.
        let a_scale = native_committed_scale(config.hia(), config.loa());
        let b_scale = native_committed_scale(config.hib(), config.lob());
        ensure_finite_derived("a_scale", a_scale)?;
        ensure_finite_derived("b_scale", b_scale)?;
        let coefficients = ColorCorrectionCoefficients {
            a_scale,
            a_base: config.loa(),
            b_scale,
            b_base: config.lob(),
            saturation: config.saturation(),
        };
        Ok(Self {
            config,
            dimensions,
            coefficients,
            memory_budget_bytes: DEFAULT_MEMORY_BUDGET_BYTES,
        })
    }

    /// Bounds the one-raster candidate payload requested by
    /// `execute_with_cancel`. Retained `default_tiling_callback` accounts for
    /// the already-present input and output with factor 2 for identity ROI. In
    /// `execute_into`, that output is the caller destination and this budget is
    /// charged only for Rust's additive private staging raster, so allocation
    /// failure precedes any write to caller-owned storage.
    ///
    /// Native `dt_tiling_piece_fits_host_memory` first evaluates its
    /// `float factor * width * height * bpp + overhead` expression in f32 and
    /// then converts the rounded result to `size_t`. This leaf deliberately does
    /// not reproduce that lossy boundary: `output_layout` uses checked integer
    /// multiplication for the exact staging payload. At the documented 4097²,
    /// factor-two boundary, the native source expression is 32 bytes below the
    /// exact integer payload.
    ///
    /// This is also deliberately an ordinary, lower-alignment `Vec` adaptation
    /// of native pixelpipe-cache-owned, cache-line-aligned buffers, not an
    /// implementation of `DT_IS_ALIGNED`. Both budgets omit allocator metadata
    /// and alignment padding; `try_reserve_exact` may still receive
    /// allocator-specific excess capacity.
    #[must_use]
    pub const fn with_memory_budget(mut self, memory_budget_bytes: usize) -> Self {
        self.memory_budget_bytes = memory_budget_bytes;
        self
    }

    #[must_use]
    pub const fn memory_budget(self) -> usize {
        self.memory_budget_bytes
    }

    #[must_use]
    pub const fn config(self) -> ColorCorrectionConfig {
        self.config
    }

    #[must_use]
    pub const fn dimensions(self) -> RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn coefficients(self) -> ColorCorrectionCoefficients {
        self.coefficients
    }

    pub fn execute(
        &self,
        input: &[ColorCorrectionPixel],
    ) -> Result<Vec<ColorCorrectionPixel>, ColorCorrectionExecutionError> {
        self.execute_with_cancel(input, |_| false)
    }

    /// Polls at row boundaries and at most every 1024 pixels. The private
    /// candidate is returned only after complete finite validation. The loop is
    /// intentionally serial: it preserves bounded cancellation and transactional
    /// publication but does not claim native `DT_OMP_FOR()` scheduling.
    pub fn execute_with_cancel<F>(
        &self,
        input: &[ColorCorrectionPixel],
        mut cancelled: F,
    ) -> Result<Vec<ColorCorrectionPixel>, ColorCorrectionExecutionError>
    where
        F: FnMut(usize) -> bool,
    {
        let (expected, required_bytes) = self.output_layout()?;
        if input.len() != expected {
            return Err(ColorCorrectionExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        if cancelled(0) {
            return Err(ColorCorrectionExecutionError::Cancelled { processed: 0 });
        }
        if required_bytes > self.memory_budget_bytes {
            return Err(ColorCorrectionExecutionError::AllocationFailed { required_bytes });
        }

        let mut output = Vec::new();
        output
            .try_reserve_exact(expected)
            .map_err(|_| ColorCorrectionExecutionError::AllocationFailed { required_bytes })?;

        let width = usize::try_from(self.dimensions.width())
            .map_err(|_| ColorCorrectionExecutionError::SizeOverflow)?;
        for (index, pixel) in input.iter().copied().enumerate() {
            if index != 0
                && (index.is_multiple_of(width)
                    || index.is_multiple_of(CANCELLATION_INTERVAL_PIXELS))
                && cancelled(index)
            {
                return Err(ColorCorrectionExecutionError::Cancelled { processed: index });
            }
            validate_input(pixel, index)?;
            let result = self.execute_pixel(pixel);
            validate_output(result, index)?;
            output.push(result);
        }
        if cancelled(expected) {
            return Err(ColorCorrectionExecutionError::Cancelled {
                processed: expected,
            });
        }
        Ok(output)
    }

    /// Computes privately and copies only a complete result into the caller's
    /// destination, keeping it unchanged on every error or cancellation.
    pub fn execute_into(
        &self,
        input: &[ColorCorrectionPixel],
        destination: &mut [ColorCorrectionPixel],
    ) -> Result<(), ColorCorrectionExecutionError> {
        self.execute_into_with_cancel(input, destination, |_| false)
    }

    pub fn execute_into_with_cancel<F>(
        &self,
        input: &[ColorCorrectionPixel],
        destination: &mut [ColorCorrectionPixel],
        cancelled: F,
    ) -> Result<(), ColorCorrectionExecutionError>
    where
        F: FnMut(usize) -> bool,
    {
        let (expected, _) = self.output_layout()?;
        if destination.len() != expected {
            return Err(ColorCorrectionExecutionError::DestinationLengthMismatch {
                expected,
                actual: destination.len(),
            });
        }
        let candidate = self.execute_with_cancel(input, cancelled)?;
        destination.copy_from_slice(&candidate);
        Ok(())
    }

    fn output_layout(&self) -> Result<(usize, usize), ColorCorrectionExecutionError> {
        let pixels = usize::try_from(self.dimensions.pixel_count())
            .map_err(|_| ColorCorrectionExecutionError::SizeOverflow)?;
        let required_bytes = pixels
            .checked_mul(size_of::<ColorCorrectionPixel>())
            .ok_or(ColorCorrectionExecutionError::SizeOverflow)?;
        Ok((pixels, required_bytes))
    }

    #[must_use]
    fn execute_pixel(&self, input: ColorCorrectionPixel) -> ColorCorrectionPixel {
        let lightness = input.lightness();
        let a = separate_rounding_opponent(
            input.a(),
            lightness,
            self.coefficients.a_scale,
            self.coefficients.a_base,
            self.coefficients.saturation,
        );
        let b = separate_rounding_opponent(
            input.b(),
            lightness,
            self.coefficients.b_scale,
            self.coefficients.b_base,
            self.coefficients.saturation,
        );
        ColorCorrectionPixel::new(lightness, a, b, input.alpha_or_spare())
    }
}

/// Deliberate Rust adaptation of the parsed C expression, rather than a claim
/// about compiler-dependent native Debug, `RelWithDebInfo`, or Release contraction.
/// Each statement supplies one f32 rounding point.
#[must_use]
fn separate_rounding_opponent(
    opponent: f32,
    lightness: f32,
    scale: f32,
    base: f32,
    saturation: f32,
) -> f32 {
    let scaled_lightness = lightness * scale;
    let with_input = opponent + scaled_lightness;
    let with_base = with_input + base;
    saturation * with_base
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "Native commit_params narrows its double division result into an f32 field."
)]
fn native_committed_scale(high: f32, low: f32) -> f32 {
    let endpoint_difference = high - low;
    (f64::from(endpoint_difference) / 100.0) as f32
}

const fn ensure_finite_derived(
    field: &'static str,
    value: f32,
) -> Result<(), ColorCorrectionPlanError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(ColorCorrectionPlanError::NonFiniteDerived(field))
    }
}

fn validate_input(
    pixel: ColorCorrectionPixel,
    index: usize,
) -> Result<(), ColorCorrectionExecutionError> {
    let channels = [
        (ColorCorrectionChannel::Lightness, pixel.lightness()),
        (ColorCorrectionChannel::A, pixel.a()),
        (ColorCorrectionChannel::B, pixel.b()),
        (ColorCorrectionChannel::AlphaOrSpare, pixel.alpha_or_spare()),
    ];
    for (channel, value) in channels {
        if !value.is_finite() {
            return Err(ColorCorrectionExecutionError::NonFiniteInput {
                pixel: index,
                channel,
            });
        }
    }
    Ok(())
}

fn validate_output(
    pixel: ColorCorrectionPixel,
    index: usize,
) -> Result<(), ColorCorrectionExecutionError> {
    for (channel, value) in [
        (ColorCorrectionChannel::A, pixel.a()),
        (ColorCorrectionChannel::B, pixel.b()),
    ] {
        if !value.is_finite() {
            return Err(ColorCorrectionExecutionError::NonFiniteOutput {
                pixel: index,
                channel,
            });
        }
    }
    Ok(())
}
