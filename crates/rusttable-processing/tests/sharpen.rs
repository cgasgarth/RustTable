//! Source-derived tests for the production CPU Sharpen operation.
//!
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::float_cmp
)]

use rusttable_processing::RasterDimensions;
use rusttable_processing::operations::sharpen::source_map::{
    SHARPEN_SOURCE_MAP, SharpenPortStatus,
};
use rusttable_processing::operations::sharpen::{
    DEFAULT_V1_FIXTURE, SHARPEN_CHANNELS, SHARPEN_COMPATIBILITY_ID, SHARPEN_DEFAULT_AMOUNT,
    SHARPEN_DEFAULT_RADIUS, SHARPEN_DEFAULT_THRESHOLD, SHARPEN_INPUT_ENCODING, SHARPEN_MAXR,
    SHARPEN_PARAMETER_BYTES, SHARPEN_SCHEMA_VERSION, SharpenConfig, SharpenHistory,
    SharpenParameterError, SharpenParametersV1, SharpenPixel, SharpenPlan, sharpen_descriptor,
};
use rusttable_processing::operations::{OperationExecutionError, ReconstructionBudget};

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("test dimensions are nonzero")
}

fn hex_digit(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        b'A'..=b'F' => value - b'A' + 10,
        _ => panic!("invalid Sharpen fixture hex digit"),
    }
}

fn default_fixture_bytes() -> [u8; SHARPEN_PARAMETER_BYTES] {
    let encoded = DEFAULT_V1_FIXTURE.trim().as_bytes();
    assert_eq!(encoded.len(), SHARPEN_PARAMETER_BYTES * 2);
    let mut bytes = [0_u8; SHARPEN_PARAMETER_BYTES];
    let (pairs, remainder) = encoded.as_chunks::<2>();
    assert!(remainder.is_empty());
    for (index, pair) in pairs.iter().enumerate() {
        bytes[index] = (hex_digit(pair[0]) << 4) | hex_digit(pair[1]);
    }
    bytes
}

fn fixture(width: u32, height: u32) -> Vec<SharpenPixel> {
    let mut pixels = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let lightness = if x == width / 2 && y == height / 2 {
                50.0
            } else {
                (x as f32 * 3.0) + (y as f32 * 2.0)
            };
            pixels.push(SharpenPixel::new(
                lightness,
                -40.0 + x as f32,
                20.0 - y as f32,
                0.15 + ((x + y) as f32 * 0.01),
            ));
        }
    }
    pixels
}

fn plan(width: u32, height: u32, radius: f32, amount: f32, threshold: f32) -> SharpenPlan {
    SharpenPlan::new(
        SharpenConfig::new(radius, amount, threshold).expect("valid sharpen parameters"),
        dimensions(width, height),
        1.0,
        1.0,
    )
    .expect("valid sharpen plan")
}

#[test]
fn native_abi_and_default_fixture_are_exactly_represented() {
    assert_eq!(
        std::mem::size_of::<SharpenParametersV1>(),
        SHARPEN_PARAMETER_BYTES
    );
    assert_eq!(
        std::mem::align_of::<SharpenParametersV1>(),
        std::mem::align_of::<f32>()
    );
    let bytes = default_fixture_bytes();
    let parameters = SharpenParametersV1::from_bytes(&bytes).unwrap();
    assert_eq!(parameters, SharpenParametersV1::defaults());
    assert_eq!(parameters.to_bytes(), bytes);
}

#[test]
fn descriptor_uses_native_cpu_memory_factor() {
    let descriptor = sharpen_descriptor();
    assert_eq!(descriptor.tiling.temporary_multiplier_milli, 2100);
}

#[test]
fn source_map_marks_only_the_cpu_leaf_as_ported() {
    assert!(SHARPEN_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.starts_with("process /") && entry.status == SharpenPortStatus::Ported
    }));
    assert!(SHARPEN_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("sharpen_hblur")
            && entry.status == SharpenPortStatus::ExplicitlyDeferred
    }));
    assert!(SHARPEN_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("history dispatch")
            && entry.status == SharpenPortStatus::ExplicitlyDeferred
    }));
    assert!(SHARPEN_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("dt_iop_alloc_image_buffers")
            && entry.status == SharpenPortStatus::ExistingDependency
    }));
}

