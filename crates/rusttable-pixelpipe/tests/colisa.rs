#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::suboptimal_flops,
    reason = "Colisa contract fixtures preserve native f32 values and arithmetic boundaries"
)]

//! Public pixelpipe contracts for Darktable's `src/iop/colisa.c`.
//!
//! These tests exercise the registered `rusttable.colisa` route without
//! reaching into pixelpipe implementation details. The direct leaf is used
//! only as the independent source-shaped oracle for the production boundary.

use std::cell::Cell;
use std::mem::size_of;

use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_gpu::{BackendPolicy, GpuRuntime, GpuRuntimeConfig, Platform, PowerPreference};
use rusttable_masks::{
    GeometryAncestry, MaskGeometry, MaskGraphBuilder, MaskIdentity, MaskNode, MaskRaster, MaskRoi,
    MaskSource,
};
use rusttable_pixelpipe::{
    CancellationReason, CpuPixelpipeError, CpuPixelpipeExecutor, CpuPixelpipeOutputMode,
    CpuPixelpipeSnapshot, CpuTilePlan, PipelineGeneration, PixelpipeBackend,
    PixelpipeExecutionService, RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image, RgbaF32Pixel,
};
use rusttable_processing::operations::colisa::{
    COLISA_TABLE_BYTES, ColisaError, ColisaFormat, ColisaParametersV1, ColisaPlan, ColisaRaster,
};
use rusttable_processing::{CompiledOperationGraph, RasterDimensions};

const COLISA_CONTRAST: f64 = 0.5;
const COLISA_BRIGHTNESS: f64 = 0.25;
const COLISA_SATURATION: f64 = 0.5;

fn scalar_operation(id: u128, key: &str, parameters: &[(&str, f64)]) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).expect("nonzero operation ID"),
        OperationKey::new(key).expect("valid operation key"),
        true,
        OperationOpacity::ONE,
        parameters.iter().map(|(name, value)| {
            (
                ParameterName::new(*name).expect("valid parameter name"),
                ParameterValue::Scalar(FiniteF64::new(*value).expect("finite parameter")),
            )
        }),
    )
    .expect("valid scalar operation")
}

fn operation_with_opacity(
    id: u128,
    opacity: OperationOpacity,
    parameters: &[(&str, f64)],
) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).expect("nonzero operation ID"),
        OperationKey::new("rusttable.colisa").expect("valid Colisa operation key"),
        true,
        opacity,
        parameters.iter().map(|(name, value)| {
            (
                ParameterName::new(*name).expect("valid parameter name"),
                ParameterValue::Scalar(FiniteF64::new(*value).expect("finite parameter")),
            )
        }),
    )
    .expect("valid Colisa operation")
}

fn colisa_operation(id: u128, parameters: [f64; 3]) -> Operation {
    scalar_operation(
        id,
        "rusttable.colisa",
        &[
            ("contrast", parameters[0]),
            ("brightness", parameters[1]),
            ("saturation", parameters[2]),
        ],
    )
}

fn colorcontrast_operation(id: u128) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).expect("nonzero operation ID"),
        OperationKey::new("rusttable.colorcontrast").expect("valid Color Contrast key"),
        true,
        OperationOpacity::ONE,
        [
            (
                "a_steepness",
                ParameterValue::Scalar(FiniteF64::new(2.0).expect("finite parameter")),
            ),
            (
                "a_offset",
                ParameterValue::Scalar(FiniteF64::new(3.0).expect("finite parameter")),
            ),
            (
                "b_steepness",
                ParameterValue::Scalar(FiniteF64::new(1.5).expect("finite parameter")),
            ),
            (
                "b_offset",
                ParameterValue::Scalar(FiniteF64::new(-4.0).expect("finite parameter")),
            ),
            ("unbound", ParameterValue::Integer(1)),
        ]
        .into_iter()
        .map(|(name, value)| {
            (
                ParameterName::new(name).expect("valid Color Contrast parameter"),
                value,
            )
        }),
    )
    .expect("valid Color Contrast operation")
}

