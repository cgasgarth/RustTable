#![allow(
    clippy::float_cmp,
    reason = "source-derived compatibility vectors intentionally assert exact f32 results"
)]
#![expect(
    clippy::suboptimal_flops,
    clippy::too_many_lines,
    reason = "Native Color Correction test vectors and descriptor coverage preserve source order and one auditable contract."
)]

use rusttable_color::ColorEncoding;
use rusttable_core::{
    FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue,
};
use rusttable_processing::descriptor::{
    AlphaPolicy, OperationFlags, ParameterDefault, ParameterKind, RoiKind,
    colorcorrection_descriptor,
};
use rusttable_processing::operations::colorcorrection::{
    COLORCORRECTION_GPU_TIER, COLORCORRECTION_SCHEMA_VERSION, COLORCORRECTION_V1_PARAMETER_BYTES,
    COLORCORRECTION_WGPU_PASS_ID, ColorCorrectionConfig, ColorCorrectionHistory,
    ColorCorrectionParameterError, ColorCorrectionParametersV1, ColorCorrectionPixel,
    ColorCorrectionPlan, ColorCorrectionPresetBlendColorSpace, presets, wgpu_passes,
};
use rusttable_processing::{
    OperationCompileError, ProcessingOperation, ProcessingOperationKind, builtin_registry,
};

fn scalar(value: f64) -> ParameterValue {
    ParameterValue::Scalar(FiniteF64::new(value).expect("finite scalar"))
}

fn operation(
    id: u128,
    parameters: impl IntoIterator<Item = (&'static str, ParameterValue)>,
) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).expect("operation ID"),
        OperationKey::new("rusttable.colorcorrection").expect("operation key"),
        true,
        OperationOpacity::new(1.0).expect("opacity"),
        parameters
            .into_iter()
            .map(|(name, value)| (ParameterName::new(name).expect("parameter name"), value)),
    )
    .expect("operation")
}

fn decode_hex(payload: &str) -> Vec<u8> {
    let (pairs, remainder) = payload.as_bytes().as_chunks::<2>();
    assert!(remainder.is_empty(), "checked hex has complete bytes");
    pairs
        .iter()
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).expect("ASCII hex"), 16)
                .expect("valid checked hex")
        })
        .collect()
}

#[test]
fn native_v1_codec_is_five_little_endian_floats_and_unknown_versions_remain_opaque() {
    let parameters = ColorCorrectionParametersV1::defaults();
    assert_eq!(COLORCORRECTION_SCHEMA_VERSION, 1);
    assert_eq!(COLORCORRECTION_V1_PARAMETER_BYTES, 20);
    assert_eq!(
        parameters.to_bytes(),
        [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x80, 0x3f,
        ]
    );
    assert_eq!(
        ColorCorrectionParametersV1::from_bytes(&parameters.to_bytes()),
        Ok(parameters)
    );
    assert!(ColorCorrectionParametersV1::from_bytes(&[0; 19]).is_err());

    let opaque_bytes = vec![0xde, 0xad, 0xbe, 0xef, 0x80];
    let opaque = ColorCorrectionHistory::decode(9, &opaque_bytes).expect("unknown history");
    assert_eq!(opaque.version(), 9);
    assert_eq!(opaque.payload(), opaque_bytes);
    assert!(opaque.current().is_err());
}

#[test]
fn checked_benchmark_xmp_decodes_native_v1_bytes_and_order_neighbors() {
    let xmp = include_str!("../../../src/tests/benchmark/darktable-bench-3.4.xmp");
    let history = xmp
        .split_once("darktable:operation=\"colorcorrection\"")
        .expect("checked benchmark contains Color Correction")
        .1;
    let payload = history
        .split_once("darktable:params=\"")
        .expect("Color Correction history contains parameters")
        .1
        .split_once('"')
        .expect("parameter quote")
        .0;
    assert_eq!(payload, "663b9a4031534c407f0495c00a727ec00000803f");
    let bytes = decode_hex(payload);
    assert_eq!(
        ColorCorrectionHistory::decode(1, &bytes).expect("checked history"),
        ColorCorrectionHistory::V1(ColorCorrectionParametersV1::new(
            f32::from_bits(0x409a_3b66),
            f32::from_bits(0x404c_5331),
            f32::from_bits(0xc095_047f),
            f32::from_bits(0xc07e_720a),
            1.0,
        ))
    );
    assert!(xmp.contains("bilat,1,colorcorrection,0,colorcontrast,0,velvia,0,vibrance,0"));
}

