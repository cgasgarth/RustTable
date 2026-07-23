//! Contract tests for issue #276's reusable scope-analysis buffers.
//!
//! Integration note for the primary implementation: `origin/main` has the
//! histogram, ROI, cancellation, image, and tile primitives but no public
//! `rusttable_pixelpipe::analysis` module yet. This file deliberately targets
//! the narrow port described below; keep the semantic assertions intact if
//! the implementation chooses different internal names.
//!
//! Pinned Darktable anchors (commit `cfe57f3bbf5269bfacf31e832267279caa6938ad`):
//! - `src/common/histogram.c:35-158, 163-197`: clamp bins, channel order,
//!   crop/ROI iteration, and sample counts.
//! - `src/libs/scopes/waveform.c:58-162, 444-480`: waveform accumulation,
//!   bounded horizontal/vertical bins, and RGB-parade channel separation.
//! - `src/libs/scopes/vectorscope.c:419-494, 501-680`: explicit color-space
//!   conversion, 2x2 sample reduction, chromaticity binning, and log scaling.
//!
//! The intended public shape is an immutable `AnalysisRequest`, a borrowed
//! `AnalysisRaster`, and `AnalysisAggregator::{aggregate,aggregate_tiles}`.
//! Aggregation must return a bounded, owned `AnalysisResult` whose fixed-point
//! bins and provenance are independent of tile/worker order.

use rusttable_color::ColorEncoding;
use rusttable_image::{ImageDimensions, Roi};
use rusttable_pixelpipe::analysis::{
    ANALYSIS_NUMERICAL_CONTRACT, AnalysisAggregator, AnalysisAlphaPolicy, AnalysisError,
    AnalysisGraticule, AnalysisIntensity, AnalysisKind, AnalysisMask, AnalysisNormalization,
    AnalysisOutputDimensions, AnalysisPlane, AnalysisRaster, AnalysisRequest, AnalysisResult,
    AnalysisSampling, AnalysisTile, CpuAnalysisGenerator, MAX_ANALYSIS_BYTES, MAX_ANALYSIS_TILES,
    WaveformOrientation,
};
use rusttable_pixelpipe::{
    CacheValue, CancellationReason, CancellationScope, CancellationStage, CpuPriority,
    HistogramMaskPolicy, HistogramNonFinitePolicy, HistogramRange, ImplementationIdentity,
    NodeBoundary, PipelineGeneration, PublicationTargetKind, RgbaF32ColorEncoding, RgbaF32Pixel,
};

const NODE_ID: [u8; 32] = [0x76; 32];

fn dimensions(width: u32, height: u32) -> ImageDimensions {
    ImageDimensions::new(width, height).expect("nonzero image dimensions")
}

fn output(width: u32, height: u32) -> AnalysisOutputDimensions {
    AnalysisOutputDimensions::new(width, height).expect("valid analysis dimensions")
}

fn request(kind: AnalysisKind, roi: Roi, output: AnalysisOutputDimensions) -> AnalysisRequest {
    let range = if kind == AnalysisKind::Vectorscope {
        HistogramRange::new(-0.5, 0.5).expect("vectorscope range")
    } else {
        HistogramRange::new(0.0, 1.0).expect("waveform range")
    };
    AnalysisRequest::new(
        kind,
        NodeBoundary::range(
            NODE_ID,
            0,
            0,
            ImplementationIdentity::new("rusttable.analysis.fixture", 1, "test")
                .expect("implementation identity"),
        ),
        roi,
        output,
        ColorEncoding::LinearSrgbD65,
        ColorEncoding::LinearSrgbD65,
        AnalysisSampling::EveryPixel,
        range,
        HistogramMaskPolicy::Ignore,
        HistogramNonFinitePolicy::Skip,
        AnalysisNormalization::Peak,
        AnalysisGraticule::Rec709,
        AnalysisIntensity::FixedPoint { fractional_bits: 8 },
        AnalysisAlphaPolicy::Ignore,
        WaveformOrientation::Horizontal,
    )
    .expect("valid analysis request")
}

