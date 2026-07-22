use std::fmt;

use crate::{FiniteF32, FiniteF32Error};
pub use rusttable_color::ColorEncoding;
use rusttable_color::{Primaries, ProfileId, TransferFunction, WhitePoint};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub type SourceColorSpace = ColorEncoding;
pub type WorkingColorSpace = ColorEncoding;

/// Provenance for the profile currently installed in a working frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WorkingProfileProvenance {
    Selected,
    FallbackRec2020,
}

/// The immutable color contract carried by every linear working frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkingFrameDescriptor {
    encoding: ColorEncoding,
    primaries: Primaries,
    white_point: WhitePoint,
    transfer: TransferFunction,
    profile_id: Option<ProfileId>,
    provenance: WorkingProfileProvenance,
}

impl WorkingFrameDescriptor {
    #[must_use]
    pub fn new(
        encoding: ColorEncoding,
        primaries: Primaries,
        white_point: WhitePoint,
        transfer: TransferFunction,
        profile_id: Option<ProfileId>,
        provenance: WorkingProfileProvenance,
    ) -> Self {
        debug_assert_eq!(transfer, TransferFunction::Linear);
        Self {
            encoding,
            primaries,
            white_point,
            transfer,
            profile_id,
            provenance,
        }
    }

    #[must_use]
    pub const fn srgb() -> Self {
        Self::builtin(
            rusttable_color::BuiltinSpace::SrgbD65,
            WorkingProfileProvenance::Selected,
        )
    }

    #[must_use]
    pub const fn rec2020() -> Self {
        Self::builtin(
            rusttable_color::BuiltinSpace::Rec2020D65,
            WorkingProfileProvenance::Selected,
        )
    }

    #[must_use]
    pub const fn fallback_rec2020() -> Self {
        Self::builtin(
            rusttable_color::BuiltinSpace::Rec2020D65,
            WorkingProfileProvenance::FallbackRec2020,
        )
    }

    #[must_use]
    pub const fn builtin(
        space: rusttable_color::BuiltinSpace,
        provenance: WorkingProfileProvenance,
    ) -> Self {
        let primaries = match space.primaries() {
            Some(value) => value,
            None => Primaries::srgb(),
        };
        Self {
            encoding: space.encoding(true),
            primaries,
            white_point: space.white_point(),
            transfer: TransferFunction::Linear,
            profile_id: None,
            provenance,
        }
    }

    #[must_use]
    pub const fn encoding(self) -> ColorEncoding {
        self.encoding
    }

    #[must_use]
    pub const fn primaries(self) -> Primaries {
        self.primaries
    }

    #[must_use]
    pub const fn white_point(self) -> WhitePoint {
        self.white_point
    }

    #[must_use]
    pub const fn transfer(self) -> TransferFunction {
        self.transfer
    }

    #[must_use]
    pub const fn profile_id(self) -> Option<ProfileId> {
        self.profile_id
    }

    #[must_use]
    pub const fn provenance(self) -> WorkingProfileProvenance {
        self.provenance
    }

