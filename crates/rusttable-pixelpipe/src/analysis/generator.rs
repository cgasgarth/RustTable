use std::fmt;

use crate::{
    CancellationError, CancellationScope, CancellationStage, HistogramMaskPolicy,
    HistogramNonFinitePolicy, HistogramRange, RgbaF32Channel,
};
use rusttable_color::{
    AdaptationMethod, AlphaTransform, BlackPointCompensation, BuiltinColorTransformPlanner,
    ColorRole, ColorTransformPlanner, ColorTransformRequest, ExtendedRange, PlannerError,
    Precision, RenderingIntent, TransformExecutionError, TransformPlan, TransformPlanError,
    relative_luminance,
};
use rusttable_core::numerics::NumericalError;

use super::{
    AnalysisAlphaPolicy, AnalysisChannel, AnalysisIntensity, AnalysisKind, AnalysisMask,
    AnalysisPlane, AnalysisProvenance, AnalysisRaster, AnalysisRequest, AnalysisRequestIdentity,
    AnalysisResult, AnalysisStatistics, AnalysisTile, MAX_ANALYSIS_TILES, WaveformOrientation,
};

/// Bounded scalar implementation behind the backend-neutral analysis port.
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuAnalysisGenerator;

/// Ergonomic serial/tiled façade over [`CpuAnalysisGenerator`].
#[derive(Debug, Clone, Copy, Default)]
pub struct AnalysisAggregator;

impl AnalysisAggregator {
    /// Aggregates an unmasked request.
    ///
    /// # Errors
    ///
    /// Returns [`AnalysisError`] for invalid input, color conversion, cancellation, or overflow.
    pub fn aggregate(
        request: &AnalysisRequest,
        raster: AnalysisRaster<'_>,
        cancellation: Option<&CancellationScope>,
    ) -> Result<AnalysisResult, AnalysisError> {
        CpuAnalysisGenerator::generate(request, raster, None, cancellation)
    }

    /// Aggregates a request with a borrowed, already-evaluated mask.
    ///
    /// # Errors
    ///
    /// Returns [`AnalysisError`] for invalid input, color conversion, cancellation, or overflow.
    pub fn aggregate_with_mask(
        request: &AnalysisRequest,
        raster: AnalysisRaster<'_>,
        mask: AnalysisMask<'_>,
        cancellation: Option<&CancellationScope>,
    ) -> Result<AnalysisResult, AnalysisError> {
        CpuAnalysisGenerator::generate(request, raster, Some(mask), cancellation)
    }

    /// Aggregates an exact tile cover and merges indexed integer partials.
    ///
    /// # Errors
    ///
    /// Returns [`AnalysisError`] for invalid tiles, missing tiles, cancellation, or overflow.
    pub fn aggregate_tiles(
        request: &AnalysisRequest,
        raster: AnalysisRaster<'_>,
        tiles: &[rusttable_image::Roi],
        cancellation: Option<&CancellationScope>,
    ) -> Result<AnalysisResult, AnalysisError> {
        let partials = tiles.iter().copied().enumerate().map(|(index, roi)| {
            CpuAnalysisGenerator::generate_tile(
                request,
                raster,
                None,
                AnalysisTile::new(index, roi),
                cancellation,
            )
        });
        CpuAnalysisGenerator::merge_tile_results(request, tiles.len(), partials)
    }
}

impl CpuAnalysisGenerator {
    /// Generates one complete request in source row-major order.
    ///
    /// # Errors
    ///
    /// Returns [`AnalysisError`] for invalid input, color conversion, cancellation, or overflow.
    pub fn generate(
        request: &AnalysisRequest,
        raster: AnalysisRaster<'_>,
        mask: Option<AnalysisMask<'_>>,
        cancellation: Option<&CancellationScope>,
    ) -> Result<AnalysisResult, AnalysisError> {
        let partial = Self::generate_tile(
            request,
            raster,
            mask,
            AnalysisTile::new(0, request.roi()),
            cancellation,
        )?;
        Ok(partial.into_result(request.clone()))
    }

