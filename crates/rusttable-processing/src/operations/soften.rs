//! CPU port of Darktable's RGB Orton soft-focus operation from
//! `src/iop/soften.c`, using the retained `src/common/box_filters.h/.cc` and
//! `src/common/colorspaces.h` as behavioral oracles.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]

use std::fmt;

use crate::common::box_filters::{
    BOX_ITERATIONS, BoxFilterError, CancellableBoxFilterError, box_mean_with_cancel,
};
use crate::{FiniteF32, LinearRgb, RasterDimensions};

use super::common::{OperationExecutionError, ReconstructionBudget, checked_bytes};

pub const SOFTEN_COMPATIBILITY_ID: &str = "soften";
pub const SOFTEN_SCHEMA_VERSION: u16 = 1;
pub const SOFTEN_PARAMETER_BYTES: usize = 16;
pub const SOFTEN_DEFAULT_SIZE: f32 = 50.0;
pub const SOFTEN_DEFAULT_SATURATION: f32 = 100.0;
pub const SOFTEN_DEFAULT_BRIGHTNESS: f32 = 0.33;
pub const SOFTEN_DEFAULT_AMOUNT: f32 = 50.0;
pub const SOFTEN_CHANNELS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoftenParametersV1 {
    pub size: f32,
    pub saturation: f32,
    pub brightness: f32,
    pub amount: f32,
}

impl SoftenParametersV1 {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            size: SOFTEN_DEFAULT_SIZE,
            saturation: SOFTEN_DEFAULT_SATURATION,
            brightness: SOFTEN_DEFAULT_BRIGHTNESS,
            amount: SOFTEN_DEFAULT_AMOUNT,
        }
    }

    #[must_use]
    pub const fn new(size: f32, saturation: f32, brightness: f32, amount: f32) -> Self {
        Self {
            size,
            saturation,
            brightness,
            amount,
        }
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; SOFTEN_PARAMETER_BYTES] {
        let mut bytes = [0; SOFTEN_PARAMETER_BYTES];
        for (index, value) in [self.size, self.saturation, self.brightness, self.amount]
            .into_iter()
            .enumerate()
        {
            let start = index * 4;
            bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SoftenCodecError> {
        if bytes.len() != SOFTEN_PARAMETER_BYTES {
            return Err(SoftenCodecError::InvalidLength {
                expected: SOFTEN_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let read = |start| f32::from_le_bytes(bytes[start..start + 4].try_into().expect("range"));
        let parameters = Self::new(read(0), read(4), read(8), read(12));
        SoftenConfig::try_from(parameters).map_err(SoftenCodecError::Parameters)?;
        Ok(parameters)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SoftenHistory {
    V1(SoftenParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoftenCodecError {
    InvalidLength { expected: usize, actual: usize },
    Parameters(SoftenParameterError),
}

impl fmt::Display for SoftenCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "soften payload has {actual} bytes; expected {expected}"
                )
            }
            Self::Parameters(error) => write!(formatter, "invalid soften parameters: {error}"),
        }
    }
}

impl std::error::Error for SoftenCodecError {}

impl SoftenHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, SoftenCodecError> {
        if version == SOFTEN_SCHEMA_VERSION {
            Ok(Self::V1(SoftenParametersV1::from_bytes(bytes)?))
        } else {
            Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            })
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(parameters) => parameters.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => SOFTEN_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoftenParameterError {
    NonFinite(&'static str),
    OutOfRange(&'static str),
}

impl fmt::Display for SoftenParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "soften {name} is non-finite"),
            Self::OutOfRange(name) => write!(formatter, "soften {name} is outside its range"),
        }
    }
}

impl std::error::Error for SoftenParameterError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SoftenConfig {
    size: FiniteF32,
    saturation: FiniteF32,
    brightness: FiniteF32,
    amount: FiniteF32,
}

impl TryFrom<SoftenParametersV1> for SoftenConfig {
    type Error = SoftenParameterError;

    fn try_from(parameters: SoftenParametersV1) -> Result<Self, Self::Error> {
        Ok(Self {
            size: bounded("size", parameters.size, 0.0, 100.0)?,
            saturation: bounded("saturation", parameters.saturation, 0.0, 100.0)?,
            brightness: bounded("brightness", parameters.brightness, -2.0, 2.0)?,
            amount: bounded("amount", parameters.amount, 0.0, 100.0)?,
        })
    }
}

impl SoftenConfig {
    pub fn new(
        size: f32,
        saturation: f32,
        brightness: f32,
        amount: f32,
    ) -> Result<Self, SoftenParameterError> {
        Self::try_from(SoftenParametersV1::new(
            size, saturation, brightness, amount,
        ))
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::try_from(SoftenParametersV1::defaults()).expect("soften defaults are valid")
    }

    #[must_use]
    pub const fn size(self) -> f32 {
        self.size.get()
    }

    #[must_use]
    pub const fn saturation(self) -> f32 {
        self.saturation.get()
    }

    #[must_use]
    pub const fn brightness(self) -> f32 {
        self.brightness.get()
    }

    #[must_use]
    pub const fn amount(self) -> f32 {
        self.amount.get()
    }
}

/// The native four-float pixel layout used by `soften.c`.
///
/// The fourth value is intentionally not treated as an independently preserved
/// alpha plane: `hsl2rgb` writes zero there and the native four-channel blend
/// mixes that zero with the source fourth channel. Production RGB callers use
/// zero for this spare channel, while this type keeps the native behavior
/// testable at the leaf boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoftenPixel {
    channels: [f32; SOFTEN_CHANNELS],
}

