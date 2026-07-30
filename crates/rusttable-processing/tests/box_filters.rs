//! Source-derived contract for Darktable's `src/common/box_filters.cc` and
//! `src/common/box_filters.h`.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::needless_range_loop,
    reason = "small source-derived dimensions and identity checks keep reference code direct"
)]

use rusttable_processing::common::box_filters::{
    BOX_ITERATIONS, BOXFILTER_KAHAN_SUM, BoxFilterError, box_max, box_mean, box_mean_horizontal,
    box_mean_vertical, box_min,
};

fn assert_close(actual: &[f32], expected: &[f32], tolerance: f32) {
    assert_eq!(actual.len(), expected.len());
    for (index, (&actual, &expected)) in actual.iter().zip(expected).enumerate() {
        assert!(
            (actual - expected).abs() <= tolerance,
            "sample {index}: expected {expected}, got {actual}"
        );
    }
}

fn repeated_reference_mean(
    mut values: Vec<f32>,
    height: usize,
    width: usize,
    channels: usize,
    radius: usize,
    iterations: u32,
) -> Vec<f32> {
    for _ in 0..iterations {
        let source = values.clone();
        for y in 0..height {
            for x in 0..width {
                let first_x = x.saturating_sub(radius);
                let last_x = x.saturating_add(radius).min(width - 1);
                for channel in 0..channels {
                    let mut sum = 0.0_f32;
                    for sample_x in first_x..=last_x {
                        sum += source[(y * width + sample_x) * channels + channel];
                    }
                    values[(y * width + x) * channels + channel] =
                        sum / (last_x - first_x + 1) as f32;
                }
            }
        }

        let source = values.clone();
        for y in 0..height {
            let first_y = y.saturating_sub(radius);
            let last_y = y.saturating_add(radius).min(height - 1);
            for x in 0..width {
                for channel in 0..channels {
                    let mut sum = 0.0_f32;
                    for sample_y in first_y..=last_y {
                        sum += source[(sample_y * width + x) * channels + channel];
                    }
                    values[(y * width + x) * channels + channel] =
                        sum / (last_y - first_y + 1) as f32;
                }
            }
        }
    }
    values
}

fn reference_extreme(
    source: &[f32],
    height: usize,
    width: usize,
    radius: usize,
    minimum: bool,
) -> Vec<f32> {
    let select = |current: f32, candidate: f32| {
        if minimum {
            current.min(candidate)
        } else {
            current.max(candidate)
        }
    };
    let initial = if minimum {
        f32::INFINITY
    } else {
        f32::NEG_INFINITY
    };
    let mut horizontal = vec![0.0; source.len()];
    for y in 0..height {
        for x in 0..width {
            let first = x.saturating_sub(radius);
            let last = x.saturating_add(radius).min(width - 1);
            horizontal[y * width + x] = (first..=last).fold(initial, |value, sample| {
                select(value, source[y * width + sample])
            });
        }
    }

    let mut output = vec![0.0; source.len()];
    for y in 0..height {
        let first = y.saturating_sub(radius);
        let last = y.saturating_add(radius).min(height - 1);
        for x in 0..width {
            output[y * width + x] = (first..=last).fold(initial, |value, sample| {
                select(value, horizontal[sample * width + x])
            });
        }
    }
    output
}

fn reference_vertical_kahan(
    source: &[f32],
    height: usize,
    width: usize,
    channels: usize,
    radius: usize,
) -> Vec<f32> {
    let stride = width * channels;
    let mut output = vec![0.0; source.len()];
    let update = |sum: &mut f32, correction: &mut f32, value: f32| {
        let adjusted = value - *correction;
        let updated = *sum + adjusted;
        *correction = (updated - *sum) - adjusted;
        *sum = updated;
    };
    let initial_last = radius.min(height - 1);
    for column in 0..stride {
        let mut sum = 0.0;
        let mut correction = 0.0;
        for y in 0..=initial_last {
            update(&mut sum, &mut correction, source[y * stride + column]);
        }
        let mut hits = initial_last + 1;
        for y in 0..height {
            output[y * stride + column] = sum / hits as f32;
            if y + 1 == height {
                break;
            }
            if y >= radius {
                hits -= 1;
                update(
                    &mut sum,
                    &mut correction,
                    -source[(y - radius) * stride + column],
                );
            }
            if let Some(next) = y
                .checked_add(radius)
                .and_then(|value| value.checked_add(1))
                .filter(|&value| value < height)
            {
                hits += 1;
                update(&mut sum, &mut correction, source[next * stride + column]);
            }
        }
    }
    output
}

