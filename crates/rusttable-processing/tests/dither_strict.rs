#![expect(
    clippy::float_cmp,
    reason = "Source-derived f32 and ABI fixtures require exact native comparisons."
)]

use rusttable_processing::descriptor::{AlphaPolicy, dither_descriptor};
use rusttable_processing::operations::dither::{
    DITHER_COMPATIBILITY_ID, DITHER_DEFAULT_DAMPING, DITHER_DEFAULT_ENABLED,
    DITHER_GENERATED_INVENTORY_ORDER, DITHER_LEGACY_ORDER, DITHER_NATIVE_FIELD_BYTES,
    DITHER_PRESET_DAMPING, DITHER_RUST_ID, DITHER_SCHEMA_VERSION, DITHER_V1_PARAMETER_BYTES,
    DITHER_V2_PARAMETER_BYTES, DITHER_V30_ORDER, DITHER_V50_ORDER, DitherConfig,
    DitherExecutionError, DitherHistory, DitherMethod, DitherParametersV1, DitherParametersV2,
    DitherPlan,
};
use rusttable_processing::{FiniteF32, LinearRgb, RasterDimensions};

const _: () = assert!(!DITHER_DEFAULT_ENABLED);

fn decode_hex(text: &str) -> Vec<u8> {
    let (pairs, remainder) = text.trim().as_bytes().as_chunks::<2>();
    assert!(remainder.is_empty(), "fixture has complete byte pairs");
    pairs
        .iter()
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("fixture is ASCII");
            u8::from_str_radix(pair, 16).expect("fixture contains hexadecimal bytes")
        })
        .collect()
}

fn rgb(red: f32, green: f32, blue: f32) -> LinearRgb {
    LinearRgb::new(
        FiniteF32::new(red).expect("finite red"),
        FiniteF32::new(green).expect("finite green"),
        FiniteF32::new(blue).expect("finite blue"),
    )
}

const fn channel_bits(pixel: LinearRgb) -> [u32; 3] {
    [
        pixel.red().get().to_bits(),
        pixel.green().get().to_bits(),
        pixel.blue().get().to_bits(),
    ]
}

#[test]
fn native_v1_v2_and_preset_fixtures_round_trip_exactly() {
    let v1_fixture = decode_hex(include_str!("fixtures/dither-params-v1.hex"));
    let v2_fixture = decode_hex(include_str!("fixtures/dither-params-v2.hex"));
    let preset_fixture = decode_hex(include_str!("fixtures/dither-preset-v2.hex"));

    assert_eq!(v1_fixture.len(), DITHER_V1_PARAMETER_BYTES);
    assert_eq!(v2_fixture.len(), DITHER_V2_PARAMETER_BYTES);
    assert_eq!(preset_fixture.len(), DITHER_V2_PARAMETER_BYTES);
    assert_eq!(DITHER_NATIVE_FIELD_BYTES, 32);

    let native_v1: &[u8; DITHER_NATIVE_FIELD_BYTES] =
        v1_fixture.as_slice().try_into().expect("native v1 bytes");
    let native_v2: &[u8; DITHER_NATIVE_FIELD_BYTES] =
        v2_fixture.as_slice().try_into().expect("native v2 bytes");
    assert_eq!(
        DitherParametersV1::from_native_bytes(native_v1).to_bytes(),
        *native_v1
    );
    assert_eq!(
        DitherParametersV2::from_native_bytes(native_v2).to_bytes(),
        *native_v2
    );

    let v1 = DitherHistory::decode(1, &v1_fixture).expect("native v1 fixture");
    let v2 = DitherHistory::decode(2, &v2_fixture).expect("native v2 fixture");
    assert_eq!(v1.version(), 1);
    assert_eq!(v2.version(), DITHER_SCHEMA_VERSION);
    assert_eq!(v1.payload().expect("encode v1"), v1_fixture);
    assert_eq!(v2.payload().expect("encode v2"), v2_fixture);
    assert_eq!(
        DitherParametersV1::defaults().to_bytes(),
        v1_fixture.as_slice()
    );
    assert_eq!(
        DitherParametersV2::defaults().to_bytes(),
        v2_fixture.as_slice()
    );
    assert_eq!(
        DitherParametersV2::native_preset().to_bytes(),
        preset_fixture.as_slice()
    );
    assert_eq!(
        DitherParametersV2::defaults().damping,
        DITHER_DEFAULT_DAMPING
    );
    assert_eq!(
        DitherParametersV2::native_preset().damping,
        DITHER_PRESET_DAMPING
    );

    let migrated = v1.current().expect("v1 migration");
    assert_eq!(migrated.to_bytes(), v2_fixture.as_slice());
}