fn graph(operations: impl IntoIterator<Item = Operation>) -> CompiledOperationGraph {
    let edit = Edit::from_parts(
        EditId::new(0x00C0_115A).expect("nonzero edit ID"),
        PhotoId::new(0x00C0_115B).expect("nonzero photo ID"),
        Revision::ZERO,
        Revision::from_u64(1),
        operations,
    )
    .expect("valid Colisa edit");
    CompiledOperationGraph::compile(&edit).expect("registered Colisa graph")
}

fn lab_image(width: u32, height: u32) -> RgbaF32Image {
    let dimensions = RasterDimensions::new(width, height).expect("nonzero dimensions");
    let pixels = (0..dimensions.pixel_count())
        .map(|index| {
            let x = f32::from(u16::try_from(index % u64::from(width)).expect("small x"));
            let y = f32::from(u16::try_from(index / u64::from(width)).expect("small y"));
            RgbaF32Pixel::new(
                35.0 + (x + y) * 2.5,
                (x - 2.0) * 7.0,
                (y - 1.0) * -5.0,
                0.2 + f32::from(u8::try_from(index % 7).expect("small alpha step")) * 0.1,
            )
        })
        .collect();
    RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::LabD50),
        pixels,
    )
    .expect("valid Lab image")
}

fn snapshot(
    image: RgbaF32Image,
    operations: impl IntoIterator<Item = Operation>,
) -> CpuPixelpipeSnapshot {
    CpuPixelpipeSnapshot::try_new(image, graph(operations), CpuPixelpipeOutputMode::FullExport)
        .expect("valid Colisa snapshot")
}

const fn colisa_parameters() -> ColisaParametersV1 {
    ColisaParametersV1::new(
        COLISA_CONTRAST as f32,
        COLISA_BRIGHTNESS as f32,
        COLISA_SATURATION as f32,
    )
}

fn direct_leaf_output(image: &RgbaF32Image, parameters: ColisaParametersV1) -> Vec<f32> {
    let samples = image
        .pixels()
        .iter()
        .flat_map(|pixel| [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()])
        .collect::<Vec<_>>();
    ColisaPlan::compile(parameters, COLISA_TABLE_BYTES)
        .expect("compile direct Colisa plan")
        .execute(
            ColisaRaster::new(
                &samples,
                image.descriptor().dimensions().width(),
                image.descriptor().dimensions().height(),
                ColisaFormat::LabF32x4,
            ),
            samples.len() * size_of::<f32>(),
            || false,
        )
        .expect("execute direct Colisa plan")
}

fn mask_graph(operation_id: u128, width: u32, height: u32) -> rusttable_masks::MaskGraph {
    let identity = MaskIdentity::new(0x00C0_115C, 1, 1, 1);
    let node = MaskNode::new(
        identity,
        "colisa-runtime-mask",
        MaskSource::Raster,
        MaskGeometry::new(
            GeometryAncestry::identity(),
            MaskRoi::full(width, height),
            true,
        ),
        Some(
            MaskRaster::new(
                width,
                height,
                vec![0.5; usize::try_from(width * height).expect("small mask")],
            )
            .expect("valid Colisa mask"),
        ),
        [],
    )
    .expect("valid Colisa mask node");
    MaskGraphBuilder::new()
        .add_mask(node)
        .add_edge(identity, operation_id, 1)
        .build()
        .expect("valid Colisa mask graph")
}

#[test]
fn colisa_leaf_accepts_lab_and_rejects_non_lab() {
    let parameters = colisa_parameters();
    let plan = ColisaPlan::compile(parameters, COLISA_TABLE_BYTES).expect("compile Colisa plan");
    let samples = [50.0_f32, 10.0, -20.0, 0.75];

    assert!(
        plan.execute(
            ColisaRaster::new(&samples, 1, 1, ColisaFormat::LabF32x4),
            samples.len() * size_of::<f32>(),
            || false,
        )
        .is_ok()
    );
    assert_eq!(
        plan.execute(
            ColisaRaster::new(&samples, 1, 1, ColisaFormat::RgbaF32x4),
            samples.len() * size_of::<f32>(),
            || false,
        ),
        Err(ColisaError::UnsupportedFormat)
    );
}