#[test]
fn v1_codec_preserves_native_abi_order_and_bytes() {
    let parameters = SharpenParametersV1::new(1.25, 0.75, 12.5);
    let mut expected = Vec::new();
    expected.extend_from_slice(&1.25f32.to_le_bytes());
    expected.extend_from_slice(&0.75f32.to_le_bytes());
    expected.extend_from_slice(&12.5f32.to_le_bytes());
    assert_eq!(SHARPEN_PARAMETER_BYTES, 3 * std::mem::size_of::<f32>());
    assert_eq!(parameters.to_bytes(), expected.as_slice());
    assert_eq!(
        SharpenParametersV1::from_bytes(&expected).unwrap(),
        parameters
    );

    let history = SharpenHistory::decode(SHARPEN_SCHEMA_VERSION, &expected).unwrap();
    assert_eq!(history.version(), SHARPEN_SCHEMA_VERSION);
    assert_eq!(history.payload(), expected);
    assert_eq!(history.current().unwrap(), parameters);

    let opaque_bytes = [0x11, 0x22, 0x33];
    let opaque = SharpenHistory::decode(99, &opaque_bytes).unwrap();
    assert_eq!(opaque.version(), 99);
    assert_eq!(opaque.payload(), opaque_bytes);
    assert!(opaque.current().is_err());
}

#[test]
fn defaults_ranges_and_commit_match_native_metadata() {
    assert_eq!(SHARPEN_COMPATIBILITY_ID, "sharpen");
    let config = SharpenConfig::defaults();
    assert_eq!(config.parameters(), SharpenParametersV1::defaults());
    assert_eq!(config.radius(), SHARPEN_DEFAULT_RADIUS);
    assert_eq!(config.amount(), SHARPEN_DEFAULT_AMOUNT);
    assert_eq!(config.threshold(), SHARPEN_DEFAULT_THRESHOLD);
    assert_eq!(config.commit().radius(), 5.0);
    assert_eq!(config.commit().amount(), SHARPEN_DEFAULT_AMOUNT);
    assert_eq!(config.commit().threshold(), SHARPEN_DEFAULT_THRESHOLD);

    // The `$MIN`/`$MAX` values are native UI metadata. `commit_params` copies
    // finite persisted values without clamping them, while the radius planner
    // still rejects a negative radius before it can become an invalid integer.
    assert!(SharpenConfig::new(-0.001, 2.001, 100.001).is_ok());
    assert!(matches!(
        SharpenConfig::new(f32::NAN, 0.5, 0.5),
        Err(SharpenParameterError::NonFinite("radius"))
    ));
    assert!(matches!(
        SharpenConfig::new(2.0, f32::INFINITY, 0.5),
        Err(SharpenParameterError::NonFinite("amount"))
    ));
}

#[test]
fn malformed_and_nonfinite_payloads_are_rejected() {
    assert!(matches!(
        SharpenParametersV1::from_bytes(&[0; SHARPEN_PARAMETER_BYTES - 1]),
        Err(rusttable_processing::operations::sharpen::SharpenCodecError::InvalidLength { .. })
    ));
    let mut nonfinite = SharpenParametersV1::defaults().to_bytes();
    nonfinite[4..8].copy_from_slice(&f32::INFINITY.to_le_bytes());
    assert!(matches!(
        SharpenParametersV1::from_bytes(&nonfinite),
        Err(
            rusttable_processing::operations::sharpen::SharpenCodecError::Parameters(
                SharpenParameterError::NonFinite("amount")
            )
        )
    ));

    // Native history has no commit-time range clamp; finite outliers remain
    // byte-faithful until an importing/execution seam decides whether to run.
    let outlier = SharpenParametersV1::new(-1.0, 3.0, 120.0);
    assert_eq!(
        SharpenParametersV1::from_bytes(&outlier.to_bytes()).unwrap(),
        outlier
    );
}

#[test]
fn lab_four_channel_boundary_is_explicit() {
    assert_eq!(SHARPEN_CHANNELS, 4);
    assert_eq!(SHARPEN_INPUT_ENCODING, "Lab D50");
    let pixel = SharpenPixel::new(50.0, -2.0, 3.0, 0.25);
    assert_eq!(pixel.channels(), [50.0, -2.0, 3.0, 0.25]);
}