fn raster(dimensions: ImageDimensions, pixels: &[RgbaF32Pixel]) -> AnalysisRaster<'_> {
    AnalysisRaster::from_rgba(dimensions, RgbaF32ColorEncoding::LinearSrgbD65, pixels)
        .expect("valid analysis raster")
}

fn rgb_ramp(width: u32, height: u32) -> Vec<RgbaF32Pixel> {
    let count = usize::try_from(u64::from(width) * u64::from(height)).expect("small fixture");
    (0..count)
        .map(|index| {
            let value = if count == 1 {
                0.0
            } else {
                f32::from(u16::try_from(index).expect("small fixture index"))
                    / f32::from(u16::try_from(count - 1).expect("small fixture length"))
            };
            RgbaF32Pixel::new(value, value, value, 1.0)
        })
        .collect()
}

fn color_bars() -> Vec<RgbaF32Pixel> {
    [
        (1.0, 0.0, 0.0),
        (0.0, 1.0, 0.0),
        (0.0, 0.0, 1.0),
        (0.0, 1.0, 1.0),
        (1.0, 0.0, 1.0),
        (1.0, 1.0, 0.0),
        (1.0, 1.0, 1.0),
        (0.0, 0.0, 0.0),
    ]
    .into_iter()
    .map(|(red, green, blue)| RgbaF32Pixel::new(red, green, blue, 1.0))
    .collect()
}

fn hue_wheel() -> Vec<RgbaF32Pixel> {
    // Twelve fixed HSV hues, avoiding a floating-point color-wheel generator
    // so this fixture remains bit-for-bit stable across toolchains.
    [
        (1.0, 0.0, 0.0),
        (1.0, 0.5, 0.0),
        (1.0, 1.0, 0.0),
        (0.5, 1.0, 0.0),
        (0.0, 1.0, 0.0),
        (0.0, 1.0, 0.5),
        (0.0, 1.0, 1.0),
        (0.0, 0.5, 1.0),
        (0.0, 0.0, 1.0),
        (0.5, 0.0, 1.0),
        (1.0, 0.0, 1.0),
        (1.0, 0.0, 0.5),
    ]
    .into_iter()
    .map(|(red, green, blue)| RgbaF32Pixel::new(red, green, blue, 1.0))
    .collect()
}

fn count(result: &AnalysisResult) -> u64 {
    result.planes().iter().flat_map(AnalysisPlane::counts).sum()
}

#[test]
fn requests_are_deterministic_and_provenance_sensitive() {
    let roi = Roi::new(1, 1, 2, 2).expect("ROI");
    let first = request(AnalysisKind::RgbWaveform, roi, output(16, 16));
    let second = request(AnalysisKind::RgbWaveform, roi, output(16, 16));
    assert_eq!(first, second);
    assert_eq!(first.identity(), second.identity());

    assert_ne!(
        first.identity(),
        request(AnalysisKind::RgbParade, roi, output(16, 16)).identity()
    );
    assert_ne!(
        first.identity(),
        request(
            AnalysisKind::RgbWaveform,
            Roi::full(dimensions(4, 4)),
            output(16, 16)
        )
        .identity()
    );
    assert_eq!(first.boundary().boundary(), Some(NODE_ID));
    assert_eq!(first.output(), output(16, 16));
}

#[test]
fn waveform_ramp_has_monotone_coverage_and_bounded_output() {
    let dimensions = dimensions(16, 4);
    let pixels = rgb_ramp(dimensions.width(), dimensions.height());
    let request = request(
        AnalysisKind::RgbWaveform,
        Roi::full(dimensions),
        output(8, 16),
    );
    let result = AnalysisAggregator::aggregate(&request, raster(dimensions, &pixels), None)
        .expect("waveform");

    assert_eq!(result.request(), &request);
    assert_eq!(result.output_dimensions(), output(8, 16));
    assert_eq!(
        result
            .planes()
            .iter()
            .map(|plane| plane.counts().len())
            .sum::<usize>(),
        8 * 16 * 3
    );
    assert!(
        result
            .planes()
            .iter()
            .flat_map(AnalysisPlane::counts)
            .all(|value| *value <= 8 * 256)
    );
    assert_eq!(result.accepted_samples(), 64);
    assert!(result.occupied_bins() >= 8);
}

