//! Source-derived CPU pixelpipe coverage for Darktable's `src/iop/soften.c`.
//!
//! The native module transforms every pixel into an overexposed HSL layer,
//! applies an eight-pass four-channel box mean, and linearly mixes that layer
//! with the source. Its tiling callback supplies neighborhood overlap, which
//! the production Rust path must honor without dropping the native RGBA layout.

use std::time::Duration;

use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_masks::{
    GeometryAncestry, MaskGeometry, MaskGraphBuilder, MaskIdentity, MaskNode, MaskRaster, MaskRoi,
    MaskSource,
};
use rusttable_pixelpipe::{
    CancellationDeadline, CancellationReason, CancellationScope, CpuPixelpipeError,
    CpuPixelpipeExecutor, CpuPixelpipeOutputMode, CpuPixelpipeScaleContext, CpuPixelpipeSnapshot,
    CpuTilePlan, PipelineGeneration, PixelpipeBackend, PixelpipeExecutionService,
    RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image, RgbaF32Pixel,
};
use rusttable_processing::{CompiledOperationGraph, RasterDimensions};

const BOX_ITERATIONS: usize = 8;

fn operation(id: u128, opacity: OperationOpacity, parameters: &[(&str, f64)]) -> Operation {
    operation_with_state(id, "rusttable.soften", true, opacity, parameters)
}

fn operation_with_state(
    id: u128,
    key: &str,
    enabled: bool,
    opacity: OperationOpacity,
    parameters: &[(&str, f64)],
) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).expect("nonzero operation ID"),
        OperationKey::new(key).expect("valid operation key"),
        enabled,
        opacity,
        parameters.iter().map(|(name, value)| {
            (
                ParameterName::new(*name).expect("valid parameter name"),
                ParameterValue::Scalar(FiniteF64::new(*value).expect("finite parameter")),
            )
        }),
    )
    .expect("valid operation")
}

fn soften_operation(
    id: u128,
    opacity: OperationOpacity,
    size: f64,
    saturation: f64,
    brightness: f64,
    amount: f64,
) -> Operation {
    operation(
        id,
        opacity,
        &[
            ("size", size),
            ("saturation", saturation),
            ("brightness", brightness),
            ("amount", amount),
        ],
    )
}

fn graph(operations: Vec<Operation>) -> CompiledOperationGraph {
    let edit = Edit::from_parts(
        EditId::new(1).expect("nonzero edit ID"),
        PhotoId::new(2).expect("nonzero photo ID"),
        Revision::ZERO,
        Revision::from_u64(3),
        operations,
    )
    .expect("valid edit");
    CompiledOperationGraph::compile(&edit).expect("registered Soften graph")
}

#[expect(
    clippy::suboptimal_flops,
    reason = "Preserve source-derived Soften fixture arithmetic order"
)]
fn input(width: u32, height: u32) -> RgbaF32Image {
    let dimensions = RasterDimensions::new(width, height).expect("nonzero dimensions");
    let pixels = (0..dimensions.pixel_count())
        .map(|index| {
            let x = f32::from(u16::try_from(index % u64::from(width)).expect("test x fits in u16"));
            let y = f32::from(u16::try_from(index / u64::from(width)).expect("test y fits in u16"));
            let alpha_step = f32::from(u8::try_from(index % 11).expect("alpha step fits in u8"));
            RgbaF32Pixel::new(
                0.08 + (x % 17.0) * 0.021 + (y % 13.0) * 0.009,
                0.14 + (x % 19.0) * 0.014 + (y % 11.0) * 0.012,
                0.21 + (x % 23.0) * 0.011 + (y % 7.0) * 0.015,
                0.11 + alpha_step * 0.071,
            )
        })
        .collect();
    RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::LinearSrgbD65),
        pixels,
    )
    .expect("valid linear RGBA input")
}

