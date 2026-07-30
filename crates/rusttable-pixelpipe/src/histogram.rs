use std::{fmt, num::NonZeroU32};

use rusttable_image::{ImageDimensions, Roi};

/// The channel interpretation of samples supplied to histogram aggregation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HistogramChannelModel {
    /// One raw sensor or grayscale channel.
    Raw,
    /// Three red, green, and blue channels.
    Rgb,
    /// Three CIE L*, a*, and b* channels.
    Lab,
}

impl HistogramChannelModel {
    /// Uppercase aliases for callers mirroring the channel-model notation.
    pub const RAW: Self = Self::Raw;
    /// Uppercase aliases for callers mirroring the channel-model notation.
    pub const RGB: Self = Self::Rgb;
    /// Uppercase aliases for callers mirroring the channel-model notation.
    pub const LAB: Self = Self::Lab;

    /// Returns the number of interleaved samples per pixel.
    #[must_use]
    pub const fn channel_count(self) -> usize {
        match self {
            Self::Raw => 1,
            Self::Rgb | Self::Lab => 3,
        }
    }

    /// Returns the logical channels in their stable result order.
    #[must_use]
    pub const fn channels(self) -> &'static [HistogramChannel] {
        match self {
            Self::Raw => &[HistogramChannel::Raw],
            Self::Rgb => &[
                HistogramChannel::Red,
                HistogramChannel::Green,
                HistogramChannel::Blue,
            ],
            Self::Lab => &[
                HistogramChannel::Lightness,
                HistogramChannel::LabA,
                HistogramChannel::LabB,
            ],
        }
    }
}

/// A named histogram channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HistogramChannel {
    /// The single channel in a raw or grayscale model.
    Raw,
    Red,
    Green,
    Blue,
    Lightness,
    LabA,
    LabB,
}

impl HistogramChannel {}

/// Inclusive lower and exclusive upper bounds for histogram binning.
/// Values below or above the range are clamped into the first or last bin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HistogramRange {
    minimum: f32,
    maximum: f32,
}

impl HistogramRange {
    /// Creates a finite, strictly increasing range.
    ///
    /// # Errors
    ///
    /// Returns [`HistogramRangeError`] for non-finite or non-increasing bounds.
    pub fn new(minimum: f32, maximum: f32) -> Result<Self, HistogramRangeError> {
        if !minimum.is_finite() || !maximum.is_finite() {
            return Err(HistogramRangeError::NonFinite);
        }
        if minimum >= maximum {
            return Err(HistogramRangeError::NotIncreasing);
        }
        Ok(Self { minimum, maximum })
    }

    #[must_use]
    pub const fn minimum(self) -> f32 {
        self.minimum
    }

    #[must_use]
    pub const fn maximum(self) -> f32 {
        self.maximum
    }
}

/// Rejection reason for an invalid histogram range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistogramRangeError {
    NonFinite,
    NotIncreasing,
}

/// How a supplied mask controls sample inclusion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HistogramMaskPolicy {
    /// Ignore a supplied mask and include every sample in the ROI.
    #[default]
    Ignore,
    /// Include samples whose mask value is greater than zero.
    IncludeNonZero,
    /// Include samples whose mask value is zero.
    ExcludeNonZero,
    /// Require a mask to be supplied, then include all its samples.
    Require,
}

/// How non-finite channel values are handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HistogramNonFinitePolicy {
    /// Omit a pixel when any selected channel is non-finite.
    #[default]
    Skip,
    /// Fail the aggregation when any selected channel is non-finite.
    Reject,
}

/// Immutable typed input for histogram aggregation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HistogramRaster<'a> {
    dimensions: ImageDimensions,
    model: HistogramChannelModel,
    samples: &'a [f32],
    mask: Option<&'a [f32]>,
}

impl<'a> HistogramRaster<'a> {
    /// Creates an interleaved raster without a mask.
    ///
    /// # Errors
    ///
    /// Returns [`HistogramRasterError`] if the sample count is not exact.
    pub fn new(
        dimensions: ImageDimensions,
        model: HistogramChannelModel,
        samples: &'a [f32],
    ) -> Result<Self, HistogramRasterError> {
        Self::with_mask(dimensions, model, samples, None)
    }

