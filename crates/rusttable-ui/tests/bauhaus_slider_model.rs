use rusttable_ui::bauhaus::slider::{
    AutomaticStepPolicy, BauhausSliderModel, MAX_GRADIENT_STOPS, SliderCurve, SliderModelError,
};

#[test]
fn soft_range_is_distinct_from_hard_range_and_reset_restores_it() {
    let mut slider = BauhausSliderModel::new(-18.0, 18.0, 0.0, 0.0, 3, true).expect("source range");

    slider.set_soft_range(-3.0, 4.0);
    assert_eq!(slider.visible_range(), (-3.0, 4.0));
    assert_eq!(slider.hard_range(), (-18.0, 18.0));

    slider.set_value(10.0);
    assert_close(slider.value(), 10.0);
    assert_eq!(slider.visible_range(), (-3.0, 10.0));

    slider.reset();
    assert_close(slider.value(), 0.0);
    assert_eq!(slider.visible_range(), (-3.0, 4.0));
}

#[test]
fn setter_clamps_to_hard_bounds_and_rounds_in_display_units() {
    let mut slider = BauhausSliderModel::new(-2.0, 2.0, 0.01, 0.0, 2, true).expect("source range");
    slider.set_factor(100.0);

    slider.set_value(0.012_36);
    assert_close(slider.value(), 0.0124);
    slider.set_value(3.0);
    assert_close(slider.value(), 2.0);
    slider.set_value(f64::NAN);
    assert_close(slider.value(), 2.0);
}

#[test]
fn percent_format_applies_source_factor_and_digit_adjustment() {
    let mut slider = BauhausSliderModel::new(0.0, 1.0, 0.0, 0.5, 3, true).expect("source range");

    slider.set_format("%");

    assert_close(slider.factor(), 100.0);
    assert_eq!(slider.digits(), 1);
    assert_eq!(slider.value_text(slider.value()), "50.0%");
}

#[test]
fn hard_range_crossing_zero_uses_explicit_sign() {
    let mut slider = BauhausSliderModel::new(-2.0, 2.0, 0.0, 0.5, 2, true).expect("source range");
    assert_eq!(slider.value_text(slider.value()), "+0.50");

    slider.set_offset(3.0);
    assert_eq!(slider.value_text(slider.value()), "3.50");
}

#[test]
fn degree_format_wraps_once_and_uses_the_full_range() {
    let mut slider = BauhausSliderModel::new(0.0, 1.0, 0.0, 0.0, 3, true).expect("source range");
    slider.set_factor(360.0);
    slider.set_format("°");

    slider.set_value(1.1);
    assert_close(slider.value(), 0.1);
    assert_eq!(slider.visible_range(), (0.0, 1.0));

    slider.set_value(-0.1);
    assert_close(slider.value(), 0.9);
}

#[test]
fn changing_curve_preserves_value_and_log_curve_round_trips() {
    let mut slider = BauhausSliderModel::new(0.0, 100.0, 0.0, 25.0, 2, true).expect("source range");

    slider.set_curve(SliderCurve::Log10);
    assert_close(slider.value(), 25.0);
    slider.set_value(75.0);
    assert_close(slider.value(), 75.0);
    assert_eq!(slider.curve(), SliderCurve::Log10);
}

#[test]
fn popup_normalized_commits_and_range_transitions_preserve_source_state() {
    let mut slider = BauhausSliderModel::new(-18.0, 18.0, 0.0, 1.0, 3, true).expect("source range");
    slider.set_soft_range(-3.0, 4.0);
    slider.set_value(1.0);

    let old_position = slider.normalized_position();
    assert!(slider.set_visible_range_preserving_position(-1.0, 2.5));
    assert_close(slider.normalized_position(), old_position);
    assert_close(slider.value(), 1.0);

    assert!(slider.set_visible_range_preserving_value(-18.0, 18.0));
    assert_close(slider.value(), 1.0);
    assert_close(slider.normalized_position(), 19.0 / 36.0);

    slider.set_normalized_position(1.5);
    assert_close(slider.value(), 18.0);
    assert!(!slider.set_visible_range_preserving_value(-19.0, 18.0));
    assert_eq!(slider.visible_range(), (-18.0, 18.0));
}

#[test]
fn automatic_step_matches_darktable_decade_selection_and_factor_sign() {
    let slider = BauhausSliderModel::new(-18.0, 18.0, 0.0, 0.0, 3, true).expect("source range");
    assert_close(
        slider.effective_step(AutomaticStepPolicy::VisibleRange),
        0.1,
    );

    let mut reverse = BauhausSliderModel::new(0.0, 1.0, 0.0, 0.5, 3, true).expect("source range");
    reverse.set_factor(-100.0);
    assert_close(
        reverse.effective_step(AutomaticStepPolicy::VisibleRange),
        -0.01,
    );
}