    #[must_use]
    /// # Panics
    ///
    /// Panics only if the closed working-frame descriptor cannot be serialized,
    /// which indicates an internal contract change without a schema update.
    pub fn identity(self) -> [u8; 32] {
        Sha256::digest(postcard::to_allocvec(&self).expect("working descriptor serializes")).into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RasterDimensions {
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RasterDimensionsError {
    ZeroWidth,
    ZeroHeight,
}

impl RasterDimensions {
    /// Creates nonzero raster dimensions.
    ///
    /// # Errors
    ///
    /// Returns a distinct error for a zero width or zero height.
    pub const fn new(width: u32, height: u32) -> Result<Self, RasterDimensionsError> {
        if width == 0 {
            return Err(RasterDimensionsError::ZeroWidth);
        }
        if height == 0 {
            return Err(RasterDimensionsError::ZeroHeight);
        }
        Ok(Self { width, height })
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
    pub fn pixel_count(self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RgbChannel {
    Red,
    Green,
    Blue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SrgbChannelError {
    NonFinite,
    BelowZero,
    AboveOne,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SrgbChannel(FiniteF32);

impl SrgbChannel {
    /// Creates a normalized transfer-encoded sRGB channel.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is non-finite or outside `0.0..=1.0`.
    pub fn new(value: f32) -> Result<Self, SrgbChannelError> {
        let value =
            FiniteF32::new(value).map_err(|_: FiniteF32Error| SrgbChannelError::NonFinite)?;
        if value.get() < 0.0 {
            return Err(SrgbChannelError::BelowZero);
        }
        if value.get() > 1.0 {
            return Err(SrgbChannelError::AboveOne);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> f32 {
        self.0.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayP3ChannelError {
    NonFinite,
    BelowZero,
    AboveOne,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DisplayP3Channel(FiniteF32);

impl DisplayP3Channel {
    /// Creates a normalized transfer-encoded Display P3 channel.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is non-finite or outside `0.0..=1.0`.
    pub fn new(value: f32) -> Result<Self, DisplayP3ChannelError> {
        let value =
            FiniteF32::new(value).map_err(|_: FiniteF32Error| DisplayP3ChannelError::NonFinite)?;
        if value.get() < 0.0 {
            return Err(DisplayP3ChannelError::BelowZero);
        }
        if value.get() > 1.0 {
            return Err(DisplayP3ChannelError::AboveOne);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> f32 {
        self.0.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceRgb {
    red: SrgbChannel,
    green: SrgbChannel,
    blue: SrgbChannel,
}

impl SourceRgb {
    #[must_use]
    pub const fn new(red: SrgbChannel, green: SrgbChannel, blue: SrgbChannel) -> Self {
        Self { red, green, blue }
    }

    #[must_use]
    pub const fn red(self) -> SrgbChannel {
        self.red
    }

    #[must_use]
    pub const fn green(self) -> SrgbChannel {
        self.green
    }

    #[must_use]
    pub const fn blue(self) -> SrgbChannel {
        self.blue
    }

    #[must_use]
    pub const fn channel(self, channel: RgbChannel) -> SrgbChannel {
        match channel {
            RgbChannel::Red => self.red,
            RgbChannel::Green => self.green,
            RgbChannel::Blue => self.blue,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LinearRgb {
    red: FiniteF32,
    green: FiniteF32,
    blue: FiniteF32,
}

impl LinearRgb {
    #[must_use]
    pub const fn new(red: FiniteF32, green: FiniteF32, blue: FiniteF32) -> Self {
        Self { red, green, blue }
    }

    #[must_use]
    pub const fn red(self) -> FiniteF32 {
        self.red
    }

    #[must_use]
    pub const fn green(self) -> FiniteF32 {
        self.green
    }

    #[must_use]
    pub const fn blue(self) -> FiniteF32 {
        self.blue
    }

    #[must_use]
    pub const fn channel(self, channel: RgbChannel) -> FiniteF32 {
        match channel {
            RgbChannel::Red => self.red,
            RgbChannel::Green => self.green,
            RgbChannel::Blue => self.blue,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageBuildError {
    PixelCountMismatch { expected: u64, actual: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRgbImage {
    dimensions: RasterDimensions,
    pixels: Vec<SourceRgb>,
}

impl SourceRgbImage {
    /// Creates a row-major normalized sRGB image without allocating from dimensions.
    ///
    /// # Errors
    ///
    /// Returns [`ImageBuildError::PixelCountMismatch`] when the supplied pixels
    /// do not exactly cover the dimensions.
    pub fn new(
        dimensions: RasterDimensions,
        pixels: Vec<SourceRgb>,
    ) -> Result<Self, ImageBuildError> {
        if u64::try_from(pixels.len()) != Ok(dimensions.pixel_count()) {
            return Err(ImageBuildError::PixelCountMismatch {
                expected: dimensions.pixel_count(),
                actual: pixels.len(),
            });
        }
        Ok(Self { dimensions, pixels })
    }

    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn space(&self) -> SourceColorSpace {
        SourceColorSpace::Srgb
    }

    #[must_use]
    pub fn pixel_slice(&self) -> &[SourceRgb] {
        &self.pixels
    }

    pub fn pixels(&self) -> impl Iterator<Item = &SourceRgb> {
        self.pixels.iter()
    }

    #[must_use]
    pub fn pixel(&self, index: usize) -> Option<&SourceRgb> {
        self.pixels.get(index)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DisplayP3Rgb {
    red: DisplayP3Channel,
    green: DisplayP3Channel,
    blue: DisplayP3Channel,
}

impl DisplayP3Rgb {
    #[must_use]
    pub const fn new(
        red: DisplayP3Channel,
        green: DisplayP3Channel,
        blue: DisplayP3Channel,
    ) -> Self {
        Self { red, green, blue }
    }

    #[must_use]
    pub const fn red(self) -> DisplayP3Channel {
        self.red
    }

    #[must_use]
    pub const fn green(self) -> DisplayP3Channel {
        self.green
    }

    #[must_use]
    pub const fn blue(self) -> DisplayP3Channel {
        self.blue
    }

    #[must_use]
    pub const fn channel(self, channel: RgbChannel) -> DisplayP3Channel {
        match channel {
            RgbChannel::Red => self.red,
            RgbChannel::Green => self.green,
            RgbChannel::Blue => self.blue,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayP3RgbImage {
    dimensions: RasterDimensions,
    pixels: Vec<DisplayP3Rgb>,
}

impl DisplayP3RgbImage {
    /// Creates a row-major normalized Display P3 image.
    ///
    /// # Errors
    ///
    /// Returns [`ImageBuildError::PixelCountMismatch`] when the supplied
    /// pixels do not exactly cover the dimensions.
    pub fn new(
        dimensions: RasterDimensions,
        pixels: Vec<DisplayP3Rgb>,
    ) -> Result<Self, ImageBuildError> {
        if u64::try_from(pixels.len()) != Ok(dimensions.pixel_count()) {
            return Err(ImageBuildError::PixelCountMismatch {
                expected: dimensions.pixel_count(),
                actual: pixels.len(),
            });
        }
        Ok(Self { dimensions, pixels })
    }

    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn space(&self) -> SourceColorSpace {
        SourceColorSpace::DisplayP3
    }

    #[must_use]
    pub fn pixel_slice(&self) -> &[DisplayP3Rgb] {
        &self.pixels
    }

    pub fn pixels(&self) -> impl Iterator<Item = &DisplayP3Rgb> {
        self.pixels.iter()
    }

    #[must_use]
    pub fn pixel(&self, index: usize) -> Option<&DisplayP3Rgb> {
        self.pixels.get(index)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingRgbImage {
    dimensions: RasterDimensions,
    pixels: Vec<LinearRgb>,
    frame: WorkingFrameDescriptor,
}

impl WorkingRgbImage {
    /// Creates a row-major linear-light sRGB working image.
    ///
    /// # Errors
    ///
    /// Returns [`ImageBuildError::PixelCountMismatch`] when the supplied pixels
    /// do not exactly cover the dimensions.
    pub fn new(
        dimensions: RasterDimensions,
        pixels: Vec<LinearRgb>,
    ) -> Result<Self, ImageBuildError> {
        if u64::try_from(pixels.len()) != Ok(dimensions.pixel_count()) {
            return Err(ImageBuildError::PixelCountMismatch {
                expected: dimensions.pixel_count(),
                actual: pixels.len(),
            });
        }
        Ok(Self {
            dimensions,
            pixels,
            frame: WorkingFrameDescriptor::srgb(),
        })
    }

    /// # Errors
    ///
    /// Returns an error when the pixel count does not match the dimensions.
    pub fn new_with_frame(
        dimensions: RasterDimensions,
        pixels: Vec<LinearRgb>,
        frame: WorkingFrameDescriptor,
    ) -> Result<Self, ImageBuildError> {
        if u64::try_from(pixels.len()) != Ok(dimensions.pixel_count()) {
            return Err(ImageBuildError::PixelCountMismatch {
                expected: dimensions.pixel_count(),
                actual: pixels.len(),
            });
        }
        Ok(Self {
            dimensions,
            pixels,
            frame,
        })
    }

    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn space(&self) -> WorkingColorSpace {
        self.frame.encoding()
    }

    #[must_use]
    pub const fn frame(&self) -> WorkingFrameDescriptor {
        self.frame
    }

    #[must_use]
    pub fn pixel_slice(&self) -> &[LinearRgb] {
        &self.pixels
    }

    pub fn pixels(&self) -> impl Iterator<Item = &LinearRgb> {
        self.pixels.iter()
    }

    #[must_use]
    pub fn pixel(&self, index: usize) -> Option<&LinearRgb> {
        self.pixels.get(index)
    }

    pub(crate) fn from_validated_parts_with_frame(
        dimensions: RasterDimensions,
        pixels: Vec<LinearRgb>,
        frame: WorkingFrameDescriptor,
    ) -> Self {
        Self {
            dimensions,
            pixels,
            frame,
        }
    }
}

/// Converts normalized transfer-encoded sRGB to linear-light sRGB.
///
/// This establishes a linear-light working space while making no claim that
/// decoded sRGB is scene-referred and does not perform ICC or display conversion.
#[must_use]
pub fn to_linear_srgb(source: &SourceRgbImage) -> WorkingRgbImage {
    let pixels = source
        .pixels
        .iter()
        .map(|pixel| {
            LinearRgb::new(
                decode_channel(pixel.red()),
                decode_channel(pixel.green()),
                decode_channel(pixel.blue()),
            )
        })
        .collect();
    WorkingRgbImage {
        dimensions: source.dimensions,
        pixels,
        frame: WorkingFrameDescriptor::srgb(),
    }
}

/// Preserves already-linear sRGB samples at the working-space boundary.
#[must_use]
pub fn linear_srgb_to_working(source: &SourceRgbImage) -> WorkingRgbImage {
    let pixels = source
        .pixels
        .iter()
        .map(|pixel| {
            LinearRgb::new(
                FiniteF32::from_proven_finite(pixel.red().get()),
                FiniteF32::from_proven_finite(pixel.green().get()),
                FiniteF32::from_proven_finite(pixel.blue().get()),
            )
        })
        .collect();
    WorkingRgbImage {
        dimensions: source.dimensions,
        pixels,
        frame: WorkingFrameDescriptor::srgb(),
    }
}

/// Converts declared Display P3 values to unclipped linear sRGB.
///
/// The transfer curve is the sRGB/Display P3 curve. The fixed D65 matrix is
/// the Display P3-to-sRGB primary conversion from the CSS Color 4 matrices;
/// both spaces use D65, so no chromatic adaptation is performed. Coefficients
/// and multiplication order are intentionally RustTable-owned and fixed.
#[must_use]
pub fn to_linear_srgb_from_display_p3(source: &DisplayP3RgbImage) -> WorkingRgbImage {
    display_p3_to_working(source, true)
}

/// Converts already-linear Display P3 values to unclipped linear sRGB.
#[must_use]
pub fn linear_display_p3_to_working(source: &DisplayP3RgbImage) -> WorkingRgbImage {
    display_p3_to_working(source, false)
}

fn display_p3_to_working(
    source: &DisplayP3RgbImage,
    decode_transfer_function: bool,
) -> WorkingRgbImage {
    const RED: [f32; 3] = [1.224_940_2, -0.224_940_18, 0.0];
    const GREEN: [f32; 3] = [-0.042_056_955, 1.042_057, 0.0];
    const BLUE: [f32; 3] = [-0.019_637_555, -0.078_636_05, 1.098_273_6];
    let pixels = source
        .pixels
        .iter()
        .map(|pixel| {
            let decode = |value| {
                if decode_transfer_function {
                    decode_transfer(value).get()
                } else {
                    value
                }
            };
            let red = decode(pixel.red().get());
            let green = decode(pixel.green().get());
            let blue = decode(pixel.blue().get());
            LinearRgb::new(
                FiniteF32::from_proven_finite(RED[0] * red + RED[1] * green + RED[2] * blue),
                FiniteF32::from_proven_finite(GREEN[0] * red + GREEN[1] * green + GREEN[2] * blue),
                FiniteF32::from_proven_finite(BLUE[0] * red + BLUE[1] * green + BLUE[2] * blue),
            )
        })
        .collect();
    WorkingRgbImage {
        dimensions: source.dimensions,
        pixels,
        frame: WorkingFrameDescriptor::srgb(),
    }
}

fn decode_channel(channel: SrgbChannel) -> FiniteF32 {
    decode_transfer(channel.get())
}

fn decode_transfer(encoded: f32) -> FiniteF32 {
    let linear = if encoded <= 0.04045 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    };
    FiniteF32::from_proven_finite(linear)
}

impl fmt::Display for RasterDimensionsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroWidth => formatter.write_str("raster width must be nonzero"),
            Self::ZeroHeight => formatter.write_str("raster height must be nonzero"),
        }
    }
}

impl std::error::Error for RasterDimensionsError {}

impl fmt::Display for SrgbChannelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite => formatter.write_str("sRGB channel must be finite"),
            Self::BelowZero => formatter.write_str("sRGB channel must not be below zero"),
            Self::AboveOne => formatter.write_str("sRGB channel must not be above one"),
        }
    }
}

impl std::error::Error for SrgbChannelError {}

impl fmt::Display for DisplayP3ChannelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite => formatter.write_str("Display P3 channel must be finite"),
            Self::BelowZero => formatter.write_str("Display P3 channel must not be below zero"),
            Self::AboveOne => formatter.write_str("Display P3 channel must not be above one"),
        }
    }
}

impl std::error::Error for DisplayP3ChannelError {}

impl fmt::Display for ImageBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PixelCountMismatch { expected, actual } => write!(
                formatter,
                "raster dimensions require {expected} pixels but received {actual}"
            ),
        }
    }
}

impl std::error::Error for ImageBuildError {}