    /// Creates an interleaved raster and optional one-value-per-pixel mask.
    ///
    /// # Errors
    ///
    /// Returns [`HistogramRasterError`] if the sample or mask count is not exact.
    pub fn with_mask(
        dimensions: ImageDimensions,
        model: HistogramChannelModel,
        samples: &'a [f32],
        mask: Option<&'a [f32]>,
    ) -> Result<Self, HistogramRasterError> {
        let pixels = usize::try_from(
            dimensions
                .pixel_count()
                .map_err(|_| HistogramRasterError::SizeOverflow)?,
        )
        .map_err(|_| HistogramRasterError::SizeOverflow)?;
        let expected_samples = pixels
            .checked_mul(model.channel_count())
            .ok_or(HistogramRasterError::SizeOverflow)?;
        if samples.len() != expected_samples {
            return Err(HistogramRasterError::SampleCount {
                expected: expected_samples,
                actual: samples.len(),
            });
        }
        if let Some(mask) = mask
            && mask.len() != pixels
        {
            return Err(HistogramRasterError::MaskCount {
                expected: pixels,
                actual: mask.len(),
            });
        }
        Ok(Self {
            dimensions,
            model,
            samples,
            mask,
        })
    }

    #[must_use]
    pub const fn dimensions(self) -> ImageDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn model(self) -> HistogramChannelModel {
        self.model
    }

    #[must_use]
    pub const fn samples(self) -> &'a [f32] {
        self.samples
    }

    #[must_use]
    pub const fn mask(self) -> Option<&'a [f32]> {
        self.mask
    }
}

/// Rejection reason for malformed histogram input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistogramRasterError {
    SizeOverflow,
    SampleCount { expected: usize, actual: usize },
    MaskCount { expected: usize, actual: usize },
}

/// A complete histogram request, including its spatial and sample policies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HistogramRequest {
    model: HistogramChannelModel,
    bins: NonZeroU32,
    range: HistogramRange,
    roi: Roi,
    mask_policy: HistogramMaskPolicy,
    nonfinite_policy: HistogramNonFinitePolicy,
}

impl HistogramRequest {
    /// Creates a checked histogram request.
    ///
    /// # Errors
    ///
    /// Returns [`HistogramRequestError::ZeroBins`] when `bins` is zero.
    pub fn new(
        model: HistogramChannelModel,
        bins: u32,
        range: HistogramRange,
        roi: Roi,
        mask_policy: HistogramMaskPolicy,
        nonfinite_policy: HistogramNonFinitePolicy,
    ) -> Result<Self, HistogramRequestError> {
        let bins = NonZeroU32::new(bins).ok_or(HistogramRequestError::ZeroBins)?;
        Ok(Self {
            model,
            bins,
            range,
            roi,
            mask_policy,
            nonfinite_policy,
        })
    }

    #[must_use]
    pub const fn model(self) -> HistogramChannelModel {
        self.model
    }

    #[must_use]
    pub const fn bins(self) -> u32 {
        self.bins.get()
    }

    #[must_use]
    pub const fn range(self) -> HistogramRange {
        self.range
    }

    #[must_use]
    pub const fn roi(self) -> Roi {
        self.roi
    }

    #[must_use]
    pub const fn mask_policy(self) -> HistogramMaskPolicy {
        self.mask_policy
    }

    #[must_use]
    pub const fn nonfinite_policy(self) -> HistogramNonFinitePolicy {
        self.nonfinite_policy
    }
}

/// Rejection reason for a histogram request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistogramRequestError {
    ZeroBins,
}

/// Counts for one logical channel in a histogram result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistogramChannelResult {
    channel: HistogramChannel,
    counts: Vec<u64>,
}

impl HistogramChannelResult {
    #[must_use]
    pub const fn channel(&self) -> HistogramChannel {
        self.channel
    }

    #[must_use]
    pub fn counts(&self) -> &[u64] {
        &self.counts
    }
}

