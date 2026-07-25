#![allow(
    clippy::float_cmp,
    reason = "compatibility vectors intentionally assert exact f32 results"
)]

use rusttable_color::{
    Adaptation, AdaptationMethod, AlphaTransform, BlackPointCompensation, BuiltinSpace,
    ColorEncoding, ColorRole, ColorTransformRequest, ExtendedRange, Pcs, Precision, Primaries,
    ProfileClass, ProfileId, ProfileModel, ProfileParserVersion, RenderingIntent, TransferFunction,
    TransformPlan, TransformStep, WhitePoint, rgb_to_xyz_matrix,
};
use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_masks::MaskRaster;
use rusttable_processing::descriptor::{
    AlphaPolicy, OperationFlags, ParameterDefault, ParameterKind, RoiKind, colorcontrast_descriptor,
};
use rusttable_processing::operations::colorcontrast::{
    COLOR_CONTRAST_GPU_TIER, COLOR_CONTRAST_V1_PARAMETER_BYTES, COLOR_CONTRAST_V2_PARAMETER_BYTES,
    COLOR_CONTRAST_WGPU_PASS_ID, ColorContrastConfig, ColorContrastHistory,
    ColorContrastParameterError, ColorContrastParametersV1, ColorContrastParametersV2,
    ColorContrastPixel, ColorContrastPlan, migrate_v1_to_v2, wgpu_passes,
};
use rusttable_processing::operations::colorin::{
    ColorInConfig, ColorInNormalization, ColorInPlan, ColorInProfile,
};
use rusttable_processing::{
    CompiledOperationGraph, CompiledPipeline, FiniteF32, FrameBoundaryMode, FrameBoundaryOptions,
    LinearRgb, OperationCompileError, OperationMaskSet, ProcessingOperation,
    ProcessingOperationKind, RasterDimensions, WorkingFrameDescriptor, WorkingRgbImage,
    builtin_registry, evaluate, evaluate_graph_at_frame_boundaries,
    evaluate_graph_at_frame_boundaries_with_masks,
};

fn finite(value: f32) -> FiniteF32 {
    FiniteF32::new(value).expect("finite fixture")
}

fn pixel(red: f32, green: f32, blue: f32) -> LinearRgb {
    LinearRgb::new(finite(red), finite(green), finite(blue))
}

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
        OperationKey::new("rusttable.colorcontrast").expect("operation key"),
        true,
        OperationOpacity::new(opacity).expect("opacity"),
        parameters
            .into_iter()
            .map(|(name, value)| (ParameterName::new(name).expect("parameter name"), value)),
    )
    .expect("operation")
}

fn pipeline(opacity: f64) -> CompiledPipeline {
    let operation = operation(
        7,
        opacity,
        [
            ("a_steepness", scalar(2.0)),
            ("a_offset", scalar(4.0)),
            ("b_steepness", scalar(0.5)),
            ("b_offset", scalar(-2.0)),
            ("unbound", ParameterValue::Integer(1)),
        ],
    );
    CompiledPipeline::compile(
        &Edit::from_parts(
            EditId::new(1).expect("edit ID"),
            PhotoId::new(2).expect("photo ID"),
            Revision::ZERO,
            Revision::ZERO,
            [operation],
        )
        .expect("edit"),
    )
    .expect("pipeline")
}

#[test]
fn v1_and_v2_codecs_are_exact_little_endian_and_preserve_unknown_history() {
    let v1 = ColorContrastParametersV1::new(1.0, 0.0, 1.0, 0.0);
    assert_eq!(v1.to_bytes().len(), COLOR_CONTRAST_V1_PARAMETER_BYTES);
    assert_eq!(
        v1.to_bytes(),
        [
            0x00, 0x00, 0x80, 0x3f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x3f, 0x00, 0x00,
            0x00, 0x00,
        ]
    );
    assert_eq!(
        ColorContrastParametersV1::from_bytes(&v1.to_bytes()),
        Ok(v1)
    );

    let v2 = ColorContrastParametersV2::defaults();
    assert_eq!(v2.to_bytes().len(), COLOR_CONTRAST_V2_PARAMETER_BYTES);
    assert_eq!(
        v2.to_bytes(),
        [
            0x00, 0x00, 0x80, 0x3f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x3f, 0x00, 0x00,
            0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
        ]
    );
    assert_eq!(
        ColorContrastParametersV2::from_bytes(&v2.to_bytes()),
        Ok(v2)
    );

    let opaque_bytes = vec![0xde, 0xad, 0xbe, 0xef, 0x80];
    let opaque = ColorContrastHistory::decode(9, &opaque_bytes).expect("unknown history");
    assert_eq!(opaque.version(), 9);
    assert_eq!(opaque.payload(), opaque_bytes);
    assert!(opaque.current().is_err());
}

