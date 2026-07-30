use rusttable_core::{Edit, EditId, Operation, OperationId, OperationKey, PhotoId, Revision};
use rusttable_image::{ColorEncoding, DecodedImage, ImageDimensions};
use rusttable_render::{RenderError, SourceColorPolicy, render_edit};

fn image(encoding: ColorEncoding) -> DecodedImage {
    DecodedImage::new_with_color_encoding(
        ImageDimensions::new(1, 1).unwrap(),
        vec![255, 0, 0, 255],
        encoding,
    )
    .unwrap()
}

fn invalid_edit() -> Edit {
    Edit::new(
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
    .unwrap()
}

#[test]
fn require_declared_srgb_rejects_display_p3_before_pipeline_compilation() {
    assert!(matches!(
        render_edit(
            &invalid_edit(),
            &image(ColorEncoding::DisplayP3),
            SourceColorPolicy::RequireDeclaredSrgb,
        ),
        Err(RenderError::SourceColor {
            actual: ColorEncoding::DisplayP3
        })
    ));
}

#[test]
fn require_declared_supported_rejects_unspecified() {
    assert!(matches!(
        render_edit(
            &Edit::new(
                EditId::new(1).unwrap(),
                PhotoId::new(2).unwrap(),
                Revision::ZERO,
                []
            )
            .unwrap(),
            &image(ColorEncoding::Unspecified),
            SourceColorPolicy::RequireDeclaredSupported,
        ),
        Err(RenderError::SourceColor {
            actual: ColorEncoding::Unspecified
        })
    ));
}