impl SoftenPixel {
    #[must_use]
    pub const fn new(red: f32, green: f32, blue: f32, fourth: f32) -> Self {
        Self {
            channels: [red, green, blue, fourth],
        }
    }

    #[must_use]
    pub const fn from_channels(channels: [f32; SOFTEN_CHANNELS]) -> Self {
        Self { channels }
    }

    #[must_use]
    pub const fn channels(self) -> [f32; SOFTEN_CHANNELS] {
        self.channels
    }

    #[must_use]
    pub const fn red(self) -> f32 {
        self.channels[0]
    }

    #[must_use]
    pub const fn green(self) -> f32 {
        self.channels[1]
    }

    #[must_use]
    pub const fn blue(self) -> f32 {
        self.channels[2]
    }

    #[must_use]
    pub const fn fourth(self) -> f32 {
        self.channels[3]
    }

    #[must_use]
    pub const fn alpha(self) -> f32 {
        self.channels[3]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SoftenPlan {
    config: SoftenConfig,
    dimensions: RasterDimensions,
    radius: u32,
}

impl SoftenPlan {
    pub fn new(
        config: SoftenConfig,
        dimensions: RasterDimensions,
    ) -> Result<Self, OperationExecutionError> {
        Self::new_with_scale(config, dimensions, 1.0, 1.0)
    }

    /// Builds a plan with the native `roi_in->scale / piece->iscale` radius
    /// conversion. `dimensions` are the full-frame `iwidth` and `iheight`.
    pub fn new_with_scale(
        config: SoftenConfig,
        dimensions: RasterDimensions,
        roi_scale: f32,
        piece_scale: f32,
    ) -> Result<Self, OperationExecutionError> {
        let radius = soften_radius(config.size(), dimensions, roi_scale, piece_scale)?;
        checked_bytes(
            usize::try_from(dimensions.pixel_count()).map_err(|_| {
                OperationExecutionError::MemoryBudgetExceeded {
                    required: usize::MAX,
                    budget: ReconstructionBudget::default().maximum_bytes(),
                }
            })?,
            4,
            ReconstructionBudget::default(),
        )?;
        Ok(Self {
            config,
            dimensions,
            radius,
        })
    }

    #[must_use]
    pub const fn radius(&self) -> u32 {
        self.radius
    }

    /// Resolves the native tiling callback's Gaussian-equivalent `wdh` for
    /// this committed parameter and ROI scale pair. GPU is intentionally
    /// unsupported; the CPU executor conservatively uses its canonical
    /// full-frame path for tiled requests so the retained four-channel box
    /// arithmetic is not altered by tile-local floating-point order.
    pub fn overlap_pixels(
        config: SoftenConfig,
        dimensions: RasterDimensions,
        roi_scale: f32,
        piece_scale: f32,
    ) -> Result<u32, OperationExecutionError> {
        let radius = soften_radius(config.size(), dimensions, roi_scale, piece_scale)?;
        // The native tiling callback performs the radius products as ints and
        // converts only for the final sqrtf division.
        let numerator = radius
            .checked_add(1)
            .and_then(|next| radius.checked_mul(next))
            .and_then(|value| value.checked_mul(BOX_ITERATIONS))
            .and_then(|value| value.checked_add(2))
            .ok_or(OperationExecutionError::MemoryBudgetExceeded {
                required: usize::MAX,
                budget: ReconstructionBudget::default().maximum_bytes(),
            })?;
        let sigma = (numerator as f32 / 3.0_f32).sqrt();
        Ok((3.0_f32 * sigma).ceil() as u32)
    }

    /// Applies source HSL adjustment, the native four-channel eight-pass box
    /// mean, and the native linear blend. The adjusted layer is never reused
    /// as the next operation's source.
    pub fn execute(
        &self,
        input: &[LinearRgb],
        dimensions: RasterDimensions,
    ) -> Result<Vec<LinearRgb>, OperationExecutionError> {
        self.execute_with_cancel(input, dimensions, || false)
    }

    /// Executes the RGB leaf while polling cancellation at row boundaries.
    /// All output is built off to the side, so cancellation or allocation
    /// failure never publishes a partial result.
    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        dimensions: RasterDimensions,
        cancelled: F,
    ) -> Result<Vec<LinearRgb>, OperationExecutionError> {
        self.validate_execution(input.len(), dimensions)?;
        if cancelled() {
            return Err(OperationExecutionError::Cancelled);
        }
        if self.config.amount().to_bits() == 0.0f32.to_bits() {
            return try_clone(input);
        }

        let channels = self.execute_channels(
            input.len(),
            dimensions,
            |index| {
                let pixel = input[index];
                [
                    pixel.red().get(),
                    pixel.green().get(),
                    pixel.blue().get(),
                    0.0,
                ]
            },
            &cancelled,
        )?;
        let width = usize::try_from(dimensions.width()).expect("validated width fits usize");
        let mut output = try_reserve::<LinearRgb>(input.len())?;
        for (index, channels) in channels.into_iter().enumerate() {
            if index % width == 0 && cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            output.push(LinearRgb::new(
                finite(channels[0], index, crate::RgbChannel::Red)?,
                finite(channels[1], index, crate::RgbChannel::Green)?,
                finite(channels[2], index, crate::RgbChannel::Blue)?,
            ));
        }
        Ok(output)
    }

    /// Executes the native four-channel layout, including its fourth-channel
    /// zeroing during HSL conversion and four-channel blend.
    pub fn execute_rgba(
        &self,
        input: &[SoftenPixel],
        dimensions: RasterDimensions,
    ) -> Result<Vec<SoftenPixel>, OperationExecutionError> {
        self.execute_rgba_with_cancel(input, dimensions, || false)
    }

    /// Cancellable form of [`Self::execute_rgba`].
    pub fn execute_rgba_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[SoftenPixel],
        dimensions: RasterDimensions,
        cancelled: F,
    ) -> Result<Vec<SoftenPixel>, OperationExecutionError> {
        self.validate_execution(input.len(), dimensions)?;
        self.execute_rgba_inner(input, dimensions, cancelled)
    }

