#![forbid(unsafe_code)]
#![allow(
    clippy::assertions_on_constants,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::chunks_exact_to_as_chunks,
    clippy::float_cmp,
    clippy::suboptimal_flops,
    reason = "source-derived metadata, fixture bytes, and native arithmetic require exact assertions"
)]

use std::cell::Cell;
use std::mem::size_of;

#[path = "../src/operations/colisa/mod.rs"]
mod colisa;

use colisa::{
    COLISA_METADATA, COLISA_PARAMETER_BYTES, COLISA_TABLE_ENTRIES, ColisaError, ColisaFormat,
    ColisaHistory, ColisaParametersV1, ColisaPlan, ColisaRaster, DEFAULT_V1_FIXTURE_HEX,
};

fn decode_hex(fixture: &str) -> Vec<u8> {
    let compact = fixture.trim();
    assert_eq!(compact.len() % 2, 0);
    compact
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).expect("ASCII fixture"), 16)
                .expect("hex fixture")
        })
        .collect()
}

const NATIVE_LUT_SCALE: f32 = 65_536.0;
const NATIVE_EXPONENTIAL_X: [f32; 4] = [0.7, 0.8, 0.9, 1.0];

// Independent equations transcribed from src/iop/colisa.c:184-234 and
// src/develop/imageop_math.h:96-135. These deliberately do not inspect the
// Rust leaf's private tables or exponential coefficients.
fn native_lookup_index(value: f32) -> usize {
    if value <= 0.0 {
        0
    } else if value >= 1.0 {
        COLISA_TABLE_ENTRIES - 1
    } else {
        (value * NATIVE_LUT_SCALE) as usize
    }
}

fn native_contrast_value(parameters: ColisaParametersV1, index: usize) -> f32 {
    let contrast = parameters.contrast + 1.0;
    if contrast <= 1.0 {
        let position = 100.0 * index as f32 / NATIVE_LUT_SCALE - 50.0;
        contrast.mul_add(position, 50.0)
    } else {
        let boost = 20.0;
        let contrast_minus_one_squared = boost * (contrast - 1.0) * (contrast - 1.0);
        let contrast_scale = (1.0 + contrast_minus_one_squared).sqrt();
        let position = 2.0 * index as f32 / NATIVE_LUT_SCALE - 1.0;
        let denominator = (1.0 + contrast_minus_one_squared * position * position).sqrt();
        50.0 * (contrast_scale * position / denominator + 1.0)
    }
}

fn native_brightness_value(parameters: ColisaParametersV1, index: usize) -> f32 {
    let brightness = parameters.brightness * 2.0;
    let gamma = if brightness >= 0.0 {
        1.0 / (1.0 + brightness)
    } else {
        1.0 - brightness
    };
    100.0 * (index as f32 / NATIVE_LUT_SCALE).powf(gamma)
}

fn native_estimate_exp(y: [f32; 4]) -> [f32; 3] {
    let x0 = NATIVE_EXPONENTIAL_X[3];
    let y0 = y[3];
    let mut exponent = 0.0;
    let mut count = 0_u32;
    for index in 0..3 {
        let yy = y[index] / y0;
        let xx = NATIVE_EXPONENTIAL_X[index] / x0;
        if yy > 0.0 && xx > 0.0 {
            exponent += (y[index] / y0).ln() / (NATIVE_EXPONENTIAL_X[index] / x0).ln();
            count += 1;
        }
    }
    if count == 0 {
        exponent = 1.0;
    } else {
        exponent *= 1.0 / count as f32;
    }
    [1.0 / x0, y0, exponent]
}

fn native_eval_exp(coefficients: [f32; 3], value: f32) -> f32 {
    coefficients[1] * (value * coefficients[0]).powf(coefficients[2])
}

