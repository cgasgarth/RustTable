#![allow(
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::manual_midpoint
)]
#![expect(
    clippy::suboptimal_flops,
    reason = "Native Tone Equalizer test vectors preserve source evaluation order and IEEE-754 parity."
)]

#[path = "../src/operations/toneequal/mod.rs"]
mod toneequal;

use toneequal::{
    BLENDING_DEFAULT, CHANNELS, DEFAULT_V2_FIXTURE, DetailsFilter, LUT_ENTRIES, LUT_RESOLUTION,
    LuminanceMethod, PARAMETER_BYTES, PARAMETER_VERSION, ToneEqualizerHistory,
    ToneEqualizerOutputMode, ToneEqualizerParametersV2, ToneEqualizerPixel, ToneEqualizerPlan,
    ToneEqualizerTile, ToneEqualizerTileContract,
};

const fn no_filter_parameters(method: LuminanceMethod) -> ToneEqualizerParametersV2 {
    ToneEqualizerParametersV2::from_values(
        [0.0; CHANNELS],
        0.0,
        std::f32::consts::SQRT_2,
        1.0,
        0.0,
        0.0,
        0.0,
        DetailsFilter::None,
        method,
        1,
    )
}

#[test]
fn fixture_and_native_v2_offsets_are_explicit() {
    assert!(DEFAULT_V2_FIXTURE.contains("payload_bytes=72"));
    assert!(DEFAULT_V2_FIXTURE.contains("corrected_output_alpha=scale_all_four_lanes"));
    assert!(DEFAULT_V2_FIXTURE.contains("mask_display_output_alpha=preserve_input"));
    assert!(
        !DEFAULT_V2_FIXTURE
            .lines()
            .any(|line| line == "alpha=preserve")
    );
    assert_eq!(PARAMETER_BYTES, 72);
    assert_eq!(LUT_ENTRIES, 80_001);
    assert_eq!(PARAMETER_VERSION, 2);

    let parameters = ToneEqualizerParametersV2::default();
    let bytes = parameters.to_bytes();
    assert_eq!(parameters.blending, BLENDING_DEFAULT);
    assert_eq!(
        f32::from_le_bytes(bytes[36..40].try_into().unwrap()),
        BLENDING_DEFAULT
    );
    assert_eq!(bytes.len(), PARAMETER_BYTES);
    assert_eq!(i32::from_le_bytes(bytes[60..64].try_into().unwrap()), 4);
    assert_eq!(i32::from_le_bytes(bytes[64..68].try_into().unwrap()), 4);
    assert_eq!(i32::from_le_bytes(bytes[68..72].try_into().unwrap()), 1);
    assert_eq!(
        ToneEqualizerParametersV2::from_bytes(&bytes).unwrap(),
        parameters
    );
}

#[test]
fn v1_migration_inserts_new_fields_and_reorders_enums() {
    let mut bytes = [0_u8; 64];
    for index in 0..13 {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&(index as f32 + 0.25).to_le_bytes());
    }
    bytes[52..56].copy_from_slice(&4_i32.to_le_bytes());
    bytes[56..60].copy_from_slice(&3_i32.to_le_bytes());
    bytes[60..64].copy_from_slice(&6_i32.to_le_bytes());

    let history = ToneEqualizerHistory::decode(1, &bytes).unwrap();
    let current = history.current();
    assert_eq!(history.version(), 2);
    assert_eq!(current.blending, 9.25);
    assert_eq!(current.feathering, 10.25);
    assert_eq!(current.contrast_boost, 11.25);
    assert_eq!(current.exposure_boost, 12.25);
    assert_eq!(current.smoothing, std::f32::consts::SQRT_2);
    assert_eq!(current.quantization, 0.0);
    assert_eq!(current.details, DetailsFilter::Eigf);
    assert_eq!(current.method, LuminanceMethod::Geomean);
    assert_eq!(current.iterations, 3);
}

#[test]
fn constant_zero_ev_curve_is_identity() {
    let plan = ToneEqualizerPlan::new(no_filter_parameters(LuminanceMethod::Norm2)).unwrap();
    assert!(plan.factors().iter().all(|factor| factor.is_finite()));
    let min = plan
        .correction_lut()
        .iter()
        .copied()
        .fold(f32::INFINITY, f32::min);
    let max = plan
        .correction_lut()
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(min >= 0.99 && max <= 1.02, "lut range={min}..{max}");
}

#[test]
fn native_luminance_methods_match_mask_equations() {
    let input = [ToneEqualizerPixel::new(0.1, 0.2, 0.4, 0.37)];
    let expected = [
        (LuminanceMethod::Mean, (0.1 + 0.2 + 0.4) / 3.0),
        (LuminanceMethod::Lightness, (0.4 + 0.1) / 2.0),
        (LuminanceMethod::Value, 0.4),
        (LuminanceMethod::Norm1, 0.7),
        (
            LuminanceMethod::Norm2,
            (0.1_f32 * 0.1 + 0.2 * 0.2 + 0.4 * 0.4).sqrt(),
        ),
        (
            LuminanceMethod::NormPower,
            (0.1_f32.powi(3) + 0.2_f32.powi(3) + 0.4_f32.powi(3)) / 0.21,
        ),
        (LuminanceMethod::Geomean, 0.2),
    ];
    for (method, luminance) in expected {
        let plan = ToneEqualizerPlan::new(no_filter_parameters(method)).unwrap();
        let result = plan
            .execute_with_cancel(
                &input,
                1,
                1,
                1.0,
                ToneEqualizerOutputMode::LuminanceMask,
                || false,
            )
            .unwrap();
        let mask = result.pixels[0].channels()[0];
        let recovered = mask * mask * 0.996_093_75 + 0.003_906_25;
        assert!((recovered - luminance).abs() < 2.0e-6, "{method:?}");
        assert_eq!(result.pixels[0].channels()[3], 0.37);
    }
}