    /// Executes a neighborhood tile using a plan whose radius was resolved
    /// from the full source frame. This preserves Darktable's `piece->iwidth`
    /// geometry while filtering only the tile's expanded ROI.
    pub fn execute_rgba_with_input_dimensions(
        &self,
        input: &[SoftenPixel],
        dimensions: RasterDimensions,
    ) -> Result<Vec<SoftenPixel>, OperationExecutionError> {
        self.execute_rgba_with_input_dimensions_with_cancel(input, dimensions, || false)
    }

    /// Cancellable neighborhood-tile form of
    /// [`Self::execute_rgba_with_input_dimensions`].
    pub fn execute_rgba_with_input_dimensions_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[SoftenPixel],
        dimensions: RasterDimensions,
        cancelled: F,
    ) -> Result<Vec<SoftenPixel>, OperationExecutionError> {
        Self::validate_input_shape(input.len(), dimensions)?;
        self.execute_rgba_inner(input, dimensions, cancelled)
    }

    fn execute_rgba_inner<F: Fn() -> bool>(
        &self,
        input: &[SoftenPixel],
        dimensions: RasterDimensions,
        cancelled: F,
    ) -> Result<Vec<SoftenPixel>, OperationExecutionError> {
        if cancelled() {
            return Err(OperationExecutionError::Cancelled);
        }
        if self.config.amount().to_bits() == 0.0f32.to_bits() {
            return try_clone(input);
        }

        let channels = self.execute_channels(
            input.len(),
            dimensions,
            |index| input[index].channels,
            &cancelled,
        )?;
        let width = usize::try_from(dimensions.width()).expect("validated width fits usize");
        let mut output = try_reserve::<SoftenPixel>(input.len())?;
        for (index, channels) in channels.into_iter().enumerate() {
            if index % width == 0 && cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            output.push(SoftenPixel::from_channels(channels));
        }
        Ok(output)
    }

    fn validate_execution(
        &self,
        input_len: usize,
        dimensions: RasterDimensions,
    ) -> Result<(), OperationExecutionError> {
        if dimensions != self.dimensions {
            return Err(OperationExecutionError::DimensionsMismatch {
                expected: usize::try_from(self.dimensions.pixel_count()).unwrap_or(usize::MAX),
                actual: input_len,
            });
        }
        Self::validate_input_shape(input_len, dimensions)
    }

    fn validate_input_shape(
        input_len: usize,
        dimensions: RasterDimensions,
    ) -> Result<(), OperationExecutionError> {
        // Keep the public execution boundary's shape error tied to the caller's
        // declared dimensions, as with every other operation leaf.
        let expected = usize::try_from(dimensions.pixel_count()).unwrap_or(usize::MAX);
        if expected != input_len {
            return Err(OperationExecutionError::DimensionsMismatch {
                expected,
                actual: input_len,
            });
        }
        Ok(())
    }

    fn execute_channels<F, G>(
        &self,
        input_len: usize,
        dimensions: RasterDimensions,
        source: G,
        cancelled: &F,
    ) -> Result<Vec<[f32; SOFTEN_CHANNELS]>, OperationExecutionError>
    where
        F: Fn() -> bool,
        G: Fn(usize) -> [f32; SOFTEN_CHANNELS],
    {
        let width = usize::try_from(dimensions.width()).expect("validated width fits usize");
        let height = usize::try_from(dimensions.height()).expect("validated height fits usize");
        let float_count = input_len.checked_mul(SOFTEN_CHANNELS).ok_or(
            OperationExecutionError::MemoryBudgetExceeded {
                required: usize::MAX,
                budget: ReconstructionBudget::default().maximum_bytes(),
            },
        )?;
        let required_bytes = float_count.checked_mul(std::mem::size_of::<f32>()).ok_or(
            OperationExecutionError::MemoryBudgetExceeded {
                required: usize::MAX,
                budget: ReconstructionBudget::default().maximum_bytes(),
            },
        )?;
        let mut blurred = Vec::new();
        blurred.try_reserve_exact(float_count).map_err(|_| {
            OperationExecutionError::AllocationFailed {
                required: required_bytes,
            }
        })?;

        // `soften.c` uses double literals for these two CPU expressions:
        // exp2f/division and saturation scaling are rounded to f32 only at
        // the native `const float` assignments.
        let saturation = (f64::from(self.config.saturation()) / 100.0) as f32;
        let brightness = (1.0_f64 / f64::from((-self.config.brightness()).exp2())) as f32;
        for index in 0..input_len {
            if index % width == 0 && cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            let source = source(index);
            let adjusted = adjust_rgb(
                [source[0], source[1], source[2]],
                saturation,
                brightness,
                index,
            )?;
            // hsl2rgb writes zero to the fourth float before the blur.
            blurred.extend_from_slice(&[adjusted[0], adjusted[1], adjusted[2], 0.0]);
        }

        box_mean_with_cancel(
            &mut blurred,
            height,
            width,
            SOFTEN_CHANNELS as u32,
            usize::try_from(self.radius).expect("soften radius fits usize"),
            BOX_ITERATIONS,
            cancelled,
        )
        .map_err(cancellable_box_filter_error)?;
        if cancelled() {
            return Err(OperationExecutionError::Cancelled);
        }

        let amount = self.config.amount() / 100.0;
        let remainder = 1.0 - amount;
        let mut output = try_reserve::<[f32; SOFTEN_CHANNELS]>(input_len)?;
        let (blurred_pixels, remainder_slice) = blurred.as_chunks::<SOFTEN_CHANNELS>();
        debug_assert!(remainder_slice.is_empty());
        for (index, processed) in blurred_pixels.iter().enumerate() {
            if index % width == 0 && cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            let original = source(index);
            let values = [
                amount * processed[0] + remainder * original[0],
                amount * processed[1] + remainder * original[1],
                amount * processed[2] + remainder * original[2],
                amount * processed[3] + remainder * original[3],
            ];
            output.push([
                finite(values[0], index, crate::RgbChannel::Red)?.get(),
                finite(values[1], index, crate::RgbChannel::Green)?.get(),
                finite(values[2], index, crate::RgbChannel::Blue)?.get(),
                values[3],
            ]);
        }
        Ok(output)
    }
}