#[test]
fn v1_migration_preserves_all_floats_and_forces_legacy_bounded_mode() {
    let source = ColorContrastParametersV1::new(
        f32::from_bits(0x3fa1_2345),
        f32::from_bits(0xc020_0001),
        f32::from_bits(0x3f12_3456),
        f32::from_bits(0x40a0_0001),
    );
    let migrated = migrate_v1_to_v2(source);
    assert_eq!(migrated.a_steepness.to_bits(), source.a_steepness.to_bits());
    assert_eq!(migrated.a_offset.to_bits(), source.a_offset.to_bits());
    assert_eq!(migrated.b_steepness.to_bits(), source.b_steepness.to_bits());
    assert_eq!(migrated.b_offset.to_bits(), source.b_offset.to_bits());
    assert_eq!(migrated.unbound, 0);
    assert_eq!(
        ColorContrastHistory::decode(1, &source.to_bytes())
            .expect("v1")
            .current()
            .expect("migration"),
        migrated
    );
}

#[test]
fn checked_benchmark_xmp_decodes_native_v2_bytes_and_order_neighbors() {
    let xmp = include_str!("../../../src/tests/benchmark/darktable-bench-4.2.xmp");
    let history = xmp
        .split_once("darktable:operation=\"colorcontrast\"")
        .expect("checked benchmark contains Color Contrast")
        .1;
    let payload = history
        .split_once("darktable:params=\"")
        .expect("Color Contrast history contains parameters")
        .1
        .split_once('"')
        .expect("parameter quote")
        .0;
    assert_eq!(payload, "52b89e3f0000000014ae873f0000000001000000");
    let (pairs, remainder) = payload.as_bytes().as_chunks::<2>();
    assert!(remainder.is_empty(), "checked hex has complete bytes");
    let bytes = pairs
        .iter()
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).expect("ASCII hex"), 16)
                .expect("valid checked hex")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        ColorContrastHistory::decode(2, &bytes).expect("checked history"),
        ColorContrastHistory::V2(ColorContrastParametersV2::new(
            f32::from_bits(0x3f9e_b852),
            0.0,
            f32::from_bits(0x3f87_ae14),
            0.0,
            1,
        ))
    );
    assert!(xmp.contains("colorcorrection,0,colorcontrast,0,velvia,0,vibrance,0"));
}

#[test]
fn runtime_config_keeps_hidden_state_and_accepts_finite_values_outside_ui_bounds() {
    let parameters = ColorContrastParametersV2::new(8.0, -300.0, -2.0, 400.0, -7);
    let config = ColorContrastConfig::try_from(parameters).expect("finite native parameters");
    assert_eq!(config.parameters(), parameters);
    assert!(config.is_unbound());
    assert_eq!(config.unbound(), -7);
    assert_eq!(
        ColorContrastConfig::new(f32::NAN, 0.0, 1.0, 0.0, 1),
        Err(ColorContrastParameterError::NonFinite("a_steepness"))
    );
    assert_eq!(
        ColorContrastConfig::new(1.0, 0.0, 1.0, f32::INFINITY, 1),
        Err(ColorContrastParameterError::NonFinite("b_offset"))
    );
}