#[test]
fn history_preserves_native_rows_and_unknown_payloads_byte_for_byte() {
    let v1_bytes = DitherParametersV1::defaults().to_bytes();
    let history = DitherHistory::decode(1, &v1_bytes).expect("known v1");
    assert_eq!(history.payload().expect("re-encode v1"), v1_bytes);
    let migrated = history.current().expect("v1 migration");
    assert_eq!(migrated.to_bytes(), v1_bytes);

    let unknown_bytes = [0xde, 0xad, 0xbe, 0xef];
    let unknown = DitherHistory::decode(77, &unknown_bytes).expect("opaque version");
    assert_eq!(unknown.version(), 77);
    assert_eq!(unknown.payload().expect("re-encode opaque"), unknown_bytes);
    assert!(unknown.current().is_err());

    let mut unknown_method = DitherParametersV2::defaults().to_bytes();
    unknown_method[..4].copy_from_slice(&i32::MIN.to_le_bytes());
    let opaque = DitherHistory::decode(2, &unknown_method).expect("opaque method");
    assert_eq!(
        opaque.payload().expect("re-encode unknown method"),
        unknown_method
    );

    assert_eq!(
        DitherHistory::decode(1, &[0; 168]),
        Err(
            rusttable_processing::operations::dither::DitherCodecError::InvalidLength {
                expected: DITHER_V1_PARAMETER_BYTES,
                actual: 168,
            }
        )
    );
}

#[test]
fn source_metadata_and_order_are_explicit_without_claiming_shared_integration() {
    assert_eq!(DITHER_COMPATIBILITY_ID, "dither");
    assert_eq!(DITHER_RUST_ID, "rusttable.dither");
    assert_eq!(DITHER_LEGACY_ORDER, 67.5);
    assert_eq!(DITHER_V30_ORDER, 75);
    assert_eq!(DITHER_V50_ORDER, 75);
    assert_eq!(DITHER_GENERATED_INVENTORY_ORDER, 90);
    assert_eq!(DitherMethod::Posterize(2).id(), 0x101);
    assert_eq!(DitherMethod::Posterize(8).id(), 0x107);
    assert_eq!(
        DitherMethod::from_id(0x107).expect("native POSTER_8"),
        DitherMethod::Posterize(8)
    );
    assert!(DitherMethod::from_id(0x100).is_err());

    let descriptor = dither_descriptor();
    assert_eq!(descriptor.id.compatibility_name, DITHER_COMPATIBILITY_ID);
    assert_eq!(descriptor.io.input.channels, 3);
    assert_eq!(descriptor.io.output.channels, 3);
    assert_eq!(descriptor.io.input.alpha, AlphaPolicy::Preserve);
    assert_eq!(descriptor.io.output.alpha, AlphaPolicy::Preserve);
    assert!(descriptor.capability.cpu_supported);
    assert!(descriptor.capability.gpu_tier.is_none());
}

