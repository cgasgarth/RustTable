use rusttable_core::{Edit, EditId, PhotoId, Revision};
use rusttable_image::{ColorEncoding, DecodedImage, ImageDimensions};
use rusttable_render::{SourceColorPolicy, render_edit};

#[test]
fn preserves_all_straight_alpha_codes() {
    let alpha = (0..=255).collect::<Vec<_>>();
    let pixels = alpha
        .iter()
        .flat_map(|&value| [0, 128, 255, value])
        .collect::<Vec<_>>();
    let input = DecodedImage::new_with_color_encoding(
        ImageDimensions::new(256, 1).unwrap(),
        pixels,
        ColorEncoding::Srgb,
    )
    .unwrap();
    let edit = Edit::new(
        EditId::new(1).unwrap(),
        PhotoId::new(2).unwrap(),
        Revision::ZERO,
        [],
    )
    .unwrap();

    let output = render_edit(&edit, &input, SourceColorPolicy::RequireDeclaredSrgb)
        .expect("empty render succeeds");

    assert_eq!(
        output
            .image()
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| pixel[3])
            .collect::<Vec<_>>(),
        alpha
    );
}

#[test]
fn exact_endpoints_remain_exact_after_quantization() {
    let input = DecodedImage::new_with_color_encoding(
        ImageDimensions::new(2, 1).unwrap(),
        vec![0, 0, 0, 11, 255, 255, 255, 22],
        ColorEncoding::Srgb,
    )
    .unwrap();
    let edit = Edit::new(
        EditId::new(1).unwrap(),
        PhotoId::new(2).unwrap(),
        Revision::ZERO,
        [],
    )
    .unwrap();

    let output = render_edit(&edit, &input, SourceColorPolicy::RequireDeclaredSrgb)
        .expect("empty render succeeds");

    assert_eq!(output.image().pixels(), input.pixels());
}