fn native_expected_pixel(parameters: ColisaParametersV1, input: [f32; 4]) -> [f32; 4] {
    let contrast_samples = NATIVE_EXPONENTIAL_X
        .map(|sample| native_contrast_value(parameters, native_lookup_index(sample)));
    let brightness_samples = NATIVE_EXPONENTIAL_X
        .map(|sample| native_brightness_value(parameters, native_lookup_index(sample)));
    let contrast_coefficients = native_estimate_exp(contrast_samples);
    let brightness_coefficients = native_estimate_exp(brightness_samples);

    let contrasted = if input[0] < 100.0 {
        native_contrast_value(parameters, native_lookup_index(input[0] / 100.0))
    } else {
        native_eval_exp(contrast_coefficients, input[0] / 100.0)
    };
    let lightness = if contrasted < 100.0 {
        native_brightness_value(parameters, native_lookup_index(contrasted / 100.0))
    } else {
        native_eval_exp(brightness_coefficients, contrasted / 100.0)
    };

    [
        lightness,
        input[1] * (parameters.saturation + 1.0),
        input[2] * (parameters.saturation + 1.0),
        input[3],
    ]
}

fn assert_pixels_close(actual: &[f32], expected: &[f32], tolerance: f32) {
    assert_eq!(actual.len(), expected.len());
    for (index, (&actual, &expected)) in actual.iter().zip(expected).enumerate() {
        assert!(
            (actual - expected).abs() <= tolerance,
            "sample {index}: actual {actual:?}, native expected {expected:?}"
        );
    }
}

fn native_expected_pixels(parameters: ColisaParametersV1, input: &[f32]) -> Vec<f32> {
    input
        .chunks_exact(4)
        .flat_map(|pixel| {
            native_expected_pixel(
                parameters,
                pixel.try_into().expect("four-channel test pixel"),
            )
        })
        .collect()
}

#[test]
fn native_fixture_and_opaque_history_round_trip() {
    let bytes = decode_hex(DEFAULT_V1_FIXTURE_HEX);
    assert_eq!(bytes.len(), COLISA_PARAMETER_BYTES);
    let history = ColisaHistory::decode(1, &bytes).expect("decode native v1");
    assert_eq!(
        history.current().expect("current v1"),
        ColisaParametersV1::defaults()
    );
    assert_eq!(history.payload().expect("encode v1"), bytes);
    assert_eq!(
        ColisaHistory::decode(1, &[0; 8]),
        Err(ColisaError::InvalidPayloadLength {
            expected: 12,
            actual: 8,
        })
    );

    let unknown_bytes = [9_u8, 8, 7, 6, 5, 4, 3];
    let unknown = ColisaHistory::decode(55, &unknown_bytes).expect("retain unknown history");
    assert_eq!(unknown.version(), 55);
    assert_eq!(
        unknown.payload().expect("copy unknown history"),
        unknown_bytes
    );
    assert_eq!(unknown.current(), Err(ColisaError::OpaqueVersion(55)));
}

#[test]
fn metadata_and_source_map_do_not_overclaim_routing() {
    assert_eq!(COLISA_METADATA.parameter_version, 1);
    assert!(COLISA_METADATA.deprecated);
    assert!(!COLISA_METADATA.default_enabled);
    assert_eq!(COLISA_METADATA.default_colorspace, "lab");
    assert!(COLISA_METADATA.allow_tiling);
    assert!(COLISA_METADATA.supports_shared_blending_native);
    assert!(!COLISA_METADATA.shared_blending_integrated);
    assert_eq!(COLISA_METADATA.legacy_order, 47.0);
    assert_eq!(COLISA_METADATA.v50_raw_order, 47.0);
    assert_eq!(COLISA_METADATA.v50_jpeg_order, 47.0);
    assert_eq!(COLISA_METADATA.generated_inventory_order, 74);

    let source_map = include_str!("../../../architecture/rusttable-colisa-cpu-source-map.toml");
    assert!(source_map.contains("production_registration = \"deferred"));
    assert!(source_map.contains("no fictitious v2 migration is invented"));
    for deferred in ["shared masks/blending", "GPU", "GTK", "history import"] {
        assert!(
            source_map.contains(deferred),
            "missing deferred responsibility: {deferred}"
        );
    }
}