/// Immutable output of serial or tile histogram aggregation.
#[derive(Debug, Clone, PartialEq)]
pub struct HistogramResult {
    request: HistogramRequest,
    channels: Vec<HistogramChannelResult>,
    considered_pixels: u64,
    accepted_pixels: u64,
    masked_pixels: u64,
    skipped_nonfinite_pixels: u64,
}

impl HistogramResult {
    fn empty(request: HistogramRequest) -> Self {
        let channels = request
            .model()
            .channels()
            .iter()
            .copied()
            .map(|channel| HistogramChannelResult {
                channel,
                counts: vec![
                    0;
                    usize::try_from(request.bins())
                        .expect("u32 fits the result index space")
                ],
            })
            .collect();
        Self {
            request,
            channels,
            considered_pixels: 0,
            accepted_pixels: 0,
            masked_pixels: 0,
            skipped_nonfinite_pixels: 0,
        }
    }

    #[must_use]
    pub const fn request(&self) -> &HistogramRequest {
        &self.request
    }

    #[must_use]
    pub fn channels(&self) -> &[HistogramChannelResult] {
        &self.channels
    }

    #[must_use]
    pub fn channel(&self, channel: HistogramChannel) -> Option<&HistogramChannelResult> {
        self.channels.iter().find(|entry| entry.channel == channel)
    }

    #[must_use]
    pub const fn considered_pixels(&self) -> u64 {
        self.considered_pixels
    }

    #[must_use]
    pub const fn accepted_pixels(&self) -> u64 {
        self.accepted_pixels
    }

    #[must_use]
    pub const fn masked_pixels(&self) -> u64 {
        self.masked_pixels
    }

    #[must_use]
    pub const fn skipped_nonfinite_pixels(&self) -> u64 {
        self.skipped_nonfinite_pixels
    }

    /// Merges a partial result from the same request into this result.
    ///
    /// # Errors
    ///
    /// Returns [`HistogramMergeError::RequestMismatch`] when the result
    /// contracts differ.
    pub fn merge(&mut self, other: &Self) -> Result<(), HistogramMergeError> {
        if self.request != other.request {
            return Err(HistogramMergeError::RequestMismatch);
        }
        for (left, right) in self.channels.iter_mut().zip(&other.channels) {
            for (left_count, right_count) in left.counts.iter_mut().zip(&right.counts) {
                *left_count = left_count
                    .checked_add(*right_count)
                    .ok_or(HistogramMergeError::CountOverflow)?;
            }
        }
        self.considered_pixels = self
            .considered_pixels
            .checked_add(other.considered_pixels)
            .ok_or(HistogramMergeError::CountOverflow)?;
        self.accepted_pixels = self
            .accepted_pixels
            .checked_add(other.accepted_pixels)
            .ok_or(HistogramMergeError::CountOverflow)?;
        self.masked_pixels = self
            .masked_pixels
            .checked_add(other.masked_pixels)
            .ok_or(HistogramMergeError::CountOverflow)?;
        self.skipped_nonfinite_pixels = self
            .skipped_nonfinite_pixels
            .checked_add(other.skipped_nonfinite_pixels)
            .ok_or(HistogramMergeError::CountOverflow)?;
        Ok(())
    }
}

/// Rejection reason for merging histogram tile results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistogramMergeError {
    RequestMismatch,
    CountOverflow,
}

/// Rejection reason while aggregating a histogram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistogramAggregationError {
    Raster(HistogramRasterError),
    ModelMismatch {
        request: HistogramChannelModel,
        raster: HistogramChannelModel,
    },
    RoiOutOfBounds,
    MaskRequired,
    NonFinite {
        pixel_index: u64,
        channel: HistogramChannel,
    },
    Merge(HistogramMergeError),
}

/// Deterministic scalar histogram aggregation over full rasters and tiles.
#[derive(Debug, Clone, Copy, Default)]
pub struct HistogramAggregator;

impl HistogramAggregator {
    /// Aggregates the request ROI in row-major pixel order.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the raster model, ROI, mask, or nonfinite
    /// value violates the request contract.
    pub fn aggregate(
        request: &HistogramRequest,
        raster: HistogramRaster<'_>,
    ) -> Result<HistogramResult, HistogramAggregationError> {
        Self::aggregate_roi(request, raster, request.roi())
    }