#[test]
fn runtime_config_accepts_all_finite_persisted_values_without_ui_clamping() {
    let parameters = ColorCorrectionParametersV1::new(400.0, -500.0, 600.0, -700.0, 8.0);
    let config = ColorCorrectionConfig::try_from(parameters).expect("finite native parameters");
    assert_eq!(config.parameters(), parameters);
    assert_eq!(config.hia(), 400.0);
    assert_eq!(config.hib(), -500.0);
    assert_eq!(config.loa(), 600.0);
    assert_eq!(config.lob(), -700.0);
    assert_eq!(config.saturation(), 8.0);
    assert_eq!(
        ColorCorrectionConfig::new(f32::NAN, 0.0, 0.0, 0.0, 1.0),
        Err(ColorCorrectionParameterError::NonFinite("hia"))
    );
    assert_eq!(
        ColorCorrectionConfig::new(0.0, 0.0, 0.0, f32::INFINITY, 1.0),
        Err(ColorCorrectionParameterError::NonFinite("lob"))
    );
}

#[test]
fn commit_and_native_lab_equation_match_source_order_without_clamping() {
    let config =
        ColorCorrectionConfig::new(10.0, -20.0, -10.0, 30.0, 1.5).expect("native parameters");
    let plan = ColorCorrectionPlan::new(config);
    let coefficients = plan.coefficients();
    assert_eq!(
        coefficients.as_array(),
        [
            1.5,
            (10.0_f32 - -10.0_f32) / 100.0,
            -10.0,
            (-20.0_f32 - 30.0_f32) / 100.0,
            30.0
        ]
    );
    let source = ColorCorrectionPixel::new(50.0, 20.0, -10.0, f32::from_bits(0x3f41_2345));
    let output = plan.execute_lab(&[source])[0];
    assert_eq!(
        output,
        ColorCorrectionPixel::new(
            50.0,
            1.5_f32 * (20.0_f32 + 50.0_f32 * 0.2_f32 - 10.0_f32),
            1.5_f32 * (-10.0_f32 + 50.0_f32 * -0.5_f32 + 30.0_f32),
            source.alpha(),
        )
    );

    let unbounded = ColorCorrectionPlan::new(
        ColorCorrectionConfig::new(400.0, -400.0, -400.0, 400.0, 8.0)
            .expect("finite outlier parameters"),
    )
    .execute_lab(&[ColorCorrectionPixel::new(100.0, 128.0, -128.0, 0.25)])[0];
    assert!(unbounded.a() > 128.0);
    assert!(unbounded.b() < -128.0);
    assert_eq!(unbounded.lightness(), 100.0);
    assert_eq!(unbounded.alpha(), 0.25);
}

#[test]
fn native_normal_blend_combines_mask_with_opacity_in_lab() {
    let source = ColorCorrectionPixel::new(50.0, 20.0, -10.0, 0.8);
    let plan = ColorCorrectionPlan::new(
        ColorCorrectionConfig::new(10.0, -20.0, -10.0, 30.0, 1.5).expect("config"),
    );
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
        ColorCorrectionPixel::new(
            blend(source.lightness(), candidate.lightness(), 100.0),
            blend(source.a(), candidate.a(), 128.0),
            blend(source.b(), candidate.b(), 128.0),
            coverage,
        )
    );
}

#[test]
fn native_presets_keep_struct_order_and_cooling_filter_negative_zero() {
    let presets = presets();
    assert_eq!(
        presets.iter().map(|preset| preset.name).collect::<Vec<_>>(),
        ["warm tone", "warming filter", "cooling filter"]
    );
    assert!(presets.iter().all(|preset| preset.enabled));
    assert!(
        presets
            .iter()
            .all(|preset| preset.blend_color_space
                == ColorCorrectionPresetBlendColorSpace::RgbDisplay)
    );
    assert_eq!(
        presets[0].parameters,
        ColorCorrectionParametersV1::new(0.0, 3.0, 0.0, 0.0, 1.0)
    );
    assert_eq!(
        presets[1].parameters,
        ColorCorrectionParametersV1::new(-0.95, 4.5, 3.55, 0.0, 1.0)
    );
    assert_eq!(
        presets[2].parameters,
        ColorCorrectionParametersV1::new(0.95, -4.5, -3.55, -0.0, 1.0)
    );
    assert_eq!(presets[2].parameters.lob.to_bits(), (-0.0_f32).to_bits());
}