#[test]
fn cpu_retains_lut_quantization_saturation_and_fourth_lane() {
    let table_bytes = 2 * COLISA_TABLE_ENTRIES * size_of::<f32>();
    let identity = ColisaPlan::compile(ColisaParametersV1::defaults(), table_bytes)
        .expect("compile identity plan");
    let input = [50.0_f32, 10.0, -20.0, 0.7];
    let output = identity
        .execute(
            ColisaRaster::new(&input, 1, 1, ColisaFormat::LabF32x4),
            input.len() * size_of::<f32>(),
            || false,
        )
        .expect("execute identity plan");
    assert_eq!(output[0].to_bits(), 50.0_f32.to_bits());
    assert_eq!(output[1].to_bits(), input[1].to_bits());
    assert_eq!(output[2].to_bits(), input[2].to_bits());
    assert_eq!(output[3].to_bits(), input[3].to_bits());

    let flat = ColisaPlan::compile(ColisaParametersV1::new(-1.0, 0.0, 0.5), table_bytes)
        .expect("compile flat contrast plan");
    let flat_output = flat
        .execute(
            ColisaRaster::new(&[12.0, 4.0, -6.0, 0.25], 1, 1, ColisaFormat::LabF32x4),
            4 * size_of::<f32>(),
            || false,
        )
        .expect("execute flat contrast plan");
    assert_eq!(flat_output[0].to_bits(), 50.0_f32.to_bits());
    assert_eq!(flat_output[1].to_bits(), 6.0_f32.to_bits());
    assert_eq!(flat_output[2].to_bits(), (-9.0_f32).to_bits());
    assert_eq!(flat_output[3].to_bits(), 0.25_f32.to_bits());
}

#[test]
fn native_curve_vectors_cover_linear_nonlinear_brightness_and_saturation() {
    let table_bytes = 2 * COLISA_TABLE_ENTRIES * size_of::<f32>();

    // Native commit_params rescales contrast and brightness before selecting
    // the linear/nonlinear and positive/negative brightness equations.
    let linear_parameters = ColisaParametersV1::new(-0.5, 0.5, -0.25);
    let linear_input = [
        40.0_f32,
        12.0,
        -8.0,
        0.125,
        f32::from_bits(50.0_f32.to_bits() - 1),
        -3.0,
        7.0,
        0.875,
    ];
    let linear_plan = ColisaPlan::compile(linear_parameters, table_bytes)
        .expect("compile native linear and positive-brightness plan");
    let linear_output = linear_plan
        .execute(
            ColisaRaster::new(&linear_input, 2, 1, ColisaFormat::LabF32x4),
            linear_input.len() * size_of::<f32>(),
            || false,
        )
        .expect("execute native linear and positive-brightness vectors");
    assert_pixels_close(
        &linear_output,
        &native_expected_pixels(linear_parameters, &linear_input),
        0.0001,
    );
    assert_eq!(linear_output[1].to_bits(), 9.0_f32.to_bits());
    assert_eq!(linear_output[2].to_bits(), (-6.0_f32).to_bits());
    assert_eq!(linear_output[3].to_bits(), 0.125_f32.to_bits());
    assert_eq!(linear_output[7].to_bits(), 0.875_f32.to_bits());

    let nonlinear_parameters = ColisaParametersV1::new(0.5, -0.5, 0.75);
    let nonlinear_input = [25.0_f32, -4.0, 6.0, 0.875, 120.0, 3.0, -7.0, 0.25];
    let nonlinear_plan = ColisaPlan::compile(nonlinear_parameters, table_bytes)
        .expect("compile native nonlinear and negative-brightness plan");
    let nonlinear_output = nonlinear_plan
        .execute(
            ColisaRaster::new(&nonlinear_input, 2, 1, ColisaFormat::LabF32x4),
            nonlinear_input.len() * size_of::<f32>(),
            || false,
        )
        .expect("execute native nonlinear and negative-brightness vectors");
    assert_pixels_close(
        &nonlinear_output,
        &native_expected_pixels(nonlinear_parameters, &nonlinear_input),
        0.0001,
    );
    assert_eq!(nonlinear_output[1].to_bits(), (-7.0_f32).to_bits());
    assert_eq!(nonlinear_output[2].to_bits(), 10.5_f32.to_bits());
    assert_eq!(nonlinear_output[3].to_bits(), 0.875_f32.to_bits());
    assert_eq!(nonlinear_output[7].to_bits(), 0.25_f32.to_bits());
}

