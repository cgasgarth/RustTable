use crate::{CacheValue, CreationCost, ValueDescriptor, ValueKind};

use super::{
    ANALYSIS_NUMERICAL_CONTRACT, AnalysisCacheIdentity, AnalysisChannel, AnalysisNormalization,
    AnalysisOutputDimensions, AnalysisRequest, AnalysisRequestIdentity,
};

/// One row-major `u64` accumulation plane. Row zero represents the range minimum, matching the
/// existing histogram bin convention; a renderer may invert it when drawing a waveform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisPlane {
    pub(super) channel: AnalysisChannel,
    pub(super) counts: Vec<u64>,
}

impl AnalysisPlane {
    pub(super) fn zeroed(channel: AnalysisChannel, cells: usize) -> Self {
        Self {
            channel,
            counts: vec![0; cells],
        }
    }

    #[must_use]
    pub const fn channel(&self) -> AnalysisChannel {
        self.channel
    }
    #[must_use]
    pub fn counts(&self) -> &[u64] {
        &self.counts
    }
    pub(super) fn counts_mut(&mut self) -> &mut [u64] {
        &mut self.counts
    }
}

/// Integer-only counters retained across full-frame and tiled execution.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnalysisStatistics {
    pub(super) considered_pixels: u64,
    pub(super) sampled_pixels: u64,
    pub(super) accepted_pixels: u64,
    pub(super) masked_pixels: u64,
    pub(super) transparent_pixels: u64,
    pub(super) skipped_nonfinite_pixels: u64,
    pub(super) clipped_low_samples: u64,
    pub(super) clipped_high_samples: u64,
    pub(super) accumulated_intensity: u64,
}

impl AnalysisStatistics {
    #[must_use]
    pub const fn considered_pixels(self) -> u64 {
        self.considered_pixels
    }
    #[must_use]
    pub const fn sampled_pixels(self) -> u64 {
        self.sampled_pixels
    }
    #[must_use]
    pub const fn accepted_pixels(self) -> u64 {
        self.accepted_pixels
    }
    #[must_use]
    pub const fn masked_pixels(self) -> u64 {
        self.masked_pixels
    }
    #[must_use]
    pub const fn transparent_pixels(self) -> u64 {
        self.transparent_pixels
    }
    #[must_use]
    pub const fn skipped_nonfinite_pixels(self) -> u64 {
        self.skipped_nonfinite_pixels
    }
    #[must_use]
    pub const fn clipped_low_samples(self) -> u64 {
        self.clipped_low_samples
    }
    #[must_use]
    pub const fn clipped_high_samples(self) -> u64 {
        self.clipped_high_samples
    }
    #[must_use]
    pub const fn accumulated_intensity(self) -> u64 {
        self.accumulated_intensity
    }

    pub(super) fn checked_merge(self, right: Self) -> Option<Self> {
        Some(Self {
            considered_pixels: self
                .considered_pixels
                .checked_add(right.considered_pixels)?,
            sampled_pixels: self.sampled_pixels.checked_add(right.sampled_pixels)?,
            accepted_pixels: self.accepted_pixels.checked_add(right.accepted_pixels)?,
            masked_pixels: self.masked_pixels.checked_add(right.masked_pixels)?,
            transparent_pixels: self
                .transparent_pixels
                .checked_add(right.transparent_pixels)?,
            skipped_nonfinite_pixels: self
                .skipped_nonfinite_pixels
                .checked_add(right.skipped_nonfinite_pixels)?,
            clipped_low_samples: self
                .clipped_low_samples
                .checked_add(right.clipped_low_samples)?,
            clipped_high_samples: self
                .clipped_high_samples
                .checked_add(right.clipped_high_samples)?,
            accumulated_intensity: self
                .accumulated_intensity
                .checked_add(right.accumulated_intensity)?,
        })
    }
}

/// Complete identity of a generated buffer, including actual transform and exact source/mask bits.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisProvenance {
    request: AnalysisRequest,
    source_dimensions: rusttable_image::ImageDimensions,
    source_raster_identity: [u8; 32],
    mask_identity: Option<[u8; 32]>,
    transform_identity: [u8; 32],
    transform_source_color_space: rusttable_color::ColorEncoding,
    transform_target_color_space: rusttable_color::ColorEncoding,
    cache_identity: AnalysisCacheIdentity,
}

impl AnalysisProvenance {
    pub(super) fn new(
        request: AnalysisRequest,
        source_dimensions: rusttable_image::ImageDimensions,
        source_raster_identity: [u8; 32],
        mask_identity: Option<[u8; 32]>,
        transform_identity: [u8; 32],
        transform_source_color_space: rusttable_color::ColorEncoding,
        transform_target_color_space: rusttable_color::ColorEncoding,
    ) -> Self {
        let cache_identity = AnalysisCacheIdentity::new(
            request.identity(),
            source_raster_identity,
            mask_identity,
            transform_identity,
        );
        Self {
            request,
            source_dimensions,
            source_raster_identity,
            mask_identity,
            transform_identity,
            transform_source_color_space,
            transform_target_color_space,
            cache_identity,
        }
    }