#[test]
fn native_lab_math_preserves_hidden_offsets_raw_unbound_and_clamps_bound_mode() {
    let input = [ColorContrastPixel::new(50.0, -20.0, 40.0, 0.75)];
    let unbound =
        ColorContrastPlan::new(ColorContrastConfig::new(2.0, 3.0, 0.5, -4.0, -9).expect("config"))
            .execute_lab(&input);
    assert_eq!(unbound, [ColorContrastPixel::new(50.0, -37.0, 16.0, 0.75)]);

    let bounded =
        ColorContrastPlan::new(ColorContrastConfig::new(2.0, 0.0, 2.0, 0.0, 0).expect("config"))
            .execute_lab(&[
                ColorContrastPixel::new(50.0, 100.0, -100.0, 0.75),
                ColorContrastPixel::new(f32::NAN, f32::NAN, f32::NAN, f32::NAN),
            ]);
    assert_eq!(
        bounded[0],
        ColorContrastPixel::new(50.0, 128.0, -128.0, 0.75)
    );
    assert_eq!(
        bounded[1],
        ColorContrastPixel::new(-f32::MAX, -128.0, -128.0, -f32::MAX)
    );

    let unbounded_nan = ColorContrastPlan::new(ColorContrastConfig::defaults()).execute_lab(&[
        ColorContrastPixel::new(f32::NAN, f32::NAN, f32::NAN, f32::NAN),
    ])[0];
    assert!(
        unbounded_nan
            .channels()
            .iter()
            .all(|channel| channel.is_nan())
    );
}

#[test]
fn native_default_parameters_are_an_exact_module_identity() {
    let input = [
        ColorContrastPixel::new(50.0, -20.0, 40.0, 0.75),
        ColorContrastPixel::new(0.0, -128.0, 128.0, 0.0),
        ColorContrastPixel::new(100.0, 0.0, 0.0, 1.0),
    ];

    assert_eq!(
        ColorContrastPlan::new(ColorContrastConfig::defaults()).execute_lab(&input),
        input
    );
}

#[test]
fn default_lab_blend_uses_native_scaling_and_combines_mask_with_opacity_before_rgb() {
    let source = ColorContrastPixel::new(50.0, 16.0, -32.0, 0.8);
    let blended =
        ColorContrastPlan::new(ColorContrastConfig::new(2.0, 4.0, 0.5, -2.0, 1).expect("config"))
            .execute_lab_normal_blend(&[source], Some(&[0.5]), 0.5);
    assert_eq!(blended, [ColorContrastPixel::new(50.0, 21.0, -28.5, 0.25)]);

    let dimensions = RasterDimensions::new(1, 1).expect("dimensions");
    let input = WorkingRgbImage::new(dimensions, vec![pixel(0.2, 0.4, 0.6)]).expect("input");
    let alpha = [f32::from_bits(0x3eab_cdef)];
    let operation_id = OperationId::new(7).expect("operation ID");
    let masks = OperationMaskSet::from_entries([(
        operation_id,
        MaskRaster::new(1, 1, vec![0.5]).expect("mask"),
    )])
    .expect("mask set");

    let half_graph = CompiledOperationGraph::from_pipeline(&pipeline(0.5));
    let masked_half = evaluate_graph_at_frame_boundaries_with_masks(
        &half_graph,
        &input,
        &alpha,
        FrameBoundaryOptions::new(FrameBoundaryMode::Preview),
        Some(&masks),
        || false,
    )
    .expect("masked half");
    let quarter_graph = CompiledOperationGraph::from_pipeline(&pipeline(0.25));
    let unmasked_quarter = evaluate_graph_at_frame_boundaries(
        &quarter_graph,
        &input,
        &alpha,
        FrameBoundaryOptions::new(FrameBoundaryMode::Preview),
        || false,
    )
    .expect("unmasked quarter");

    assert_eq!(
        masked_half.image().pixel_slice(),
        unmasked_quarter.image().pixel_slice(),
        "coverage must be composed and blended in Lab before converting back to RGB"
    );
    assert_ne!(
        masked_half.image().pixel_slice(),
        &[pixel(0.2, 0.5, 0.6)],
        "Lab a*/b* must never be applied directly to linear RGB channels"
    );
    assert_eq!(masked_half.alpha(), alpha.as_slice());
    assert_eq!(unmasked_quarter.alpha(), alpha.as_slice());
}

fn external_working_request(source: ColorEncoding, target: ColorEncoding) -> ColorTransformRequest {
    ColorTransformRequest::new(
        source,
        target,
        ColorRole::Working,
        RenderingIntent::Relative,
        BlackPointCompensation::Disabled,
        AdaptationMethod::Bradford,
        Precision::F32,
        AlphaTransform::Preserve,
        ExtendedRange::Extended,
        1,
    )
    .expect("external working request")
}