#[test]
fn automatic_step_policy_is_supplied_at_each_read_from_the_live_global_setting() {
    let mut slider = BauhausSliderModel::new(-18.0, 18.0, 0.0, 0.0, 3, true).expect("source range");
    slider.set_soft_range(-3.0, 4.0);
    slider.set_value(10.0);

    assert_close(
        slider.effective_step(AutomaticStepPolicy::VisibleRange),
        0.1,
    );
    assert_close(slider.effective_step(AutomaticStepPolicy::SoftRange), 0.05);
    assert_close(
        slider.effective_step(AutomaticStepPolicy::VisibleRange),
        0.1,
    );
}

#[test]
fn hard_bound_setters_collapse_at_crossed_or_equal_endpoints() {
    let mut lower = BauhausSliderModel::new(-2.0, 2.0, 0.0, 0.5, 2, true).expect("source range");
    lower.set_soft_range(-1.0, 1.0);
    lower.set_hard_minimum(2.0);
    assert_eq!(lower.hard_range(), (2.0, 2.0));
    assert_eq!(lower.visible_range(), (2.0, 2.0));
    assert_close(lower.value(), 2.0);

    let mut upper = BauhausSliderModel::new(-2.0, 2.0, 0.0, -0.5, 2, true).expect("source range");
    upper.set_hard_maximum(-3.0);
    assert_eq!(upper.hard_range(), (-3.0, -3.0));
    assert_eq!(upper.visible_range(), (-3.0, -3.0));
    assert_close(upper.value(), -3.0);
}

#[test]
fn exact_degree_endpoints_do_not_wrap() {
    let mut slider = BauhausSliderModel::new(0.0, 1.0, 0.0, 0.5, 3, true).expect("source range");
    slider.set_factor(360.0);
    slider.set_format("°");

    slider.set_value(1.0);
    assert_close(slider.value(), 1.0);
    slider.set_value(0.0);
    assert_close(slider.value(), 0.0);
}

#[test]
fn source_float_boundary_does_not_wrap_a_sub_f32_degree_overflow() {
    let mut slider = BauhausSliderModel::new(0.0, 1.0, 0.0, 0.5, 3, true).expect("source range");
    slider.set_factor(360.0);
    slider.set_format("°");

    slider.set_value(1.0 + 1.0e-8);

    assert_close(slider.value(), 1.0);
}

#[test]
fn public_double_boundary_exposes_exact_source_float_state() {
    let slider =
        BauhausSliderModel::new(0.1, 0.3, -0.01, 0.2, 7, true).expect("source float state");

    assert_eq!(
        slider.hard_range().0.to_bits(),
        f64::from(0.1_f32).to_bits()
    );
    assert_eq!(
        slider.hard_range().1.to_bits(),
        f64::from(0.3_f32).to_bits()
    );
    assert_eq!(
        slider.configured_step().to_bits(),
        f64::from(-0.01_f32).to_bits()
    );
    assert_eq!(
        slider.default_value().to_bits(),
        f64::from(0.2_f32).to_bits()
    );
}

#[test]
fn source_constructor_accepts_collapsed_ranges_and_negative_steps() {
    let collapsed =
        BauhausSliderModel::new(1.0, 1.0, 0.1, 1.0, 3, true).expect("collapsed source range");
    assert_close(collapsed.value(), 1.0);

    let negative_step =
        BauhausSliderModel::new(0.0, 1.0, -0.1, 0.5, 3, true).expect("signed source step");
    assert_close(
        negative_step.effective_step(AutomaticStepPolicy::VisibleRange),
        0.1,
    );

    let mut reverse = negative_step;
    reverse.set_factor(-1.0);
    reverse.set_step(-0.25);
    assert_close(
        reverse.effective_step(AutomaticStepPolicy::SoftRange),
        -0.25,
    );
}

#[test]
fn negative_factor_selects_and_retains_the_reverse_curve() {
    let mut slider = BauhausSliderModel::new(0.0, 1.0, 0.0, 0.25, 3, true).expect("source range");

    slider.set_factor(-100.0);
    assert_eq!(slider.curve(), SliderCurve::ReverseLinear);
    assert_close(slider.value(), 0.75);
    assert_close(slider.display_value(), -75.0);

    slider.set_factor(100.0);
    assert_eq!(slider.curve(), SliderCurve::ReverseLinear);
    assert_close(slider.value(), 0.75);
}

#[test]
fn zero_factor_retains_source_ieee_display_conversion() {
    let mut slider = BauhausSliderModel::new(0.0, 1.0, 0.0, 0.5, 3, true).expect("source range");

    slider.set_factor(0.0);
    assert_close(slider.factor(), 0.0);

    // `(offset - offset) / 0` is NaN, which the public source setter ignores.
    slider.set_display_value(0.0);
    assert_close(slider.value(), 0.5);

    // A nonzero quotient becomes infinity, clamps to the hard edge, and then
    // the source's zero rounding base produces a NaN normalized position.
    slider.set_display_value(1.0);
    assert!(slider.value().is_nan());
}