    #[must_use]
    pub const fn request(&self) -> &AnalysisRequest {
        &self.request
    }
    #[must_use]
    pub const fn request_identity(&self) -> AnalysisRequestIdentity {
        self.request.identity()
    }
    #[must_use]
    pub const fn source_raster_identity(&self) -> [u8; 32] {
        self.source_raster_identity
    }
    #[must_use]
    pub const fn source_dimensions(&self) -> rusttable_image::ImageDimensions {
        self.source_dimensions
    }
    #[must_use]
    pub const fn mask_identity(&self) -> Option<[u8; 32]> {
        self.mask_identity
    }
    #[must_use]
    pub const fn transform_identity(&self) -> [u8; 32] {
        self.transform_identity
    }
    #[must_use]
    pub const fn transform_source_color_space(&self) -> rusttable_color::ColorEncoding {
        self.transform_source_color_space
    }
    #[must_use]
    pub const fn transform_target_color_space(&self) -> rusttable_color::ColorEncoding {
        self.transform_target_color_space
    }
    #[must_use]
    pub const fn numerical_contract(&self) -> rusttable_core::numerics::NumericalContract {
        ANALYSIS_NUMERICAL_CONTRACT
    }
    #[must_use]
    pub const fn cache_identity(&self) -> AnalysisCacheIdentity {
        self.cache_identity
    }
    #[must_use]
    pub const fn roi(&self) -> rusttable_image::Roi {
        self.request.roi()
    }
    #[must_use]
    pub const fn color_space(&self) -> rusttable_color::ColorEncoding {
        self.request.analysis_color_space()
    }
}

/// Immutable reusable analysis product. It owns only bounded integer planes and typed metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisResult {
    dimensions: AnalysisOutputDimensions,
    planes: Vec<AnalysisPlane>,
    statistics: AnalysisStatistics,
    normalization_denominator: u64,
    provenance: AnalysisProvenance,
}

impl AnalysisResult {
    pub(super) fn new(
        dimensions: AnalysisOutputDimensions,
        planes: Vec<AnalysisPlane>,
        statistics: AnalysisStatistics,
        provenance: AnalysisProvenance,
    ) -> Self {
        let normalization_denominator = match provenance.request().normalization() {
            AnalysisNormalization::None => 1,
            AnalysisNormalization::Peak => planes
                .iter()
                .flat_map(AnalysisPlane::counts)
                .copied()
                .max()
                .unwrap_or(0)
                .max(1),
            AnalysisNormalization::SampleIntensity => statistics.accumulated_intensity().max(1),
        };
        Self {
            dimensions,
            planes,
            statistics,
            normalization_denominator,
            provenance,
        }
    }

    #[must_use]
    pub const fn dimensions(&self) -> AnalysisOutputDimensions {
        self.dimensions
    }
    #[must_use]
    pub fn planes(&self) -> &[AnalysisPlane] {
        &self.planes
    }
    #[must_use]
    pub fn plane(&self, channel: AnalysisChannel) -> Option<&AnalysisPlane> {
        self.planes.iter().find(|plane| plane.channel == channel)
    }
    #[must_use]
    pub const fn statistics(&self) -> AnalysisStatistics {
        self.statistics
    }
    #[must_use]
    pub const fn normalization_denominator(&self) -> u64 {
        self.normalization_denominator
    }
    #[must_use]
    pub const fn provenance(&self) -> &AnalysisProvenance {
        &self.provenance
    }
    #[must_use]
    pub fn resident_bytes(&self) -> u64 {
        self.provenance.request().resident_bytes()
    }
    #[must_use]
    pub const fn request(&self) -> &AnalysisRequest {
        self.provenance.request()
    }
    #[must_use]
    pub const fn output_dimensions(&self) -> AnalysisOutputDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn accepted_samples(&self) -> u64 {
        self.statistics.accepted_pixels()
    }
    #[must_use]
    pub const fn skipped_nonfinite_samples(&self) -> u64 {
        self.statistics.skipped_nonfinite_pixels()
    }
    #[must_use]
    pub const fn transparent_samples(&self) -> u64 {
        self.statistics.transparent_pixels()
    }
    #[must_use]
    pub const fn clipped_samples(&self) -> u64 {
        self.statistics
            .clipped_low_samples()
            .saturating_add(self.statistics.clipped_high_samples())
    }
    #[must_use]
    pub fn occupied_bins(&self) -> usize {
        self.planes
            .iter()
            .flat_map(AnalysisPlane::counts)
            .filter(|count| **count != 0)
            .count()
    }
    #[must_use]
    pub fn channel_occupied_bins(&self) -> [usize; 3] {
        let mut occupied = [0; 3];
        for (target, plane) in occupied.iter_mut().zip(&self.planes) {
            *target = plane.counts().iter().filter(|count| **count != 0).count();
        }
        occupied
    }
}

impl CacheValue for AnalysisResult {
    fn descriptor(&self) -> ValueDescriptor {
        ValueDescriptor::new(
            ValueKind::Analysis,
            self.resident_bytes(),
            0,
            CreationCost::Expensive,
            true,
        )
    }

    fn validate(&self) -> Result<(), String> {
        let cells = usize::try_from(
            self.dimensions
                .pixel_count()
                .map_err(|error| error.to_string())?,
        )
        .map_err(|_| "analysis dimensions exceed address space".to_owned())?;
        if self.planes.len() != usize::from(self.provenance.request().kind().plane_count())
            || self.planes.iter().any(|plane| plane.counts.len() != cells)
        {
            return Err("analysis plane shape does not match provenance".to_owned());
        }
        Ok(())
    }
}
