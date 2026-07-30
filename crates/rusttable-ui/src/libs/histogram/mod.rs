//! GTK-independent histogram projection for a validated darkroom preview.
//!
//! The application worker owns the call to this module.  GTK receives only the bounded result,
//! so histogram aggregation never scans preview pixels from the main loop.

use crate::presentation::PreviewDimensions;

/// Number of stable buckets shown by the darkroom histogram.
pub const DARKROOM_HISTOGRAM_BINS: usize = 256;

/// Maximum number of pixels sampled for one histogram result.
pub const DARKROOM_HISTOGRAM_MAX_SAMPLES: usize = 1_000_000;

/// A channel represented by the darkroom histogram.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HistogramChannel {
    Red,
    Green,
    Blue,
    Luminance,
}

/// One bucket of the RGB/luminance histogram.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HistogramBin {
    red: u32,
    green: u32,
    blue: u32,
    luminance: u32,
}

impl HistogramBin {
    #[must_use]
    pub const fn red(self) -> u32 {
        self.red
    }

    #[must_use]
    pub const fn green(self) -> u32 {
        self.green
    }

    #[must_use]
    pub const fn blue(self) -> u32 {
        self.blue
    }

    #[must_use]
    pub const fn luminance(self) -> u32 {
        self.luminance
    }

    #[must_use]
    pub const fn channel(self, channel: HistogramChannel) -> u32 {
        match channel {
            HistogramChannel::Red => self.red,
            HistogramChannel::Green => self.green,
            HistogramChannel::Blue => self.blue,
            HistogramChannel::Luminance => self.luminance,
        }
    }
}

/// A deterministic, bounded histogram result for one preview frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistogramData {
    bins: Vec<HistogramBin>,
    total_pixels: usize,
    sampled_pixels: usize,
}

impl HistogramData {
    /// Adapts legacy pre-binned callers while keeping the live path on [`Self::from_rgba8`].
    pub(crate) fn from_rgb_bin_values(red: &[f32], green: &[f32], blue: &[f32]) -> Option<Self> {
        if red.is_empty()
            || red.len() != green.len()
            || red.len() != blue.len()
            || red.len() > DARKROOM_HISTOGRAM_BINS
        {
            return None;
        }
        let mut bins = vec![HistogramBin::default(); DARKROOM_HISTOGRAM_BINS];
        for (index, values) in red.iter().zip(green).zip(blue).enumerate() {
            let ((red, green), blue) = values;
            let red = finite_count(*red)?;
            let green = finite_count(*green)?;
            let blue = finite_count(*blue)?;
            bins[index] = HistogramBin {
                red,
                green,
                blue,
                luminance: red.max(green).max(blue),
            };
        }
        Some(Self {
            bins,
            total_pixels: red.len(),
            sampled_pixels: red.len(),
        })
    }

    /// Computes RGB and luminance buckets directly from validated RGBA8 display pixels.
    ///
    /// Alpha is intentionally ignored: the display frame is already composited for the
    /// viewport, and histogram values describe the visible RGB samples only.
    ///
    /// # Errors
    ///
    /// Returns [`HistogramError`] when the dimensions and byte payload do not match or are
    /// unrepresentable.
    pub fn from_rgba8(
        dimensions: PreviewDimensions,
        pixels: &[u8],
    ) -> Result<Self, HistogramError> {
        let expected = expected_bytes(dimensions)?;
        if pixels.len() != expected {
            return Err(HistogramError::IncorrectByteLength {
                expected,
                actual: pixels.len(),
            });
        }
        let total_pixels = expected / 4;
        if total_pixels == 0 {
            return Err(HistogramError::Empty);
        }
        let mut bins = vec![HistogramBin::default(); DARKROOM_HISTOGRAM_BINS];
        let stride = sample_stride(total_pixels);
        let mut sampled_pixels = 0;
        for pixel_index in (0..total_pixels).step_by(stride) {
            let offset = pixel_index * 4;
            let red = pixels[offset];
            let green = pixels[offset + 1];
            let blue = pixels[offset + 2];
            let luminance = luminance_u8(red, green, blue);
            let bin = &mut bins[usize::from(red)];
            bin.red = bin.red.saturating_add(1);
            bins[usize::from(green)].green = bins[usize::from(green)].green.saturating_add(1);
            bins[usize::from(blue)].blue = bins[usize::from(blue)].blue.saturating_add(1);
            bins[usize::from(luminance)].luminance =
                bins[usize::from(luminance)].luminance.saturating_add(1);
            sampled_pixels += 1;
        }
        Ok(Self {
            bins,
            total_pixels,
            sampled_pixels,
        })
    }