#[test]
fn parade_keeps_rgb_bar_energy_in_separate_channel_planes() {
    let dimensions = dimensions(8, 1);
    let request = request(
        AnalysisKind::RgbParade,
        Roi::full(dimensions),
        output(24, 16),
    );
    let bars = color_bars();
    let result = AnalysisAggregator::aggregate(&request, raster(dimensions, &bars), None)
        .expect("RGB parade");

    assert_eq!(result.accepted_samples(), 8);
    assert_eq!(result.channel_occupied_bins(), [8, 8, 8]);
    assert_eq!(count(&result), 8 * 256 * 3);
}

#[test]
fn vectorscope_hue_wheel_preserves_distinct_hue_regions() {
    let dimensions = dimensions(12, 1);
    let request = request(
        AnalysisKind::Vectorscope,
        Roi::full(dimensions),
        output(32, 32),
    );
    let wheel = hue_wheel();
    let result = AnalysisAggregator::aggregate(&request, raster(dimensions, &wheel), None)
        .expect("vectorscope");

    assert_eq!(result.accepted_samples(), 12);
    assert_eq!(result.occupied_bins(), 12);
    assert_eq!(count(&result), 12 * 256);
    assert_eq!(
        result.provenance().color_space(),
        ColorEncoding::LinearSrgbD65
    );
}

#[test]
fn full_frame_and_exact_tile_cover_are_identical() {
    let dimensions = dimensions(13, 9);
    let pixels = rgb_ramp(dimensions.width(), dimensions.height());
    let request = request(
        AnalysisKind::RgbWaveform,
        Roi::new(2, 1, 9, 7).expect("zoomed ROI"),
        output(16, 16),
    )
    .with_waveform_orientation(WaveformOrientation::Vertical);
    let full = AnalysisAggregator::aggregate(&request, raster(dimensions, &pixels), None)
        .expect("full frame");
    let tiles = [
        Roi::new(2, 1, 3, 3).expect("tile"),
        Roi::new(5, 1, 3, 3).expect("tile"),
        Roi::new(8, 1, 3, 3).expect("tile"),
        Roi::new(2, 4, 3, 4).expect("tile"),
        Roi::new(5, 4, 3, 4).expect("tile"),
        Roi::new(8, 4, 3, 4).expect("tile"),
    ];
    let tiled_result =
        AnalysisAggregator::aggregate_tiles(&request, raster(dimensions, &pixels), &tiles, None)
            .expect("exact tile cover");
    assert_eq!(full, tiled_result);
}

#[test]
fn tile_worker_order_does_not_change_fixed_point_output() {
    let dimensions = dimensions(13, 9);
    let pixels = rgb_ramp(dimensions.width(), dimensions.height());
    let request = request(
        AnalysisKind::Vectorscope,
        Roi::full(dimensions),
        output(32, 32),
    );
    let forward = [
        Roi::new(0, 0, 5, 4).expect("tile"),
        Roi::new(5, 0, 4, 4).expect("tile"),
        Roi::new(9, 0, 4, 4).expect("tile"),
        Roi::new(0, 4, 5, 5).expect("tile"),
        Roi::new(5, 4, 4, 5).expect("tile"),
        Roi::new(9, 4, 4, 5).expect("tile"),
    ];
    let reverse = forward.into_iter().rev().collect::<Vec<_>>();
    let first =
        AnalysisAggregator::aggregate_tiles(&request, raster(dimensions, &pixels), &forward, None)
            .expect("forward workers");
    let second =
        AnalysisAggregator::aggregate_tiles(&request, raster(dimensions, &pixels), &reverse, None)
            .expect("reverse workers");
    assert_eq!(first, second);

    let partial = CpuAnalysisGenerator::generate_tile(
        &request,
        raster(dimensions, &pixels),
        None,
        AnalysisTile::new(0, forward[0]),
        None,
    )
    .expect("indexed partial");
    assert!(matches!(
        CpuAnalysisGenerator::merge_tiles(&request, 2, [partial.clone(), partial]),
        Err(AnalysisError::Numerical(_))
    ));
    assert!(matches!(
        CpuAnalysisGenerator::merge_tiles(&request, MAX_ANALYSIS_TILES + 1, []),
        Err(AnalysisError::TileLimitExceeded { .. })
    ));
}