#[expect(
    clippy::too_many_arguments,
    reason = "The source Soften fixture includes each native scale and parameter field"
)]
fn soften_snapshot(
    width: u32,
    height: u32,
    id: u128,
    opacity: OperationOpacity,
    size: f64,
    saturation: f64,
    brightness: f64,
    amount: f64,
) -> CpuPixelpipeSnapshot {
    CpuPixelpipeSnapshot::new(
        input(width, height),
        graph(vec![soften_operation(
            id, opacity, size, saturation, brightness, amount,
        )]),
        CpuPixelpipeOutputMode::FullExport,
    )
}

fn mask_graph(
    operation_id: u128,
    width: u32,
    height: u32,
    values: Vec<f32>,
) -> rusttable_masks::MaskGraph {
    let identity = MaskIdentity::new(2, 3, 7, 1);
    let node = MaskNode::new(
        identity,
        "soften-mask",
        MaskSource::Raster,
        MaskGeometry::new(
            GeometryAncestry::identity(),
            MaskRoi::full(width, height),
            true,
        ),
        Some(MaskRaster::new(width, height, values).expect("valid Soften mask")),
        [],
    )
    .expect("valid Soften mask node");
    MaskGraphBuilder::new()
        .add_mask(node)
        .add_edge(identity, operation_id, 1)
        .build()
        .expect("valid Soften mask graph")
}

fn impulse_input(width: u32, height: u32) -> RgbaF32Image {
    let dimensions = RasterDimensions::new(width, height).expect("nonzero dimensions");
    let center_x = width / 2;
    let center_y = height / 2;
    let pixels = (0..dimensions.pixel_count())
        .map(|index| {
            let x = u32::try_from(index % u64::from(width)).expect("impulse x fits");
            let y = u32::try_from(index / u64::from(width)).expect("impulse y fits");
            let value = f32::from(u8::from(x == center_x && y == center_y));
            let alpha = 0.2 + f32::from(u8::try_from(index % 7).expect("alpha fits")) * 0.1;
            RgbaF32Pixel::new(value, value, value, alpha)
        })
        .collect();
    RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::LinearSrgbD65),
        pixels,
    )
    .expect("valid impulse input")
}

#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Preserve native Soften's checked-width and integer-radius conversions"
)]
fn native_radius(width: u32, height: u32, size: f32, roi_scale: f32, piece_iscale: f32) -> usize {
    // soften.c: mrad = hypotf(iwidth * iscale, iheight * iscale) * .01f;
    // rad is converted to int before the ROI scale is applied and ceiled.
    let scaled_width = width as f32 * piece_iscale;
    let scaled_height = height as f32 * piece_iscale;
    let maximum = scaled_width.hypot(scaled_height) * 0.01;
    let maximum_radius = maximum as u32;
    let requested = (maximum_radius as f32 * (size + 1.0).min(100.0) / 100.0) as u32;
    let requested = requested.min(maximum_radius);
    let radius = (requested as f32 * roi_scale / piece_iscale).ceil() as u32;
    usize::try_from(maximum_radius.min(radius)).expect("native radius fits usize")
}

#[expect(
    clippy::cast_precision_loss,
    reason = "Preserve native Soften's sample-count division order in the parity oracle"
)]
fn box_mean_gray(mut values: Vec<f32>, width: usize, height: usize, radius: usize) -> Vec<f32> {
    for _ in 0..BOX_ITERATIONS {
        let mut horizontal = vec![0.0; values.len()];
        for y in 0..height {
            for x in 0..width {
                let start = x.saturating_sub(radius);
                let end = (x + radius + 1).min(width);
                let (sum, count) = values[y * width + start..y * width + end]
                    .iter()
                    .fold((0.0_f32, 0_usize), |(sum, count), value| {
                        (sum + value, count + 1)
                    });
                horizontal[y * width + x] = sum / count as f32;
            }
        }
        for y in 0..height {
            for x in 0..width {
                let start = y.saturating_sub(radius);
                let end = (y + radius + 1).min(height);
                let mut sum = 0.0_f32;
                for source_y in start..end {
                    sum += horizontal[source_y * width + x];
                }
                values[y * width + x] = sum / (end - start) as f32;
            }
        }
    }
    values
}

