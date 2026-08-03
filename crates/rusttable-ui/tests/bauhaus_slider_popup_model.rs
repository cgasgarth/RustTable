use rusttable_ui::bauhaus::slider_popup::{
    SliderRange, SliderRanges, SmoothScrollAccumulator, ZoomRangeChange,
    full_circle_pointer_position, line_offset, linear_pointer_position, loupe_scale,
    should_activate_change, zoom_range,
};

#[test]
fn line_offset_is_linear_above_the_loupe_and_clamped_to_the_slider_range() {
    assert_source_float_close(line_offset(0.4, 0.02, 0.8, 0.2, 0.2), 0.4);
    assert_source_float_close(line_offset(0.98, 0.2, 1.0, 0.2, 0.2), 0.02);
    assert_source_float_close(line_offset(0.02, 0.2, 0.0, 0.2, 0.2), -0.02);
}

#[test]
fn line_offset_reaches_source_quadratic_fine_tuning_at_popup_bottom() {
    // At y == 1 the source equation reduces to
    // 2 * scale * (x - 0.5).
    assert_source_float_close(line_offset(0.4, 0.02, 0.75, 1.0, 0.2), 0.01);
    assert_source_float_close(line_offset(0.4, 0.02, 0.25, 1.0, 0.2), -0.01);
}

#[test]
fn loupe_scale_uses_display_precision_and_factor_magnitude() {
    let range = SliderRange::new(-3.0, 4.0);
    let positive = loupe_scale(3, range, 2.0);
    let negative = loupe_scale(3, range, -2.0);

    assert_source_float_close(positive, 5.0 * 0.001 / 7.0 / 2.0);
    assert_source_float_close(negative, positive);
}

#[test]
fn linear_pointer_mapping_uses_old_position_plus_the_clamped_offset() {
    let range = SliderRange::new(-3.0, 4.0);
    let positive_scale = loupe_scale(3, range, 2.0);
    let negative_scale = loupe_scale(3, range, -2.0);

    assert_source_float_close(
        linear_pointer_position(0.4, positive_scale, 0.75, 1.0, 0.2),
        0.4 + positive_scale * 0.5,
    );
    assert_source_float_close(
        linear_pointer_position(0.4, negative_scale, 0.75, 1.0, 0.2),
        0.4 + positive_scale * 0.5,
    );
}

#[test]
fn full_circle_mapping_preserves_dead_zone_and_clockwise_positions() {
    assert_source_float_close(full_circle_pointer_position(0.37, 0.5, 0.5), 0.37);
    assert_source_float_close(full_circle_pointer_position(0.37, 0.5, 0.0), 0.5);
    assert_source_float_close(full_circle_pointer_position(0.37, 1.0, 0.5), 0.75);
    assert_source_float_close(full_circle_pointer_position(0.37, 0.5, 1.0), 0.0);
    assert_source_float_close(full_circle_pointer_position(0.37, 0.0, 0.5), 0.25);
}

#[test]
#[expect(
    clippy::float_cmp,
    reason = "The fixture asserts the exact source f32 narrowing sentinel for popup geometry."
)]
fn public_popup_inputs_are_narrowed_before_source_float_math() {
    let sub_float_ulp = f64::from(f32::EPSILON) / 4.0;
    let range = SliderRange::new(1.0, 1.0 + sub_float_ulp);

    assert_eq!(range.minimum(), 1.0);
    assert_eq!(range.maximum(), 1.0);
    assert_eq!(range.width(), 0.0);
    assert_eq!(line_offset(0.5, 0.1, 0.5 + sub_float_ulp, 0.0, 0.2), 0.0);
}

#[test]
fn line_crossing_activation_distinguishes_a_change_from_circle_wraparound() {
    assert!(should_activate_change(true, 0.0, 0.0));
    assert!(should_activate_change(false, 0.1, -0.1));
    assert!(!should_activate_change(false, 0.49, -0.49));
    assert!(should_activate_change(false, 0.4, 0.95));
    assert!(!should_activate_change(false, 0.0, 0.4));
}

#[test]
fn zoom_is_value_centered_and_uses_source_power_of_two_transition() {
    let ranges = SliderRanges::new(
        SliderRange::new(-3.0, 4.0),
        SliderRange::new(-3.0, 4.0),
        SliderRange::new(-18.0, 18.0),
    );

    assert_eq!(
        zoom_range(ranges, 0.0, 3, 1.0, -2.0),
        ZoomRangeChange::Zoomed(SliderRange::new(-1.5, 2.0))
    );
    assert_eq!(
        zoom_range(ranges, 0.0, 3, 1.0, 2.0),
        ZoomRangeChange::Zoomed(SliderRange::new(-6.0, 8.0))
    );
}