#[test]
fn direct_colisa_leaf_matches_registered_production_output() {
    let input = lab_image(5, 3);
    let direct = direct_leaf_output(&input, colisa_parameters());
    let production = CpuPixelpipeExecutor
        .execute(&snapshot(
            input,
            [colisa_operation(
                1,
                [COLISA_CONTRAST, COLISA_BRIGHTNESS, COLISA_SATURATION],
            )],
        ))
        .expect("production Colisa execution");

    let actual = production
        .image()
        .pixels()
        .iter()
        .flat_map(|pixel| [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()])
        .collect::<Vec<_>>();
    assert_eq!(actual, direct);
}

#[test]
fn colisa_full_frame_and_tiled_outputs_are_identical() {
    let snapshot = snapshot(
        lab_image(11, 7),
        [colisa_operation(
            2,
            [COLISA_CONTRAST, COLISA_BRIGHTNESS, COLISA_SATURATION],
        )],
    );
    let executor = CpuPixelpipeExecutor;
    let full = executor.execute(&snapshot).expect("full-frame Colisa");
    let tiled = executor
        .execute_tiled(&snapshot, CpuTilePlan::new(3, 2).expect("tile plan"))
        .expect("tiled Colisa");

    assert_eq!(tiled.image(), full.image());
    assert_eq!(tiled.receipt(), full.receipt());
}

#[test]
fn mixed_lab_chain_preserves_authored_colisa_order() {
    let input = lab_image(5, 3);
    let executor = CpuPixelpipeExecutor;
    let colisa_then_contrast = snapshot(
        input.clone(),
        [
            colisa_operation(3, [0.5, 0.25, 0.5]),
            colorcontrast_operation(4),
        ],
    );
    let colisa_only = executor
        .execute(&snapshot(
            input.clone(),
            [colisa_operation(5, [0.5, 0.25, 0.5])],
        ))
        .expect("Colisa stage");
    let separate_contrast = executor
        .execute(&snapshot(
            colisa_only.image().clone(),
            [colorcontrast_operation(6)],
        ))
        .expect("Color Contrast stage after Colisa");
    let authored = executor
        .execute(&colisa_then_contrast)
        .expect("mixed Colisa and Color Contrast chain");

    assert_eq!(authored.image(), separate_contrast.image());
    assert_eq!(
        authored
            .receipt()
            .nodes()
            .iter()
            .map(|node| node.operation_id().get())
            .collect::<Vec<_>>(),
        [3, 4]
    );

    let reordered = executor
        .execute(&snapshot(
            input,
            [
                colorcontrast_operation(7),
                colisa_operation(8, [0.5, 0.25, 0.5]),
            ],
        ))
        .expect("reordered mixed Lab chain");
    assert_ne!(reordered.image(), authored.image());
}

#[test]
fn colisa_cancellation_does_not_publish_a_partial_result() {
    let input = lab_image(32, 32);
    let snapshot = snapshot(
        input,
        [colisa_operation(
            9,
            [COLISA_CONTRAST, COLISA_BRIGHTNESS, COLISA_SATURATION],
        )],
    );
    let scope = rusttable_pixelpipe::CancellationScope::root(
        PipelineGeneration::new(9).expect("nonzero generation"),
    );
    scope.cancel(CancellationReason::EditChanged);
    let service = PixelpipeExecutionService::cpu_only();

    assert!(matches!(
        service.execute_with_cancellation(&snapshot, &scope),
        Err(CpuPixelpipeError::Cancelled(_))
    ));
    let published = service
        .execute(&snapshot)
        .expect("uncancelled Colisa execution");
    assert_eq!(published.receipt().snapshot_identity(), snapshot.identity());
    assert_eq!(
        published.receipt().backend(),
        PixelpipeBackend::CpuCanonical
    );
}

#[test]
fn colisa_memory_budgets_reach_both_leaf_boundaries() {
    let parameters = colisa_parameters();
    assert_eq!(
        ColisaPlan::compile(parameters, COLISA_TABLE_BYTES - 1),
        Err(ColisaError::WorkingMemoryBudgetExceeded {
            required: COLISA_TABLE_BYTES,
            budget: COLISA_TABLE_BYTES - 1,
        })
    );

    let plan = ColisaPlan::compile(parameters, COLISA_TABLE_BYTES).expect("compile Colisa plan");
    let samples = [50.0_f32, 10.0, -20.0, 0.75];
    assert_eq!(
        plan.execute(
            ColisaRaster::new(&samples, 1, 1, ColisaFormat::LabF32x4),
            samples.len() * size_of::<f32>() - 1,
            || false,
        ),
        Err(ColisaError::OutputMemoryBudgetExceeded {
            required: samples.len() * size_of::<f32>(),
            budget: samples.len() * size_of::<f32>() - 1,
        })
    );
}

