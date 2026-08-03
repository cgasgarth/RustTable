#![allow(
    clippy::float_cmp,
    reason = "source-derived compatibility vectors intentionally assert exact f32 results"
)]
#![expect(
    clippy::imprecise_flops,
    clippy::suboptimal_flops,
    reason = "Native Vibrance test vectors preserve source evaluation order and IEEE-754 parity."
)]

use rusttable_color::ColorEncoding;
use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_processing::descriptor::{
    AlphaPolicy, OperationFlags, ParameterDefault, ParameterKind, RoiKind, vibrance_descriptor,
};
use rusttable_processing::operations::vibrance::{
    VIBRANCE_DEFAULT_AMOUNT, VIBRANCE_GPU_TIER, VIBRANCE_SCHEMA_VERSION,
    VIBRANCE_V2_PARAMETER_BYTES, VIBRANCE_WGPU_PASS_ID, VibranceConfig, VibranceHistory,
    VibranceParameterError, VibranceParametersV2, VibrancePixel, VibrancePlan, wgpu_passes,
};
use rusttable_processing::{
    CompiledPipeline, OperationCompileError, ProcessingOperation, ProcessingOperationKind,
    builtin_registry,
};

fn scalar(value: f64) -> ParameterValue {
    ParameterValue::Scalar(FiniteF64::new(value).expect("finite scalar"))
}

fn operation(
    id: u128,
    opacity: f64,
    parameters: impl IntoIterator<Item = (&'static str, ParameterValue)>,
) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).expect("operation ID"),
        OperationKey::new("rusttable.vibrance").expect("operation key"),
        true,
        OperationOpacity::new(opacity).expect("opacity"),
        parameters
            .into_iter()
            .map(|(name, value)| (ParameterName::new(name).expect("parameter name"), value)),
    )
    .expect("operation")
}

#[test]
fn native_v2_codec_is_one_little_endian_float_and_unknown_versions_remain_opaque() {
    let parameters = VibranceParametersV2::defaults();
    assert_eq!(VIBRANCE_SCHEMA_VERSION, 2);
    assert_eq!(VIBRANCE_V2_PARAMETER_BYTES, 4);
    assert_eq!(parameters.amount, VIBRANCE_DEFAULT_AMOUNT);
    assert_eq!(parameters.to_bytes(), [0x00, 0x00, 0xc8, 0x41]);
    assert_eq!(
        VibranceParametersV2::from_bytes(&parameters.to_bytes()),
        Ok(parameters)
    );
    assert!(VibranceParametersV2::from_bytes(&[0, 0, 0]).is_err());

    let opaque_bytes = vec![0xde, 0xad, 0xbe, 0xef, 0x80];
    let opaque = VibranceHistory::decode(9, &opaque_bytes).expect("unknown history");
    assert_eq!(opaque.version(), 9);
    assert_eq!(opaque.payload(), opaque_bytes);
    assert!(opaque.current().is_err());
}

#[test]
fn checked_benchmark_xmp_decodes_native_default_and_order_neighbors() {
    let xmp = include_str!("../../../src/tests/benchmark/darktable-bench-3.4.xmp");
    let history = xmp
        .split_once("darktable:operation=\"vibrance\"")
        .expect("checked benchmark contains Vibrance")
        .1;
    let payload = history
        .split_once("darktable:params=\"")
        .expect("Vibrance history contains parameters")
        .1
        .split_once('"')
        .expect("parameter quote")
        .0;
    assert_eq!(payload, "0000c841");
    assert_eq!(
        VibranceHistory::decode(2, &[0x00, 0x00, 0xc8, 0x41]).expect("checked history"),
        VibranceHistory::V2(VibranceParametersV2::defaults())
    );
    assert!(xmp.contains("colorcontrast,0,velvia,0,vibrance,0,colorzones,0"));
}

#[test]
fn runtime_config_accepts_all_finite_persisted_values_without_ui_clamping() {
    let negative = VibranceConfig::new(-50.0).expect("finite negative source value");
    assert_eq!(negative.amount(), -50.0);
    assert_eq!(negative.normalized_amount(), -0.5);
    let above_ui_range = VibranceConfig::new(250.0).expect("finite source value above UI range");
    assert_eq!(above_ui_range.amount(), 250.0);
    assert_eq!(
        VibranceConfig::new(f32::NAN),
        Err(VibranceParameterError::NonFinite("amount"))
    );
    assert_eq!(
        VibranceConfig::new(f32::INFINITY),
        Err(VibranceParameterError::NonFinite("amount"))
    );
}