#[test]
fn guided_and_eigf_use_source_leaf_not_identity_fallback() {
    let input: Vec<_> = (0..16)
        .map(|index| {
            if index < 8 {
                ToneEqualizerPixel::new(0.25, 0.5, 1.0, 0.8)
            } else {
                ToneEqualizerPixel::new(1.0, 0.5, 0.25, 0.8)
            }
        })
        .collect();
    for details in [DetailsFilter::Guided, DetailsFilter::Eigf] {
        let parameters = ToneEqualizerParametersV2::from_values(
            [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5],
            25.0,
            std::f32::consts::SQRT_2,
            1.0,
            0.0,
            1.0,
            0.0,
            details,
            LuminanceMethod::Norm2,
            1,
        );
        let plan = ToneEqualizerPlan::new(parameters).unwrap();
        let result = plan
            .execute_with_cancel(
                &input,
                4,
                4,
                1.0,
                ToneEqualizerOutputMode::Corrected,
                || false,
            )
            .unwrap();
        let mut correction_changed_alpha = false;
        for (output, source) in result.pixels.iter().zip(input.iter()) {
            let output_channels = output.channels();
            let source_channels = source.channels();
            let correction = output_channels[0] / source_channels[0];
            assert!((output_channels[3] - source_channels[3] * correction).abs() < 1.0e-6);
            correction_changed_alpha |= (output_channels[3] - source_channels[3]).abs() > 1.0e-4;
        }
        assert!(correction_changed_alpha);
    }
}

#[test]
fn correction_scales_non_unit_alpha_across_native_four_lanes() {
    let parameters = ToneEqualizerParametersV2::from_values(
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        5.0,
        std::f32::consts::SQRT_2,
        1.0,
        0.0,
        0.0,
        0.0,
        DetailsFilter::None,
        LuminanceMethod::Mean,
        1,
    );
    let input = [ToneEqualizerPixel::new(0.25, 0.25, 0.25, 0.42)];
    let plan = ToneEqualizerPlan::new(parameters).unwrap();
    let result = plan
        .execute_with_cancel(
            &input,
            1,
            1,
            1.0,
            ToneEqualizerOutputMode::Corrected,
            || false,
        )
        .unwrap();

    let lut_index = 6 * LUT_RESOLUTION;
    let correction = plan.correction_lut()[lut_index];
    assert!((correction - 1.0).abs() > 0.1);
    assert_eq!(
        result.pixels[0].channels(),
        [
            0.25 * correction,
            0.25 * correction,
            0.25 * correction,
            0.42 * correction
        ]
    );
}

#[test]
fn mask_display_preserves_alpha_and_uses_native_gamma() {
    let input = [ToneEqualizerPixel::new(0.25, 0.25, 0.25, 0.42)];
    let plan = ToneEqualizerPlan::new(no_filter_parameters(LuminanceMethod::Mean)).unwrap();
    let result = plan
        .execute_with_cancel(
            &input,
            1,
            1,
            1.0,
            ToneEqualizerOutputMode::LuminanceMask,
            || false,
        )
        .unwrap();
    let expected = ((0.25_f32 - 0.003_906_25) / 0.996_093_75).sqrt();
    assert!((result.pixels[0].channels()[0] - expected).abs() < 1.0e-6);
    assert_eq!(result.pixels[0].channels()[3], 0.42);
}

#[test]
fn cancellation_and_tile_contract_never_publish_partial_output() {
    let input = vec![ToneEqualizerPixel::new(0.25, 0.5, 1.0, 1.0); 16];
    let plan = ToneEqualizerPlan::new(no_filter_parameters(LuminanceMethod::Norm2)).unwrap();
    let mut polls = 0;
    let cancelled = plan.execute_with_cancel(
        &input,
        4,
        4,
        1.0,
        ToneEqualizerOutputMode::Corrected,
        || {
            polls += 1;
            polls > 1
        },
    );
    assert!(matches!(
        cancelled,
        Err(toneequal::ToneEqualizerExecutionError::Cancelled)
    ));
    assert_eq!(
        plan.tile_contract(),
        ToneEqualizerTileContract::WholeRasterOnly
    );
    let invalid = plan.execute_tiles_with_cancel(
        &input,
        4,
        4,
        &[
            ToneEqualizerTile::new(0, 0, 2, 4, 0),
            ToneEqualizerTile::new(2, 0, 2, 4, 0),
        ],
        1.0,
        ToneEqualizerOutputMode::Corrected,
        || false,
    );
    assert!(matches!(
        invalid,
        Err(toneequal::ToneEqualizerExecutionError::InvalidTile)
    ));
}

#[test]
fn radius_preserves_modify_roi_in_scaling_equation() {
    let parameters = ToneEqualizerParametersV2::from_values(
        [0.0; CHANNELS],
        5.0,
        std::f32::consts::SQRT_2,
        1.0,
        0.0,
        0.0,
        0.0,
        DetailsFilter::Eigf,
        LuminanceMethod::Norm2,
        1,
    );
    let plan = ToneEqualizerPlan::new(parameters).unwrap();
    assert_eq!(plan.radius_for(100, 80, 1.0), 2);
    assert_eq!(plan.radius_for(100, 80, 0.5), 0);
}