#[test]
fn colisa_rejects_runtime_masks_and_nonunit_opacity() {
    let operation_id = 10;
    let parameters = [COLISA_CONTRAST, 0.0, COLISA_SATURATION];
    let partial = snapshot(
        lab_image(4, 3),
        [operation_with_opacity(
            operation_id,
            OperationOpacity::new(0.5).expect("partial opacity"),
            &[
                ("contrast", parameters[0]),
                ("brightness", parameters[1]),
                ("saturation", parameters[2]),
            ],
        )],
    );
    let masked = snapshot(
        lab_image(4, 3),
        [colisa_operation(operation_id, parameters)],
    )
    .with_mask_graph(mask_graph(operation_id, 4, 3));

    for snapshot in [partial, masked] {
        let error = CpuPixelpipeExecutor
            .execute(&snapshot)
            .expect_err("unsupported Colisa blend must fail closed");
        assert!(matches!(
            error,
            CpuPixelpipeError::Evaluation {
                source: rusttable_processing::EvaluationError::OperationExecution {
                    operation_id: actual_id,
                    reason,
                    ..
                }
            } if actual_id.get() == operation_id
                && reason == "Colisa masks and outer blending are deferred; only unmasked full-opacity execution is available"
        ));
    }
}

#[tokio::test]
async fn colisa_uses_cpu_when_a_gpu_service_is_requested() {
    let runtime = GpuRuntime::initialize(GpuRuntimeConfig {
        policy: BackendPolicy {
            platform: Platform::current(),
            backends: Vec::new(),
            power: PowerPreference::Unspecified,
        },
        allow_cpu_fallback: true,
        ..GpuRuntimeConfig::default()
    })
    .await
    .expect("deterministic CPU-only GPU runtime");
    assert!(runtime.is_cpu_only());

    let snapshot = snapshot(
        lab_image(5, 3),
        [colisa_operation(
            11,
            [COLISA_CONTRAST, COLISA_BRIGHTNESS, COLISA_SATURATION],
        )],
    );
    let canonical = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("canonical Colisa execution");
    let selected = PixelpipeExecutionService::with_gpu(runtime)
        .execute(&snapshot)
        .expect("GPU-requested Colisa execution");

    assert_eq!(selected.image(), canonical.image());
    assert_eq!(selected.receipt().backend(), PixelpipeBackend::CpuCanonical);
    assert_eq!(selected.receipt().dispatches(), 0);
    assert!(selected.receipt().gpu_fallback().is_none());
}

#[test]
fn colisa_snapshot_identity_changes_for_each_parameter() {
    let baseline = snapshot(lab_image(2, 2), [colisa_operation(12, [0.0, 0.0, 0.0])]).identity();
    for parameters in [[0.25, 0.0, 0.0], [0.0, 0.25, 0.0], [0.0, 0.0, 0.25]] {
        assert_ne!(
            baseline,
            snapshot(lab_image(2, 2), [colisa_operation(12, parameters)]).identity()
        );
    }
}

#[test]
fn direct_leaf_cancellation_preserves_destination() {
    let plan = ColisaPlan::compile(colisa_parameters(), COLISA_TABLE_BYTES).expect("Colisa plan");
    let input = (0..257)
        .flat_map(|_| [50.0_f32, 10.0, -20.0, 0.75])
        .collect::<Vec<_>>();
    let polls = Cell::new(0_u32);
    let mut destination = vec![-1.0_f32, -2.0, -3.0];
    let error = plan
        .execute_and_publish(
            ColisaRaster::new(&input, 257, 1, ColisaFormat::LabF32x4),
            &mut destination,
            input.len() * size_of::<f32>(),
            || {
                let next = polls.get() + 1;
                polls.set(next);
                next >= 3
            },
        )
        .expect_err("cancellation must precede publication");
    assert_eq!(error, ColisaError::Cancelled);
    assert_eq!(destination, [-1.0_f32, -2.0, -3.0]);
}