#[test]
fn native_lab_equation_uses_chroma_weight_without_clamping_and_preserves_alpha() {
    let source = VibrancePixel::new(50.0, 3.0, 4.0, f32::from_bits(0x3f41_2345));
    let output =
        VibrancePlan::new(VibranceConfig::new(100.0).expect("config")).execute_lab(&[source])[0];

    let amount = 100.0_f32 * 0.01_f32;
    let saturation_weight = (3.0_f32 * 3.0_f32 + 4.0_f32 * 4.0_f32).sqrt() / 256.0_f32;
    let lightness_scale = 1.0_f32 - amount * saturation_weight * 0.25_f32;
    let chroma_scale = 1.0_f32 + amount * saturation_weight;
    assert_eq!(
        output,
        VibrancePixel::new(
            50.0_f32 * lightness_scale,
            3.0_f32 * chroma_scale,
            4.0_f32 * chroma_scale,
            source.alpha(),
        )
    );

    let unbounded = VibrancePlan::new(VibranceConfig::new(250.0).expect("config"))
        .execute_lab(&[VibrancePixel::new(100.0, 512.0, -768.0, 0.5)])[0];
    assert!(unbounded.a() > 512.0);
    assert!(unbounded.b() < -768.0);
    assert!(unbounded.lightness() < 0.0);
    assert_eq!(unbounded.alpha(), 0.5);

    let overflow = VibrancePlan::new(VibranceConfig::new(25.0).expect("config")).execute_lab(&[
        VibrancePixel::new(1.0, f32::MAX, 1.0, f32::from_bits(0x3eab_cdef)),
    ])[0];
    assert!(
        overflow.lightness().is_infinite() && overflow.lightness().is_sign_negative(),
        "native sqrtf(a*a + b*b) overflows before scaling"
    );
    assert!(overflow.a().is_infinite() && overflow.a().is_sign_positive());
    assert!(overflow.b().is_infinite() && overflow.b().is_sign_positive());
    assert_eq!(overflow.alpha().to_bits(), 0x3eab_cdef);
}

#[test]
fn zero_amount_is_exact_for_finite_lab_channels() {
    let source = [
        VibrancePixel::new(50.0, -20.0, 40.0, 0.75),
        VibrancePixel::new(0.0, -128.0, 128.0, 0.0),
        VibrancePixel::new(100.0, 0.0, 0.0, 1.0),
    ];
    assert_eq!(
        VibrancePlan::new(VibranceConfig::new(0.0).expect("config")).execute_lab(&source),
        source
    );
}

#[test]
fn native_normal_blend_combines_mask_with_opacity_in_lab() {
    let source = VibrancePixel::new(50.0, 3.0, 4.0, 0.8);
    let plan = VibrancePlan::new(VibranceConfig::new(100.0).expect("config"));
    let candidate = plan.execute_lab(&[source])[0];
    let output = plan.execute_lab_normal_blend(&[source], Some(&[0.5]), 0.5)[0];
    let coverage = 0.25_f32;
    let blend = |source: f32, candidate: f32, scale: f32| {
        let inverse_scale = 1.0_f32 / scale;
        (source * inverse_scale * (1.0_f32 - coverage) + candidate * inverse_scale * coverage)
            * scale
    };
    assert_eq!(
        output,
        VibrancePixel::new(
            blend(source.lightness(), candidate.lightness(), 100.0),
            blend(source.a(), candidate.a(), 128.0),
            blend(source.b(), candidate.b(), 128.0),
            coverage,
        )
    );
}