fn external_lab_plans(frame: WorkingFrameDescriptor) -> (TransformPlan, TransformPlan) {
    let primaries = frame.primaries();
    let rgb_to_xyz = rgb_to_xyz_matrix(
        [
            (primaries.red().0.get(), primaries.red().1.get()),
            (primaries.green().0.get(), primaries.green().1.get()),
            (primaries.blue().0.get(), primaries.blue().1.get()),
        ],
        frame.white_point(),
    )
    .expect("valid matrix working profile");
    let to_lab = TransformPlan::new(
        external_working_request(frame.encoding(), ColorEncoding::LabD50),
        vec![
            TransformStep::Matrix(rgb_to_xyz),
            TransformStep::Adaptation(
                Adaptation::between(
                    frame.white_point(),
                    WhitePoint::D50,
                    AdaptationMethod::Bradford,
                )
                .expect("D65-to-D50 adaptation"),
            ),
            TransformStep::XyzToLab {
                white_point: WhitePoint::D50,
            },
        ],
    )
    .expect("external RGB-to-Lab plan");
    let from_lab = TransformPlan::new(
        external_working_request(ColorEncoding::LabD50, frame.encoding()),
        vec![
            TransformStep::LabToXyz {
                white_point: WhitePoint::D50,
            },
            TransformStep::Adaptation(
                Adaptation::between(
                    WhitePoint::D50,
                    frame.white_point(),
                    AdaptationMethod::Bradford,
                )
                .expect("D50-to-D65 adaptation"),
            ),
            TransformStep::Matrix(rgb_to_xyz.inverse().expect("matrix inverse")),
        ],
    )
    .expect("Lab-to-external RGB plan");
    (to_lab, from_lab)
}

#[test]
fn matrix_colorin_frame_executes_colorcontrast_through_the_exact_lab_boundary() {
    let profile_id = ProfileId::from_content(
        b"Color Contrast matrix working profile",
        ProfileClass::Working,
        ProfileModel::Matrix,
        Pcs::XyzD50,
        ProfileParserVersion::new(1).expect("parser version"),
    )
    .expect("profile ID");
    let colorin = ColorInPlan::new(
        ColorInConfig::new(
            ColorInProfile::Builtin(BuiltinSpace::SrgbD65),
            ColorInProfile::Matrix {
                id: profile_id,
                primaries: Primaries::display_p3(),
                transfer: TransferFunction::Linear,
            },
            RenderingIntent::Relative,
            ColorInNormalization::Off,
            false,
        )
        .expect("matrix ColorIn config"),
    )
    .expect("matrix ColorIn plan");
    let dimensions = RasterDimensions::new(2, 1).expect("dimensions");
    let source = [pixel(0.2, 0.4, 0.6), pixel(0.85, 0.1, 0.35)];
    let converted = colorin.execute(&source).expect("ColorIn execution");
    let working = WorkingRgbImage::new_with_frame(
        dimensions,
        converted.pixels().to_vec(),
        colorin.output_frame(),
    )
    .expect("matrix working frame");

    let actual = evaluate(&pipeline(1.0), &working).expect("ColorIn to Color Contrast evaluation");
    let (to_lab, from_lab) = external_lab_plans(working.frame());
    let lab = working
        .pixels()
        .enumerate()
        .map(|(index, rgb)| {
            let lab = to_lab
                .apply_rgb(
                    [rgb.red().get(), rgb.green().get(), rgb.blue().get()],
                    || false,
                )
                .unwrap_or_else(|error| panic!("reference ingress pixel {index}: {error}"));
            ColorContrastPixel::new(lab[0], lab[1], lab[2], 1.0)
        })
        .collect::<Vec<_>>();
    let lab = ColorContrastPlan::new(
        ColorContrastConfig::new(2.0, 4.0, 0.5, -2.0, 1).expect("Color Contrast config"),
    )
    .execute_lab(&lab);
    let expected = lab
        .iter()
        .enumerate()
        .map(|(index, lab)| {
            let lab = lab.channels();
            let rgb = from_lab
                .apply_rgb([lab[0], lab[1], lab[2]], || false)
                .unwrap_or_else(|error| panic!("reference egress pixel {index}: {error}"));
            pixel(rgb[0], rgb[1], rgb[2])
        })
        .collect::<Vec<_>>();

    assert_eq!(actual.frame(), working.frame());
    assert_eq!(actual.pixel_slice(), expected.as_slice());
}