#[test]
fn disabled_single_node_soften_is_an_exact_pass_through() {
    let source = input(137, 113);
    let snapshot = CpuPixelpipeSnapshot::new(
        source.clone(),
        graph(vec![operation_with_state(
            9,
            "rusttable.soften",
            false,
            OperationOpacity::ONE,
            &[
                ("size", 100.0),
                ("saturation", 78.0),
                ("brightness", 0.37),
                ("amount", 73.0),
            ],
        )]),
        CpuPixelpipeOutputMode::FullExport,
    );
    let executor = CpuPixelpipeExecutor;

    assert_eq!(
        executor
            .execute(&snapshot)
            .expect("disabled Soften")
            .image(),
        &source
    );
    let tiled = PixelpipeExecutionService::cpu_only()
        .execute_tiled(&snapshot, CpuTilePlan::new(17, 13).expect("tile plan"))
        .expect("tiled disabled Soften");
    assert_eq!(tiled.image(), &source);
    assert_eq!(
        tiled.receipt().backend(),
        PixelpipeBackend::CpuTiledFallback
    );
}

#[test]
#[expect(
    clippy::suboptimal_flops,
    reason = "Preserve the native Soften alpha blend arithmetic order"
)]
fn mixed_soften_graph_preserves_rgba_and_uses_snapshot_scale() {
    let source = input(137, 113);
    let scale = CpuPixelpipeScaleContext::new(0.5, 2.0).expect("native scale context");
    let soften = soften_operation(11, OperationOpacity::ONE, 100.0, 78.0, 0.37, 50.0);
    let single = CpuPixelpipeSnapshot::new(
        source.clone(),
        graph(vec![soften]),
        CpuPixelpipeOutputMode::FullExport,
    )
    .with_scale_context(scale);
    let mixed = CpuPixelpipeSnapshot::new(
        source.clone(),
        graph(vec![
            operation_with_state(
                12,
                "rusttable.linear_offset",
                true,
                OperationOpacity::ONE,
                &[("value", 0.0)],
            ),
            soften_operation(13, OperationOpacity::ONE, 100.0, 78.0, 0.37, 50.0),
        ]),
        CpuPixelpipeOutputMode::FullExport,
    )
    .with_scale_context(scale);

    let executor = CpuPixelpipeExecutor;
    let expected = executor.execute(&single).expect("single Soften");
    let actual = executor.execute(&mixed).expect("mixed Soften graph");
    assert_eq!(actual.image(), expected.image());
    assert!(
        (actual.image().pixels()[0].alpha() - source.pixels()[0].alpha() * 0.5).abs() < 1.0e-7,
        "mixed Soften must retain its native alpha blend"
    );
}

#[test]
fn tiled_soften_uses_the_neighborhood_executor_instead_of_full_frame_fallback() {
    let snapshot = soften_snapshot(137, 113, 10, OperationOpacity::ONE, 100.0, 78.0, 0.37, 73.0);
    let service = PixelpipeExecutionService::cpu_only();
    let tiled = service
        .execute_tiled(&snapshot, CpuTilePlan::new(17, 13).expect("tile plan"))
        .expect("tiled Soften");

    assert_eq!(
        tiled.receipt().backend(),
        PixelpipeBackend::CpuTiledFallback
    );
    assert!(tiled.receipt().tiling().is_some());
    assert_eq!(
        tiled.image().descriptor().dimensions(),
        snapshot.input().descriptor().dimensions()
    );
}