fn cancellable_box_filter_error(error: CancellableBoxFilterError) -> OperationExecutionError {
    match error {
        CancellableBoxFilterError::Cancelled => OperationExecutionError::Cancelled,
        CancellableBoxFilterError::Filter(error) => box_filter_error(error),
    }
}

fn box_filter_error(error: BoxFilterError) -> OperationExecutionError {
    match error {
        BoxFilterError::AllocationFailed { required_bytes } => {
            OperationExecutionError::AllocationFailed {
                required: required_bytes,
            }
        }
        BoxFilterError::SizeOverflow => OperationExecutionError::MemoryBudgetExceeded {
            required: usize::MAX,
            budget: ReconstructionBudget::default().maximum_bytes(),
        },
        BoxFilterError::BufferShape { expected, actual } => {
            OperationExecutionError::DimensionsMismatch { expected, actual }
        }
        BoxFilterError::NonFiniteInput { sample } => OperationExecutionError::NonFiniteResult {
            pixel: sample / SOFTEN_CHANNELS,
            channel: match sample % SOFTEN_CHANNELS {
                0 => crate::RgbChannel::Red,
                1 => crate::RgbChannel::Green,
                _ => crate::RgbChannel::Blue,
            },
        },
        BoxFilterError::InvalidDimensions { .. }
        | BoxFilterError::UnsupportedChannels { .. }
        | BoxFilterError::ScratchShape { .. } => OperationExecutionError::UnsupportedCapability(
            "box mean rejected a validated soften buffer",
        ),
    }
}

