use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_image::{ColorEncoding, DecodedImage, ImageDimensions};
use rusttable_render::{RenderProvenance, SourceColorDecision, SourceColorPolicy, render_edit};

fn scalar(value: f64) -> ParameterValue {
    ParameterValue::Scalar(FiniteF64::new(value).expect("finite scalar"))
}

fn operation(id: u128, opacity: f64, key: &str, parameters: Vec<(&str, f64)>) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).expect("nonzero operation ID"),
        OperationKey::new(key).expect("valid operation key"),
        true,
        OperationOpacity::new(opacity).expect("valid opacity"),
        parameters.into_iter().map(|(name, value)| {
            (
                ParameterName::new(name).expect("valid parameter name"),
                scalar(value),
            )
        }),
    )
    .expect("unique parameters")
}

fn edit(operations: Vec<Operation>) -> Edit {
    Edit::from_parts(
        EditId::new(7).expect("nonzero edit ID"),
        PhotoId::new(8).expect("nonzero photo ID"),
        Revision::from_u64(9),
        Revision::from_u64(10),
        operations,
    )
    .expect("valid edit")
}

fn image(width: u32, pixels: Vec<u8>, encoding: ColorEncoding) -> DecodedImage {
    DecodedImage::new_with_color_encoding(
        ImageDimensions::new(width, 1).expect("valid dimensions"),
        pixels,
        encoding,
    )
    .expect("matching pixels")
}

#[test]
fn declared_and_assumed_srgb_are_explicit_decisions() {
    let input = image(1, vec![128, 64, 32, 17], ColorEncoding::Unspecified);
    let source = edit(vec![]);

    let assumed = render_edit(
        &source,
        &input,
        SourceColorPolicy::AssumeSrgbWhenUnspecified,
    )
    .expect("assumption policy accepts unspecified input");
    assert_eq!(
        assumed.source_color_decision(),
        SourceColorDecision::AssumedSrgb
    );
    assert_eq!(assumed.image().color_encoding(), ColorEncoding::Srgb);

    let declared_input = image(1, input.pixels().to_vec(), ColorEncoding::Srgb);
    let declared = render_edit(
        &source,
        &declared_input,
        SourceColorPolicy::RequireDeclaredSrgb,
    )
    .expect("declared sRGB is accepted");
    assert_eq!(
        declared.source_color_decision(),
        SourceColorDecision::DeclaredSrgb
    );
}

#[test]
fn empty_pipeline_round_trips_all_u8_codes_and_alpha() {
    let pixels = (0..=255)
        .flat_map(|value| [value, value, value, 255 - value])
        .collect::<Vec<_>>();
    let input = image(256, pixels.clone(), ColorEncoding::Srgb);
    let output = render_edit(
        &edit(vec![]),
        &input,
        SourceColorPolicy::RequireDeclaredSrgb,
    )
    .expect("empty pipeline succeeds");

    assert_eq!(output.image().pixels(), pixels.as_slice());
    assert_eq!(output.clipping().below_zero().red(), 0);
    assert_eq!(output.clipping().above_one().blue(), 0);
}

#[test]
fn renders_ordered_opacity_and_gain_with_provenance() {
    let input = image(1, vec![64, 128, 192, 77], ColorEncoding::Srgb);
    let source = edit(vec![
        operation(1, 1.0, "rusttable.linear_offset", vec![("value", 0.1)]),
        operation(2, 0.5, "rusttable.exposure", vec![("stops", 1.0)]),
        Operation::new_with_opacity(
            OperationId::new(3).unwrap(),
            OperationKey::new("rusttable.rgb_gain").unwrap(),
            true,
            OperationOpacity::new(0.5).unwrap(),
            [
                (ParameterName::new("red").unwrap(), scalar(2.0)),
                (ParameterName::new("green").unwrap(), scalar(1.0)),
                (ParameterName::new("blue").unwrap(), scalar(0.5)),
            ],
        )
        .unwrap(),
    ]);
    let output = render_edit(&source, &input, SourceColorPolicy::RequireDeclaredSrgb)
        .expect("ordered render succeeds");

    assert_eq!(
        output.provenance(),
        RenderProvenance::new(
            source.id(),
            source.photo_id(),
            source.base_photo_revision(),
            source.revision()
        )
    );
    assert_eq!(
        output.source_color_decision(),
        SourceColorDecision::DeclaredSrgb
    );
    assert_eq!(
        output.image().layout(),
        rusttable_image::PixelLayout::Rgba8StraightAlpha
    );
    assert_eq!(output.image().pixels()[3], 77);
}
