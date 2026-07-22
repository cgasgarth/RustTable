use rusttable_core::{AssetId, ByteLength, ContentHash, Edit, EditId, PhotoId, Revision};
use rusttable_image::{ColorEncoding, ImageDimensions, ImageProbe, InputFormat};
use rusttable_render::{
    PreviewBounds, RenderPlan, RenderSourceProvenance, RenderTarget, SourceColorDecision,
    SourceColorPolicy, render_edit_with_provenance,
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

fn source() -> RenderSourceProvenance {
    source_for(ImageDimensions::new(1, 1).unwrap())
}

fn source_for(dimensions: ImageDimensions) -> RenderSourceProvenance {
    RenderSourceProvenance::new(
        PhotoId::new(2).unwrap(),
        AssetId::new(3).unwrap(),
        ContentHash::Sha256([7; 32]),
        ByteLength::from_bytes(42),
        ImageProbe::new(InputFormat::Png, dimensions),
    )
}

#[test]
fn successful_render_owns_source_bound_receipt_and_plan() {
    let input = rusttable_image::DecodedImage::new(
        ImageDimensions::new(1, 1).unwrap(),
        vec![128, 64, 32, 17],
    )
    .unwrap();
    let plan = RenderPlan::for_source(
        input.dimensions(),
        RenderTarget::PreviewFit(PreviewBounds::new(1, 1).unwrap()),
    );
    let output = render_edit_with_provenance(
        &edit(),
        &input,
        SourceColorPolicy::AssumeSrgbWhenUnspecified,
        plan,
        source(),
    )
    .unwrap();

    assert_eq!(output.output().plan(), plan);
    assert_eq!(output.receipt().context().plan(), plan);
    assert_eq!(
        output.receipt().context().policy(),
        SourceColorPolicy::AssumeSrgbWhenUnspecified
    );
    assert_eq!(output.receipt().source(), source());
    assert_eq!(
        output.receipt().source_color_decision(),
        SourceColorDecision::AssumedSrgb
    );
    assert_eq!(output.receipt().output_dimensions(), input.dimensions());
    assert_eq!(
        output.receipt().render_provenance(),
        output.output().provenance()
    );
    assert_eq!(output.receipt().clipping(), output.output().clipping());
    assert_eq!(output.receipt().output_encoding(), ColorEncoding::Srgb);
    assert_eq!(
        output.receipt().working_profile().encoding(),
        ColorEncoding::LinearSrgbD65
    );
}

#[test]
fn preview_resampling_policy_is_visible_in_receipt_cache_identity() {
    let dimensions = ImageDimensions::new(4, 2).unwrap();
    let input = rusttable_image::DecodedImage::new_with_color_encoding(
        dimensions,
        vec![
            0, 0, 0, 255, 255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 128, 128, 128, 255, 200,
            200, 200, 255, 64, 64, 64, 255, 32, 32, 32, 255,
        ],
        ColorEncoding::Srgb,
    )
    .unwrap();
    let full_plan = RenderPlan::for_source(dimensions, RenderTarget::FullResolution);
    let preview_plan = RenderPlan::for_source(
        dimensions,
        RenderTarget::PreviewFit(PreviewBounds::new(2, 2).unwrap()),
    );
    let full = render_edit_with_provenance(
        &edit(),
        &input,
        SourceColorPolicy::RequireDeclaredSrgb,
        full_plan,
        source_for(dimensions),
    )
    .unwrap();
    let preview = render_edit_with_provenance(
        &edit(),
        &input,
        SourceColorPolicy::RequireDeclaredSrgb,
        preview_plan,
        source_for(dimensions),
    )
    .unwrap();

    assert_ne!(
        full.receipt().identity_hash(),
        preview.receipt().identity_hash()
    );
    let encoding = preview.receipt().canonical_encoding();
    assert!(encoding.contains("Filtered"));
    assert!(encoding.contains("Bicubic"));
    assert!(encoding.contains("Reflect"));
    assert!(encoding.contains("Premultiplied"));
}