#[test]
fn shrinking_edge_windows_use_actual_hit_counts() {
    let mut row = vec![1.0, 2.0, 3.0, 4.0, 5.0];
    box_mean(&mut row, 1, 5, 1, 1, 1).expect("one-channel mean");
    assert_eq!(row, [1.5, 2.0, 3.0, 4.0, 4.5]);

    let mut asymmetric = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    box_mean(&mut asymmetric, 2, 3, 1, 1, 1).expect("asymmetric mean");
    assert_eq!(asymmetric, [3.0, 3.5, 4.0, 3.0, 3.5, 4.0]);

    let mut impulse = vec![0.0, 9.0, 0.0];
    box_mean(&mut impulse, 1, 3, 1, 1, 2).expect("repeated edge normalization");
    assert_eq!(impulse, [3.75, 4.0, 3.75]);

    let mut center_impulse = vec![0.0, 0.0, 0.0, 0.0, 9.0, 0.0, 0.0, 0.0, 0.0];
    box_mean(&mut center_impulse, 3, 3, 1, 1, 1).expect("two-dimensional edge windows");
    assert_eq!(
        center_impulse,
        [2.25, 1.5, 2.25, 1.5, 1.0, 1.5, 2.25, 1.5, 2.25]
    );
}

#[test]
fn radius_larger_than_both_dimensions_averages_every_actual_sample() {
    let mut values = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    box_mean(&mut values, 2, 3, 1, usize::MAX, 1).expect("unbounded radius is safe");
    assert_eq!(values, [3.5; 6]);
}

#[test]
fn zero_and_eight_iterations_have_pinned_behavior() {
    assert_eq!(BOX_ITERATIONS, 8);

    let source = vec![0.0, 0.0, 1.0, 0.0, 0.0];
    let mut identity = source.clone();
    box_mean(&mut identity, 1, 5, 1, 2, 0).expect("zero iterations");
    assert_eq!(identity, source);

    let expected = repeated_reference_mean(source.clone(), 1, 5, 1, 1, BOX_ITERATIONS);
    let mut actual = source;
    box_mean(&mut actual, 1, 5, 1, 1, BOX_ITERATIONS).expect("eight iterations");
    assert_close(&actual, &expected, 8.0 * f32::EPSILON);
}

#[test]
fn ordinary_mean_keeps_one_two_and_four_channels_independent() {
    for channels in [1_u32, 2, 4] {
        let channels_usize = channels as usize;
        let mut values = Vec::new();
        for x in 0..3 {
            for channel in 0..channels_usize {
                values.push((x + 1) as f32 * (channel + 1) as f32);
            }
        }

        box_mean(&mut values, 1, 3, channels, 1, 1).expect("supported ordinary mode");
        let expected = (0..3)
            .flat_map(|x| {
                (0..channels_usize)
                    .map(move |channel| [1.5_f32, 2.0, 2.5][x] * (channel + 1) as f32)
            })
            .collect::<Vec<_>>();
        assert_eq!(values, expected);
    }
}

#[test]
fn vertical_full_lane_tail_and_circular_cache_match_the_reference() {
    // Five RGBA pixels exercise one 16-float lane plus the retained four-float
    // tail. Seven rows with radius one force the four-row circular cache to
    // wrap while original rows are being overwritten in place.
    let source = (0..5 * 7 * 4)
        .map(|index| ((index * 17 + 5) % 31) as f32 - 9.0)
        .collect::<Vec<_>>();
    let expected = repeated_reference_mean(source.clone(), 7, 5, 4, 1, 2);
    let mut actual = source;
    box_mean(&mut actual, 7, 5, 4, 1, 2).expect("lane-oriented box mean");
    assert_close(&actual, &expected, 32.0 * f32::EPSILON);
}

#[test]
fn large_mean_schedule_is_bit_deterministic_and_matches_the_reference() {
    // This shape crosses the scoped-row-worker threshold on hosts exposing at
    // least two CPUs. Row independence must make scheduling unobservable.
    let source = (0..512 * 256 * 4)
        .map(|index| (index % 251) as f32 / 251.0)
        .collect::<Vec<_>>();
    let expected = repeated_reference_mean(source.clone(), 256, 512, 4, 3, 2);
    let mut first = source.clone();
    let mut second = source;
    box_mean(&mut first, 256, 512, 4, 3, 2).expect("first scheduled mean");
    box_mean(&mut second, 256, 512, 4, 3, 2).expect("second scheduled mean");
    assert_eq!(first, second);
    assert_close(&first, &expected, 32.0 * f32::EPSILON);
}

#[test]
fn large_compensated_vertical_schedule_preserves_exact_update_order() {
    let height = 257;
    let width = 512;
    let channels = 4;
    let source = (0..height * width * channels)
        .map(|index| ((index * 19 + 7) % 1_009) as f32 - 504.0)
        .collect::<Vec<_>>();
    let expected = reference_vertical_kahan(&source, height, width, channels, 3);
    let mut actual = source;
    box_mean_vertical(
        &mut actual,
        height,
        width,
        channels as u32 | BOXFILTER_KAHAN_SUM,
        3,
    )
    .expect("scheduled compensated vertical mean");
    assert_eq!(actual, expected);
}