#[test]
fn radius_scaling_uses_native_ceil_and_maxr_quantization() {
    let dimensions = dimensions(64, 64);
    let config = SharpenConfig::new(1.0, 0.5, 0.5).unwrap();
    let half_scale = SharpenPlan::new(config, dimensions, 0.5, 1.0).unwrap();
    assert_eq!(half_scale.dimensions(), dimensions);
    assert_eq!(half_scale.committed().radius(), 2.5);
    assert_eq!(half_scale.effective_radius(), 1.25);
    assert_eq!(half_scale.radius(), 2);

    let full_scale = SharpenPlan::new(config, dimensions, 1.0, 1.0).unwrap();
    assert_eq!(full_scale.radius(), 3);
    let capped = SharpenPlan::new(
        SharpenConfig::new(99.0, 0.5, 0.5).unwrap(),
        dimensions,
        1.0,
        1.0,
    )
    .unwrap();
    assert_eq!(capped.radius(), SHARPEN_MAXR);

    let overflowed_config = SharpenConfig::new(f32::MAX, 0.5, 0.5).unwrap();
    assert!(overflowed_config.commit().radius().is_infinite());
    assert!(overflowed_config.commit().radius().is_sign_positive());
    let overflowed = SharpenPlan::new(overflowed_config, dimensions, 1.0, 1.0)
        .expect("finite radius commit overflow reaches native MAXR cap");
    assert_eq!(overflowed.radius(), SHARPEN_MAXR);

    let weights = full_scale.gaussian_weights();
    assert_eq!(weights.len() % 4, 0);
    let active = &weights[..=2 * full_scale.radius() as usize];
    assert!((active.iter().sum::<f32>() - 1.0).abs() < 1.0e-6);
    assert!(
        weights[active.len()..]
            .iter()
            .all(|value| value.to_bits() == 0)
    );
}

#[test]
fn gaussian_sigma_uses_native_double_intermediates_and_assignment_rounding() {
    let operation = plan(11, 11, 2.0, 0.5, 0.5);
    assert_eq!(operation.effective_radius().to_bits(), 5.0_f32.to_bits());
    assert_eq!(operation.radius(), 5);

    let effective_radius = f64::from(operation.effective_radius());
    let native_sigma2 =
        ((1.0_f64 / (2.5_f64 * 2.5_f64)) * effective_radius * effective_radius) as f32;
    let f32_only_sigma2 = (1.0_f32 / (2.5_f32 * 2.5_f32))
        * operation.effective_radius()
        * operation.effective_radius();
    assert_eq!(native_sigma2.to_bits(), 4.0_f32.to_bits());
    assert_ne!(native_sigma2.to_bits(), f32_only_sigma2.to_bits());

    let radius = i32::try_from(operation.radius()).expect("bounded native radius");
    let mut expected = Vec::new();
    let mut weight = 0.0_f32;
    for offset in -radius..=radius {
        let offset = offset as f32;
        let value = (-(offset * offset) / (2.0_f32 * native_sigma2)).exp();
        expected.push(value);
        weight += value;
    }
    for value in &mut expected {
        *value /= weight;
    }

    let actual = operation.gaussian_weights();
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "native Gaussian weight {index}"
        );
    }
}

#[test]
fn radius_zero_and_undersized_images_are_identity() {
    let input = fixture(5, 5);
    let zero = plan(5, 5, 0.0, 2.0, 0.0);
    assert_eq!(zero.radius(), 0);
    assert_eq!(zero.gaussian_weights(), [1.0, 0.0, 0.0, 0.0]);
    assert_eq!(zero.execute(&input).unwrap(), input);
    let identity_bytes = input.len() * std::mem::size_of::<SharpenPixel>();
    let tightly_budgeted = SharpenPlan::new_with_budget(
        SharpenConfig::new(0.0, 2.0, 0.0).unwrap(),
        dimensions(5, 5),
        1.0,
        1.0,
        ReconstructionBudget::new(identity_bytes),
    )
    .expect("identity only allocates the copied output");
    assert_eq!(tightly_budgeted.execute(&input).unwrap(), input);

    // radius=1 commits to 2.5 and quantizes to 3; a 6-pixel edge is too small.
    let undersized = plan(6, 8, 1.0, 2.0, 0.0);
    assert_eq!(undersized.radius(), 3);
    assert_eq!(undersized.execute(&fixture(6, 8)).unwrap(), fixture(6, 8));
}