#[test]
fn nonunit_scale_uses_soften_c_roi_radius_formula() {
    const WIDTH: u32 = 400;
    const HEIGHT: u32 = 300;
    const ROI_SCALE: f32 = 0.5;
    const PIECE_ISCALE: f32 = 2.0;
    let radius = native_radius(WIDTH, HEIGHT, 100.0, ROI_SCALE, PIECE_ISCALE);
    assert_eq!(radius, 3, "source-derived scaled radius");

    let snapshot = CpuPixelpipeSnapshot::new(
        impulse_input(WIDTH, HEIGHT),
        graph(vec![soften_operation(
            20,
            OperationOpacity::ONE,
            100.0,
            0.0,
            0.0,
            100.0,
        )]),
        CpuPixelpipeOutputMode::FullExport,
    )
    .with_scale_context(
        CpuPixelpipeScaleContext::new(ROI_SCALE, PIECE_ISCALE)
            .expect("native non-unit scale context"),
    );
    let result = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("scaled Soften");
    let expected = box_mean_gray(
        vec![0.0; usize::try_from(WIDTH * HEIGHT).expect("test pixel count fits")]
            .into_iter()
            .enumerate()
            .map(|(index, _)| {
                let x = u32::try_from(index % usize::try_from(WIDTH).expect("width fits"))
                    .expect("x fits");
                let y = u32::try_from(index / usize::try_from(WIDTH).expect("width fits"))
                    .expect("y fits");
                f32::from(u8::from(x == WIDTH / 2 && y == HEIGHT / 2))
            })
            .collect(),
        usize::try_from(WIDTH).expect("width fits"),
        usize::try_from(HEIGHT).expect("height fits"),
        radius,
    );
    let center = usize::try_from(HEIGHT / 2 * WIDTH + WIDTH / 2).expect("center fits");
    assert!(
        (result.image().pixels()[center].red() - expected[center]).abs() < 1.0e-6,
        "actual={} expected={} radius={radius}",
        result.image().pixels()[center].red(),
        expected[center]
    );
    assert_eq!(
        result.image().pixels()[center].red().to_bits(),
        result.image().pixels()[center].green().to_bits()
    );
    assert_eq!(
        result.image().pixels()[center].red().to_bits(),
        result.image().pixels()[center].blue().to_bits()
    );
}

#[test]
fn four_channel_native_blend_scales_alpha_by_the_soften_amount() {
    const AMOUNT: f32 = 50.0;
    let source = input(137, 113);
    let result = CpuPixelpipeExecutor
        .execute(&soften_snapshot(
            137,
            113,
            30,
            OperationOpacity::ONE,
            100.0,
            100.0,
            0.33,
            f64::from(AMOUNT),
        ))
        .expect("four-channel Soften");

    for (index, (actual, source)) in result
        .image()
        .pixels()
        .iter()
        .zip(source.pixels())
        .enumerate()
    {
        // soften.c calls hsl2rgb on all four floats; that helper writes zero to
        // the fourth float, then dt_iop_image_linear_blend mixes all channels.
        let expected_alpha = source.alpha() * (1.0 - AMOUNT / 100.0);
        assert!(
            (actual.alpha() - expected_alpha).abs() < 1.0e-6,
            "alpha {index}: actual={} expected={expected_alpha}",
            actual.alpha()
        );
    }
}