    /// Aggregates one tile. The request ROI is intersected with the tile.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the tile or raster violates the request
    /// contract.
    pub fn aggregate_tile(
        request: &HistogramRequest,
        raster: HistogramRaster<'_>,
        tile: Roi,
    ) -> Result<HistogramResult, HistogramAggregationError> {
        if tile.within(raster.dimensions()).is_err() {
            return Err(HistogramAggregationError::RoiOutOfBounds);
        }
        if request.roi().within(raster.dimensions()).is_err() {
            return Err(HistogramAggregationError::RoiOutOfBounds);
        }
        let Some(roi) = request.roi().intersection(tile) else {
            return Ok(HistogramResult::empty(*request));
        };
        Self::aggregate_roi(request, raster, roi)
    }

    /// Alias for [`Self::aggregate`] that makes the execution mode explicit.
    ///
    /// # Errors
    ///
    /// Returns the same typed errors as [`Self::aggregate`].
    pub fn aggregate_serial(
        request: &HistogramRequest,
        raster: HistogramRaster<'_>,
    ) -> Result<HistogramResult, HistogramAggregationError> {
        Self::aggregate(request, raster)
    }

    /// Aggregates and merges tiles in the supplied deterministic order.
    ///
    /// Tile overlap is intentionally not deduplicated: callers must provide
    /// an exact cover of the requested ROI, just as a tiled pixelpipe does.
    ///
    /// # Errors
    ///
    /// Returns a typed error when a tile violates the request contract or
    /// partial counts overflow.
    pub fn aggregate_tiles(
        request: &HistogramRequest,
        raster: HistogramRaster<'_>,
        tiles: &[Roi],
    ) -> Result<HistogramResult, HistogramAggregationError> {
        let mut result = HistogramResult::empty(*request);
        for &tile in tiles {
            let partial = Self::aggregate_tile(request, raster, tile)?;
            result
                .merge(&partial)
                .map_err(HistogramAggregationError::Merge)?;
        }
        Ok(result)
    }

    /// Merges already-computed tile results in their supplied stable order.
    ///
    /// # Errors
    ///
    /// Returns a typed error when a tile was built from a different request or
    /// its counts overflow the result.
    pub fn merge_tiles(
        request: &HistogramRequest,
        tiles: &[HistogramResult],
    ) -> Result<HistogramResult, HistogramAggregationError> {
        let mut result = HistogramResult::empty(*request);
        for tile in tiles {
            result
                .merge(tile)
                .map_err(HistogramAggregationError::Merge)?;
        }
        Ok(result)
    }