#[test]
fn borders_are_copied_and_interior_uses_scalar_usm() {
    let width = 11;
    let height = 11;
    let input = fixture(width, height);
    let sharpened = plan(width, height, 1.0, 1.0, 0.0)
        .execute(&input)
        .expect("sharpened fixture");
    let radius = 3usize;
    for y in 0..height as usize {
        for x in 0..width as usize {
            let index = y * width as usize + x;
            if y < radius
                || y >= height as usize - radius
                || x < radius
                || x >= width as usize - radius
            {
                assert_eq!(sharpened[index], input[index], "border pixel {x},{y}");
            }
        }
    }
    assert_ne!(
        sharpened[(height as usize / 2) * width as usize + width as usize / 2].lightness(),
        input[(height as usize / 2) * width as usize + width as usize / 2].lightness()
    );
}

#[test]
fn threshold_and_amount_follow_native_usm_equation() {
    let width = 11;
    let height = 11;
    let input = vec![SharpenPixel::new(0.0, 1.0, 2.0, 0.75); (width * height) as usize];
    let center = (height / 2 * width + width / 2) as usize;
    let mut impulse = input.clone();
    impulse[center] = SharpenPixel::new(50.0, 1.0, 2.0, 0.75);

    let no_amount = plan(width, height, 1.0, 0.0, 0.0)
        .execute(&impulse)
        .unwrap();
    assert_eq!(no_amount, impulse);

    let thresholded = plan(width, height, 1.0, 2.0, 100.0)
        .execute(&impulse)
        .unwrap();
    assert_eq!(thresholded, impulse);

    let operation = plan(width, height, 1.0, 1.0, 0.0);
    let sharpened = operation.execute(&impulse).unwrap();
    let center_weight = operation.gaussian_weights()[operation.radius() as usize];
    let blurred_center = center_weight * (center_weight * impulse[center].lightness());
    let expected_center =
        impulse[center].lightness() + (impulse[center].lightness() - blurred_center);
    assert_eq!(
        sharpened[center].lightness().to_bits(),
        expected_center.to_bits()
    );
    assert!(sharpened[center].lightness() > impulse[center].lightness());
}

#[test]
fn a_b_and_alpha_are_preserved_bit_exactly() {
    let input = fixture(11, 11);
    let output = plan(11, 11, 1.0, 2.0, 0.0).execute(&input).unwrap();
    for (source, processed) in input.iter().zip(output) {
        assert_eq!(source.a().to_bits(), processed.a().to_bits());
        assert_eq!(source.b().to_bits(), processed.b().to_bits());
        assert_eq!(source.alpha().to_bits(), processed.alpha().to_bits());
    }
}

#[test]
fn convolution_reads_only_lightness_and_preserves_chroma_alpha() {
    let input = fixture(11, 11);
    let mut changed_non_luma = input.clone();
    for (index, pixel) in changed_non_luma.iter_mut().enumerate() {
        *pixel = SharpenPixel::new(
            pixel.lightness(),
            1000.0 + index as f32,
            -1000.0 - index as f32,
            0.01 + index as f32,
        );
    }
    let operation = plan(11, 11, 1.0, 1.25, 0.0);
    let original_output = operation.execute(&input).unwrap();
    let changed_output = operation.execute(&changed_non_luma).unwrap();
    for ((original, changed), source) in original_output
        .iter()
        .zip(&changed_output)
        .zip(&changed_non_luma)
    {
        assert_eq!(
            original.lightness().to_bits(),
            changed.lightness().to_bits()
        );
        assert_eq!(changed.a().to_bits(), source.a().to_bits());
        assert_eq!(changed.b().to_bits(), source.b().to_bits());
        assert_eq!(changed.alpha().to_bits(), source.alpha().to_bits());
    }
}

#[test]
fn scratch_budget_accounts_for_one_luma_float_per_pixel() {
    let dimensions = dimensions(11, 11);
    let pixel_count = (dimensions.width() * dimensions.height()) as usize;
    let output_bytes = pixel_count * std::mem::size_of::<SharpenPixel>();
    let temporary_bytes = dimensions.width() as usize * std::mem::size_of::<f32>();
    // radius=2 commits to 5, so wd=11 and native wd4=3 (12 floats).
    let kernel_bytes = 3 * 4 * std::mem::size_of::<f32>();
    let required = output_bytes + temporary_bytes + kernel_bytes;
    let config = SharpenConfig::defaults();
    assert!(
        SharpenPlan::new_with_budget(
            config,
            dimensions,
            1.0,
            1.0,
            ReconstructionBudget::new(required),
        )
        .is_ok()
    );
    assert!(matches!(
        SharpenPlan::new_with_budget(
            config,
            dimensions,
            1.0,
            1.0,
            ReconstructionBudget::new(required - std::mem::size_of::<f32>()),
        ),
        Err(OperationExecutionError::MemoryBudgetExceeded { .. })
    ));
}