#[test]
fn hdr_and_clipped_values_are_clamped_without_losing_alpha_policy() {
    let dimensions = dimensions(4, 1);
    let pixels = vec![
        RgbaF32Pixel::new(-2.0, 0.0, 0.0, 1.0),
        RgbaF32Pixel::new(2.0, 0.0, 0.0, 1.0),
        RgbaF32Pixel::new(0.5, 0.5, 0.5, 0.0),
        RgbaF32Pixel::new(0.5, 0.5, 0.5, 0.5),
    ];
    let request = request(
        AnalysisKind::RgbParade,
        Roi::full(dimensions),
        output(12, 8),
    )
    .with_alpha_policy(AnalysisAlphaPolicy::Weight);
    let result = AnalysisAggregator::aggregate(&request, raster(dimensions, &pixels), None)
        .expect("HDR/clipped analysis");

    assert_eq!(result.accepted_samples(), 4);
    assert_eq!(result.clipped_samples(), 2);
    assert_eq!(result.transparent_samples(), 1);
    assert!(
        result
            .planes()
            .iter()
            .flat_map(AnalysisPlane::counts)
            .all(|value| *value <= 256)
    );
    assert_eq!(count(&result), (2 * 256 + 128) * 3);

    let excluded_request = request.with_alpha_policy(AnalysisAlphaPolicy::ExcludeTransparent);
    let excluded =
        AnalysisAggregator::aggregate(&excluded_request, raster(dimensions, &pixels), None)
            .expect("transparent exclusion");
    assert_eq!(excluded.accepted_samples(), 3);
    assert_eq!(excluded.transparent_samples(), 1);
    assert_eq!(count(&excluded), 3 * 256 * 3);
}

#[test]
fn nonfinite_values_are_skipped_or_rejected_by_request_policy() {
    let dimensions = dimensions(4, 1);
    let pixels = vec![
        RgbaF32Pixel::new(0.1, 0.2, 0.3, 1.0),
        RgbaF32Pixel::new(f32::NAN, 0.2, 0.3, 1.0),
        RgbaF32Pixel::new(f32::INFINITY, 0.2, 0.3, 1.0),
        RgbaF32Pixel::new(0.9, 0.8, 0.7, 1.0),
    ];
    let skip = request(
        AnalysisKind::RgbWaveform,
        Roi::full(dimensions),
        output(8, 8),
    )
    .with_nonfinite_policy(HistogramNonFinitePolicy::Skip);
    let skipped = AnalysisAggregator::aggregate(&skip, raster(dimensions, &pixels), None)
        .expect("skip non-finite");
    assert_eq!(skipped.accepted_samples(), 2);
    assert_eq!(skipped.skipped_nonfinite_samples(), 2);

    let reject = skip.with_nonfinite_policy(HistogramNonFinitePolicy::Reject);
    assert!(matches!(
        AnalysisAggregator::aggregate(&reject, raster(dimensions, &pixels), None),
        Err(AnalysisError::NonFinite { .. })
    ));
}

