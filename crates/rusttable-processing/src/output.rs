use crate::{FiniteF32, RasterDimensions, RgbChannel, SrgbChannel, WorkingRgbImage};

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
