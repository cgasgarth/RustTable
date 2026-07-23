use rusttable_image::DecodedImage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualFingerprint {
    gradient: u64,
    luminance: u64,
    width: u32,
    height: u32,
}

impl VisualFingerprint {
    #[must_use]
    pub const fn new(gradient: u64, luminance: u64, width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        Some(Self {
            gradient,
            luminance,
            width,
            height,
        })
    }

    #[must_use]
    pub fn from_decoded(image: &DecodedImage) -> Self {
        let dimensions = image.oriented_dimensions();
        let mut samples = [[0_u8; 9]; 8];
        for (sample_y, row) in samples.iter_mut().enumerate() {
            for (sample_x, sample) in row.iter_mut().enumerate() {
                let output_x = sample_coordinate(sample_x, 9, dimensions.width());
                let output_y = sample_coordinate(sample_y, 8, dimensions.height());
                let (source_x, source_y) = image
                    .source_orientation()
                    .inverse()
                    .map_source_to_output(dimensions, output_x, output_y);
                *sample = luma(image, source_x, source_y);
            }
        }
        let total = samples
            .iter()
            .flat_map(|row| row.iter().take(8))
            .map(|value| u32::from(*value))
            .sum::<u32>();
        let average = total / 64;
        let mut gradient = 0_u64;
        let mut luminance = 0_u64;
        for (y, row) in samples.iter().enumerate() {
            for x in 0..8 {
                let bit = y * 8 + x;
                if row[x] > row[x + 1] {
                    gradient |= 1_u64 << bit;
                }
                if u32::from(row[x]) >= average {
                    luminance |= 1_u64 << bit;
                }
            }
        }
        Self {
            gradient,
            luminance,
            width: dimensions.width(),
            height: dimensions.height(),
        }
    }

    #[must_use]
    pub const fn gradient(self) -> u64 {
        self.gradient
    }

    #[must_use]
    pub const fn luminance(self) -> u64 {
        self.luminance
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
    pub const fn distance(self, other: Self) -> u32 {
        (self.gradient ^ other.gradient).count_ones()
            + (self.luminance ^ other.luminance).count_ones()
    }

    #[must_use]
    pub fn has_similar_aspect(self, other: Self) -> bool {
        let left = u128::from(self.width) * u128::from(other.height);
        let right = u128::from(other.width) * u128::from(self.height);
        left.abs_diff(right) * 100 <= left.max(right)
    }

    #[must_use]
    pub const fn index_chunks(self) -> [u16; 8] {
        let gradient = self.gradient.to_be_bytes();
        let luminance = self.luminance.to_be_bytes();
        [
            u16::from_be_bytes([gradient[0], gradient[1]]),
            u16::from_be_bytes([gradient[2], gradient[3]]),
            u16::from_be_bytes([gradient[4], gradient[5]]),
            u16::from_be_bytes([gradient[6], gradient[7]]),
            u16::from_be_bytes([luminance[0], luminance[1]]),
            u16::from_be_bytes([luminance[2], luminance[3]]),
            u16::from_be_bytes([luminance[4], luminance[5]]),
            u16::from_be_bytes([luminance[6], luminance[7]]),
        ]
    }
}

fn sample_coordinate(index: usize, count: u32, length: u32) -> u32 {
    let index = u32::try_from(index).unwrap_or_default();
    (index * length + length / 2) / count
}

fn luma(image: &DecodedImage, x: u32, y: u32) -> u8 {
    let width = u64::from(image.dimensions().width());
    let offset = (u64::from(y) * width + u64::from(x)) * 4;
    let Ok(offset) = usize::try_from(offset) else {
        return 0;
    };
    let Some(pixel) = image.pixels().get(offset..offset + 4) else {
        return 0;
    };
    let weighted = 77 * u32::from(pixel[0]) + 150 * u32::from(pixel[1]) + 29 * u32::from(pixel[2]);
    let alpha_composited = (weighted / 256) * u32::from(pixel[3]) / 255;
    u8::try_from(alpha_composited).unwrap_or(u8::MAX)
}
