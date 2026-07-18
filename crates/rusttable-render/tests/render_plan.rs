use rusttable_image::ImageDimensions;
use rusttable_render::{PreviewBounds, RenderPlan, RenderSampling, RenderTarget};

#[test]
fn preview_bounds_have_distinct_zero_axis_errors() {
    assert!(matches!(
        PreviewBounds::new(0, 1),
        Err(rusttable_render::PreviewBoundsError::ZeroWidth)
    ));
    assert!(matches!(
        PreviewBounds::new(1, 0),
        Err(rusttable_render::PreviewBoundsError::ZeroHeight)
    ));
}

#[test]
fn full_resolution_plan_preserves_source_and_identity_sampling() {
    let source = ImageDimensions::new(7, 3).unwrap();
    let plan = RenderPlan::for_source(source, RenderTarget::FullResolution);

    assert_eq!(plan.source_dimensions(), source);
    assert_eq!(plan.output_dimensions(), source);
    assert_eq!(plan.sampling(), RenderSampling::Identity);
}

#[test]
fn preview_plan_fits_without_upscaling_using_integer_arithmetic() {
    let source = ImageDimensions::new(5, 3).unwrap();
    let bounds = PreviewBounds::new(2, 2).unwrap();
    let plan = RenderPlan::for_source(source, RenderTarget::PreviewFit(bounds));

    assert_eq!(
        plan.output_dimensions(),
        ImageDimensions::new(2, 1).unwrap()
    );
    assert_eq!(plan.sampling(), RenderSampling::CenterPoint);
}

#[test]
fn plans_are_deterministic_and_bounds_compare_by_value() {
    let source = ImageDimensions::new(10, 4).unwrap();
    let first_bounds = PreviewBounds::new(4, 4).unwrap();
    let second_bounds = PreviewBounds::new(4, 4).unwrap();

    assert_eq!(first_bounds, second_bounds);
    assert_eq!(
        RenderPlan::for_source(source, RenderTarget::PreviewFit(first_bounds)),
        RenderPlan::for_source(source, RenderTarget::PreviewFit(second_bounds))
    );
}