#[test]
fn compensated_mean_retains_low_order_terms_across_cancellation() {
    let one_channel = [
        100_000_000.0,
        1.0,
        1.0,
        1.0,
        1.0,
        1.0,
        1.0,
        1.0,
        1.0,
        -100_000_000.0,
    ];
    let mut ordinary = one_channel
        .iter()
        .flat_map(|&value| [value, value])
        .collect::<Vec<_>>();
    let mut compensated = ordinary.clone();

    box_mean(&mut ordinary, 1, 10, 2, usize::MAX, 1).expect("ordinary two-channel mean");
    box_mean(
        &mut compensated,
        1,
        10,
        2 | BOXFILTER_KAHAN_SUM,
        usize::MAX,
        1,
    )
    .expect("compensated two-channel mean");

    assert_eq!(ordinary[0], 0.0);
    assert_eq!(compensated[0], 0.8);
    assert!(
        compensated
            .iter()
            .all(|&value| value.to_bits() == compensated[0].to_bits())
    );
}

#[test]
fn compensated_horizontal_subtraction_preserves_retained_update_order() {
    // `_sub<N, true>` in src/common/box_filters.cc feeds the negated outgoing
    // sample through the same Kahan update as addition. The final zero is the
    // retained f32/correction-order result, not the exact mean of -3 and -2.
    let mut values = [-100_000_000.0, -100_000_000.0, -3.0, -2.0]
        .into_iter()
        .flat_map(|value| [value, 0.0, 0.0, 0.0])
        .collect::<Vec<_>>();

    box_mean_horizontal(&mut values, 4, 4 | BOXFILTER_KAHAN_SUM, 1, None)
        .expect("four-channel compensated row");

    let first_channel = values
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| pixel[0])
        .collect::<Vec<_>>();
    assert_eq!(
        first_channel,
        [-100_000_000.0, -66_666_668.0, -33_333_334.0, 0.0]
    );
}

#[test]
fn horizontal_kahan_supports_four_and_nine_channel_rows_and_scratch() {
    for channels in [4_u32, 9] {
        let channel_count = channels as usize;
        let mut values = (0..3)
            .flat_map(|x| (0..channel_count).map(move |channel| (10 * x + channel) as f32))
            .collect::<Vec<_>>();
        let mut scratch = vec![0.0; values.len()];

        box_mean_horizontal(
            &mut values,
            3,
            channels | BOXFILTER_KAHAN_SUM,
            1,
            Some(&mut scratch),
        )
        .expect("supported horizontal mode");

        let expected = (0..3)
            .flat_map(|x| {
                (0..channel_count).map(move |channel| [5.0_f32, 10.0, 15.0][x] + channel as f32)
            })
            .collect::<Vec<_>>();
        assert_eq!(values, expected);
    }
}

#[test]
fn horizontal_and_vertical_kahan_compose_like_the_full_filter() {
    let source = (0..24)
        .map(|index| ((index * 17 + 3) % 29) as f32 - 8.0)
        .collect::<Vec<_>>();
    let mut composed = source.clone();
    let (rows, remainder) = composed.as_chunks_mut::<12>();
    assert!(remainder.is_empty());
    for row in rows {
        box_mean_horizontal(row, 3, 4 | BOXFILTER_KAHAN_SUM, 1, None).expect("horizontal pass");
    }
    box_mean_vertical(&mut composed, 2, 3, 4 | BOXFILTER_KAHAN_SUM, 1).expect("vertical pass");

    let mut combined = source;
    box_mean(&mut combined, 2, 3, 4 | BOXFILTER_KAHAN_SUM, 1, 1).expect("combined pass");
    assert_eq!(composed, combined);
}

#[test]
fn vertical_kahan_accepts_one_through_sixteen_channels() {
    for channels in [1_u32, 9, 16] {
        let channel_count = channels as usize;
        let mut values = (0..3)
            .flat_map(|y| (0..channel_count).map(move |channel| (10 * y + channel) as f32))
            .collect::<Vec<_>>();
        box_mean_vertical(&mut values, 3, 1, channels | BOXFILTER_KAHAN_SUM, 1)
            .expect("supported vertical channel count");

        for (y, row) in values.chunks_exact(channel_count).enumerate() {
            for (channel, &actual) in row.iter().enumerate() {
                assert_eq!(actual, [5.0_f32, 10.0, 15.0][y] + channel as f32);
            }
        }
    }
}

