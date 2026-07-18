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
    RenderSourceProvenance::new(
        PhotoId::new(2).unwrap(),
        AssetId::new(3).unwrap(),
        ContentHash::Sha256([7; 32]),
        ByteLength::from_bytes(42),
        ImageProbe::new(InputFormat::Png, ImageDimensions::new(1, 1).unwrap()),
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
}
