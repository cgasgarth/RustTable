#![forbid(unsafe_code)]
#![allow(
    clippy::assertions_on_constants,
    clippy::chunks_exact_to_as_chunks,
    clippy::float_cmp,
    reason = "source-derived metadata and native branch values require exact assertions"
)]

use std::cell::Cell;
use std::mem::size_of;

#[path = "../src/operations/profile_gamma/mod.rs"]
mod profile_gamma;

use profile_gamma::{
    DEFAULT_V1_FIXTURE_HEX, DEFAULT_V2_FIXTURE_HEX, PROFILE_GAMMA_METADATA,
    PROFILE_GAMMA_TABLE_ENTRIES, PROFILE_GAMMA_V1_PARAMETER_BYTES,
    PROFILE_GAMMA_V2_PARAMETER_BYTES, ProfileGammaError, ProfileGammaFormat, ProfileGammaHistory,
    ProfileGammaMode, ProfileGammaParametersV1, ProfileGammaParametersV2, ProfileGammaPlan,
    ProfileGammaRaster,
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

#[test]
fn native_v1_v2_fixtures_and_opaque_history_round_trip() {
    let v1_bytes = decode_hex(DEFAULT_V1_FIXTURE_HEX);
    assert_eq!(v1_bytes.len(), PROFILE_GAMMA_V1_PARAMETER_BYTES);
    let v1 = ProfileGammaHistory::decode(1, &v1_bytes).expect("decode native v1");
    assert_eq!(v1.payload().expect("encode native v1"), v1_bytes);
    let migrated = v1.current().expect("migrate native v1");
    assert_eq!(migrated.mode, ProfileGammaMode::Gamma as i32);
    assert_eq!(migrated.linear.to_bits(), 0.1_f32.to_bits());
    assert_eq!(migrated.gamma.to_bits(), 0.45_f32.to_bits());
    assert_eq!(migrated.dynamic_range.to_bits(), 10.0_f32.to_bits());
    assert_eq!(migrated.grey_point.to_bits(), 18.0_f32.to_bits());
    assert_eq!(migrated.shadows_range.to_bits(), (-5.0_f32).to_bits());
    assert_eq!(migrated.security_factor.to_bits(), 0.0_f32.to_bits());

    let v2_bytes = decode_hex(DEFAULT_V2_FIXTURE_HEX);
    assert_eq!(v2_bytes.len(), PROFILE_GAMMA_V2_PARAMETER_BYTES);
    let v2 = ProfileGammaHistory::decode(2, &v2_bytes).expect("decode native v2");
    assert_eq!(
        v2.current().expect("current v2"),
        ProfileGammaParametersV2::defaults()
    );
    assert_eq!(v2.payload().expect("encode native v2"), v2_bytes);

    assert_eq!(
        ProfileGammaHistory::decode(1, &[0; 4]),
        Err(ProfileGammaError::InvalidPayloadLength {
            version: 1,
            expected: 8,
            actual: 4,
        })
    );
    let opaque_bytes = [0xde, 0xad, 0xbe, 0xef, 0x42];
    let opaque = ProfileGammaHistory::decode(77, &opaque_bytes).expect("retain unknown history");
    assert_eq!(opaque.version(), 77);
    assert_eq!(
        opaque.payload().expect("clone opaque history"),
        opaque_bytes
    );
    assert_eq!(opaque.current(), Err(ProfileGammaError::OpaqueVersion(77)));
}

#[test]
fn metadata_and_source_map_keep_integration_deferred() {
    assert_eq!(PROFILE_GAMMA_METADATA.parameter_version, 2);
    assert!(!PROFILE_GAMMA_METADATA.default_enabled);
    assert!(PROFILE_GAMMA_METADATA.allow_tiling);
    assert!(PROFILE_GAMMA_METADATA.one_instance);
    assert!(PROFILE_GAMMA_METADATA.supports_shared_blending_native);
    assert!(!PROFILE_GAMMA_METADATA.shared_blending_integrated);
    assert_eq!(PROFILE_GAMMA_METADATA.legacy_order, 25.0);
    assert_eq!(PROFILE_GAMMA_METADATA.v50_raw_order, 26.0);
    assert_eq!(PROFILE_GAMMA_METADATA.v50_jpeg_order, 28.0);
    assert_eq!(PROFILE_GAMMA_METADATA.generated_inventory_order, 103);

    let source_map =
        include_str!("../../../architecture/rusttable-profile-gamma-cpu-source-map.toml");
    assert!(source_map.contains("production_registration = \"deferred"));
    assert!(
        source_map.contains(
            "v1 is the source-declared pair of contiguous floats (8 little-endian bytes)"
        )
    );
    assert!(source_map.contains("unsuffixed 1.0 promotes the surrounding g/a expressions"));
    assert!(source_map.contains("non-default LUT golden"));
    for deferred in [
        "shared masks/blending",
        "GPU",
        "GTK",
        "image-picker analysis",
    ] {
        assert!(
            source_map.contains(deferred),
            "missing deferred responsibility: {deferred}"
        );
    }
}

#[test]
fn gamma_and_log_cpu_paths_retain_native_boundaries() {
    let table_bytes = PROFILE_GAMMA_TABLE_ENTRIES * size_of::<f32>();
    let gamma_parameters = ProfileGammaParametersV1::new(0.1, 0.45);
    let gamma_plan = ProfileGammaPlan::compile(
        profile_gamma::migrate_v1_to_v2(gamma_parameters),
        table_bytes,
    )
    .expect("compile gamma plan");
    let input = [0.0_f32, 0.05, 0.5, 0.75, 0.25, 0.5, 0.75, 0.25];
    let output = gamma_plan
        .execute(
            ProfileGammaRaster::new(&input, 2, 1, ProfileGammaFormat::RgbaF32x4),
            input.len() * size_of::<f32>(),
            || false,
        )
        .expect("execute gamma plan");

    let linear = gamma_parameters.linear;
    let gamma = gamma_parameters.gamma;
    let exponent = gamma * (1.0 - linear) / (1.0 - gamma * linear);
    let a = 1.0 / (1.0 + linear * (exponent - 1.0));
    let b = linear * (exponent - 1.0) * a;
    let c = a.mul_add(linear, b).powf(exponent) / linear;
    assert_eq!(output[0].to_bits(), 0.0_f32.to_bits());
    let quantized_005 = 3_276.0_f32 / 65_536.0_f32;
    assert!((output[1] - c * quantized_005).abs() <= 2.0e-6);
    assert!((output[2] - a.mul_add(0.5, b).powf(exponent)).abs() <= 2.0e-6);
    assert_eq!(output[3].to_bits(), input[3].to_bits());
    assert_eq!(output[7].to_bits(), input[7].to_bits());

    let log_plan = ProfileGammaPlan::compile(ProfileGammaParametersV2::defaults(), 0)
        .expect("log mode allocates no table");
    let log_input = [0.18_f32, -1.0, 0.36];
    let log_output = log_plan
        .execute(
            ProfileGammaRaster::new(&log_input, 1, 1, ProfileGammaFormat::RgbF32x3),
            log_input.len() * size_of::<f32>(),
            || false,
        )
        .expect("execute log plan");
    assert!((log_output[0] - 0.5).abs() < 2.0e-3);
    assert_eq!(
        log_output[1].to_bits(),
        profile_gamma::PROFILE_GAMMA_NOISE_FLOOR.to_bits()
    );
    assert!(log_output[2] > log_output[0]);
}

#[test]
fn gamma_lut_uses_c_mixed_double_narrowing_for_non_default_inputs() {
    let parameters = ProfileGammaParametersV2::new(
        ProfileGammaMode::Gamma as i32,
        0.001,
        0.034,
        10.0,
        18.0,
        -5.0,
        0.0,
    );
    let table_bytes = PROFILE_GAMMA_TABLE_ENTRIES * size_of::<f32>();
    let plan = ProfileGammaPlan::compile(parameters, table_bytes).expect("compile gamma plan");
    let sample = 32_767.0_f32 / 65_536.0_f32;
    let output = plan
        .execute(
            ProfileGammaRaster::new(
                &[sample, sample, sample],
                1,
                1,
                ProfileGammaFormat::RgbF32x3,
            ),
            3 * size_of::<f32>(),
            || false,
        )
        .expect("execute gamma plan");

    // Captured from src/iop/profile_gamma.c:501-503. The old all-f32
    // expression produces 0x3f7a08db for this LUT index.
    assert_eq!(output[0].to_bits(), 0x3f7a_08dc);
    assert_eq!(output[1].to_bits(), output[0].to_bits());
    assert_eq!(output[2].to_bits(), output[0].to_bits());
}

#[test]
fn execution_fails_closed_and_never_publishes_partial_output() {
    let table_bytes = PROFILE_GAMMA_TABLE_ENTRIES * size_of::<f32>();
    let gamma = profile_gamma::migrate_v1_to_v2(ProfileGammaParametersV1::new(0.1, 0.45));
    assert_eq!(
        ProfileGammaPlan::compile(gamma, table_bytes - 1),
        Err(ProfileGammaError::WorkingMemoryBudgetExceeded {
            required: table_bytes,
            budget: table_bytes - 1,
        })
    );
    let mut nonfinite = ProfileGammaParametersV2::defaults();
    nonfinite.grey_point = f32::NAN;
    assert_eq!(
        ProfileGammaPlan::compile(nonfinite, 0),
        Err(ProfileGammaError::NonFiniteParameter("grey_point"))
    );

    let plan = ProfileGammaPlan::compile(ProfileGammaParametersV2::defaults(), 0)
        .expect("compile log plan");
    let valid = [0.18_f32, 0.2, 0.3, 0.4];
    assert_eq!(
        plan.execute(
            ProfileGammaRaster::new(&valid, 1, 1, ProfileGammaFormat::LabF32x4),
            usize::MAX,
            || false,
        ),
        Err(ProfileGammaError::UnsupportedFormat)
    );
    assert_eq!(
        plan.execute(
            ProfileGammaRaster::new(&valid[..3], 1, 1, ProfileGammaFormat::RgbaF32x4),
            usize::MAX,
            || false,
        ),
        Err(ProfileGammaError::InputLengthMismatch {
            expected: 4,
            actual: 3,
        })
    );
    assert_eq!(
        plan.execute(
            ProfileGammaRaster::new(&valid, 1, 1, ProfileGammaFormat::RgbaF32x4),
            15,
            || false,
        ),
        Err(ProfileGammaError::OutputMemoryBudgetExceeded {
            required: 16,
            budget: 15,
        })
    );
    let invalid = [0.18_f32, f32::INFINITY, 0.3, 0.4];
    assert_eq!(
        plan.execute(
            ProfileGammaRaster::new(&invalid, 1, 1, ProfileGammaFormat::RgbaF32x4),
            usize::MAX,
            || false,
        ),
        Err(ProfileGammaError::NonFiniteInput { index: 1 })
    );

    let many = vec![0.18_f32; 257 * 4];
    let polls = Cell::new(0_u32);
    let mut destination = vec![9.0_f32, 8.0, 7.0];
    let error = plan
        .execute_and_publish(
            ProfileGammaRaster::new(&many, 257, 1, ProfileGammaFormat::RgbaF32x4),
            &mut destination,
            usize::MAX,
            || {
                let next = polls.get() + 1;
                polls.set(next);
                next >= 3
            },
        )
        .expect_err("cancel before publication");
    assert_eq!(error, ProfileGammaError::Cancelled);
    assert_eq!(destination, [9.0_f32, 8.0, 7.0]);
}
