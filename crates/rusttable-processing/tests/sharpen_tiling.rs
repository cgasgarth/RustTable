//! Source-derived tests for `src/iop/sharpen.c` dynamic radius and tiling.

#![allow(clippy::cast_possible_truncation)]

use rusttable_processing::operations::sharpen::tiling::{
    SHARPEN_CPU_MEMORY_FACTOR, SHARPEN_MAX_BUFFER_FACTOR, SHARPEN_MAX_RADIUS,
    SHARPEN_RADIUS_MULTIPLIER, SHARPEN_TILE_ALIGNMENT, SHARPEN_TILING_OVERHEAD_BYTES, SharpenRoi,
    SharpenTilingError, SharpenTilingPlan,
};

fn assert_f32_bits(actual: f32, expected: f32) {
    assert_eq!(actual.to_bits(), expected.to_bits());
}

#[test]
fn native_radius_commit_quantization_and_cap_are_preserved() {
    let zero = SharpenTilingPlan::new(0.0, 1.0, 1.0).expect("zero radius");
    assert_f32_bits(zero.parameter_radius(), 0.0);
    assert_f32_bits(zero.committed_radius(), 0.0);
    assert_eq!(zero.radius(), 0);

    let subpixel = SharpenTilingPlan::new(0.01, 1.0, 1.0).expect("subpixel radius");
    assert_eq!(subpixel.radius(), 1);
    let exact = SharpenTilingPlan::new(0.4, 1.0, 1.0).expect("exact radius");
    assert_f32_bits(exact.committed_radius(), 1.0);
    assert_eq!(exact.radius(), 1);
    let rounded_up = SharpenTilingPlan::new(0.400_1, 1.0, 1.0).expect("fractional radius");
    assert_eq!(rounded_up.radius(), 2);

    let default = SharpenTilingPlan::new(2.0, 1.0, 1.0).expect("native default");
    assert_f32_bits(default.committed_radius(), 5.0);
    assert_eq!(default.radius(), 5);
    assert_eq!(default.kernel_width(), 11);

    let capped = SharpenTilingPlan::new(99.0, 1.0, 1.0).expect("native maximum");
    assert_f32_bits(capped.committed_radius(), 99.0 * SHARPEN_RADIUS_MULTIPLIER);
    assert_eq!(capped.radius(), SHARPEN_MAX_RADIUS);

    let overflowed = SharpenTilingPlan::new(f32::MAX, 1.0, 1.0)
        .expect("finite parameter may overflow during native commit");
    assert!(overflowed.committed_radius().is_infinite());
    assert!(overflowed.committed_radius().is_sign_positive());
    assert_eq!(overflowed.radius(), SHARPEN_MAX_RADIUS);
    let committed_overflow = SharpenTilingPlan::from_committed_radius(f32::INFINITY, 1.0, 1.0)
        .expect("positive committed infinity reaches native cap");
    assert_eq!(committed_overflow.radius(), SHARPEN_MAX_RADIUS);
}

#[test]
fn roi_in_scale_and_piece_iscale_resolve_the_dynamic_overlap() {
    let downscaled = SharpenTilingPlan::new(2.0, 0.5, 1.0).expect("downscaled ROI");
    assert_f32_bits(downscaled.roi_scale(), 0.5);
    assert_f32_bits(downscaled.input_scale(), 1.0);
    assert_eq!(downscaled.radius(), 3);

    let piece_downscaled = SharpenTilingPlan::new(2.0, 1.0, 2.0).expect("piece scale");
    assert_eq!(piece_downscaled.radius(), 3);

    let upscaled = SharpenTilingPlan::new(2.0, 2.0, 1.0).expect("upscaled ROI");
    assert_eq!(upscaled.radius(), 10);

    let committed =
        SharpenTilingPlan::from_committed_radius(5.0, 0.25, 2.0).expect("committed radius seam");
    assert_eq!(committed.radius(), 1);

    assert_eq!(
        SharpenTilingPlan::new(2.0, 0.0, 1.0),
        Err(SharpenTilingError::InvalidRoiScale)
    );
    assert_eq!(
        SharpenTilingPlan::new(2.0, 1.0, f32::NAN),
        Err(SharpenTilingError::InvalidInputScale)
    );
    assert_eq!(
        SharpenTilingPlan::new(f32::INFINITY, 1.0, 1.0),
        Err(SharpenTilingError::NonFiniteRadius)
    );
    let finite_outlier = SharpenTilingPlan::new(100.0, 1.0, 1.0).expect("native radius cap");
    assert_eq!(finite_outlier.radius(), SHARPEN_MAX_RADIUS);
    assert_eq!(
        SharpenTilingPlan::new(-0.1, 1.0, 1.0),
        Err(SharpenTilingError::RadiusOutOfRange)
    );
}

