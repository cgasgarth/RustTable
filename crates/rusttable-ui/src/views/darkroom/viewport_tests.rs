use super::{canvas_projection, scroll_zoom_direction, stepped_zoom};
use crate::viewport_presentation::{DarkroomZoom, ViewportPan};

#[test]
fn fit_projection_preserves_aspect_inside_darktable_border() {
    let projection = canvas_projection(
        900,
        600,
        32,
        20,
        10,
        DarkroomZoom::Fit,
        ViewportPan::default(),
    )
    .expect("valid fitted image");

    assert_eq!(projection.viewport, (10, 10, 880, 580));
    assert_eq!(projection.image, (10, 25, 880, 550));
}

#[test]
fn small_fill_and_percentage_use_darktable_zoom_scales() {
    let small = canvas_projection(
        900,
        600,
        32,
        20,
        10,
        DarkroomZoom::Small,
        ViewportPan::default(),
    )
    .expect("small");
    assert_eq!(small.image, (230, 162, 440, 275));

    let fill = canvas_projection(
        900,
        600,
        32,
        20,
        10,
        DarkroomZoom::Fill,
        ViewportPan::default(),
    )
    .expect("fill");
    assert_eq!(fill.image, (-14, 10, 928, 580));

    let one_hundred = canvas_projection(
        900,
        600,
        640,
        400,
        10,
        DarkroomZoom::OneHundredPercent,
        ViewportPan::default(),
    )
    .expect("100%");
    assert_eq!(one_hundred.image, (130, 100, 640, 400));
}

#[test]
fn normalized_pan_is_pixel_precise_and_bounded_by_scaled_image() {
    let centered = canvas_projection(
        900,
        600,
        1_600,
        1_000,
        10,
        DarkroomZoom::OneHundredPercent,
        ViewportPan::default(),
    )
    .expect("centered");
    assert_eq!(centered.image, (-350, -200, 1_600, 1_000));

    let panned = canvas_projection(
        900,
        600,
        1_600,
        1_000,
        10,
        DarkroomZoom::OneHundredPercent,
        ViewportPan::new(500, -1_000),
    )
    .expect("panned");
    assert_eq!(panned.image, (-530, 10, 1_600, 1_000));
}

#[test]
fn wheel_zoom_matches_darktable_and_ignores_zero_or_horizontal_only_scroll() {
    assert_eq!(scroll_zoom_direction(-1.0), Some(true));
    assert_eq!(scroll_zoom_direction(1.0), Some(false));
    assert_eq!(scroll_zoom_direction(0.0), None);
    assert_eq!(scroll_zoom_direction(f64::NAN), None);
}

#[test]
fn zoom_steps_are_monotonic_by_projected_scale_not_menu_order() {
    let viewport = (900, 600);
    let image = (1_500, 1_000);
    let border = 10;

    assert_eq!(
        stepped_zoom(DarkroomZoom::FiftyPercent, true, viewport, image, border),
        Some(DarkroomZoom::Fit)
    );
    assert_eq!(
        stepped_zoom(DarkroomZoom::Fit, true, viewport, image, border),
        Some(DarkroomZoom::Fill)
    );
    assert_eq!(
        stepped_zoom(DarkroomZoom::Fill, true, viewport, image, border),
        Some(DarkroomZoom::OneHundredPercent)
    );
    assert_eq!(
        stepped_zoom(DarkroomZoom::Fill, false, viewport, image, border),
        Some(DarkroomZoom::Fit)
    );
}

#[test]
fn projection_rejects_empty_dimensions_or_consumed_border() {
    assert!(
        canvas_projection(
            0,
            600,
            32,
            20,
            10,
            DarkroomZoom::Fit,
            ViewportPan::default()
        )
        .is_none()
    );
    assert!(
        canvas_projection(
            20,
            600,
            32,
            20,
            10,
            DarkroomZoom::Fit,
            ViewportPan::default()
        )
        .is_none()
    );
}