    /// Computes the same result from normalized interleaved RGB samples.
    ///
    /// This narrow data entry point keeps policy tests independent of GTK and makes malformed
    /// floating-point producer output explicit instead of silently turning it into a fake chart.
    ///
    /// # Errors
    ///
    /// Returns [`HistogramError`] when the dimensions and sample payload do not match or when a
    /// sampled channel is non-finite.
    pub fn from_rgb_f32(
        dimensions: PreviewDimensions,
        samples: &[f32],
    ) -> Result<Self, HistogramError> {
        let total_pixels = usize::try_from(dimensions.width())
            .ok()
            .and_then(|width| {
                usize::try_from(dimensions.height())
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or(HistogramError::SizeOverflow)?;
        let expected = total_pixels
            .checked_mul(3)
            .ok_or(HistogramError::SizeOverflow)?;
        if samples.len() != expected {
            return Err(HistogramError::IncorrectSampleLength {
                expected,
                actual: samples.len(),
            });
        }
        if total_pixels == 0 {
            return Err(HistogramError::Empty);
        }
        let mut bins = vec![HistogramBin::default(); DARKROOM_HISTOGRAM_BINS];
        let stride = sample_stride(total_pixels);
        let mut sampled_pixels = 0;
        for pixel_index in (0..total_pixels).step_by(stride) {
            let offset = pixel_index * 3;
            let red = samples[offset];
            let green = samples[offset + 1];
            let blue = samples[offset + 2];
            let values = [red, green, blue];
            for (channel_index, value) in values.into_iter().enumerate() {
                if !value.is_finite() {
                    return Err(HistogramError::NonFinite {
                        pixel_index,
                        channel: match channel_index {
                            0 => HistogramChannel::Red,
                            1 => HistogramChannel::Green,
                            _ => HistogramChannel::Blue,
                        },
                    });
                }
            }
            let red_bin = normalized_bin(red);
            let green_bin = normalized_bin(green);
            let blue_bin = normalized_bin(blue);
            let luminance_bin = normalized_bin(0.2126 * red + 0.7152 * green + 0.0722 * blue);
            bins[red_bin].red = bins[red_bin].red.saturating_add(1);
            bins[green_bin].green = bins[green_bin].green.saturating_add(1);
            bins[blue_bin].blue = bins[blue_bin].blue.saturating_add(1);
            bins[luminance_bin].luminance = bins[luminance_bin].luminance.saturating_add(1);
            sampled_pixels += 1;
        }
        Ok(Self {
            bins,
            total_pixels,
            sampled_pixels,
        })
    }

    #[must_use]
    pub fn bins(&self) -> &[HistogramBin] {
        &self.bins
    }

    #[must_use]
    pub const fn total_pixels(&self) -> usize {
        self.total_pixels
    }

    #[must_use]
    pub const fn sampled_pixels(&self) -> usize {
        self.sampled_pixels
    }

    #[must_use]
    pub fn maximum(&self) -> u32 {
        self.bins
            .iter()
            .flat_map(|bin| [bin.red, bin.green, bin.blue, bin.luminance])
            .max()
            .unwrap_or(0)
    }

    #[must_use]
    pub fn sample(&self, bin: usize) -> Option<HistogramSample> {
        self.bins
            .get(bin)
            .copied()
            .map(|values| HistogramSample { bin, values })
    }
}

/// A histogram bucket selected by a viewport sample gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistogramSample {
    bin: usize,
    values: HistogramBin,
}

impl HistogramSample {
    #[must_use]
    pub const fn bin(self) -> usize {
        self.bin
    }

    #[must_use]
    pub const fn values(self) -> HistogramBin {
        self.values
    }
}

/// Reasons a live histogram cannot truthfully be displayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistogramError {
    SizeOverflow,
    Empty,
    PreviewUnavailable,
    IncorrectByteLength {
        expected: usize,
        actual: usize,
    },
    IncorrectSampleLength {
        expected: usize,
        actual: usize,
    },
    NonFinite {
        pixel_index: usize,
        channel: HistogramChannel,
    },
}