#[test]
fn malformed_frames_scales_and_nonfinite_pixels_fail_closed() {
    let valid = fixture(11, 11);
    let operation = plan(11, 11, 1.0, 1.0, 0.0);
    assert!(matches!(
        operation.execute(&valid[..valid.len() - 1]),
        Err(OperationExecutionError::DimensionsMismatch { .. })
    ));

    let mut nonfinite = valid.clone();
    nonfinite[4] = SharpenPixel::new(f32::NAN, 0.0, 0.0, 1.0);
    assert!(matches!(
        operation.execute(&nonfinite),
        Err(OperationExecutionError::NonFiniteResult { pixel: 4, .. })
    ));

    let config = SharpenConfig::defaults();
    assert!(matches!(
        SharpenPlan::new(config, dimensions(11, 11), f32::NAN, 1.0),
        Err(OperationExecutionError::UnsupportedCapability(_))
    ));
    assert!(matches!(
        SharpenPlan::new(config, dimensions(11, 11), 1.0, 0.0),
        Err(OperationExecutionError::UnsupportedCapability(_))
    ));
    assert!(matches!(
        SharpenPlan::new(
            SharpenConfig::new(-1.0, 0.5, 0.5).unwrap(),
            dimensions(11, 11),
            1.0,
            1.0,
        ),
        Err(OperationExecutionError::UnsupportedCapability(_))
    ));
}

#[test]
fn allocation_budget_and_cancellation_are_explicit_boundaries() {
    let config = SharpenConfig::defaults();
    let budget_error = SharpenPlan::new_with_budget(
        config,
        dimensions(11, 11),
        1.0,
        1.0,
        ReconstructionBudget::new(1),
    )
    .unwrap_err();
    assert!(matches!(
        budget_error,
        OperationExecutionError::MemoryBudgetExceeded { .. }
    ));

    let operation = plan(11, 11, 1.0, 1.0, 0.0);
    let input = fixture(11, 11);
    let mut calls = 0;
    let cancelled = operation.execute_with_cancel(&input, || {
        calls += 1;
        // One initial check and one check per validation row precede the
        // processing-row checks; cancellation therefore happens mid-raster.
        calls >= 20
    });
    assert_eq!(cancelled, Err(OperationExecutionError::Cancelled));
    assert!(calls >= 20);
}

#[test]
fn scalar_execution_is_deterministic() {
    let input = fixture(13, 13);
    let operation = plan(13, 13, 1.0, 1.75, 0.25);
    let first = operation.execute(&input).unwrap();
    let second = operation.execute(&input).unwrap();
    assert_eq!(first.len(), second.len());
    for (left, right) in first.iter().zip(second) {
        for (left, right) in left.channels().into_iter().zip(right.channels()) {
            assert_eq!(left.to_bits(), right.to_bits());
        }
    }
}

#[test]
fn finite_overflow_is_not_published() {
    let mut input = fixture(11, 11);
    let center = 5 * 11 + 5;
    input[center] = SharpenPixel::new(f32::MAX, 0.0, 0.0, 1.0);
    let error = plan(11, 11, 1.0, 2.0, 0.0).execute(&input);
    assert!(matches!(
        error,
        Err(OperationExecutionError::NonFiniteResult { .. })
    ));
}

// Keep the path-harness aliases used above from being mistaken for production
// registration; the canonical integrator owns the shared operation hubs.
const _: usize = SHARPEN_CHANNELS;
const _: f32 = SHARPEN_DEFAULT_RADIUS;
const _: f32 = SHARPEN_DEFAULT_AMOUNT;
const _: f32 = SHARPEN_DEFAULT_THRESHOLD;
const _: &str = SHARPEN_INPUT_ENCODING;
const _: u16 = SHARPEN_SCHEMA_VERSION;
const _: usize = SHARPEN_PARAMETER_BYTES;
