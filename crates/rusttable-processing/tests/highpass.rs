#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    reason = "source-derived tests assert native f32 arithmetic and integer boundaries"
)]

#[path = "../src/operations/highpass.rs"]
mod highpass;

use std::cell::Cell;

use highpass::{
    HIGHPASS_PARAMETER_BYTES, HIGHPASS_SCHEMA_VERSION, HighpassConfig, HighpassHistory,
    HighpassParameterError, HighpassParametersV1, HighpassPixel, HighpassPlan,
};
use rusttable_processing::RasterDimensions;
use rusttable_processing::common::box_filters::{BOX_ITERATIONS, box_mean_with_cancel};
use rusttable_processing::operations::OperationExecutionError;

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("dimensions")
}

fn fixture_bytes() -> Vec<u8> {
    let text = include_str!("fixtures/highpass-params-v1.hex");
    (0..text.trim().len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text.trim()[index..index + 2], 16).expect("hex fixture"))
        .collect()
}

#[test]
fn v1_payload_is_exact_native_order_and_unknown_history_stays_opaque() {
    let defaults = HighpassParametersV1::defaults();
    assert_eq!(HIGHPASS_PARAMETER_BYTES, 8);
    assert_eq!(highpass::HIGHPASS_PARAMETER_MINIMUM, 0.0);
    assert_eq!(highpass::HIGHPASS_PARAMETER_MAXIMUM, 100.0);
    assert_eq!(
        highpass::HIGHPASS_PARAMETER_NAMES,
        ["sharpness", "contrast"]
    );
    assert_eq!(
        highpass::HIGHPASS_SHARPNESS_TOOLTIP,
        "the sharpness of highpass filter"
    );
    assert_eq!(
        highpass::HIGHPASS_CONTRAST_TOOLTIP,
        "the contrast of highpass filter"
    );
    assert_eq!(defaults.to_bytes(), [0, 0, 0x48, 0x42, 0, 0, 0x48, 0x42]);
    assert_eq!(defaults.to_bytes().as_slice(), fixture_bytes().as_slice());
    assert_eq!(
        HighpassParametersV1::from_bytes(&defaults.to_bytes()),
        Ok(defaults)
    );

    assert_eq!(
        HighpassParametersV1::new(12.5, 87.25).to_bytes().as_slice(),
        HighpassHistory::decode(
            HIGHPASS_SCHEMA_VERSION,
            &HighpassParametersV1::new(12.5, 87.25).to_bytes(),
        )
        .expect("v1 history")
        .payload()
        .as_slice()
    );
    assert_eq!(
        HighpassParametersV1::from_bytes(&[0; 7]),
        Err(highpass::HighpassCodecError::InvalidLength {
            expected: HIGHPASS_PARAMETER_BYTES,
            actual: 7,
        })
    );

    let opaque = HighpassHistory::decode(99, &[0, 1, 2, 255]).expect("opaque history");
    assert!(matches!(
        opaque,
        HighpassHistory::Opaque {
            version: 99,
            ref bytes
        } if bytes == &[0, 1, 2, 255]
    ));
    assert_eq!(opaque.version(), 99);
    assert_eq!(opaque.payload(), vec![0, 1, 2, 255]);
    assert!(highpass::HIGHPASS_MIGRATION_EDGES.is_empty());
}

#[test]
fn finite_out_of_range_history_values_are_preserved_but_nonfinite_execution_is_rejected() {
    let parameters = HighpassParametersV1::new(125.0, -25.0);
    let config = HighpassConfig::try_from(parameters).expect("finite history remains executable");
    assert_eq!(config.sharpness().to_bits(), 125.0_f32.to_bits());
    assert_eq!(config.contrast().to_bits(), (-25.0_f32).to_bits());
    assert_eq!(
        HighpassConfig::new(f32::NAN, 50.0),
        Err(HighpassParameterError::NonFinite("sharpness"))
    );
    assert_eq!(
        HighpassConfig::new(50.0, f32::INFINITY),
        Err(HighpassParameterError::NonFinite("contrast"))
    );
}