#[test]
fn one_channel_min_and_max_match_shrinking_boxes() {
    let source = vec![9.0, 2.0, 7.0, 4.0, 5.0, 6.0, 3.0, 8.0, 1.0];
    let mut minimum = source.clone();
    box_min(&mut minimum, 3, 3, 1, 1).expect("box minimum");
    assert_eq!(minimum, [2.0, 2.0, 2.0, 2.0, 1.0, 1.0, 3.0, 1.0, 1.0]);

    let mut maximum = source;
    box_max(&mut maximum, 3, 3, 1, 1).expect("box maximum");
    assert_eq!(maximum, [9.0, 9.0, 7.0, 9.0, 9.0, 8.0, 8.0, 8.0, 8.0]);

    let mut all_min = vec![9.0, 2.0, 7.0, 4.0, 5.0, 6.0];
    box_min(&mut all_min, 2, 3, 1, usize::MAX).expect("large-radius minimum");
    assert_eq!(all_min, [2.0; 6]);

    let mut all_max = vec![9.0, 2.0, 7.0, 4.0, 5.0, 6.0];
    box_max(&mut all_max, 2, 3, 1, usize::MAX).expect("large-radius maximum");
    assert_eq!(all_max, [9.0; 6]);
}

#[test]
fn large_extrema_row_and_column_schedules_match_the_reference() {
    // Just over two worker quanta exercises scoped row and compact vertical
    // blocks on hosts exposing at least two CPUs.
    let height = 513;
    let width = 1_024;
    let source = (0..height * width)
        .map(|index| ((index * 37 + 11) % 10_007) as f32 - 5_003.0)
        .collect::<Vec<_>>();

    let mut minimum = source.clone();
    box_min(&mut minimum, height, width, 1, 2).expect("scheduled minimum");
    assert_eq!(minimum, reference_extreme(&source, height, width, 2, true));

    let mut maximum = source.clone();
    box_max(&mut maximum, height, width, 1, 2).expect("scheduled maximum");
    assert_eq!(maximum, reference_extreme(&source, height, width, 2, false));
}

#[test]
fn malformed_shapes_overflow_and_unsupported_modes_are_errors() {
    let mut empty = Vec::new();
    assert_eq!(
        box_mean(&mut empty, 0, 1, 1, 1, 1),
        Err(BoxFilterError::InvalidDimensions {
            width: 1,
            height: 0
        })
    );
    assert_eq!(
        box_mean(&mut empty, 2, usize::MAX, 4, 1, 1),
        Err(BoxFilterError::SizeOverflow)
    );

    let mut short = vec![0.0; 7];
    assert_eq!(
        box_mean(&mut short, 1, 2, 4, 1, 1),
        Err(BoxFilterError::BufferShape {
            expected: 8,
            actual: 7
        })
    );

    let mut row = vec![0.0; 8];
    assert!(matches!(
        box_mean(&mut row, 1, 2, 3, 1, 1),
        Err(BoxFilterError::UnsupportedChannels {
            operation: "box_mean",
            channels: 3
        })
    ));
    assert!(matches!(
        box_mean_horizontal(&mut row, 2, 4, 1, None),
        Err(BoxFilterError::UnsupportedChannels {
            operation: "box_mean_horizontal",
            channels: 4
        })
    ));

    let mut short_scratch = vec![0.0; 7];
    assert_eq!(
        box_mean_horizontal(
            &mut row,
            2,
            4 | BOXFILTER_KAHAN_SUM,
            1,
            Some(&mut short_scratch)
        ),
        Err(BoxFilterError::ScratchShape {
            minimum: 8,
            actual: 7
        })
    );
    assert_eq!(
        box_mean_horizontal(
            &mut row,
            2,
            4 | BOXFILTER_KAHAN_SUM,
            0,
            Some(&mut short_scratch)
        ),
        Err(BoxFilterError::ScratchShape {
            minimum: 8,
            actual: 7
        })
    );

    let mut seventeen_channels = vec![0.0; 17];
    assert!(matches!(
        box_mean_vertical(&mut seventeen_channels, 1, 1, 0x11 | BOXFILTER_KAHAN_SUM, 1),
        Err(BoxFilterError::UnsupportedChannels {
            operation: "box_mean_vertical",
            ..
        })
    ));
    assert!(matches!(
        box_min(&mut row, 1, 2, 4, 1),
        Err(BoxFilterError::UnsupportedChannels {
            operation: "box_min",
            channels: 4
        })
    ));
}

#[test]
fn non_finite_input_is_rejected_before_in_place_mutation() {
    let mut values = vec![1.0, 2.0, f32::INFINITY, 4.0];
    let original = values.clone();
    assert_eq!(
        box_mean(&mut values, 2, 2, 1, 1, 1),
        Err(BoxFilterError::NonFiniteInput { sample: 2 })
    );
    assert_eq!(values, original);

    assert_eq!(
        box_min(&mut values, 2, 2, 1, 1),
        Err(BoxFilterError::NonFiniteInput { sample: 2 })
    );
    assert_eq!(values, original);
}
