use rusttable_core::{Edit, EditId, PhotoId, Revision};
use rusttable_image::{ColorEncoding, DecodedImage, ImageDimensions};
use rusttable_render::{SourceColorDecision, SourceColorPolicy, render_edit};

fn input() -> DecodedImage {
    DecodedImage::new_with_color_encoding(
        ImageDimensions::new(1, 1).unwrap(),
        vec![255, 0, 0, 42],
        ColorEncoding::DisplayP3,
    )
    .unwrap()
}

fn edit() -> Edit {
    Edit::new(
        EditId::new(1).unwrap(),
        PhotoId::new(2).unwrap(),
        Revision::ZERO,
        [],
    )
    .unwrap()
}

#[test]
fn supported_policy_renders_display_p3_to_srgb_with_alpha_and_clipping() {
    let output = render_edit(
        &edit(),
        &input(),
        SourceColorPolicy::RequireDeclaredSupported,
    )
    .unwrap();

    assert_eq!(
        output.source_color_decision(),
        SourceColorDecision::DeclaredDisplayP3
    );
    assert_eq!(output.image().pixels(), &[255, 0, 0, 42]);
    assert!(output.clipping().above_one().red() > 0);
    assert!(output.clipping().below_zero().green() > 0);
    assert!(output.clipping().below_zero().blue() > 0);
}

#[test]
fn assume_srgb_policy_accepts_declared_display_p3_without_relabeling() {
    let output = render_edit(
        &edit(),
        &input(),
        SourceColorPolicy::AssumeSrgbWhenUnspecified,
    )
    .unwrap();

    assert_eq!(
        output.source_color_decision(),
        SourceColorDecision::DeclaredDisplayP3
    );
}