    /// Generates one indexed tile. Sampling uses global source coordinates and the request ROI is
    /// intersected with the tile, matching #275's exact-cover tile boundary.
    ///
    /// # Errors
    ///
    /// Returns [`AnalysisError`] for invalid input, color conversion, cancellation, or overflow.
    pub fn generate_tile(
        request: &AnalysisRequest,
        raster: AnalysisRaster<'_>,
        mask: Option<AnalysisMask<'_>>,
        tile: AnalysisTile,
        cancellation: Option<&CancellationScope>,
    ) -> Result<AnalysisPartial, AnalysisError> {
        let analysis_scope = cancellation.map(|scope| scope.child(CancellationStage::Analysis));
        let cancellation = analysis_scope.as_ref();
        validate_inputs(request, raster, mask, tile)?;
        check_cancellation(cancellation)?;
        let plan = transform_plan(request)?;
        let transform_identity = plan.identity().map_err(AnalysisError::TransformPlan)?;
        let transform_source_color_space = plan.request().source();
        let transform_target_color_space = plan.request().target();
        let source = PartialProvenance {
            source_dimensions: raster.dimensions(),
            source_raster_identity: raster.identity(),
            mask_identity: mask.map(AnalysisMask::identity),
            transform_identity,
            transform_source_color_space,
            transform_target_color_space,
        };
        let mut partial = AnalysisPartial::zeroed(tile.index(), request, source)?;
        let Some(roi) = request.roi().intersection(tile.roi()) else {
            return Ok(partial);
        };
        let source_width = u64::from(raster.dimensions().width());
        for y in roi.y()..roi.bottom() {
            check_cancellation(cancellation)?;
            for x in roi.x()..roi.right() {
                partial.statistics.considered_pixels =
                    checked_increment(partial.statistics.considered_pixels)?;
                if !request.sampling().includes(x, y) {
                    continue;
                }
                partial.statistics.sampled_pixels =
                    checked_increment(partial.statistics.sampled_pixels)?;
                let pixel_index_u64 = u64::from(y)
                    .checked_mul(source_width)
                    .and_then(|index| index.checked_add(u64::from(x)))
                    .ok_or(AnalysisError::ArithmeticOverflow)?;
                let pixel_index = usize::try_from(pixel_index_u64)
                    .map_err(|_| AnalysisError::ArithmeticOverflow)?;
                if !mask_includes(
                    request.mask_policy(),
                    mask.map(|value| value.values()[pixel_index]),
                ) {
                    partial.statistics.masked_pixels =
                        checked_increment(partial.statistics.masked_pixels)?;
                    continue;
                }
                let pixel = raster.pixels()[pixel_index];
                let components = [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()];
                if let Some((channel, _)) = components
                    .iter()
                    .copied()
                    .enumerate()
                    .find(|(_, value)| !value.is_finite())
                {
                    if request.nonfinite_policy() == HistogramNonFinitePolicy::Reject {
                        return Err(AnalysisError::NonFinite {
                            pixel_index: pixel_index_u64,
                            channel: rgba_channel(channel),
                        });
                    }
                    partial.statistics.skipped_nonfinite_pixels =
                        checked_increment(partial.statistics.skipped_nonfinite_pixels)?;
                    continue;
                }
                let (intensity, transparent) =
                    fixed_intensity(request.intensity(), request.alpha_policy(), pixel.alpha());
                if transparent {
                    partial.statistics.transparent_pixels =
                        checked_increment(partial.statistics.transparent_pixels)?;
                }
                if request.alpha_policy() == AnalysisAlphaPolicy::ExcludeTransparent && transparent
                {
                    continue;
                }
                let transformed = match plan
                    .apply_rgb([pixel.red(), pixel.green(), pixel.blue()], || {
                        cancellation.is_some_and(|scope| scope.token().is_cancelled())
                    }) {
                    Ok(value) => value,
                    Err(TransformExecutionError::Cancelled) => {
                        check_cancellation(cancellation)?;
                        return Err(AnalysisError::Transform(TransformExecutionError::Cancelled));
                    }
                    Err(error) => return Err(AnalysisError::Transform(error)),
                };
                accumulate(request, &mut partial, x, y, transformed, intensity)?;
                partial.statistics.accepted_pixels =
                    checked_increment(partial.statistics.accepted_pixels)?;
                partial.statistics.accumulated_intensity = partial
                    .statistics
                    .accumulated_intensity
                    .checked_add(intensity)
                    .ok_or(AnalysisError::CountOverflow)?;
            }
        }
        check_cancellation(cancellation)?;
        Ok(partial)
    }