#[test]
fn cpu_tiling_contract_uses_native_factor_overlap_and_alignment() {
    let plan = SharpenTilingPlan::new(2.0, 1.0, 1.0).expect("tiling plan");
    assert_f32_bits(SHARPEN_CPU_MEMORY_FACTOR, 2.1);
    assert_f32_bits(SHARPEN_MAX_BUFFER_FACTOR, 1.0);
    assert_eq!(SHARPEN_TILING_OVERHEAD_BYTES, 0);
    assert_eq!(plan.overlap(), 5);
    assert_eq!(SHARPEN_TILE_ALIGNMENT, 1);
}

#[test]
fn required_input_expands_each_border_and_clips_to_the_image() {
    let plan = SharpenTilingPlan::new(2.0, 1.0, 1.0).expect("radius five");

    let interior = plan
        .tile(20, 12, SharpenRoi::new(8, 4, 3, 2).unwrap())
        .expect("interior tile");
    assert_eq!(interior.input(), SharpenRoi::new(3, 0, 13, 11).unwrap());
    assert_eq!((interior.crop_x(), interior.crop_y()), (5, 4));
    assert_eq!(interior.output(), SharpenRoi::new(8, 4, 3, 2).unwrap());

    let top_left = plan
        .tile(20, 12, SharpenRoi::new(0, 0, 4, 3).unwrap())
        .expect("top-left tile");
    assert_eq!(top_left.input(), SharpenRoi::new(0, 0, 9, 8).unwrap());
    assert_eq!((top_left.crop_x(), top_left.crop_y()), (0, 0));
    assert!(top_left.is_identity(plan));

    let bottom_right = plan
        .tile(20, 12, SharpenRoi::new(17, 10, 3, 2).unwrap())
        .expect("bottom-right tile");
    assert_eq!(bottom_right.input(), SharpenRoi::new(12, 5, 8, 7).unwrap());
    assert_eq!((bottom_right.crop_x(), bottom_right.crop_y()), (5, 5));

    assert_eq!(
        plan.tile(20, 12, SharpenRoi::new(19, 0, 2, 1).unwrap()),
        Err(SharpenTilingError::RoiOutsideImage)
    );
    assert_eq!(
        SharpenRoi::new(0, 0, 0, 1),
        Err(SharpenTilingError::EmptyRoi)
    );
}

#[test]
fn zero_radius_and_images_below_the_kernel_are_identity() {
    let zero = SharpenTilingPlan::new(0.0, 1.0, 1.0).expect("zero radius");
    assert!(zero.is_identity_for(100, 100));

    let plan = SharpenTilingPlan::new(2.0, 1.0, 1.0).expect("radius five");
    assert!(plan.is_identity_for(10, 11));
    assert!(plan.is_identity_for(11, 10));
    assert!(!plan.is_identity_for(11, 11));
    assert_eq!(
        plan.tile(0, 11, SharpenRoi::new(0, 0, 1, 1).unwrap()),
        Err(SharpenTilingError::InvalidImageDimensions)
    );
}

#[test]
fn overlapped_tiles_match_full_frame_source_equations() {
    const WIDTH: u32 = 23;
    const HEIGHT: u32 = 17;
    let input = patterned_lab(WIDTH, HEIGHT);
    let plan = SharpenTilingPlan::new(1.6, 0.75, 1.25).expect("scaled radius");
    assert_eq!(plan.radius(), 3);

    let settings = (0.7, 0.025);
    let full = sharpen_reference(&input, WIDTH, HEIGHT, plan, settings);
    let tiled = sharpen_tiled_reference(&input, WIDTH, HEIGHT, plan, 4, 3, settings);

    assert_eq!(tiled.len(), full.len());
    for (index, (actual, expected)) in tiled.iter().zip(&full).enumerate() {
        for channel in 0..4 {
            assert!(
                (actual[channel] - expected[channel]).abs() <= 2.0e-6,
                "pixel {index} channel {channel}: {} != {}",
                actual[channel],
                expected[channel]
            );
        }
    }
}