#[test]
fn native_threshold_extrapolation_and_exact_floor_quantization() {
    let table_bytes = 2 * COLISA_TABLE_ENTRIES * size_of::<f32>();
    let parameters = ColisaParametersV1::new(0.5, 0.25, -0.5);
    let input = [
        -25.0_f32, 4.0, -6.0, 0.2, 99.999, -1.0, 2.0, 0.4, 100.0, -1.0, 2.0, 0.4, 120.0, 1.5, -2.5,
        0.6,
    ];
    let plan = ColisaPlan::compile(parameters, table_bytes).expect("compile threshold plan");
    let output = plan
        .execute(
            ColisaRaster::new(&input, 4, 1, ColisaFormat::LabF32x4),
            input.len() * size_of::<f32>(),
            || false,
        )
        .expect("execute threshold and extrapolation vectors");
    assert_pixels_close(&output, &native_expected_pixels(parameters, &input), 0.0001);

    // process uses '< 100.0f' for the LUT and the exponential fit at 100.0f.
    // The fitted value at x=1 is the LUT's last entry; above it the fit leaves
    // the last-entry plateau and reaches the brightness extrapolation branch.
    assert_eq!(output[4].to_bits(), output[8].to_bits());
    assert!(output[12] > output[8]);

    let boundary_below = f32::from_bits(50.0_f32.to_bits() - 1);
    let boundary_exact = 50.0_f32;
    let boundary_above = f32::from_bits(50.0_f32.to_bits() + 1);
    let boundary_input = [
        boundary_below,
        1.0_f32,
        2.0,
        0.1,
        boundary_exact,
        1.0,
        2.0,
        0.2,
        boundary_above,
        1.0,
        2.0,
        0.3,
    ];
    let identity = ColisaPlan::compile(ColisaParametersV1::defaults(), table_bytes)
        .expect("compile quantization plan");
    let boundary_output = identity
        .execute(
            ColisaRaster::new(&boundary_input, 3, 1, ColisaFormat::LabF32x4),
            boundary_input.len() * size_of::<f32>(),
            || false,
        )
        .expect("execute exact lookup boundary vectors");
    assert_pixels_close(
        &boundary_output,
        &native_expected_pixels(ColisaParametersV1::defaults(), &boundary_input),
        0.0,
    );
    assert_eq!(boundary_output[0].to_bits(), native_floor_value().to_bits());
    assert_eq!(boundary_output[4].to_bits(), 50.0_f32.to_bits());
    assert_eq!(boundary_output[8].to_bits(), 50.0_f32.to_bits());
    assert_ne!(boundary_output[0].to_bits(), boundary_output[4].to_bits());
}

fn native_floor_value() -> f32 {
    100.0_f32 * (32_767.0_f32 / NATIVE_LUT_SCALE)
}