#[test]
fn empty_roi_returns_a_bounded_empty_buffer() {
    let dimensions = dimensions(8, 8);
    let request = request(
        AnalysisKind::RgbWaveform,
        Roi::new(3, 3, 0, 2).expect("empty ROI"),
        output(16, 16),
    );
    let result = AnalysisAggregator::aggregate(
        &request,
        raster(
            dimensions,
            &rgb_ramp(dimensions.width(), dimensions.height()),
        ),
        None,
    )
    .expect("empty analysis");

    assert_eq!(result.accepted_samples(), 0);
    assert_eq!(result.occupied_bins(), 0);
    assert!(
        result
            .planes()
            .iter()
            .flat_map(AnalysisPlane::counts)
            .all(|value| *value == 0)
    );
    assert_eq!(result.request().roi(), request.roi());
}

#[test]
fn zoomed_roi_excludes_pixels_outside_the_requested_region() {
    let dimensions = dimensions(4, 4);
    let request = request(
        AnalysisKind::RgbParade,
        Roi::new(1, 1, 2, 2).expect("zoomed ROI"),
        output(12, 8),
    );
    let result = AnalysisAggregator::aggregate(
        &request,
        raster(
            dimensions,
            &color_bars()
                .into_iter()
                .cycle()
                .take(16)
                .collect::<Vec<_>>(),
        ),
        None,
    )
    .expect("zoomed analysis");

    assert_eq!(result.accepted_samples(), 4);
    assert_eq!(count(&result), 4 * 256 * 3);
    assert_eq!(result.provenance().roi(), request.roi());
}

#[test]
fn cancellation_is_observed_at_the_analysis_boundary() {
    let generation = PipelineGeneration::new(276).expect("generation");
    let scope = CancellationScope::root(generation);
    scope.cancel(CancellationReason::UserRequested);
    let request = request(
        AnalysisKind::Vectorscope,
        Roi::full(dimensions(8, 8)),
        output(32, 32),
    );

    assert!(matches!(
        AnalysisAggregator::aggregate(
            &request,
            raster(dimensions(8, 8), &rgb_ramp(8, 8)),
            Some(&scope),
        ),
        Err(AnalysisError::Cancelled(error))
            if error.stage() == Some(CancellationStage::Analysis)
    ));
}

#[test]
fn analysis_request_and_result_bounds_are_checked() {
    assert!(AnalysisOutputDimensions::new(0, 8).is_err());
    assert!(AnalysisOutputDimensions::new(8, 0).is_err());
    assert!(AnalysisOutputDimensions::new(u32::MAX, u32::MAX).is_err());

    let dimensions = dimensions(8, 8);
    let outside = request(
        AnalysisKind::RgbWaveform,
        Roi::new(7, 7, 2, 2).expect("representable but out-of-bounds ROI"),
        output(8, 8),
    );
    assert!(matches!(
        AnalysisAggregator::aggregate(&outside, raster(dimensions, &rgb_ramp(8, 8)), None,),
        Err(AnalysisError::RoiOutOfBounds)
    ));
}

#[test]
fn deterministic_grid_sampling_keeps_global_phase_across_tiles() {
    let dimensions = dimensions(10, 6);
    let pixels = rgb_ramp(dimensions.width(), dimensions.height());
    let request = request(
        AnalysisKind::RgbWaveform,
        Roi::full(dimensions),
        output(10, 16),
    )
    .with_sampling(AnalysisSampling::grid(3, 2, 1, 0).expect("grid sampling"));
    let full = AnalysisAggregator::aggregate(&request, raster(dimensions, &pixels), None)
        .expect("sampled full frame");
    let tiles = [
        Roi::new(0, 0, 4, 3).expect("tile"),
        Roi::new(4, 0, 6, 3).expect("tile"),
        Roi::new(0, 3, 4, 3).expect("tile"),
        Roi::new(4, 3, 6, 3).expect("tile"),
    ];
    let tiled_result =
        AnalysisAggregator::aggregate_tiles(&request, raster(dimensions, &pixels), &tiles, None)
            .expect("sampled tiles");

    assert_eq!(full, tiled_result);
    assert_eq!(full.accepted_samples(), 9);
}