fn bounded(
    name: &'static str,
    value: f32,
    minimum: f32,
    maximum: f32,
) -> Result<FiniteF32, SoftenParameterError> {
    if !value.is_finite() {
        return Err(SoftenParameterError::NonFinite(name));
    }
    if !(minimum..=maximum).contains(&value) {
        return Err(SoftenParameterError::OutOfRange(name));
    }
    Ok(FiniteF32::new(value).expect("finite value was checked"))
}

fn soften_radius(
    size: f32,
    dimensions: RasterDimensions,
    roi_scale: f32,
    piece_scale: f32,
) -> Result<u32, OperationExecutionError> {
    if !roi_scale.is_finite() || roi_scale <= 0.0 {
        return Err(OperationExecutionError::UnsupportedCapability(
            "soften roi scale must be finite and positive",
        ));
    }
    if !piece_scale.is_finite() || piece_scale <= 0.0 {
        return Err(OperationExecutionError::UnsupportedCapability(
            "soften piece scale must be finite and positive",
        ));
    }

    // Keep the native float operations and integer truncation points:
    // `mrad` is truncated before the size multiplier, while `rad` uses the
    // double-precision fmin expression in soften.c and the ROI conversion
    // is a float ceilf expression.
    let width = dimensions.width() as f32 * piece_scale;
    let height = dimensions.height() as f32 * piece_scale;
    let maximum = width.hypot(height) * 0.01_f32;
    let base = maximum as u32;
    let capped_size = f64::from(size + 1.0_f32).min(100.0);
    let requested = (f64::from(base) * (capped_size / 100.0)) as u32;
    let scaled = (requested as f32 * roi_scale / piece_scale).ceil() as u32;
    Ok(base.min(scaled))
}

fn adjust_rgb(
    pixel: [f32; 3],
    saturation: f32,
    brightness: f32,
    index: usize,
) -> Result<[f32; 3], OperationExecutionError> {
    let (hue, mut saturation_value, mut lightness) = rgb_to_hsl(pixel);
    saturation_value = clip_native(saturation_value * saturation);
    lightness = clip_native(lightness * brightness);
    let output = hsl_to_rgb(hue, saturation_value, lightness);
    Ok([
        finite(output[0], index, crate::RgbChannel::Red)?.get(),
        finite(output[1], index, crate::RgbChannel::Green)?.get(),
        finite(output[2], index, crate::RgbChannel::Blue)?.get(),
    ])
}

