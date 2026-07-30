#![allow(clippy::float_cmp)]

use rusttable_color::ColorEncoding;
use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterText, ParameterValue, PhotoId, Revision,
};
use rusttable_masks::MaskRaster;
use rusttable_processing::descriptor::{
    AlphaPolicy, OperationFlags, RoiKind, channelmixer_descriptor,
};
use rusttable_processing::operations::channelmixer::{
    ChannelMixerAlgorithm, ChannelMixerOperationMode, ChannelMixerParametersV2, ChannelMixerPixel,
    ChannelMixerPlan,
};
use rusttable_processing::{
    CompiledOperationGraph, DeviceCapabilitySnapshot, FrameBoundaryMode, FrameBoundaryOptions,
    LinearRgb, OperationMaskSet, ProcessingOperation, ProcessingOperationKind, RasterDimensions,
    WorkingRgbImage, builtin_registry, evaluate_graph_at_frame_boundaries_with_masks,
};

fn array(values: [f32; 7]) -> ParameterValue {
    let text = values
        .into_iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(",");
    ParameterValue::Text(ParameterText::new(format!("[{text}]")).expect("vector text"))
}

fn channel_mixer_operation(
    id: u128,
    enabled: bool,
    opacity: f64,
    parameters: ChannelMixerParametersV2,
) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).expect("operation ID"),
        OperationKey::new("rusttable.channelmixer").expect("operation key"),
        enabled,
        OperationOpacity::new(opacity).expect("opacity"),
        [
            (
                ParameterName::new("red").expect("parameter name"),
                array(parameters.red),
            ),
            (
                ParameterName::new("green").expect("parameter name"),
                array(parameters.green),
            ),
            (
                ParameterName::new("blue").expect("parameter name"),
                array(parameters.blue),
            ),
            (
                ParameterName::new("algorithm_version").expect("parameter name"),
                ParameterValue::Integer(i64::from(parameters.algorithm_version as i32)),
            ),
        ],
    )
    .expect("Channel Mixer operation")
}

fn identity_parameters() -> ChannelMixerParametersV2 {
    ChannelMixerParametersV2::defaults()
}

fn pixel(red: f32, green: f32, blue: f32) -> LinearRgb {
    LinearRgb::new(
        rusttable_processing::FiniteF32::new(red).expect("finite red"),
        rusttable_processing::FiniteF32::new(green).expect("finite green"),
        rusttable_processing::FiniteF32::new(blue).expect("finite blue"),
    )
}

#[test]
fn descriptor_registry_and_backend_capability_are_truthful() {
    let descriptor = channelmixer_descriptor();
    descriptor.validate().expect("Channel Mixer descriptor");
    assert!(descriptor.flags.contains(OperationFlags::DEPRECATED));
    assert!(descriptor.flags.contains(OperationFlags::MASKS));
    assert!(descriptor.flags.contains(OperationFlags::BLENDING));
    assert_eq!(descriptor.roi, RoiKind::Identity);
    assert_eq!(descriptor.io.input.alpha, AlphaPolicy::Preserve);
    assert_eq!(descriptor.io.output.alpha, AlphaPolicy::Preserve);
    assert_eq!(descriptor.migration.source_versions, [1, 2]);
    assert_eq!(descriptor.capability.gpu_tier, None);

    let registry = builtin_registry();
    let definition = registry
        .definition("rusttable.channelmixer")
        .expect("registered Channel Mixer");
    assert!(definition.cpu().is_some());
    assert!(definition.gpu().is_none());
    assert_eq!(definition.migrations()[0].from_version(), 1);
    assert_eq!(definition.migrations()[0].to_version(), 2);
    assert!(!definition.ui_availability().is_usable());
    let capability = registry
        .capability(
            "rusttable.channelmixer",
            &DeviceCapabilitySnapshot::gpu(
                9,
                ["f32-storage".to_owned()],
                ["rgba32float".to_owned()],
            ),
            ColorEncoding::LinearSrgbD65,
            Some("preview"),
        )
        .expect("capability");
    assert_eq!(
        capability.backend,
        rusttable_processing::ExecutionBackend::Cpu
    );
    assert!(capability.available);

    let materialized = registry
        .materialize_operation(
            "rusttable.channelmixer",
            OperationId::new(100).expect("operation ID"),
        )
        .expect("default operation");
    let compiled = ProcessingOperation::compile(&materialized).expect("default compiles");
    assert_eq!(
        compiled.kind(),
        &ProcessingOperationKind::ChannelMixer {
            config: rusttable_processing::ChannelMixerConfig::defaults()
        }
    );
}

#[test]
fn compiler_accepts_native_rows_and_rejects_wrong_shapes_or_types() {
    let defaults = identity_parameters();
    let operation = channel_mixer_operation(101, true, 1.0, defaults);
    let compiled = ProcessingOperation::compile(&operation).expect("native rows compile");
    assert!(matches!(
        compiled.kind(),
        ProcessingOperationKind::ChannelMixer { .. }
    ));

    let malformed = Operation::new(
        OperationId::new(102).expect("operation ID"),
        OperationKey::new("rusttable.channelmixer").expect("operation key"),
        true,
        [(
            ParameterName::new("red").expect("parameter name"),
            ParameterValue::Scalar(FiniteF64::new(1.0).expect("finite scalar")),
        )],
    )
    .expect("operation");
    assert!(matches!(
        ProcessingOperation::compile(&malformed),
        Err(rusttable_processing::OperationCompileError::WrongParameterType { .. })
    ));
}

