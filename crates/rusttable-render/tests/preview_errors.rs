use std::error::Error;

use rusttable_core::{Edit, EditId, Operation, OperationId, OperationKey, PhotoId, Revision};
use rusttable_image::{ColorEncoding, DecodedImage, ImageDimensions};
use rusttable_render::{
    RenderError, RenderPlan, RenderTarget, SourceColorPolicy, render_edit_with_plan,
};

fn input() -> DecodedImage {
    DecodedImage::new_with_color_encoding(
        ImageDimensions::new(2, 1).unwrap(),
        vec![255, 0, 0, 255, 0, 255, 0, 255],
        ColorEncoding::Unspecified,
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
fn plan_source_mismatch_precedes_policy_and_pipeline_errors() {
    let plan = RenderPlan::for_source(
        ImageDimensions::new(9, 9).unwrap(),
        RenderTarget::FullResolution,
    );
    let error = render_edit_with_plan(
        &invalid_edit(),
        &input(),
        SourceColorPolicy::RequireDeclaredSrgb,
        plan,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        RenderError::PlanSourceDimensions { planned, actual }
            if planned == ImageDimensions::new(9, 9).unwrap()
                && actual == input().dimensions()
    ));
    assert!(error.source().is_none());
}