#[test]
fn planning_and_execution_fail_closed_transactionally() {
    let table_bytes = 2 * COLISA_TABLE_ENTRIES * size_of::<f32>();
    let planning_polls = Cell::new(0_u32);
    assert_eq!(
        ColisaPlan::compile_with_cancellation(ColisaParametersV1::defaults(), table_bytes, || {
            let next = planning_polls.get() + 1;
            planning_polls.set(next);
            next >= 2
        },),
        Err(ColisaError::Cancelled)
    );
    assert_eq!(
        ColisaPlan::compile(ColisaParametersV1::defaults(), table_bytes - 1),
        Err(ColisaError::WorkingMemoryBudgetExceeded {
            required: table_bytes,
            budget: table_bytes - 1,
        })
    );
    assert_eq!(
        ColisaPlan::compile(ColisaParametersV1::new(f32::NAN, 0.0, 0.0), table_bytes),
        Err(ColisaError::NonFiniteParameter("contrast"))
    );
    assert_eq!(
        ColisaPlan::compile(ColisaParametersV1::new(0.0, 1.01, 0.0), table_bytes),
        Err(ColisaError::ParameterOutOfRange("brightness"))
    );

    let plan = ColisaPlan::compile(ColisaParametersV1::defaults(), table_bytes)
        .expect("compile default plan");
    let valid = [50.0_f32, 1.0, 2.0, 0.5];
    assert_eq!(
        plan.execute(
            ColisaRaster::new(&valid, 1, 1, ColisaFormat::RgbaF32x4),
            usize::MAX,
            || false,
        ),
        Err(ColisaError::UnsupportedFormat)
    );
    assert_eq!(
        plan.execute(
            ColisaRaster::new(&valid[..3], 1, 1, ColisaFormat::LabF32x4),
            usize::MAX,
            || false,
        ),
        Err(ColisaError::InputLengthMismatch {
            expected: 4,
            actual: 3,
        })
    );
    assert_eq!(
        plan.execute(
            ColisaRaster::new(&valid, 1, 1, ColisaFormat::LabF32x4),
            15,
            || false,
        ),
        Err(ColisaError::OutputMemoryBudgetExceeded {
            required: 16,
            budget: 15,
        })
    );
    let invalid = [50.0_f32, 1.0, f32::NEG_INFINITY, 0.5];
    assert_eq!(
        plan.execute(
            ColisaRaster::new(&invalid, 1, 1, ColisaFormat::LabF32x4),
            usize::MAX,
            || false,
        ),
        Err(ColisaError::NonFiniteInput { index: 2 })
    );
    let many = vec![50.0_f32; 257 * 4];
    let polls = Cell::new(0_u32);
    let mut cancelled_destination = vec![3.0_f32, 2.0, 1.0];
    let error = plan
        .execute_and_publish(
            ColisaRaster::new(&many, 257, 1, ColisaFormat::LabF32x4),
            &mut cancelled_destination,
            usize::MAX,
            || {
                let next = polls.get() + 1;
                polls.set(next);
                next >= 3
            },
        )
        .expect_err("cancel before publication");
    assert_eq!(error, ColisaError::Cancelled);
    assert_eq!(cancelled_destination, [3.0_f32, 2.0, 1.0]);
}

#[test]
fn invalid_later_pixel_does_not_publish_partial_output() {
    let table_bytes = 2 * COLISA_TABLE_ENTRIES * size_of::<f32>();
    let plan = ColisaPlan::compile(ColisaParametersV1::defaults(), table_bytes)
        .expect("compile default plan");
    let invalid_after_first_pixel = [50.0_f32, 1.0, 2.0, 0.5, 50.0, 1.0, f32::NAN, 0.5];
    let mut destination = vec![-4.0_f32, -3.0, -2.0, -1.0];
    assert_eq!(
        plan.execute_and_publish(
            ColisaRaster::new(&invalid_after_first_pixel, 2, 1, ColisaFormat::LabF32x4,),
            &mut destination,
            usize::MAX,
            || false,
        ),
        Err(ColisaError::NonFiniteInput { index: 6 })
    );
    assert_eq!(destination, [-4.0_f32, -3.0, -2.0, -1.0]);
}
