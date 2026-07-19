use std::error::Error;

use rusttable_core::{AssetId, ByteLength, ContentHash, Edit, EditId, PhotoId, Revision};
use rusttable_image::{ColorEncoding, DecodedImage, ImageDimensions, ImageProbe, InputFormat};
use rusttable_render::{
    RenderFailureStage, RenderPlan, RenderSourceProvenance, RenderTarget, SourceColorPolicy,
    render_edit_with_provenance,
};

fn source(photo_id: u128, dimensions: ImageDimensions) -> RenderSourceProvenance {
    RenderSourceProvenance::new(
        PhotoId::new(photo_id).unwrap(),
        AssetId::new(3).unwrap(),
        ContentHash::Sha256([7; 32]),
        ByteLength::from_bytes(42),
        ImageProbe::new(InputFormat::Png, dimensions),
    )
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

fn input() -> DecodedImage {
    DecodedImage::new_with_color_encoding(
        ImageDimensions::new(1, 1).unwrap(),
        vec![255, 0, 0, 255],
        ColorEncoding::Srgb,
    )
    .unwrap()
}

#[test]
fn photo_mismatch_precedes_dimension_and_plan_validation() {
    let plan = RenderPlan::for_source(input().dimensions(), RenderTarget::FullResolution);
    let error = render_edit_with_provenance(
        &edit(),
        &input(),
        SourceColorPolicy::RequireDeclaredSrgb,
        plan,
        source(9, ImageDimensions::new(9, 9).unwrap()),
    )
    .unwrap_err();

    assert_eq!(error.stage(), RenderFailureStage::SourcePhoto);
    assert!(error.source().is_none());
}

#[test]
fn matching_photo_with_probe_dimension_mismatch_fails_before_plan() {
    let plan = RenderPlan::for_source(input().dimensions(), RenderTarget::FullResolution);
    let error = render_edit_with_provenance(
        &edit(),
        &input(),
        SourceColorPolicy::RequireDeclaredSrgb,
        plan,
        source(2, ImageDimensions::new(9, 9).unwrap()),
    )
    .unwrap_err();

    assert_eq!(error.stage(), RenderFailureStage::SourceDimensions);
    assert_ne!(
        error.context().source().probe().dimensions(),
        input().dimensions()
    );
}