    fn aggregate_roi(
        request: &HistogramRequest,
        raster: HistogramRaster<'_>,
        roi: Roi,
    ) -> Result<HistogramResult, HistogramAggregationError> {
        if raster.model() != request.model() {
            return Err(HistogramAggregationError::ModelMismatch {
                request: request.model(),
                raster: raster.model(),
            });
        }
        if request.roi().within(raster.dimensions()).is_err()
            || roi.within(raster.dimensions()).is_err()
        {
            return Err(HistogramAggregationError::RoiOutOfBounds);
        }
        if matches!(request.mask_policy(), HistogramMaskPolicy::Require) && raster.mask().is_none()
        {
            return Err(HistogramAggregationError::MaskRequired);
        }
        let mut result = HistogramResult::empty(*request);
        let width = u64::from(raster.dimensions().width());
        let channel_count = request.model().channel_count();
        for y in roi.y()..roi.bottom() {
            for x in roi.x()..roi.right() {
                let pixel_index_u64 = u64::from(y)
                    .checked_mul(width)
                    .and_then(|index| index.checked_add(u64::from(x)))
                    .ok_or(HistogramAggregationError::RoiOutOfBounds)?;
                result.considered_pixels = result
                    .considered_pixels
                    .checked_add(1)
                    .ok_or(HistogramMergeError::CountOverflow)
                    .map_err(HistogramAggregationError::Merge)?;
                let pixel_index = usize::try_from(pixel_index_u64)
                    .map_err(|_| HistogramAggregationError::RoiOutOfBounds)?;
                let mask_value = raster.mask().map(|mask| mask[pixel_index]);
                if !mask_includes(request.mask_policy(), mask_value) {
                    result.masked_pixels = result
                        .masked_pixels
                        .checked_add(1)
                        .ok_or(HistogramMergeError::CountOverflow)
                        .map_err(HistogramAggregationError::Merge)?;
                    continue;
                }
                let start = pixel_index
                    .checked_mul(channel_count)
                    .ok_or(HistogramAggregationError::RoiOutOfBounds)?;
                let samples = &raster.samples()[start..start + channel_count];
                let mut bins = [0_usize; 3];
                for (channel_index, sample) in samples.iter().copied().enumerate() {
                    let channel = request.model().channels()[channel_index];
                    if !sample.is_finite() {
                        if matches!(request.nonfinite_policy(), HistogramNonFinitePolicy::Reject) {
                            return Err(HistogramAggregationError::NonFinite {
                                pixel_index: pixel_index_u64,
                                channel,
                            });
                        }
                        result.skipped_nonfinite_pixels = result
                            .skipped_nonfinite_pixels
                            .checked_add(1)
                            .ok_or(HistogramMergeError::CountOverflow)
                            .map_err(HistogramAggregationError::Merge)?;
                        continue;
                    }
                    bins[channel_index] = bin_for(*request, sample);
                }
                if samples.iter().any(|sample| !sample.is_finite()) {
                    continue;
                }
                for (channel_index, bin) in bins[..channel_count].iter().copied().enumerate() {
                    result.channels[channel_index].counts[bin] += 1;
                }
                result.accepted_pixels = result
                    .accepted_pixels
                    .checked_add(1)
                    .ok_or(HistogramMergeError::CountOverflow)
                    .map_err(HistogramAggregationError::Merge)?;
            }
        }
        Ok(result)
    }
}

fn mask_includes(policy: HistogramMaskPolicy, value: Option<f32>) -> bool {
    match policy {
        HistogramMaskPolicy::Ignore | HistogramMaskPolicy::Require => true,
        HistogramMaskPolicy::IncludeNonZero => value.is_some_and(|value| value > 0.0),
        HistogramMaskPolicy::ExcludeNonZero => value.is_none_or(|value| value == 0.0),
    }
}

fn bin_for(request: HistogramRequest, value: f32) -> usize {
    let range = request.range();
    if value <= range.minimum() {
        return 0;
    }
    if value >= range.maximum() {
        return request.bins() as usize - 1;
    }
    let fraction = (f64::from(value) - f64::from(range.minimum()))
        / (f64::from(range.maximum()) - f64::from(range.minimum()));
    let bins = request.bins();
    let mut lower = 0_u32;
    let mut upper = bins;
    while lower + 1 < upper {
        let midpoint = lower + (upper - lower) / 2;
        if fraction >= f64::from(midpoint) / f64::from(bins) {
            lower = midpoint;
        } else {
            upper = midpoint;
        }
    }
    usize::try_from(lower).expect("u32 fits the result index space")
}

impl fmt::Display for HistogramRangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinite => "histogram range must be finite",
            Self::NotIncreasing => "histogram range minimum must be below maximum",
        })
    }
}

impl std::error::Error for HistogramRangeError {}

impl fmt::Display for HistogramRasterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SizeOverflow => formatter.write_str("histogram raster size overflowed"),
            Self::SampleCount { expected, actual } => {
                write!(
                    formatter,
                    "histogram has {actual} samples, expected {expected}"
                )
            }
            Self::MaskCount { expected, actual } => {
                write!(
                    formatter,
                    "histogram mask has {actual} values, expected {expected}"
                )
            }
        }
    }
}

impl std::error::Error for HistogramRasterError {}

impl fmt::Display for HistogramRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("histogram bin count must be nonzero")
    }
}

impl std::error::Error for HistogramRequestError {}