    /// Merges indexed tile products with #286's checked integer policy. Exact `u64` addition makes
    /// worker completion order irrelevant; duplicate, missing, or out-of-range indexes are
    /// rejected. At most one caller-owned partial and one bounded result buffer are accessed at a
    /// time.
    ///
    /// # Errors
    ///
    /// Returns [`AnalysisError`] for provenance mismatch, malformed indexes, or count overflow.
    pub fn merge_tiles(
        request: &AnalysisRequest,
        expected_tiles: usize,
        partials: impl IntoIterator<Item = AnalysisPartial>,
    ) -> Result<AnalysisResult, AnalysisError> {
        Self::merge_tile_results(request, expected_tiles, partials.into_iter().map(Ok))
    }

    fn merge_tile_results(
        request: &AnalysisRequest,
        expected_tiles: usize,
        partials: impl IntoIterator<Item = Result<AnalysisPartial, AnalysisError>>,
    ) -> Result<AnalysisResult, AnalysisError> {
        if expected_tiles == 0 {
            return Err(AnalysisError::MissingTiles);
        }
        if expected_tiles > MAX_ANALYSIS_TILES {
            return Err(AnalysisError::TileLimitExceeded {
                requested: expected_tiles,
                limit: MAX_ANALYSIS_TILES,
            });
        }
        let cells = cells(request)?;
        let mut planes = request
            .kind()
            .channels()
            .iter()
            .copied()
            .map(|channel| AnalysisPlane::zeroed(channel, cells))
            .collect::<Vec<_>>();
        let mut statistics = AnalysisStatistics::default();
        let mut provenance = None;
        let mut seen = vec![false; expected_tiles];
        let mut seen_count = 0_usize;
        for partial in partials {
            let partial = partial?;
            if partial.tile_index >= expected_tiles {
                return Err(AnalysisError::Numerical(
                    NumericalError::InvalidReductionPlan,
                ));
            }
            if seen[partial.tile_index] {
                return Err(AnalysisError::Numerical(
                    NumericalError::DuplicateReductionLeaf,
                ));
            }
            seen[partial.tile_index] = true;
            seen_count += 1;
            let template = provenance.get_or_insert(partial.provenance);
            if partial.request_identity != request.identity()
                || partial.provenance != *template
                || partial.channels.as_slice() != request.kind().channels()
                || partial.planes.len() != planes.len()
            {
                return Err(AnalysisError::PartialMismatch);
            }
            for (target, source) in planes.iter_mut().zip(partial.planes) {
                if target.channel != source.channel || source.counts().len() != cells {
                    return Err(AnalysisError::PartialMismatch);
                }
                for (target, source) in target.counts_mut().iter_mut().zip(source.counts()) {
                    *target = target
                        .checked_add(*source)
                        .ok_or(AnalysisError::CountOverflow)?;
                }
            }
            statistics = statistics
                .checked_merge(partial.statistics)
                .ok_or(AnalysisError::CountOverflow)?;
        }
        if seen_count != expected_tiles {
            return Err(AnalysisError::Numerical(
                NumericalError::MissingReductionLeaf,
            ));
        }
        let template = provenance.ok_or(AnalysisError::MissingTiles)?;
        let provenance = AnalysisProvenance::new(
            request.clone(),
            template.source_dimensions,
            template.source_raster_identity,
            template.mask_identity,
            template.transform_identity,
            template.transform_source_color_space,
            template.transform_target_color_space,
        );
        Ok(AnalysisResult::new(
            request.output(),
            planes,
            statistics,
            provenance,
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PartialProvenance {
    source_dimensions: rusttable_image::ImageDimensions,
    source_raster_identity: [u8; 32],
    mask_identity: Option<[u8; 32]>,
    transform_identity: [u8; 32],
    transform_source_color_space: rusttable_color::ColorEncoding,
    transform_target_color_space: rusttable_color::ColorEncoding,
}

/// Owned integer tile product. It contains no borrowed image or mask data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisPartial {
    tile_index: usize,
    request_identity: AnalysisRequestIdentity,
    provenance: PartialProvenance,
    channels: Vec<AnalysisChannel>,
    planes: Vec<AnalysisPlane>,
    statistics: AnalysisStatistics,
}

impl AnalysisPartial {
    fn zeroed(
        tile_index: usize,
        request: &AnalysisRequest,
        provenance: PartialProvenance,
    ) -> Result<Self, AnalysisError> {
        let cells = cells(request)?;
        let channels = request.kind().channels().to_vec();
        let planes = channels
            .iter()
            .copied()
            .map(|channel| AnalysisPlane::zeroed(channel, cells))
            .collect();
        Ok(Self {
            tile_index,
            request_identity: request.identity(),
            provenance,
            channels,
            planes,
            statistics: AnalysisStatistics::default(),
        })
    }

    fn into_result(self, request: AnalysisRequest) -> AnalysisResult {
        let provenance = AnalysisProvenance::new(
            request,
            self.provenance.source_dimensions,
            self.provenance.source_raster_identity,
            self.provenance.mask_identity,
            self.provenance.transform_identity,
            self.provenance.transform_source_color_space,
            self.provenance.transform_target_color_space,
        );
        AnalysisResult::new(
            provenance.request().output(),
            self.planes,
            self.statistics,
            provenance,
        )
    }

    #[must_use]
    pub const fn tile_index(&self) -> usize {
        self.tile_index
    }
    #[must_use]
    pub fn planes(&self) -> &[AnalysisPlane] {
        &self.planes
    }
    #[must_use]
    pub const fn statistics(&self) -> AnalysisStatistics {
        self.statistics
    }
}

#[derive(Debug)]
pub enum AnalysisError {
    RasterColorMismatch,
    RoiOutOfBounds,
    TileOutOfBounds,
    MaskRequired,
    UnexpectedMaskDimensions,
    UnsupportedAnalysisColorSpace,
    ColorPlanner(PlannerError),
    TransformPlan(TransformPlanError),
    Transform(TransformExecutionError),
    Cancelled(CancellationError),
    NonFinite {
        pixel_index: u64,
        channel: RgbaF32Channel,
    },
    ArithmeticOverflow,
    CountOverflow,
    MissingTiles,
    TileLimitExceeded {
        requested: usize,
        limit: usize,
    },
    PartialMismatch,
    Numerical(NumericalError),
}

fn validate_inputs(
    request: &AnalysisRequest,
    raster: AnalysisRaster<'_>,
    mask: Option<AnalysisMask<'_>>,
    tile: AnalysisTile,
) -> Result<(), AnalysisError> {
    if raster.source_color_space() != request.source_color_space() {
        return Err(AnalysisError::RasterColorMismatch);
    }
    if request.roi().within(raster.dimensions()).is_err() {
        return Err(AnalysisError::RoiOutOfBounds);
    }
    if tile.roi().within(raster.dimensions()).is_err() {
        return Err(AnalysisError::TileOutOfBounds);
    }
    if request.mask_policy() == HistogramMaskPolicy::Require && mask.is_none() {
        return Err(AnalysisError::MaskRequired);
    }
    if let Some(mask) = mask {
        let expected = raster
            .dimensions()
            .pixel_count()
            .map_err(|_| AnalysisError::ArithmeticOverflow)?;
        if u64::try_from(mask.values().len()) != Ok(expected) {
            return Err(AnalysisError::UnexpectedMaskDimensions);
        }
    }
    Ok(())
}

fn transform_plan(request: &AnalysisRequest) -> Result<TransformPlan, AnalysisError> {
    let target = match request.kind() {
        AnalysisKind::LuminanceWaveform => request
            .analysis_color_space()
            .builtin()
            .filter(|space| space.primaries().is_some())
            .map(|space| space.encoding(true))
            .ok_or(AnalysisError::UnsupportedAnalysisColorSpace)?,
        AnalysisKind::RgbWaveform | AnalysisKind::RgbParade | AnalysisKind::Vectorscope => {
            request.analysis_color_space()
        }
    };
    if target
        .builtin()
        .is_none_or(|space| space.primaries().is_none())
    {
        return Err(AnalysisError::UnsupportedAnalysisColorSpace);
    }
    let transform_request = ColorTransformRequest::new(
        request.source_color_space(),
        target,
        ColorRole::Analysis,
        RenderingIntent::Relative,
        BlackPointCompensation::Disabled,
        AdaptationMethod::Bradford,
        Precision::F32,
        AlphaTransform::Preserve,
        ExtendedRange::Extended,
        1,
    )
    .map_err(|error| AnalysisError::ColorPlanner(PlannerError::Request(error)))?;
    BuiltinColorTransformPlanner
        .plan(&transform_request)
        .map_err(AnalysisError::ColorPlanner)
}

fn accumulate(
    request: &AnalysisRequest,
    partial: &mut AnalysisPartial,
    source_x: u32,
    source_y: u32,
    rgb: [f32; 3],
    intensity: u64,
) -> Result<(), AnalysisError> {
    match request.kind() {
        AnalysisKind::LuminanceWaveform => {
            let space = request
                .analysis_color_space()
                .builtin()
                .ok_or(AnalysisError::UnsupportedAnalysisColorSpace)?;
            let luminance = relative_luminance(rgb, space)
                .map_err(|_| AnalysisError::UnsupportedAnalysisColorSpace)?;
            let (x, y, clipping) = waveform_coordinates(request, source_x, source_y, luminance)?;
            record_clipping(&mut partial.statistics, clipping)?;
            add_to_plane(
                &mut partial.planes[0],
                request.output().width(),
                x,
                y,
                intensity,
            )?;
        }
        AnalysisKind::RgbWaveform | AnalysisKind::RgbParade => {
            for (plane, value) in partial.planes.iter_mut().zip(rgb) {
                let (x, y, clipping) = waveform_coordinates(request, source_x, source_y, value)?;
                record_clipping(&mut partial.statistics, clipping)?;
                add_to_plane(plane, request.output().width(), x, y, intensity)?;
            }
        }
        AnalysisKind::Vectorscope => {
            let (kr, kb) = request.graticule().luma_coefficients();
            let kg = 1.0 - kr - kb;
            let y = kr * rgb[0] + kg * rgb[1] + kb * rgb[2];
            let cb = (rgb[2] - y) / (2.0 * (1.0 - kb));
            let cr = (rgb[0] - y) / (2.0 * (1.0 - kr));
            if !cb.is_finite() || !cr.is_finite() {
                return Err(AnalysisError::ArithmeticOverflow);
            }
            let (x, x_clip) = bin_for(cb, request.output().width(), request.range());
            let (y, y_clip) = bin_for(cr, request.output().height(), request.range());
            record_clipping(&mut partial.statistics, x_clip)?;
            record_clipping(&mut partial.statistics, y_clip)?;
            add_to_plane(
                &mut partial.planes[0],
                request.output().width(),
                x,
                y,
                intensity,
            )?;
        }
    }
    Ok(())
}

fn waveform_coordinates(
    request: &AnalysisRequest,
    source_x: u32,
    source_y: u32,
    value: f32,
) -> Result<(u32, u32, Clipping), AnalysisError> {
    match request.waveform_orientation() {
        WaveformOrientation::Horizontal => {
            let x = position_bin(
                source_x,
                request.roi().x(),
                request.roi().width(),
                request.output().width(),
            )?;
            let (y, clipping) = bin_for(value, request.output().height(), request.range());
            Ok((x, y, clipping))
        }
        WaveformOrientation::Vertical => {
            let (x, clipping) = bin_for(value, request.output().width(), request.range());
            let y = position_bin(
                source_y,
                request.roi().y(),
                request.roi().height(),
                request.output().height(),
            )?;
            Ok((x, y, clipping))
        }
    }
}

fn position_bin(
    source: u32,
    origin: u32,
    extent: u32,
    output_extent: u32,
) -> Result<u32, AnalysisError> {
    if extent == 0 {
        return Ok(0);
    }
    let relative = u64::from(source - origin);
    let mapped = relative
        .checked_mul(u64::from(output_extent))
        .ok_or(AnalysisError::ArithmeticOverflow)?
        / u64::from(extent);
    u32::try_from(mapped.min(u64::from(output_extent - 1)))
        .map_err(|_| AnalysisError::ArithmeticOverflow)
}

fn bin_for(value: f32, bins: u32, range: HistogramRange) -> (u32, Clipping) {
    if value <= range.minimum() {
        return (
            0,
            if value < range.minimum() {
                Clipping::Low
            } else {
                Clipping::None
            },
        );
    }
    if value >= range.maximum() {
        return (
            bins - 1,
            if value > range.maximum() {
                Clipping::High
            } else {
                Clipping::None
            },
        );
    }
    let fraction = (f64::from(value) - f64::from(range.minimum()))
        / (f64::from(range.maximum()) - f64::from(range.minimum()));
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
    (lower, Clipping::None)
}

#[derive(Clone, Copy)]
enum Clipping {
    None,
    Low,
    High,
}

fn record_clipping(
    statistics: &mut AnalysisStatistics,
    clipping: Clipping,
) -> Result<(), AnalysisError> {
    match clipping {
        Clipping::None => {}
        Clipping::Low => {
            statistics.clipped_low_samples = checked_increment(statistics.clipped_low_samples)?;
        }
        Clipping::High => {
            statistics.clipped_high_samples = checked_increment(statistics.clipped_high_samples)?;
        }
    }
    Ok(())
}

fn add_to_plane(
    plane: &mut AnalysisPlane,
    width: u32,
    x: u32,
    y: u32,
    intensity: u64,
) -> Result<(), AnalysisError> {
    let index = u64::from(y)
        .checked_mul(u64::from(width))
        .and_then(|value| value.checked_add(u64::from(x)))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(AnalysisError::ArithmeticOverflow)?;
    let count = plane
        .counts_mut()
        .get_mut(index)
        .ok_or(AnalysisError::ArithmeticOverflow)?;
    *count = count
        .checked_add(intensity)
        .ok_or(AnalysisError::CountOverflow)?;
    Ok(())
}

fn fixed_intensity(
    intensity: AnalysisIntensity,
    alpha_policy: AnalysisAlphaPolicy,
    alpha: f32,
) -> (u64, bool) {
    let transparent = alpha <= 0.0;
    let quantum = intensity.quantum();
    let contribution = match alpha_policy {
        AnalysisAlphaPolicy::Ignore | AnalysisAlphaPolicy::ExcludeTransparent => quantum,
        AnalysisAlphaPolicy::Weight => {
            let value = (f64::from(alpha.clamp(0.0, 1.0))
                * f64::from(u32::try_from(quantum).expect("analysis quantum is at most 2^16")))
            .round_ties_even();
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let fixed = value as u64;
            fixed
        }
    };
    (contribution, transparent)
}

fn mask_includes(policy: HistogramMaskPolicy, value: Option<f32>) -> bool {
    match policy {
        HistogramMaskPolicy::Ignore | HistogramMaskPolicy::Require => true,
        HistogramMaskPolicy::IncludeNonZero => value.is_some_and(|value| value > 0.0),
        HistogramMaskPolicy::ExcludeNonZero => value.is_none_or(|value| value == 0.0),
    }
}

fn check_cancellation(scope: Option<&CancellationScope>) -> Result<(), AnalysisError> {
    scope
        .map_or(Ok(()), CancellationScope::check)
        .map_err(AnalysisError::Cancelled)
}

const fn rgba_channel(index: usize) -> RgbaF32Channel {
    match index {
        0 => RgbaF32Channel::Red,
        1 => RgbaF32Channel::Green,
        2 => RgbaF32Channel::Blue,
        _ => RgbaF32Channel::Alpha,
    }
}

fn checked_increment(value: u64) -> Result<u64, AnalysisError> {
    value.checked_add(1).ok_or(AnalysisError::CountOverflow)
}

fn cells(request: &AnalysisRequest) -> Result<usize, AnalysisError> {
    usize::try_from(
        request
            .output()
            .pixel_count()
            .map_err(|_| AnalysisError::ArithmeticOverflow)?,
    )
    .map_err(|_| AnalysisError::ArithmeticOverflow)
}

impl fmt::Display for AnalysisError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RasterColorMismatch => {
                formatter.write_str("analysis raster color space does not match request")
            }
            Self::RoiOutOfBounds => formatter.write_str("analysis ROI is outside raster bounds"),
            Self::TileOutOfBounds => formatter.write_str("analysis tile is outside raster bounds"),
            Self::MaskRequired => formatter.write_str("analysis request requires a mask"),
            Self::UnexpectedMaskDimensions => {
                formatter.write_str("analysis mask dimensions do not match raster")
            }
            Self::UnsupportedAnalysisColorSpace => {
                formatter.write_str("analysis requires a built-in RGB color space")
            }
            Self::ColorPlanner(error) => error.fmt(formatter),
            Self::TransformPlan(error) => {
                write!(formatter, "analysis color transform plan failed: {error:?}")
            }
            Self::Transform(error) => error.fmt(formatter),
            Self::Cancelled(error) => error.fmt(formatter),
            Self::NonFinite {
                pixel_index,
                channel,
            } => write!(
                formatter,
                "analysis pixel {pixel_index} has non-finite {channel:?}"
            ),
            Self::ArithmeticOverflow => formatter.write_str("analysis arithmetic overflowed"),
            Self::CountOverflow => formatter.write_str("analysis integer accumulation overflowed"),
            Self::MissingTiles => {
                formatter.write_str("analysis tile merge is missing all or expected tiles")
            }
            Self::TileLimitExceeded { requested, limit } => {
                write!(
                    formatter,
                    "analysis tile count {requested} exceeds limit {limit}"
                )
            }
            Self::PartialMismatch => {
                formatter.write_str("analysis tile partial provenance mismatch")
            }
            Self::Numerical(error) => {
                write!(formatter, "analysis deterministic merge failed: {error:?}")
            }
        }
    }
}

impl std::error::Error for AnalysisError {}