fn patterned_lab(width: u32, height: u32) -> Vec<[f32; 4]> {
    (0..width * height)
        .map(|index| {
            let x = f32::from(u16::try_from(index % width).expect("test x fits"));
            let y = f32::from(u16::try_from(index / width).expect("test y fits"));
            let checker = if (index + index / width).is_multiple_of(3) {
                0.18
            } else {
                -0.07
            };
            [
                0.3 + x * 0.011 + y * 0.007 + checker,
                -0.2 + x * 0.009,
                0.15 - y * 0.006,
                0.2 + f32::from(u8::try_from(index % 9).expect("alpha step fits")) * 0.08,
            ]
        })
        .collect()
}

fn sharpen_tiled_reference(
    input: &[[f32; 4]],
    width: u32,
    height: u32,
    plan: SharpenTilingPlan,
    tile_width: u32,
    tile_height: u32,
    settings: (f32, f32),
) -> Vec<[f32; 4]> {
    let mut output = input.to_vec();
    for y in (0..height).step_by(tile_height as usize) {
        for x in (0..width).step_by(tile_width as usize) {
            let output_roi =
                SharpenRoi::new(x, y, tile_width.min(width - x), tile_height.min(height - y))
                    .unwrap();
            let tile = plan.tile(width, height, output_roi).unwrap();
            let input_roi = tile.input();
            let mut tile_pixels =
                Vec::with_capacity((input_roi.width() * input_roi.height()) as usize);
            for local_y in 0..input_roi.height() {
                let source_start = ((input_roi.y() + local_y) * width + input_roi.x()) as usize;
                let source_end = source_start + input_roi.width() as usize;
                tile_pixels.extend_from_slice(&input[source_start..source_end]);
            }
            let tile_output = sharpen_reference(
                &tile_pixels,
                input_roi.width(),
                input_roi.height(),
                plan,
                settings,
            );
            for local_y in 0..output_roi.height() {
                for local_x in 0..output_roi.width() {
                    let tile_index = ((tile.crop_y() + local_y) * input_roi.width()
                        + tile.crop_x()
                        + local_x) as usize;
                    let output_index =
                        ((output_roi.y() + local_y) * width + output_roi.x() + local_x) as usize;
                    output[output_index] = tile_output[tile_index];
                }
            }
        }
    }
    output
}

fn sharpen_reference(
    input: &[[f32; 4]],
    width: u32,
    height: u32,
    plan: SharpenTilingPlan,
    settings: (f32, f32),
) -> Vec<[f32; 4]> {
    let (amount, threshold) = settings;
    assert_eq!(input.len(), (width * height) as usize);
    if plan.is_identity_for(width, height) {
        return input.to_vec();
    }

    let radius = plan.radius();
    let resolved = plan.committed_radius() * plan.roi_scale() / plan.input_scale();
    let resolved_f64 = f64::from(resolved);
    let sigma2 = ((1.0_f64 / (2.5_f64 * 2.5_f64)) * resolved_f64 * resolved_f64) as f32;
    let mut kernel = (0..=radius * 2)
        .map(|offset| {
            let offset = i16::try_from(offset).expect("native radius is capped");
            let radius = i16::try_from(radius).expect("native radius is capped");
            let distance = f32::from(offset - radius);
            (-(distance * distance) / (2.0 * sigma2)).exp()
        })
        .collect::<Vec<_>>();
    let weight = kernel.iter().sum::<f32>();
    for value in &mut kernel {
        *value /= weight;
    }

    let mut output = input.to_vec();
    for y in radius..height - radius {
        for x in radius..width - radius {
            let mut blurred = 0.0;
            for kernel_y in 0..=radius * 2 {
                for kernel_x in 0..=radius * 2 {
                    let source_x = x + kernel_x - radius;
                    let source_y = y + kernel_y - radius;
                    let source = input[(source_y * width + source_x) as usize][0];
                    blurred += kernel[kernel_y as usize] * kernel[kernel_x as usize] * source;
                }
            }
            let index = (y * width + x) as usize;
            let difference = input[index][0] - blurred;
            let detail = if difference.abs() > threshold {
                difference.signum() * (difference.abs() - threshold).max(0.0)
            } else {
                0.0
            };
            output[index][0] = input[index][0] + detail * amount;
        }
    }
    output
}