/// Direct source port of `colorspaces.h::rgb2hsl`.
#[allow(
    clippy::excessive_precision,
    clippy::float_cmp,
    clippy::manual_midpoint,
    reason = "the retained C source fixes these f32 operations and exact comparisons"
)]
fn rgb_to_hsl(rgb: [f32; 3]) -> (f32, f32, f32) {
    let [red, green, blue] = rgb;
    let maximum = red.max(green).max(blue);
    let minimum = red.min(green).min(blue);
    let delta = maximum - minimum;
    let mut hue = 0.0_f32;
    let mut saturation = 0.0_f32;
    // In the retained expression the sum is formed as f32, then division by
    // the unsuffixed C literal 2.0 is performed as double and assigned back to
    // float.
    let lightness = (f64::from(minimum + maximum) / 2.0) as f32;

    if delta != 0.0_f32 {
        saturation = if lightness < 0.5_f32 {
            delta / (maximum + minimum).max(1.525_878_906_25e-5_f32)
        } else {
            // `2.0 - pmax - pmin` is a double expression before fmaxf
            // converts it back to float.
            let denominator = (2.0_f64 - f64::from(maximum) - f64::from(minimum)) as f32;
            delta / denominator.max(1.525_878_906_25e-5_f32)
        };
        let hue_angle = if maximum == red {
            (green - blue) / delta
        } else if maximum == green {
            (2.0_f64 + f64::from((blue - red) / delta)) as f32
        } else {
            (4.0_f64 + f64::from((red - green) / delta)) as f32
        };
        // `hv /= 6.0` and the wrap additions use the native double literal,
        // with each result assigned back to the float hue variable.
        hue = (f64::from(hue_angle) / 6.0) as f32;
        if f64::from(hue) < 0.0 {
            hue = (f64::from(hue) + 1.0) as f32;
        } else if f64::from(hue) > 1.0 {
            hue = (f64::from(hue) - 1.0) as f32;
        }
    }
    (hue, saturation, lightness)
}

/// Direct source port of `colorspaces.h::hsl2rgb`.
fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    if saturation == 0.0_f32 {
        return [lightness; 3];
    }
    let second = if lightness < 0.5_f32 {
        // The native `1.0` promotes this branch through double before the
        // result is assigned to its float temporary.
        (f64::from(lightness) * (1.0 + f64::from(saturation))) as f32
    } else {
        // No double literal occurs in the other native branch.
        lightness + saturation - lightness * saturation
    };
    let first = (2.0_f64 * f64::from(lightness) - f64::from(second)) as f32;
    let angle = hue * 6.0_f32;
    [
        hue_to_rgb(
            first,
            second,
            if angle < 4.0_f32 {
                angle + 2.0_f32
            } else {
                angle - 4.0_f32
            },
        ),
        hue_to_rgb(first, second, angle),
        hue_to_rgb(
            first,
            second,
            if angle > 2.0_f32 {
                angle - 2.0_f32
            } else {
                angle + 4.0_f32
            },
        ),
    ]
}

fn clip_native(value: f32) -> f32 {
    if value >= 0.0 {
        if value <= 1.0 { value } else { 1.0 }
    } else {
        0.0
    }
}

fn hue_to_rgb(first: f32, second: f32, hue: f32) -> f32 {
    if hue < 1.0 {
        first + (second - first) * hue
    } else if hue < 3.0 {
        second
    } else if hue < 4.0 {
        first + (second - first) * (4.0 - hue)
    } else {
        first
    }
}

fn finite(
    value: f32,
    pixel: usize,
    channel: crate::RgbChannel,
) -> Result<FiniteF32, OperationExecutionError> {
    FiniteF32::new(value).map_err(|_| OperationExecutionError::NonFiniteResult { pixel, channel })
}

fn try_reserve<T>(length: usize) -> Result<Vec<T>, OperationExecutionError> {
    let required = length.saturating_mul(std::mem::size_of::<T>());
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| OperationExecutionError::AllocationFailed { required })?;
    Ok(values)
}

fn try_clone<T: Clone>(input: &[T]) -> Result<Vec<T>, OperationExecutionError> {
    let mut output = try_reserve(input.len())?;
    output.extend_from_slice(input);
    Ok(output)
}