#[test]
fn zoom_rejects_hard_range_overflow_and_sub_precision_ranges() {
    let hard = SliderRange::new(-18.0, 18.0);
    let hard_ranges = SliderRanges::new(hard, SliderRange::new(-3.0, 4.0), hard);
    assert_eq!(
        zoom_range(hard_ranges, 0.0, 3, 1.0, 1.0),
        ZoomRangeChange::Rejected
    );

    let narrow = SliderRanges::new(
        SliderRange::new(0.0, 0.02),
        SliderRange::new(0.0, 0.02),
        SliderRange::new(0.0, 1.0),
    );
    assert_eq!(
        zoom_range(narrow, 0.01, 3, 1.0, -4.0),
        ZoomRangeChange::Rejected
    );
}

#[test]
fn zero_zoom_toggles_hard_and_soft_ranges_then_restores_visible_value() {
    let hard = SliderRange::new(-18.0, 18.0);
    let soft = SliderRange::new(-3.0, 4.0);

    assert_eq!(
        zoom_range(SliderRanges::new(hard, soft, hard), 10.0, 3, 1.0, 0.0),
        ZoomRangeChange::Toggled(SliderRange::new(-3.0, 10.0))
    );
    assert_eq!(
        zoom_range(
            SliderRanges::new(SliderRange::new(-3.0, 10.0), soft, hard),
            10.0,
            3,
            1.0,
            0.0,
        ),
        ZoomRangeChange::Toggled(hard)
    );
}

#[test]
fn negative_factor_keeps_the_sources_signed_minimum_visible_test() {
    let ranges = SliderRanges::new(
        SliderRange::new(0.0, 0.001),
        SliderRange::new(0.0, 0.001),
        SliderRange::new(0.0, 1.0),
    );

    assert!(matches!(
        zoom_range(ranges, 0.0005, 3, -1.0, -10.0),
        ZoomRangeChange::Zoomed(_)
    ));
}

#[test]
fn smooth_scroll_accumulates_units_truncates_toward_zero_and_resets_on_stop() {
    let mut scroll = SmoothScrollAccumulator::default();

    assert_eq!(scroll.push(0.0, 0.4), None);
    assert_eq!(scroll.push(0.0, 0.4), None);
    assert_eq!(scroll.push(0.0, 0.4), Some((0, 1)));
    assert_double_close(scroll.remainder().1, 0.2);

    assert_eq!(scroll.push(-1.4, -0.4), Some((-1, 0)));
    assert_double_close(scroll.remainder().0, -0.4);
    assert_double_close(scroll.remainder().1, -0.2);

    scroll.stop();
    assert_eq!(scroll.remainder(), (0.0, 0.0));
    assert_eq!(scroll.push(0.0, 0.8), None);
}

#[test]
fn opposing_smooth_units_still_report_a_handled_zero_sum() {
    let mut scroll = SmoothScrollAccumulator::default();

    assert_eq!(scroll.push_sum(1.2, -1.2), Some(0));
    assert_double_close(scroll.remainder().0, 0.2);
    assert_double_close(scroll.remainder().1, -0.2);
}

#[test]
fn one_shared_scroll_remainder_interleaves_controllers_and_any_stop_resets_it() {
    let mut shared = SmoothScrollAccumulator::default();

    assert_eq!(shared.push_sum(0.0, 0.6), None, "controller A");
    assert_eq!(shared.push_sum(0.0, 0.6), Some(1), "controller B");
    assert_double_close(shared.remainder().1, 0.2);

    shared.stop(); // A stop event from either controller resets the source global.
    assert_eq!(shared.push_sum(0.0, 0.8), None);
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "The fixture intentionally converts expected values through the source f32 numeric boundary."
)]
fn assert_source_float_close(actual: f64, expected: f64) {
    let expected = f64::from(expected as f32);
    let tolerance = 4.0 * f64::from(f32::EPSILON) * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected source float {expected} ± {tolerance}, got {actual}"
    );
}

fn assert_double_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1.0e-12,
        "expected {expected}, got {actual}"
    );
}