#[test]
fn native_random_uses_one_tea_sample_for_all_rgb_channels_and_ignores_compat_seed() {
    let dimensions = RasterDimensions::new(3, 2).expect("dimensions");
    let input = vec![
        rgb(0.20, 0.40, 0.60),
        rgb(0.25, 0.45, 0.65),
        rgb(0.30, 0.50, 0.70),
        rgb(0.35, 0.55, 0.75),
        rgb(0.40, 0.60, 0.80),
        rgb(0.45, 0.65, 0.85),
    ];
    let source = DitherConfig::new(DitherMethod::Random, -20.0).expect("random config");
    let compatibility_seeded = source.with_seed(u64::MAX);
    let source_output = DitherPlan::new(source, dimensions)
        .execute(&input)
        .expect("source random");
    let seeded_output = DitherPlan::new(compatibility_seeded, dimensions)
        .execute(&input)
        .expect("compatibility seed is not a native parameter");
    assert_eq!(source_output, seeded_output);

    // Captured from the scalar source equations: row seed `j * height`, eight
    // TEA rounds per pixel, one TPDF sample shared by RGB, all at f32 bounds.
    let bits = source_output
        .into_iter()
        .map(channel_bits)
        .collect::<Vec<_>>();
    assert_eq!(
        bits,
        [
            [0x3ecf_2a32, 0x3f1a_c84c, 0x3f4d_fb80],
            [0x3e69_e59a, 0x3edb_5933, 0x3f20_dfcc],
            [0x3e70_2b97, 0x3ede_7c32, 0x3f22_714c],
            [0x3ea9_82a8, 0x3f07_f488, 0x3f3b_27bb],
            [0x3f1a_b7b6, 0x3f4d_eaea, 0x3f80_0000],
            [0x3f05_2538, 0x3f38_586c, 0x3f6b_8ba0],
        ]
    );
}

#[test]
fn floyd_steinberg_scalar_order_and_fma_boundaries_are_exact() {
    let dimensions = RasterDimensions::new(3, 3).expect("dimensions");
    let input = vec![
        rgb(0.10, 0.20, 0.30),
        rgb(0.40, 0.50, 0.60),
        rgb(0.70, 0.80, 0.90),
        rgb(0.15, 0.25, 0.35),
        rgb(0.45, 0.55, 0.65),
        rgb(0.75, 0.85, 0.95),
        rgb(0.05, 0.33, 0.66),
        rgb(0.22, 0.57, 0.88),
        rgb(0.12, 0.48, 0.78),
    ];
    let output = DitherPlan::new(
        DitherConfig::new(DitherMethod::Fs2BitRgb, -100.0).expect("config"),
        dimensions,
    )
    .execute(&input)
    .expect("Floyd-Steinberg output");
    let bits = output.into_iter().map(channel_bits).collect::<Vec<_>>();
    assert_eq!(
        bits,
        [
            [0, 0x3eaa_aaab, 0x3eaa_aaab],
            [0x3eaa_aaab, 0x3eaa_aaab, 0x3f2a_aaab],
            [0x3f2a_aaab, 0x3f80_0000, 0x3f80_0000],
            [0x3eaa_aaab, 0x3eaa_aaab, 0x3eaa_aaab],
            [0x3eaa_aaab, 0x3f2a_aaab, 0x3f2a_aaab],
            [0x3f2a_aaab, 0x3f2a_aaab, 0x3f80_0000],
            [0, 0x3eaa_aaab, 0x3f2a_aaab],
            [0x3eaa_aaab, 0x3eaa_aaab, 0x3f2a_aaab],
            [0, 0x3f2a_aaab, 0x3f2a_aaab],
        ]
    );
}

#[test]
fn cancellation_shape_and_nonfinite_output_fail_without_publication() {
    let dimensions = RasterDimensions::new(64, 64).expect("dimensions");
    let input = vec![rgb(0.4, 0.5, 0.6); 64 * 64];
    let baseline = input.clone();
    let plan = DitherPlan::new(
        DitherConfig::new(DitherMethod::Fs4BitRgb, -100.0).expect("config"),
        dimensions,
    );
    let mut polls = 0;
    let result = plan.execute_with_cancellation(&input, || {
        polls += 1;
        polls == 3
    });
    assert_eq!(result, Err(DitherExecutionError::Cancelled));
    assert_eq!(input, baseline);
    assert!(polls <= 4);

    assert!(matches!(
        plan.execute(&input[..input.len() - 1]),
        Err(DitherExecutionError::DimensionsMismatch { .. })
    ));

    let extreme = DitherPlan::new(
        DitherConfig::new(DitherMethod::Posterize(8), -100.0).expect("config"),
        RasterDimensions::new(1, 1).expect("one pixel"),
    )
    .execute(&[rgb(f32::MAX, 0.0, 0.0)]);
    assert_eq!(
        extreme,
        Err(DitherExecutionError::NonFiniteOutput {
            pixel: 0,
            channel: 0,
        })
    );
}
