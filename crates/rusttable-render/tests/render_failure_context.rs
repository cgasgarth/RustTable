use rusttable_core::{
    AssetId, ByteLength, ContentHash, Edit, EditId, Operation, OperationId, OperationKey, PhotoId,
    Revision,
};
use rusttable_image::{ColorEncoding, DecodedImage, ImageDimensions, ImageProbe, InputFormat};
use rusttable_render::{
    RenderFailureStage, RenderPlan, RenderSourceProvenance, RenderTarget, SourceColorPolicy,
    render_edit_with_provenance,
};

#[test]
fn delegated_plan_failure_keeps_nested_render_error_and_context() {
    let edit = Edit::new(
        EditId::new(1).unwrap(),
        PhotoId::new(2).unwrap(),
        Revision::ZERO,
        [Operation::new(
            OperationId::new(3).unwrap(),
            OperationKey::new("rusttable.invalid").unwrap(),
            true,
            [],
        )
        .unwrap()],
    )
    .unwrap();
    let input = DecodedImage::new_with_color_encoding(
        ImageDimensions::new(1, 1).unwrap(),
        vec![255, 0, 0, 255],
        ColorEncoding::Srgb,
    )
    .unwrap();
    let plan = RenderPlan::for_source(input.dimensions(), RenderTarget::FullResolution);
    let source = RenderSourceProvenance::new(
        PhotoId::new(2).unwrap(),
        AssetId::new(3).unwrap(),
        ContentHash::Sha256([1; 32]),
        ByteLength::from_bytes(4),
        ImageProbe::new(InputFormat::Png, input.dimensions()),
    );
    let error = render_edit_with_provenance(
        &edit,
        &input,
        SourceColorPolicy::RequireDeclaredSrgb,
        plan,
        source,
    )
    .unwrap_err();

    assert_eq!(error.stage(), RenderFailureStage::Pipeline);
    assert!(std::error::Error::source(&error).is_some());
}