#[test]
fn native_modes_execute_with_exact_mode_selection_and_alpha_preservation() {
    let mut gray = identity_parameters();
    gray.red[6] = 0.5;
    gray.green[6] = 0.25;
    gray.blue[6] = 0.25;
    let mut hsl_v2 = identity_parameters();
    hsl_v2.red[0] = 1.0;
    let mut hsl_v1 = hsl_v2;
    hsl_v1.algorithm_version = ChannelMixerAlgorithm::V1;

    let plans = [
        (identity_parameters(), ChannelMixerOperationMode::Rgb),
        (gray, ChannelMixerOperationMode::Gray),
        (hsl_v2, ChannelMixerOperationMode::HslV2),
        (hsl_v1, ChannelMixerOperationMode::HslV1),
    ];
    let source_alpha = f32::from_bits(0x7fc0_1234);
    for (parameters, mode) in plans {
        let plan = ChannelMixerPlan::commit_params(parameters).expect("finite parameters");
        assert_eq!(plan.operation_mode(), mode);
        let output = plan.execute(&[ChannelMixerPixel::new(0.2, 0.4, 0.6, source_alpha)]);
        assert!(
            output[0].channels()[..3]
                .iter()
                .all(|value| value.is_finite())
        );
        assert_eq!(output[0].alpha().to_bits(), source_alpha.to_bits());
    }
}

#[test]
fn production_normal2_routes_opacity_and_mask_as_direct_weighted_products() {
    let mut parameters = identity_parameters();
    parameters.red[3] = 0.0;
    parameters.green[3] = 1.0;
    let operation = channel_mixer_operation(106, true, 0.5, parameters);
    let edit = Edit::from_parts(
        EditId::new(106).expect("edit ID"),
        PhotoId::new(107).expect("photo ID"),
        Revision::ZERO,
        Revision::from_u64(1),
        [operation],
    )
    .expect("edit");
    let graph = CompiledOperationGraph::compile(&edit).expect("graph");
    let dimensions = RasterDimensions::new(1, 1).expect("dimensions");
    let input = WorkingRgbImage::new(dimensions, vec![pixel(0.1, 0.9, 0.4)]).expect("input");
    let masks = OperationMaskSet::from_entries([(
        OperationId::new(106).expect("mask operation ID"),
        MaskRaster::new(1, 1, vec![0.2]).expect("mask"),
    )])
    .expect("mask set");
    let output = evaluate_graph_at_frame_boundaries_with_masks(
        &graph,
        &input,
        &[1.0],
        FrameBoundaryOptions::new(FrameBoundaryMode::Preview),
        Some(&masks),
        || false,
    )
    .expect("masked Channel Mixer graph");
    let actual = output.image().pixel_slice()[0];
    let expected = 0.1_f32 * (1.0_f32 - 0.1_f32) + 0.9_f32 * 0.1_f32;
    let delta_first = 0.1_f32 + (0.9_f32 - 0.1_f32) * 0.1_f32;
    assert_ne!(expected.to_bits(), delta_first.to_bits());
    assert_eq!(actual.red().get().to_bits(), expected.to_bits());
    assert_eq!(
        actual.green().get().to_bits(),
        (0.9_f32 * (1.0_f32 - 0.1_f32) + 0.9_f32 * 0.1_f32).to_bits()
    );
    assert_eq!(
        actual.blue().get().to_bits(),
        (0.4_f32 * (1.0_f32 - 0.1_f32) + 0.4_f32 * 0.1_f32).to_bits()
    );
}

#[test]
fn mixed_graph_routes_opacity_mask_and_disabled_nodes_through_cpu() {
    let mut swap = identity_parameters();
    swap.red[3] = 0.0;
    swap.red[5] = 1.0;
    swap.green[3] = 1.0;
    swap.green[4] = 0.0;
    swap.blue[4] = 1.0;
    swap.blue[5] = 0.0;
    let channel = channel_mixer_operation(103, true, 0.5, swap);
    let disabled = channel_mixer_operation(105, false, 1.0, swap);
    let exposure = Operation::new(
        OperationId::new(104).expect("operation ID"),
        OperationKey::new("rusttable.exposure").expect("operation key"),
        true,
        [(
            ParameterName::new("stops").expect("parameter name"),
            ParameterValue::Scalar(FiniteF64::new(0.0).expect("finite scalar")),
        )],
    )
    .expect("exposure operation");
    let edit = Edit::from_parts(
        EditId::new(103).expect("edit ID"),
        PhotoId::new(104).expect("photo ID"),
        Revision::ZERO,
        Revision::from_u64(1),
        [channel, exposure, disabled],
    )
    .expect("edit");
    let graph = CompiledOperationGraph::compile(&edit).expect("mixed graph");
    let dimensions = RasterDimensions::new(1, 1).expect("dimensions");
    let input = WorkingRgbImage::new(dimensions, vec![pixel(0.2, 0.4, 0.6)]).expect("input");
    let masks = OperationMaskSet::from_entries([(
        OperationId::new(103).expect("mask operation ID"),
        MaskRaster::new(1, 1, vec![0.5]).expect("mask"),
    )])
    .expect("mask set");
    let alpha = [f32::from_bits(0x3eab_cdef)];
    let output = evaluate_graph_at_frame_boundaries_with_masks(
        &graph,
        &input,
        &alpha,
        FrameBoundaryOptions::new(FrameBoundaryMode::Preview),
        Some(&masks),
        || false,
    )
    .expect("masked mixed graph");
    let actual = output.image().pixel_slice()[0];
    assert!((actual.red().get() - 0.25).abs() < 1.0e-6);
    assert!((actual.green().get() - 0.45).abs() < 1.0e-6);
    assert!((actual.blue().get() - 0.50).abs() < 1.0e-6);
    assert_eq!(output.alpha()[0].to_bits(), alpha[0].to_bits());
}