#[test]
fn borrowed_mask_policy_and_identity_are_consumed_without_mask_evaluation() {
    let dimensions = dimensions(4, 1);
    let pixels = color_bars()[..4].to_vec();
    let mask_values = [0.0, 1.0, -1.0, 0.25];
    let mask = AnalysisMask::new(dimensions, &mask_values).expect("mask boundary");
    let request = request(
        AnalysisKind::RgbWaveform,
        Roi::full(dimensions),
        output(4, 8),
    )
    .with_mask_policy(HistogramMaskPolicy::IncludeNonZero);
    let result =
        AnalysisAggregator::aggregate_with_mask(&request, raster(dimensions, &pixels), mask, None)
            .expect("masked analysis");

    assert_eq!(result.accepted_samples(), 2);
    assert_eq!(result.statistics().masked_pixels(), 2);
    assert_eq!(result.provenance().mask_identity(), Some(mask.identity()));
    assert_eq!(count(&result), 2 * 256 * 3);

    let required = request.with_mask_policy(HistogramMaskPolicy::Require);
    assert!(matches!(
        AnalysisAggregator::aggregate(&required, raster(dimensions, &pixels), None),
        Err(AnalysisError::MaskRequired)
    ));
}

#[test]
fn color_transform_and_luminance_use_shared_color_math() {
    let dimensions = dimensions(1, 1);
    let encoded = [RgbaF32Pixel::new(0.5, 0.5, 0.5, 1.0)];
    let transformed_request = request(
        AnalysisKind::RgbWaveform,
        Roi::full(dimensions),
        output(1, 16),
    )
    .with_color_spaces(ColorEncoding::SrgbD65, ColorEncoding::LinearSrgbD65)
    .expect("explicit built-in spaces");
    let transformed = AnalysisAggregator::aggregate(
        &transformed_request,
        AnalysisRaster::from_rgba(dimensions, RgbaF32ColorEncoding::SrgbD65, &encoded)
            .expect("encoded raster"),
        None,
    )
    .expect("color transform");
    for plane in transformed.planes() {
        assert_eq!(plane.counts().iter().position(|count| *count != 0), Some(3));
    }
    assert_eq!(transformed.provenance().source_dimensions(), dimensions);
    assert_eq!(
        transformed.provenance().transform_source_color_space(),
        ColorEncoding::SrgbD65
    );
    assert_eq!(
        transformed.provenance().transform_target_color_space(),
        ColorEncoding::LinearSrgbD65
    );
    assert_eq!(
        transformed.provenance().numerical_contract(),
        ANALYSIS_NUMERICAL_CONTRACT
    );

    let red = [RgbaF32Pixel::new(1.0, 0.0, 0.0, 1.0)];
    let luminance_request = request(
        AnalysisKind::LuminanceWaveform,
        Roi::full(dimensions),
        output(1, 16),
    );
    let luminance =
        AnalysisAggregator::aggregate(&luminance_request, raster(dimensions, &red), None)
            .expect("relative luminance");
    assert_eq!(
        luminance.planes()[0]
            .counts()
            .iter()
            .position(|count| *count != 0),
        Some(3)
    );
}

#[test]
fn randomized_scalar_reference_matches_fixed_rgb_waveform() {
    let dimensions = dimensions(17, 13);
    let mut state = 0x2760_0158_u32;
    let pixels = (0..dimensions.pixel_count().expect("pixel count"))
        .map(|_| {
            let mut next = || {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                f32::from_bits(0x3f80_0000 | (state >> 9)) - 1.0
            };
            RgbaF32Pixel::new(next(), next(), next(), 1.0)
        })
        .collect::<Vec<_>>();
    let request = request(
        AnalysisKind::RgbWaveform,
        Roi::full(dimensions),
        output(11, 13),
    );
    let result = AnalysisAggregator::aggregate(&request, raster(dimensions, &pixels), None)
        .expect("randomized waveform");
    let reference = scalar_rgb_waveform_reference(dimensions, &pixels, 11, 13, 256);

    for (plane, expected) in result.planes().iter().zip(reference) {
        assert_eq!(plane.counts(), expected);
    }
}

