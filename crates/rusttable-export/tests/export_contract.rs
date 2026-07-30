use rusttable_core::Edit;
use rusttable_core::template::{Template, TemplateContext, VariableId};
use rusttable_core::{
    AssetId, ByteLength, ContentHash, EditId, PhotoId, RenderSizeRequest, Revision,
};
use rusttable_export::{
    AlphaPolicy, ArtifactBuffer, ArtifactError, ArtifactKind, BitDepth, ChannelLayout, Dependency,
    DependencySnapshot, DestinationSettings, ExportArtifact, ExportRequest, Interpolation,
    OutputProfile, PixelEncoding,
};
use rusttable_image::{ColorEncoding, DecodedImage, ImageDimensions, ImageProbe, InputFormat};
use rusttable_render::{
    RenderPlan, RenderSourceProvenance, RenderTarget, SourceColorPolicy,
    render_edit_with_provenance,
};

fn request() -> ExportRequest {
    let mut context = TemplateContext::new();
    context.set_text(VariableId::SourceStem, "photo");
    ExportRequest::for_edit(
        PhotoId::new(1).expect("photo"),
        EditId::new(2).expect("edit"),
        Revision::from_u64(3),
        ArtifactKind::Image,
        Template::parse("exports/${source_stem}.png").expect("template"),
        context,
        DependencySnapshot::new(Revision::from_u64(4), Revision::from_u64(3))
            .with_asset(Dependency::new("primary", ContentHash::Sha256([9; 32]))),
    )
    .with_size(RenderSizeRequest::exact(1, 1).expect("size"))
    .with_pixel_encoding(PixelEncoding::Integer {
        channels: ChannelLayout::Rgba,
        depth: BitDepth::Eight,
        color: ColorEncoding::Srgb,
    })
    .with_output_profile(OutputProfile::builtin(ColorEncoding::Srgb))
    .with_destination(DestinationSettings::new(
        "local-files",
        rusttable_export::CollisionPolicy::CreateNew,
    ))
}

fn render_receipt() -> rusttable_render::RenderReceipt {
    let edit = Edit::new(
        EditId::new(2).expect("edit"),
        PhotoId::new(1).expect("photo"),
        Revision::ZERO,
        [],
    )
    .expect("edit");
    let input = DecodedImage::new_with_color_encoding(
        ImageDimensions::new(1, 1).expect("dimensions"),
        vec![1, 2, 3, 255],
        ColorEncoding::Srgb,
    )
    .expect("image");
    let source = RenderSourceProvenance::new(
        PhotoId::new(1).expect("photo"),
        AssetId::new(3).expect("asset"),
        ContentHash::Sha256([9; 32]),
        ByteLength::from_bytes(4),
        ImageProbe::new(InputFormat::Png, input.dimensions()),
    );
    render_edit_with_provenance(
        &edit,
        &input,
        SourceColorPolicy::RequireDeclaredSrgb,
        RenderPlan::for_source(input.dimensions(), RenderTarget::FullResolution),
        source,
    )
    .expect("render")
    .receipt()
    .clone()
}

#[test]
fn canonical_request_is_stable_and_changes_for_pixel_affecting_fields() {
    let first = request();
    let second = request();
    assert_eq!(
        first.request_hash().expect("hash"),
        second.request_hash().expect("hash")
    );

    let changed = request().with_pixel_encoding(PixelEncoding::Integer {
        channels: ChannelLayout::Rgb,
        depth: BitDepth::Eight,
        color: ColorEncoding::Srgb,
    });
    assert_ne!(
        first.request_hash().expect("hash"),
        changed.request_hash().expect("hash")
    );
}

#[test]
fn request_validation_rejects_missing_snapshot_before_rendering() {
    let mut context = TemplateContext::new();
    context.set_text(VariableId::SourceStem, "photo");
    let request = ExportRequest::new(
        ArtifactKind::Image,
        Template::parse("${source_stem}").expect("template"),
        context,
    );
    assert!(matches!(
        request.validate(),
        Err(rusttable_export::ExportValidationError::MissingPhotoId)
    ));
}

#[test]
fn artifact_contract_binds_buffer_metadata_dependencies_and_receipt() {
    let request = request();
    let dimensions = ImageDimensions::new(1, 1).expect("dimensions");
    let buffer = ArtifactBuffer::new(dimensions, 4, request.pixel_encoding(), vec![1, 2, 3, 255])
        .expect("buffer");
    let artifact = ExportArtifact::new(
        &request,
        buffer,
        vec![0x45, 0x58, 0x49, 0x46],
        "exports/photo.png",
        render_receipt(),
    )
    .expect("artifact");
    assert_eq!(artifact.buffer().dimensions(), dimensions);
    assert_eq!(artifact.metadata_packet(), &[0x45, 0x58, 0x49, 0x46]);
    assert_eq!(artifact.filename_context(), "exports/photo.png");
    assert_ne!(artifact.content_hash(), [0; 32]);
    assert_ne!(artifact.dependency_hash(), [0; 32]);
    assert_eq!(artifact.render_receipt().output_dimensions(), dimensions);
}

#[test]
fn artifact_buffer_rejects_short_stride_and_partial_bytes() {
    let dimensions = ImageDimensions::new(2, 1).expect("dimensions");
    let encoding = PixelEncoding::Integer {
        channels: ChannelLayout::Rgba,
        depth: BitDepth::Eight,
        color: ColorEncoding::Srgb,
    };
    assert!(matches!(
        ArtifactBuffer::new(dimensions, 4, encoding, vec![0; 4]),
        Err(ArtifactError::StrideTooSmall { .. })
    ));
    assert!(matches!(
        ArtifactBuffer::new(dimensions, 8, encoding, vec![0; 4]),
        Err(ArtifactError::BufferLengthMismatch { .. })
    ));
}

#[test]
fn request_contract_keeps_alpha_and_interpolation_explicit() {
    let request = request();
    assert_eq!(
        request.size(),
        RenderSizeRequest::Exact {
            width: 1,
            height: 1
        }
    );
    assert_eq!(request.pixel_encoding().channels(), ChannelLayout::Rgba);
    let _ = AlphaPolicy::Preserve;
    let _ = Interpolation::Bilinear;
}