#[test]
fn radius_uses_native_truncation_thresholds_and_scaled_ceil_cap() {
    let frame = dimensions(8, 8);
    for (sharpness, expected) in [(0.0, 0), (5.25, 1), (50.0, 8), (99.0, 16), (100.0, 16)] {
        let config = HighpassConfig::new(sharpness, 50.0).expect("config");
        assert_eq!(
            HighpassPlan::new(config, frame).expect("plan").radius(),
            expected
        );
    }

    let config = HighpassConfig::new(50.0, 50.0).expect("config");
    assert_eq!(
        HighpassPlan::new_with_scale(config, frame, 0.5, 0.3)
            .expect("ordered scaled radius")
            .radius(),
        14
    );
    assert_eq!(
        HighpassPlan::new_with_scale(config, frame, 2.0, 1.0)
            .expect("capped scaled radius")
            .radius(),
        16
    );
    assert!(HighpassPlan::new_with_scale(config, frame, 0.0, 1.0).is_err());
    assert!(HighpassPlan::new_with_scale(config, frame, 1.0, f32::INFINITY).is_err());
    assert!(
        HighpassPlan::new(
            HighpassConfig::new(-10.0, 50.0).expect("finite config"),
            frame
        )
        .is_err()
    );
}

#[test]
fn tiling_keeps_native_factors_and_radius_zero_still_overlaps_three_pixels() {
    let zero = HighpassConfig::new(-1.0, 50.0).expect("zero-radius config");
    let tiling = HighpassPlan::tiling(zero, 1.0, 1.0).expect("zero-radius tiling");
    assert_eq!(tiling.overlap, 3);
    assert_eq!(tiling.factor.to_bits(), 2.1_f32.to_bits());
    assert_eq!(tiling.factor_cl.to_bits(), 3.0_f32.to_bits());
    assert_eq!(tiling.maxbuf.to_bits(), 1.0_f32.to_bits());
    assert_eq!(tiling.overhead, 0);
    assert_eq!(tiling.align, 1);

    let radius_one = HighpassConfig::new(5.25, 50.0).expect("radius-one config");
    assert_eq!(
        HighpassPlan::tiling(radius_one, 1.0, 1.0)
            .expect("tiling")
            .overlap,
        8
    );
}

#[test]
fn cpu_leaf_matches_eight_ordinary_box_passes_and_zeroes_lab_chroma_and_fourth() {
    let dimensions = dimensions(4, 3);
    let input = [
        HighpassPixel::new(-10.0, 12.0, -23.0, 0.1),
        HighpassPixel::new(0.0, 1.0, 2.0, 0.2),
        HighpassPixel::new(20.0, 3.0, 4.0, 0.3),
        HighpassPixel::new(150.0, 5.0, 6.0, 0.4),
        HighpassPixel::new(50.0, 7.0, 8.0, 0.5),
        HighpassPixel::new(75.0, 9.0, 10.0, 0.6),
        HighpassPixel::new(100.0, 11.0, 12.0, 0.7),
        HighpassPixel::new(25.0, 13.0, 14.0, 0.8),
        HighpassPixel::new(12.5, 15.0, 16.0, 0.9),
        HighpassPixel::new(44.0, 17.0, 18.0, 1.0),
        HighpassPixel::new(88.0, 19.0, 20.0, 1.1),
        HighpassPixel::new(101.0, 21.0, 22.0, 1.2),
    ];
    let config = HighpassConfig::new(5.25, 50.0).expect("config");
    let plan = HighpassPlan::new(config, dimensions).expect("plan");
    assert_eq!(plan.radius(), 1);
    let output = plan.execute(&input, dimensions).expect("highpass");

    let mut expected_blur = input
        .iter()
        .map(|pixel| {
            let lightness = pixel.lightness();
            100.0_f32 - lightness.clamp(0.0_f32, 100.0_f32)
        })
        .collect::<Vec<_>>();
    box_mean_with_cancel(&mut expected_blur, 3, 4, 1, 1, BOX_ITERATIONS, || false)
        .expect("native box mean");
    let contrast_scale = (50.0_f32 / 100.0_f32) * 7.5_f32 * 0.5_f32;
    for (actual, (input, blurred)) in output.iter().zip(input.iter().zip(expected_blur)) {
        let expected = ((blurred + input.lightness()) - 100.0_f32) * contrast_scale + 50.0_f32;
        let expected = if expected >= 0.0 {
            if expected <= 100.0 { expected } else { 100.0 }
        } else {
            0.0
        };
        assert_eq!(actual.lightness().to_bits(), expected.to_bits());
        assert_eq!(actual.a().to_bits(), 0.0_f32.to_bits());
        assert_eq!(actual.b().to_bits(), 0.0_f32.to_bits());
        assert_eq!(actual.fourth().to_bits(), 0.0_f32.to_bits());
    }
}