fn expected_bytes(dimensions: PreviewDimensions) -> Result<usize, HistogramError> {
    let width = usize::try_from(dimensions.width()).map_err(|_| HistogramError::SizeOverflow)?;
    let height = usize::try_from(dimensions.height()).map_err(|_| HistogramError::SizeOverflow)?;
    width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(HistogramError::SizeOverflow)
}

fn sample_stride(total_pixels: usize) -> usize {
    total_pixels
        .saturating_add(DARKROOM_HISTOGRAM_MAX_SAMPLES - 1)
        .checked_div(DARKROOM_HISTOGRAM_MAX_SAMPLES)
        .unwrap_or(1)
        .max(1)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn normalized_bin(value: f32) -> usize {
    if value <= 0.0 {
        return 0;
    }
    if value >= 1.0 {
        return DARKROOM_HISTOGRAM_BINS - 1;
    }
    (value * DARKROOM_HISTOGRAM_BINS as f32) as usize
}

fn luminance_u8(red: u8, green: u8, blue: u8) -> u8 {
    let weighted =
        54_u32 * u32::from(red) + 183_u32 * u32::from(green) + 19_u32 * u32::from(blue) + 128;
    u8::try_from((weighted / 256).min(u32::from(u8::MAX))).unwrap_or(u8::MAX)
}

#[allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn finite_count(value: f32) -> Option<u32> {
    if !value.is_finite() || value.is_sign_negative() {
        return None;
    }
    Some(value.round().min(u32::MAX as f32) as u32)
}

#[cfg(test)]
mod tests {
    use super::{
        DARKROOM_HISTOGRAM_BINS, DARKROOM_HISTOGRAM_MAX_SAMPLES, HistogramChannel, HistogramData,
        HistogramError,
    };
    use crate::presentation::PreviewDimensions;

    fn dimensions(width: u32, height: u32) -> PreviewDimensions {
        PreviewDimensions::new(width, height).expect("test dimensions")
    }

    #[test]
    fn rgba_histogram_has_stable_rgb_and_luminance_bins() {
        let data = HistogramData::from_rgba8(dimensions(2, 1), &[255, 0, 0, 255, 0, 255, 0, 255])
            .expect("valid preview");
        assert_eq!(data.bins().len(), DARKROOM_HISTOGRAM_BINS);
        assert_eq!(data.bins()[255].red(), 1);
        assert_eq!(data.bins()[255].green(), 1);
        assert_eq!(data.bins()[54].luminance(), 1);
        assert_eq!(data.sampled_pixels(), 2);
    }

    #[test]
    fn malformed_and_nonfinite_inputs_are_explicit_failures() {
        assert!(matches!(
            HistogramData::from_rgba8(dimensions(2, 1), &[0; 7]),
            Err(HistogramError::IncorrectByteLength { .. })
        ));
        assert!(matches!(
            HistogramData::from_rgb_f32(dimensions(1, 1), &[0.0, f32::NAN, 1.0]),
            Err(HistogramError::NonFinite {
                channel: HistogramChannel::Green,
                ..
            })
        ));
    }

    #[test]
    fn large_input_uses_a_deterministic_bounded_sample() {
        let dimensions = dimensions(2_000, 1_000);
        let pixels = vec![128_u8; 2_000 * 1_000 * 4];
        let data = HistogramData::from_rgba8(dimensions, &pixels).expect("valid preview");
        assert!(data.sampled_pixels() <= DARKROOM_HISTOGRAM_MAX_SAMPLES);
        assert_eq!(
            data.sampled_pixels(),
            HistogramData::from_rgba8(dimensions, &pixels)
                .expect("same preview")
                .sampled_pixels()
        );
    }

    #[test]
    fn out_of_range_float_samples_are_clamped_not_fabricated() {
        let data = HistogramData::from_rgb_f32(dimensions(1, 1), &[-1.0, 2.0, 0.5])
            .expect("finite sample");
        assert_eq!(data.bins()[0].red(), 1);
        assert_eq!(data.bins()[255].green(), 1);
        assert_eq!(data.sample(128).expect("middle bucket").bin(), 128);
    }
}