#[test]
#[expect(
    clippy::suboptimal_flops,
    reason = "Preserve the native Soften masked blend arithmetic order"
)]
fn mask_and_operation_opacity_scale_the_soften_candidate_and_alpha() {
    const WIDTH: u32 = 137;
    const HEIGHT: u32 = 113;
    const OPERATION_ID: u128 = 40;
    const OPACITY: f32 = 0.4;
    let source = input(WIDTH, HEIGHT);
    let full_candidate = CpuPixelpipeExecutor
        .execute(&CpuPixelpipeSnapshot::new(
            source.clone(),
            graph(vec![soften_operation(
                OPERATION_ID,
                OperationOpacity::ONE,
                100.0,
                78.0,
                0.37,
                100.0,
            )]),
            CpuPixelpipeOutputMode::FullExport,
        ))
        .expect("full-opacity Soften candidate");
    let mask_values = (0..u64::from(WIDTH) * u64::from(HEIGHT))
        .map(|index| {
            let x = u32::try_from(index % u64::from(WIDTH)).expect("mask x fits");
            if x < WIDTH / 3 {
                0.0
            } else if x < WIDTH * 2 / 3 {
                0.5
            } else {
                1.0
            }
        })
        .collect::<Vec<_>>();
    let snapshot = CpuPixelpipeSnapshot::new(
        source.clone(),
        graph(vec![soften_operation(
            OPERATION_ID,
            OperationOpacity::new(f64::from(OPACITY)).expect("partial opacity"),
            100.0,
            78.0,
            0.37,
            100.0,
        )]),
        CpuPixelpipeOutputMode::FullExport,
    )
    .with_mask_graph(mask_graph(OPERATION_ID, WIDTH, HEIGHT, mask_values.clone()));
    let result = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("masked and partially opaque Soften");

    for (index, ((actual, candidate), source)) in result
        .image()
        .pixels()
        .iter()
        .zip(full_candidate.image().pixels())
        .zip(source.pixels())
        .enumerate()
    {
        let x = u32::try_from(index % usize::try_from(WIDTH).expect("width fits")).expect("x fits");
        let coverage = mask_values[index] * OPACITY;
        for (actual, expected) in [
            (
                actual.red(),
                source.red() + (candidate.red() - source.red()) * coverage,
            ),
            (
                actual.green(),
                source.green() + (candidate.green() - source.green()) * coverage,
            ),
            (
                actual.blue(),
                source.blue() + (candidate.blue() - source.blue()) * coverage,
            ),
        ] {
            assert!((actual - expected).abs() < 1.0e-6, "masked Soften at x={x}");
        }
        let expected_alpha = source.alpha() * (1.0 - coverage);
        assert!(
            (actual.alpha() - expected_alpha).abs() < 1.0e-6,
            "masked alpha at x={x}: actual={} expected={expected_alpha}",
            actual.alpha()
        );
    }
}

#[test]
fn cancellation_during_full_frame_soften_exposes_no_partial_result() {
    let snapshot = soften_snapshot(
        512,
        384,
        50,
        OperationOpacity::ONE,
        100.0,
        78.0,
        0.37,
        100.0,
    );
    let scope = CancellationScope::root(PipelineGeneration::new(6).expect("generation"))
        .with_deadline(CancellationDeadline::after(Duration::from_millis(1)));
    let error = CpuPixelpipeExecutor
        .execute_with_cancellation(&snapshot, &scope)
        .expect_err("deadline must interrupt full-frame Soften publication");
    assert!(matches!(error, CpuPixelpipeError::Cancelled(_)));

    let published = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("fresh uncancelled Soften publishes the complete frame");
    assert_eq!(published.image().pixels().len(), 512 * 384);
}

#[test]
fn cpu_publication_is_canonical_and_cancellation_does_not_poison_soften_cache() {
    let snapshot = soften_snapshot(137, 113, 60, OperationOpacity::ONE, 100.0, 78.0, 0.37, 73.0);
    let service = PixelpipeExecutionService::cpu_only();
    let scope = CancellationScope::root(PipelineGeneration::new(7).expect("generation"));
    scope.cancel(CancellationReason::Shutdown);
    assert!(matches!(
        service.execute_with_cancellation(&snapshot, &scope),
        Err(CpuPixelpipeError::Cancelled(_))
    ));

    let published = service
        .execute(&snapshot)
        .expect("uncancelled Soften remains publishable");
    assert_eq!(published.receipt().snapshot_identity(), snapshot.identity());
    assert_eq!(
        published.receipt().backend(),
        PixelpipeBackend::CpuCanonical
    );
    assert_eq!(published.receipt().dispatches(), 0);
    assert!(published.receipt().tiling().is_none());

    let tiled = service
        .execute_tiled(&snapshot, CpuTilePlan::new(17, 13).expect("tile plan"))
        .expect("tiled Soften publication");
    assert_eq!(
        tiled.receipt().backend(),
        PixelpipeBackend::CpuTiledFallback
    );
    assert!(tiled.receipt().tiling().is_some());
    assert_eq!(tiled.image().descriptor(), published.image().descriptor());
    assert_eq!(
        tiled.image().pixels().len(),
        published.image().pixels().len()
    );
}