#[test]
fn descriptor_registry_and_compilation_expose_lab_d50_and_hidden_parameters() {
    let descriptor = colorcontrast_descriptor();
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
    assert_eq!(descriptor.migration.source_versions, [1, 2]);
    assert_eq!(descriptor.migration.target_version, 2);
    assert_eq!(
        descriptor
            .parameters
            .iter()
            .map(|parameter| parameter.id.as_str())
            .collect::<Vec<_>>(),
        [
            "a_steepness",
            "a_offset",
            "b_steepness",
            "b_offset",
            "unbound",
        ]
    );
    assert!(matches!(
        &descriptor.parameters[0].kind,
        ParameterKind::Scalar {
            minimum: 0.0,
            maximum: 5.0
        }
    ));
    assert!(matches!(
        &descriptor.parameters[4].kind,
        ParameterKind::Integer { .. }
    ));
    assert_eq!(
        &descriptor.parameters[4].default,
        &ParameterDefault::Integer(1)
    );
    assert!(descriptor.parameters[1].ui_hint.is_none());
    assert!(descriptor.parameters[3].ui_hint.is_none());
    assert!(descriptor.parameters[4].ui_hint.is_none());

    let registry = builtin_registry();
    let definition = registry
        .definition("rusttable.colorcontrast")
        .expect("registry definition");
    assert!(definition.cpu().is_some());
    let gpu = definition
        .gpu()
        .expect("qualified Color Contrast GPU binding");
    assert_eq!(gpu.binding_id(), COLOR_CONTRAST_WGPU_PASS_ID);
    assert_eq!(gpu.tier(), COLOR_CONTRAST_GPU_TIER);
    assert_eq!(
        gpu.required_features(),
        ["f32-storage", "deterministic-row-major"]
    );
    assert_eq!(gpu.required_formats(), ["rgba32float"]);
    assert_eq!(wgpu_passes(), [COLOR_CONTRAST_WGPU_PASS_ID]);
    assert_eq!(definition.migrations().len(), 1);
    assert_eq!(
        definition.migrations()[0].evidence_id(),
        "colorcontrast.migration.v1-v2"
    );
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
    let velvia = order.iter().position(|id| *id == "velvia").expect("velvia");
    assert_eq!(colorcorrection, relight + 1);
    assert_eq!(colorcontrast, colorcorrection + 1);
    assert_eq!(velvia, colorcontrast + 1);

    let default = registry
        .materialize_operation(
            "rusttable.colorcontrast",
            OperationId::new(12).expect("operation ID"),
        )
        .expect("default operation");
    assert_eq!(
        ProcessingOperation::compile(&default)
            .expect("compiled defaults")
            .kind(),
        &ProcessingOperationKind::ColorContrast {
            config: ColorContrastConfig::defaults()
        }
    );
}

#[test]
fn operation_compilation_keeps_exact_native_int_and_rejects_wrong_hidden_type() {
    let compiled = ProcessingOperation::compile(&operation(
        13,
        1.0,
        [
            ("a_steepness", scalar(8.0)),
            ("a_offset", scalar(-300.0)),
            ("b_steepness", scalar(-2.0)),
            ("b_offset", scalar(400.0)),
            ("unbound", ParameterValue::Integer(-7)),
        ],
    ))
    .expect("finite persisted state compiles");
    assert_eq!(
        compiled.kind(),
        &ProcessingOperationKind::ColorContrast {
            config: ColorContrastConfig::new(8.0, -300.0, -2.0, 400.0, -7).expect("history config")
        }
    );

    let wrong_type = operation(14, 1.0, [("unbound", scalar(1.0))]);
    assert!(matches!(
        ProcessingOperation::compile(&wrong_type),
        Err(OperationCompileError::WrongParameterType { .. })
    ));
}
