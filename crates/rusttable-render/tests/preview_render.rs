use rusttable_core::{Edit, EditId, PhotoId, Revision};
use rusttable_image::{ColorEncoding, DecodedImage, ImageDimensions};
use rusttable_render::{
    PreviewBounds, RenderPlan, RenderSampling, RenderTarget, SourceColorDecision,
    SourceColorPolicy, render_edit, render_edit_with_plan,
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
