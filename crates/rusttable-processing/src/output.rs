use crate::operations::colorout::{ColorOutConfig, ColorOutPlan};
use crate::{
    FiniteF32, RasterDimensions, RgbChannel, SrgbChannel, WorkingFrameDescriptor, WorkingRgbImage,
};
use rusttable_color::{BuiltinSpace, ColorEncoding};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EncodedSrgb {
    red: SrgbChannel,
    green: SrgbChannel,
    blue: SrgbChannel,
}

impl EncodedSrgb {
    fn new(red: SrgbChannel, green: SrgbChannel, blue: SrgbChannel) -> Self {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedSrgbImage {
    dimensions: RasterDimensions,
    pixels: Vec<EncodedSrgb>,
}

impl EncodedSrgbImage {
    fn new(dimensions: RasterDimensions, pixels: Vec<EncodedSrgb>) -> Self {
        Self { dimensions, pixels }
    }

    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub fn pixel_slice(&self) -> &[EncodedSrgb] {
        &self.pixels
    }

    pub fn pixels(&self) -> impl Iterator<Item = &EncodedSrgb> {
        self.pixels.iter()
    }

    #[must_use]
    pub fn pixel(&self, index: usize) -> Option<&EncodedSrgb> {
        self.pixels.get(index)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChannelCounts {
    red: u64,
    green: u64,
    blue: u64,
}

impl ChannelCounts {
    #[must_use]
    pub const fn red(self) -> u64 {
        self.red
    }

    #[must_use]
    pub const fn green(self) -> u64 {
        self.green
    }

    #[must_use]
    pub const fn blue(self) -> u64 {
        self.blue
    }

    fn increment(&mut self, channel: RgbChannel) {
        let count = match channel {
            RgbChannel::Red => &mut self.red,
            RgbChannel::Green => &mut self.green,
            RgbChannel::Blue => &mut self.blue,
        };
        *count = count.saturating_add(1);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GamutClipReport {
    below_zero: ChannelCounts,
    above_one: ChannelCounts,
}

impl GamutClipReport {
    #[must_use]
    pub const fn below_zero(self) -> ChannelCounts {
        self.below_zero
    }

    #[must_use]
    pub const fn above_one(self) -> ChannelCounts {
        self.above_one
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedSrgbOutput {
    image: EncodedSrgbImage,
    clipping: GamutClipReport,
}

impl EncodedSrgbOutput {
    #[must_use]
    pub const fn image(&self) -> &EncodedSrgbImage {
        &self.image
    }

    #[must_use]
    pub const fn clipping(&self) -> GamutClipReport {
        self.clipping
    }
}

/// Encodes a finite linear-light sRGB working image at the bounded output boundary.
///
/// The result is transfer-encoded sRGB for a later output bridge. It is not an
/// ICC display transform, proofing transform, tone mapper, or encoded file.
#[must_use]
pub fn encode_linear_srgb(input: &WorkingRgbImage) -> EncodedSrgbOutput {
    let mut clipping = GamutClipReport::default();
    let pixels = input
        .pixels()
        .map(|pixel| {
            EncodedSrgb::new(
                encode_channel(pixel.red(), RgbChannel::Red, &mut clipping),
                encode_channel(pixel.green(), RgbChannel::Green, &mut clipping),
                encode_channel(pixel.blue(), RgbChannel::Blue, &mut clipping),
            )
        })
        .collect();

    EncodedSrgbOutput {
        image: EncodedSrgbImage::new(input.dimensions(), pixels),
        clipping,
    }
}

/// Converts the active working profile to transfer-encoded sRGB for preview.
///
/// Unlike [`encode_linear_srgb`], this applies the profile-aware output matrix
/// before the sRGB transfer curve. The descriptor is part of the conversion,
/// so a Rec.2020 frame cannot be silently interpreted as linear sRGB.
///
/// # Panics
///
/// Panics only if an internally validated working descriptor cannot produce a
/// finite sRGB output plan.
#[must_use]
pub fn encode_working_to_srgb(input: &WorkingRgbImage) -> EncodedSrgbOutput {
    if input.frame() == WorkingFrameDescriptor::srgb() {
        return encode_linear_srgb(input);
    }
    let plan = ColorOutPlan::new_with_working_frame(
        ColorOutConfig::builtin(BuiltinSpace::SrgbD65),
        input.frame(),
    )
    .expect("validated working descriptor plans to sRGB");
    let execution = plan
        .execute(input.pixel_slice())
        .expect("finite working pixels transform to sRGB");
    let mut clipping = GamutClipReport::default();
    let pixels = execution
        .pixels()
        .iter()
        .map(|pixel| {
            EncodedSrgb::new(
                clamp_encoded(pixel.red().get(), RgbChannel::Red, &mut clipping),
                clamp_encoded(pixel.green().get(), RgbChannel::Green, &mut clipping),
                clamp_encoded(pixel.blue().get(), RgbChannel::Blue, &mut clipping),
            )
        })
        .collect();
    EncodedSrgbOutput {
        image: EncodedSrgbImage::new(input.dimensions(), pixels),
        clipping,
    }
}

/// Converts the active working profile to linear sRGB for a file-output
/// boundary while retaining the explicit sRGB frame descriptor.
///
/// # Panics
///
/// Panics only if an internally validated working descriptor cannot produce a
/// finite sRGB output plan.
#[must_use]
pub fn convert_working_to_linear_srgb(input: &WorkingRgbImage) -> WorkingRgbImage {
    if input.frame() == WorkingFrameDescriptor::srgb()
        || input.frame().encoding() == ColorEncoding::LinearSrgbD65
            && input.frame().primaries() == WorkingFrameDescriptor::srgb().primaries()
            && input.frame().white_point() == WorkingFrameDescriptor::srgb().white_point()
    {
        return input.clone();
    }
    let plan = ColorOutPlan::new_with_working_frame(
        ColorOutConfig::builtin(BuiltinSpace::SrgbD65),
        input.frame(),
    )
    .expect("validated working descriptor plans to sRGB");
    let execution = plan
        .execute(input.pixel_slice())
        .expect("finite working pixels transform to sRGB");
    let decode = |value: f32| {
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let pixels = execution
        .pixels()
        .iter()
        .map(|pixel| {
            crate::LinearRgb::new(
                FiniteF32::new(decode(pixel.red().get())).expect("finite red"),
                FiniteF32::new(decode(pixel.green().get())).expect("finite green"),
                FiniteF32::new(decode(pixel.blue().get())).expect("finite blue"),
            )
        })
        .collect();
    WorkingRgbImage::new_with_frame(input.dimensions(), pixels, WorkingFrameDescriptor::srgb())
        .expect("converted pixel count matches input")
}

fn clamp_encoded(value: f32, channel: RgbChannel, clipping: &mut GamutClipReport) -> SrgbChannel {
    let clipped = if value < 0.0 {
        clipping.below_zero.increment(channel);
        0.0
    } else if value > 1.0 {
        clipping.above_one.increment(channel);
        1.0
    } else {
        value
    };
    SrgbChannel::new(clipped).expect("clipped finite encoded sRGB is normalized")
}

fn encode_channel(
    channel: FiniteF32,
    channel_kind: RgbChannel,
    clipping: &mut GamutClipReport,
) -> SrgbChannel {
    let value = channel.get();
    let clipped = if value < 0.0 {
        clipping.below_zero.increment(channel_kind);
        0.0
    } else if value > 1.0 {
        clipping.above_one.increment(channel_kind);
        1.0
    } else {
        value
    };
    let encoded = if clipped.to_bits() == 0.0f32.to_bits() {
        0.0
    } else if clipped.to_bits() == 1.0f32.to_bits() {
        1.0
    } else if clipped <= 0.003_130_8 {
        12.92 * clipped
    } else {
        1.055 * clipped.powf(1.0 / 2.4) - 0.055
    };
    SrgbChannel::new(encoded).expect("clipped finite linear sRGB encodes to normalized sRGB")
}