#[test]
fn scalar_leaf_handles_tiny_rasters_and_records_the_gpu_fourth_channel_difference() {
    let dimensions = dimensions(1, 1);
    let input = [HighpassPixel::from_channels([50.0, 42.0, -17.0, 0.75])];
    assert_eq!(input[0].channels(), [50.0, 42.0, -17.0, 0.75]);
    let plan = HighpassPlan::new(HighpassConfig::defaults(), dimensions).expect("plan");
    assert_eq!(plan.dimensions(), dimensions);
    assert_eq!(plan.tiling_for_plan().overlap, 42);
    let output = plan
        .execute(&input, dimensions)
        .expect("one-pixel highpass");
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].fourth().to_bits(), 0.0_f32.to_bits());
    let cpu_zeroes_fourth = highpass::HIGHPASS_CPU_ZEROES_FOURTH_CHANNEL;
    let gpu_preserves_fourth = highpass::HIGHPASS_GPU_PRESERVES_FOURTH_CHANNEL;
    let gpu_executable = highpass::HIGHPASS_GPU_EXECUTABLE;
    assert!(cpu_zeroes_fourth);
    assert!(gpu_preserves_fourth);
    assert!(!gpu_executable);
    assert_eq!(highpass::HIGHPASS_GPU_PROGRAM, 4);
    assert_eq!(
        highpass::HIGHPASS_GPU_KERNELS,
        [
            "highpass_invert",
            "highpass_hblur",
            "highpass_vblur",
            "highpass_mix"
        ]
    );
}

#[test]
fn expanded_tile_uses_the_committed_radius_and_matches_a_same_shape_scalar_leaf() {
    let frame_dimensions = dimensions(5, 4);
    let tile_dimensions = dimensions(3, 2);
    let config = HighpassConfig::new(5.25, 63.0).expect("config");
    let plan = HighpassPlan::new(config, frame_dimensions).expect("frame plan");
    let tile = [
        HighpassPixel::new(1.0, 0.0, 0.0, 0.2),
        HighpassPixel::new(2.0, 0.0, 0.0, 0.2),
        HighpassPixel::new(3.0, 0.0, 0.0, 0.2),
        HighpassPixel::new(4.0, 0.0, 0.0, 0.2),
        HighpassPixel::new(5.0, 0.0, 0.0, 0.2),
        HighpassPixel::new(6.0, 0.0, 0.0, 0.2),
    ];
    let expanded = plan
        .execute_with_input_dimensions(&tile, tile_dimensions)
        .expect("expanded tile");
    let independent = HighpassPlan::new(config, tile_dimensions)
        .expect("same radius tile plan")
        .execute(&tile, tile_dimensions)
        .expect("tile scalar leaf");
    assert_eq!(expanded, independent);
}

#[test]
fn cancellation_shape_and_nonfinite_errors_publish_no_partial_output() {
    let frame_dimensions = dimensions(8, 8);
    let input = vec![HighpassPixel::new(30.0, 1.0, 2.0, 0.9); 64];
    let plan = HighpassPlan::new(
        HighpassConfig::new(100.0, 100.0).expect("config"),
        frame_dimensions,
    )
    .expect("plan");
    let polls = Cell::new(0);
    let error = plan
        .execute_with_cancel(&input, frame_dimensions, || {
            let next = polls.get() + 1;
            polls.set(next);
            next > 3
        })
        .expect_err("mid-operation cancellation");
    assert_eq!(error, OperationExecutionError::Cancelled);
    assert!(polls.get() > 3);
    assert!(
        input
            .iter()
            .all(|pixel| pixel.fourth().to_bits() == 0.9_f32.to_bits())
    );

    let wrong_shape = plan
        .execute(&input[..1], frame_dimensions)
        .expect_err("shape error");
    assert_eq!(
        wrong_shape,
        OperationExecutionError::DimensionsMismatch {
            expected: 64,
            actual: 1
        }
    );

    let mut nonfinite = input;
    nonfinite[17] = HighpassPixel::new(f32::NAN, 1.0, 2.0, 0.9);
    assert!(matches!(
        plan.execute(&nonfinite, frame_dimensions),
        Err(OperationExecutionError::NonFiniteResult { pixel: 17, .. })
    ));

    let huge = dimensions(100_000, 100_000);
    assert!(matches!(
        HighpassPlan::new(HighpassConfig::defaults(), huge),
        Err(OperationExecutionError::MemoryBudgetExceeded { .. })
    ));
}