#[test]
fn cache_scheduler_and_ownership_contracts_are_bounded() {
    let dimensions = dimensions(2, 1);
    let mut pixels = vec![
        RgbaF32Pixel::new(0.2, 0.3, 0.4, 1.0),
        RgbaF32Pixel::new(0.8, 0.7, 0.6, 1.0),
    ];
    let before = pixels.clone();
    let request = request(
        AnalysisKind::RgbWaveform,
        Roi::full(dimensions),
        output(2, 8),
    );
    let result = AnalysisAggregator::aggregate(&request, raster(dimensions, &pixels), None)
        .expect("analysis result");
    let retained = result.clone();
    assert_eq!(pixels, before, "generator must not mutate pipeline pixels");
    pixels[0] = RgbaF32Pixel::new(1.0, 1.0, 1.0, 1.0);
    assert_eq!(
        result, retained,
        "result must not retain mutable raster storage"
    );
    let changed = AnalysisAggregator::aggregate(&request, raster(dimensions, &pixels), None)
        .expect("changed-source analysis result");
    assert_ne!(
        result.provenance().cache_identity(),
        changed.provenance().cache_identity(),
        "exact source bits must participate in cache identity"
    );

    assert_eq!(
        request.scheduler_priority(),
        CpuPriority::BackgroundAnalysis
    );
    let claim = request.resource_claim(2).expect("scheduler claim");
    assert_eq!(claim.memory_bytes(), request.resident_bytes() * 3);
    assert_eq!(claim.max_parallelism(), 2);
    assert_eq!(
        CacheValue::descriptor(&result).resident_bytes(),
        request.resident_bytes()
    );
    assert_eq!(
        result
            .provenance()
            .cache_identity()
            .publication_target()
            .kind(),
        PublicationTargetKind::Cache
    );

    let oversized = AnalysisRequest::new(
        AnalysisKind::RgbParade,
        request.boundary().clone(),
        request.roi(),
        output(4096, 4096),
        request.source_color_space(),
        request.analysis_color_space(),
        request.sampling(),
        request.range(),
        request.mask_policy(),
        request.nonfinite_policy(),
        request.normalization(),
        request.graticule(),
        request.intensity(),
        request.alpha_policy(),
        request.waveform_orientation(),
    );
    assert!(oversized.is_err());
    assert!(request.resident_bytes() <= MAX_ANALYSIS_BYTES);
}

fn scalar_rgb_waveform_reference(
    dimensions: ImageDimensions,
    pixels: &[RgbaF32Pixel],
    output_width: u32,
    output_height: u32,
    intensity: u64,
) -> [Vec<u64>; 3] {
    let cells =
        usize::try_from(u64::from(output_width) * u64::from(output_height)).expect("fixture cells");
    let mut planes = [vec![0; cells], vec![0; cells], vec![0; cells]];
    for y in 0..dimensions.height() {
        for x in 0..dimensions.width() {
            let pixel_index =
                usize::try_from(u64::from(y) * u64::from(dimensions.width()) + u64::from(x))
                    .expect("fixture index");
            let output_x = x * output_width / dimensions.width();
            for (plane, value) in planes.iter_mut().zip([
                pixels[pixel_index].red(),
                pixels[pixel_index].green(),
                pixels[pixel_index].blue(),
            ]) {
                let output_y = unit_bin(value, output_height);
                let output_index = usize::try_from(
                    u64::from(output_y) * u64::from(output_width) + u64::from(output_x),
                )
                .expect("fixture output index");
                plane[output_index] += intensity;
            }
        }
    }
    planes
}

fn unit_bin(value: f32, bins: u32) -> u32 {
    if value >= 1.0 {
        return bins - 1;
    }
    let mut lower = 0_u32;
    let mut upper = bins;
    while lower + 1 < upper {
        let midpoint = lower + (upper - lower) / 2;
        if f64::from(value) >= f64::from(midpoint) / f64::from(bins) {
            lower = midpoint;
        } else {
            upper = midpoint;
        }
    }
    lower
}