#[test]
fn descriptor_registry_and_compilation_expose_the_native_v1_lab_contract() {
    let descriptor = colorcorrection_descriptor();
    descriptor.validate().expect("descriptor");
    for flag in [
        OperationFlags::MULTI_INSTANCE,
        OperationFlags::STYLE_ELIGIBLE,
        OperationFlags::HISTORY_VISIBLE,
        OperationFlags::TILEABLE,
        OperationFlags::DETERMINISTIC_CPU,
        OperationFlags::DETERMINISTIC_GPU,
        OperationFlags::COLOR,
        OperationFlags::MASKS,
        OperationFlags::BLENDING,
    ] {
        assert!(descriptor.flags.contains(flag), "missing flag {flag:?}");
    }
    assert!(!descriptor.flags.contains(OperationFlags::DEPRECATED));
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
    assert_eq!(descriptor.migration.source_versions, [1]);
    assert_eq!(descriptor.migration.target_version, 1);
    assert_eq!(
        descriptor
            .parameters
            .iter()
            .map(|parameter| parameter.id.as_str())
            .collect::<Vec<_>>(),
        ["hia", "hib", "loa", "lob", "saturation"]
    );
    assert!(matches!(
        &descriptor.parameters[0].kind,
        ParameterKind::Scalar {
            minimum: -40.0,
            maximum: 40.0
        }
    ));
    assert!(matches!(
        &descriptor.parameters[4].kind,
        ParameterKind::Scalar {
            minimum: -3.0,
            maximum: 3.0
        }
    ));
    assert_eq!(
        &descriptor.parameters[4].default,
        &ParameterDefault::Scalar(1.0)
    );

    let registry = builtin_registry();
    let definition = registry
        .definition("rusttable.colorcorrection")
        .expect("registry definition");
    assert!(definition.cpu().is_some());
    let gpu = definition
        .gpu()
        .expect("qualified Color Correction GPU binding");
    assert_eq!(gpu.binding_id(), COLORCORRECTION_WGPU_PASS_ID);
    assert_eq!(gpu.tier(), COLORCORRECTION_GPU_TIER);
    assert_eq!(
        gpu.required_features(),
        ["f32-storage", "deterministic-row-major"]
    );
    assert_eq!(gpu.required_formats(), ["rgba32float"]);
    assert_eq!(wgpu_passes(), [COLORCORRECTION_WGPU_PASS_ID]);
    assert!(definition.migrations().is_empty());
    let order = registry
        .definitions_in_declaration_order()
        .into_iter()
        .map(|definition| definition.descriptor().id.compatibility_name.as_str())
        .collect::<Vec<_>>();
    let relight = order
        .iter()
        .position(|id| *id == "relight")
        .expect("relight");
    let colorcorrection = order
        .iter()
        .position(|id| *id == "colorcorrection")
        .expect("colorcorrection");
    let colorcontrast = order
        .iter()
        .position(|id| *id == "colorcontrast")
        .expect("colorcontrast");
    assert_eq!(colorcorrection, relight + 1);
    assert_eq!(colorcontrast, colorcorrection + 1);

    let compiled = ProcessingOperation::compile(&operation(
        11,
        [
            ("hia", scalar(400.0)),
            ("hib", scalar(-500.0)),
            ("loa", scalar(600.0)),
            ("lob", scalar(-700.0)),
            ("saturation", scalar(8.0)),
        ],
    ))
    .expect("compiled operation");
    assert_eq!(
        compiled.kind(),
        &ProcessingOperationKind::ColorCorrection {
            config: ColorCorrectionConfig::new(400.0, -500.0, 600.0, -700.0, 8.0).expect("config")
        }
    );

    let default = registry
        .materialize_operation(
            "rusttable.colorcorrection",
            OperationId::new(12).expect("operation ID"),
        )
        .expect("default operation");
    assert_eq!(
        ProcessingOperation::compile(&default)
            .expect("compiled defaults")
            .kind(),
        &ProcessingOperationKind::ColorCorrection {
            config: ColorCorrectionConfig::defaults()
        }
    );
    assert!(matches!(
        ProcessingOperation::compile(&operation(13, [("unexpected", scalar(1.0))])),
        Err(OperationCompileError::UnexpectedParameter { .. })
    ));
}