#[test]
fn negative_printf_precision_uses_c_default_precision() {
    let mut slider = BauhausSliderModel::new(0.0, 1.0, 0.0, 0.5, 1, true).expect("source range");

    slider.set_format("%");

    assert_eq!(slider.digits(), -1);
    assert_eq!(slider.value_text(slider.value()), "50.000000%");
}

#[test]
fn gradient_stops_replace_exact_positions_and_enforce_source_capacity() {
    let mut slider = BauhausSliderModel::new(0.0, 1.0, 0.0, 0.5, 3, false).expect("source range");

    let stop_count = u32::try_from(MAX_GRADIENT_STOPS).expect("source stop capacity");
    for index in 0..MAX_GRADIENT_STOPS {
        let index = u32::try_from(index).expect("source stop index");
        let position = f64::from(index) / f64::from(stop_count);
        assert!(slider.set_stop(position, [position, 0.0, 1.0]));
    }
    assert_eq!(slider.stops().len(), MAX_GRADIENT_STOPS);
    assert!(!slider.set_stop(0.999, [1.0, 1.0, 1.0]));

    assert!(slider.set_stop(0.0, [0.25, 0.5, 0.75]));
    assert_eq!(
        slider.stops()[0].rgb().map(f64::to_bits),
        [0.25, 0.5, 0.75].map(f64::to_bits)
    );
    assert_eq!(slider.stops().len(), MAX_GRADIENT_STOPS);

    slider.clear_stops();
    assert!(slider.stops().is_empty());
}

#[test]
fn gradient_identity_and_components_use_source_float_precision() {
    let mut slider = BauhausSliderModel::new(0.0, 1.0, 0.0, 0.5, 3, false).expect("source range");
    let first_position = 0.3_f64;
    let source_position = 0.3_f32;
    let same_source_float = f64::from(source_position) + 1.0e-10;

    assert!(slider.set_stop(first_position, [0.1, 0.2, 0.3]));
    assert!(slider.set_stop(same_source_float, [0.4, 0.5, 0.6]));

    assert_eq!(slider.stops().len(), 1);
    assert_eq!(
        slider.stops()[0].position().to_bits(),
        f64::from(source_position).to_bits()
    );
    for (actual, expected) in slider.stops()[0]
        .rgb()
        .into_iter()
        .zip([0.4_f32, 0.5_f32, 0.6_f32])
    {
        assert_eq!(actual.to_bits(), f64::from(expected).to_bits());
    }
}

#[test]
fn invalid_construction_rejects_only_nonfinite_or_inverted_numeric_boundaries() {
    assert!(matches!(
        BauhausSliderModel::new(2.0, 1.0, 0.1, 1.0, 2, true),
        Err(SliderModelError::InvalidRange)
    ));
    assert!(matches!(
        BauhausSliderModel::new(0.0, 1.0, f64::MAX, 0.5, 2, true),
        Err(SliderModelError::NonFinite)
    ));
    assert!(matches!(
        BauhausSliderModel::new(0.0, f64::INFINITY, 0.1, 0.5, 2, true),
        Err(SliderModelError::NonFinite)
    ));
}

#[test]
fn mask_feather_retains_its_default_below_minimum_through_source_rounding() {
    let mut slider =
        BauhausSliderModel::new(0.0001, 1.0, 0.0, 0.0, 2, true).expect("mask feather source range");

    assert_close(slider.value(), 0.0);
    assert_close(slider.default_value(), 0.0);

    slider.reset();
    assert_close(slider.value(), 0.0);
}

#[test]
fn raw_setter_ignores_nan_while_normalized_setter_retains_source_ieee_behavior() {
    let mut slider = BauhausSliderModel::new(-2.0, 2.0, 0.0, 0.5, 2, true).expect("source range");

    slider.set_value(f64::INFINITY);
    assert_close(slider.value(), 2.0);
    slider.set_value(f64::NEG_INFINITY);
    assert_close(slider.value(), -2.0);
    slider.set_value(f64::NAN);
    assert_close(slider.value(), -2.0);

    slider.set_normalized_position(f64::INFINITY);
    assert_close(slider.value(), 2.0);
    slider.set_normalized_position(f64::NEG_INFINITY);
    assert_close(slider.value(), -2.0);
    slider.set_normalized_position(f64::NAN);
    assert!(slider.value().is_nan());
}

#[test]
fn infinite_degree_input_generates_the_source_nan_after_wrap() {
    let mut slider = BauhausSliderModel::new(0.0, 1.0, 0.0, 0.5, 3, true).expect("source range");
    slider.set_factor(360.0);
    slider.set_format("°");

    slider.set_value(f64::INFINITY);

    assert!(slider.value().is_nan());
}

fn assert_close(actual: f64, expected: f64) {
    let tolerance = f64::from(f32::EPSILON) * expected.abs().max(1.0) * 4.0;
    assert!(
        (actual - expected).abs() <= tolerance,
        "actual={actual:?} expected={expected:?} tolerance={tolerance:?}"
    );
}