#[test]
fn descriptor_and_registry_expose_the_complete_deprecated_lab_contract() {
    let descriptor = vibrance_descriptor();
    descriptor.validate().expect("descriptor");
    for flag in [
        OperationFlags::DEPRECATED,
        OperationFlags::STYLE_ELIGIBLE,
        OperationFlags::HISTORY_VISIBLE,
        OperationFlags::TILEABLE,
        OperationFlags::DETERMINISTIC_CPU,
        OperationFlags::DETERMINISTIC_GPU,
        OperationFlags::COLOR,
        OperationFlags::MASKS,
        OperationFlags::BLENDING,
        OperationFlags::MULTI_INSTANCE,
    ] {
        assert!(descriptor.flags.contains(flag), "missing flag {flag:?}");
    }
    assert_eq!(descriptor.stage, "display-referred-lab-d50");
    assert_eq!(descriptor.roi, RoiKind::Identity);
    assert_eq!(descriptor.io.input.channels, 4);
    assert_eq!(descriptor.io.input.alpha, AlphaPolicy::Preserve);
    assert_eq!(
        descriptor.io.input.encodings.as_slice(),
        &[ColorEncoding::LabD50]
    );
    assert_eq!(
        descriptor.io.output.encodings.as_slice(),
        &[ColorEncoding::LabD50]
    );
    assert_eq!(descriptor.migration.source_versions, [2]);
    assert_eq!(descriptor.migration.target_version, 2);
    assert_eq!(descriptor.parameters.len(), 1);
    let amount = &descriptor.parameters[0];
    assert_eq!(amount.id, "amount");
    assert!(matches!(
        &amount.kind,
        ParameterKind::Scalar {
            minimum: 0.0,
            maximum: 100.0
        }
    ));
    assert_eq!(&amount.default, &ParameterDefault::Scalar(25.0));

    let definition = builtin_registry()
        .definition("rusttable.vibrance")
        .expect("registry definition");
    assert!(definition.cpu().is_some());
    assert!(definition.migrations().is_empty());
    let gpu = definition.gpu().expect("qualified Vibrance GPU binding");
    assert_eq!(gpu.binding_id(), VIBRANCE_WGPU_PASS_ID);
    assert_eq!(gpu.tier(), VIBRANCE_GPU_TIER);
    assert_eq!(wgpu_passes(), [VIBRANCE_WGPU_PASS_ID]);
    assert!(
        definition
            .evidence_ids()
            .iter()
            .any(|evidence| evidence == "iop.vibrance.order.v30-58")
    );

    let order = builtin_registry()
        .definitions_in_declaration_order()
        .into_iter()
        .map(|definition| definition.descriptor().id.compatibility_name.as_str())
        .collect::<Vec<_>>();
    let velvia = order.iter().position(|id| *id == "velvia").expect("velvia");
    let vibrance = order
        .iter()
        .position(|id| *id == "vibrance")
        .expect("vibrance");
    assert_eq!(vibrance, velvia + 1);
}

#[test]
fn operation_compilation_and_default_materialization_are_checked() {
    let compiled = ProcessingOperation::compile(&operation(11, 1.0, [("amount", scalar(40.0))]))
        .expect("compiled operation");
    assert_eq!(
        compiled.kind(),
        &ProcessingOperationKind::Vibrance {
            config: VibranceConfig::new(40.0).expect("config")
        }
    );

    let default = builtin_registry()
        .materialize_operation(
            "rusttable.vibrance",
            OperationId::new(12).expect("operation ID"),
        )
        .expect("default operation");
    assert_eq!(
        ProcessingOperation::compile(&default)
            .expect("compiled defaults")
            .kind(),
        &ProcessingOperationKind::Vibrance {
            config: VibranceConfig::defaults()
        }
    );

    assert!(matches!(
        ProcessingOperation::compile(&operation(13, 1.0, [("unexpected", scalar(1.0))])),
        Err(OperationCompileError::UnexpectedParameter { .. })
    ));
}

#[test]
fn compiled_pipeline_keeps_native_declaration_position() {
    let edit = Edit::from_parts(
        EditId::new(1).expect("edit ID"),
        PhotoId::new(2).expect("photo ID"),
        Revision::ZERO,
        Revision::ZERO,
        [operation(14, 1.0, [("amount", scalar(25.0))])],
    )
    .expect("edit");
    let pipeline = CompiledPipeline::compile(&edit).expect("pipeline");
    let first = pipeline.steps().next().expect("Vibrance pipeline step");
    assert!(matches!(
        first.operation().kind(),
        ProcessingOperationKind::Vibrance { .. }
    ));
}
