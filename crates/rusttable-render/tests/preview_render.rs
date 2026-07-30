use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
    PhotoId, Revision,
};
use rusttable_image::{ColorEncoding, DecodedImage, ImageDimensions};
use rusttable_processing::{
    CompiledPipeline, RasterDimensions, SourceRgb, SourceRgbImage, SrgbChannel, evaluate,
    to_linear_srgb,
};
use rusttable_render::{
    PreparedCpuPixelpipeResult, PreviewBounds, RenderPlan, RenderProvenance, RenderSampling,
    RenderTarget, SourceColorDecision, SourceColorPolicy, render_edit, render_edit_with_plan,
    render_prepared_cpu_pixelpipe,
};

fn edit() -> Edit {
    Edit::new(
        EditId::new(1).unwrap(),
        PhotoId::new(2).unwrap(),
        Revision::ZERO,
        [],
    )
    .unwrap()
}

fn image(width: u32, height: u32, pixels: Vec<u8>, encoding: ColorEncoding) -> DecodedImage {
    DecodedImage::new_with_color_encoding(
        ImageDimensions::new(width, height).unwrap(),
        pixels,
        encoding,
    )
    .unwrap()
}

fn vignette_edit() -> Edit {
    let scalar = |value: f64| ParameterValue::Scalar(FiniteF64::new(value).unwrap());
    Edit::new(
        EditId::new(3).unwrap(),
        PhotoId::new(2).unwrap(),
        Revision::ZERO,
        [Operation::new(
            OperationId::new(4).unwrap(),
            OperationKey::new("rusttable.vignette").unwrap(),
            true,
            [
                (ParameterName::new("scale").unwrap(), scalar(65.0)),
                (ParameterName::new("falloff_scale").unwrap(), scalar(35.0)),
                (ParameterName::new("brightness").unwrap(), scalar(-0.65)),
                (ParameterName::new("saturation").unwrap(), scalar(-0.2)),
                (ParameterName::new("shape").unwrap(), scalar(1.4)),
            ],
        )
        .unwrap()],
    )
    .unwrap()
}

fn full_resolution_reference(input: &DecodedImage, edit: &Edit) -> rusttable_render::RenderOutput {
    let dimensions =
        RasterDimensions::new(input.dimensions().width(), input.dimensions().height()).unwrap();
    let source = SourceRgbImage::new(
        dimensions,
        input
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| {
                SourceRgb::new(
                    SrgbChannel::new(f32::from(pixel[0]) / 255.0).unwrap(),
                    SrgbChannel::new(f32::from(pixel[1]) / 255.0).unwrap(),
                    SrgbChannel::new(f32::from(pixel[2]) / 255.0).unwrap(),
                )
            })
            .collect(),
    )
    .unwrap();
    let working = to_linear_srgb(&source);
    let pipeline = CompiledPipeline::compile(edit).unwrap();
    let evaluated = evaluate(&pipeline, &working).unwrap();
    let alpha = input
        .pixels()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| f32::from(pixel[3]) / 255.0)
        .collect();
    let prepared = PreparedCpuPixelpipeResult::new(
        evaluated,
        alpha,
        SourceColorDecision::DeclaredSrgb,
        RenderProvenance::new(
            edit.id(),
            edit.photo_id(),
            edit.base_photo_revision(),
            edit.revision(),
        ),
    )
    .unwrap();
    render_prepared_cpu_pixelpipe(
        &prepared,
        RenderTarget::PreviewFit(PreviewBounds::new(4, 4).unwrap()),
    )
    .unwrap()
}

#[test]
fn filtered_preview_reconstructs_source_pixels_in_linear_light() {
    let input = image(
        4,
        3,
        (0..12)
            .flat_map(|index| [index, index + 20, index + 40, 200 + index])
            .collect(),
        ColorEncoding::Srgb,
    );
    let plan = RenderPlan::for_source(
        input.dimensions(),
        RenderTarget::PreviewFit(PreviewBounds::new(2, 2).unwrap()),
    );
    let output = render_edit_with_plan(
        &edit(),
        &input,
        SourceColorPolicy::RequireDeclaredSrgb,
        plan,
    )
    .unwrap();

    assert_eq!(
        output.image().dimensions(),
        ImageDimensions::new(2, 1).unwrap()
    );
    assert_eq!(output.image().pixels(), &[5, 25, 45, 205, 7, 27, 47, 206]);
    assert_eq!(output.plan(), plan);
    assert_eq!(output.plan().sampling(), RenderSampling::Filtered);
    assert_eq!(
        output.source_color_decision(),
        SourceColorDecision::DeclaredSrgb
    );
}

#[test]
fn full_plan_delegation_matches_compatibility_render() {
    let input = image(1, 1, vec![128, 64, 32, 17], ColorEncoding::Srgb);
    let plan = RenderPlan::for_source(input.dimensions(), RenderTarget::FullResolution);
    let compatibility =
        render_edit(&edit(), &input, SourceColorPolicy::RequireDeclaredSrgb).unwrap();
    let planned = render_edit_with_plan(
        &edit(),
        &input,
        SourceColorPolicy::RequireDeclaredSrgb,
        plan,
    )
    .unwrap();

    assert_eq!(planned, compatibility);
    assert_eq!(planned.plan().sampling(), RenderSampling::Identity);
}

#[test]
fn display_p3_preview_keeps_declared_source_policy_and_alpha() {
    let input = image(
        2,
        1,
        vec![255, 0, 0, 7, 0, 255, 0, 9],
        ColorEncoding::DisplayP3,
    );
    let plan = RenderPlan::for_source(
        input.dimensions(),
        RenderTarget::PreviewFit(PreviewBounds::new(1, 1).unwrap()),
    );
    let output = render_edit_with_plan(
        &edit(),
        &input,
        SourceColorPolicy::RequireDeclaredSupported,
        plan,
    )
    .unwrap();

    assert_eq!(
        output.image().dimensions(),
        ImageDimensions::new(1, 1).unwrap()
    );
    assert_eq!(output.image().pixels()[3], 8);
    assert_eq!(
        output.source_color_decision(),
        SourceColorDecision::DeclaredDisplayP3
    );
}

#[test]
fn scale_sensitive_graph_evaluates_at_source_before_preview_downsample() {
    let input = image(
        12,
        8,
        (0..96)
            .flat_map(|index| {
                [
                    u8::try_from(index * 17 % 251).expect("bounded fixture channel"),
                    u8::try_from(index * 31 % 251).expect("bounded fixture channel"),
                    u8::try_from(index * 47 % 251).expect("bounded fixture channel"),
                    255,
                ]
            })
            .collect(),
        ColorEncoding::Srgb,
    );
    let edit = vignette_edit();
    let plan = RenderPlan::for_source(
        input.dimensions(),
        RenderTarget::PreviewFit(PreviewBounds::new(4, 4).unwrap()),
    );

    let actual =
        render_edit_with_plan(&edit, &input, SourceColorPolicy::RequireDeclaredSrgb, plan).unwrap();
    let expected = full_resolution_reference(&input, &edit);

    assert_eq!(actual.image(), expected.image());
    assert_eq!(actual.plan().evaluation_dimensions(), input.dimensions());
}
