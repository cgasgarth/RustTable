#![forbid(unsafe_code)]
#![allow(
    clippy::assertions_on_constants,
    clippy::chunks_exact_to_as_chunks,
    clippy::float_cmp,
    reason = "source-derived metadata and fixture bytes require exact assertions"
)]

use std::cell::Cell;
use std::mem::size_of;

#[path = "../src/operations/splittoning/mod.rs"]
mod splittoning;

use splittoning::{
    DEFAULT_V1_FIXTURE_HEX, SPLIT_TONING_BALANCE_HISTORY_MAX, SPLIT_TONING_BALANCE_HISTORY_MIN,
    SPLIT_TONING_BALANCE_UI_MAX, SPLIT_TONING_BALANCE_UI_MIN, SPLIT_TONING_METADATA,
    SPLIT_TONING_PARAMETER_BYTES, SplitToningError, SplitToningFormat, SplitToningHistory,
    SplitToningParametersV1, SplitToningPlan, SplitToningRaster,
};

const AUTHENTIC_PLATINOTYPE_FIXTURE_HEX: &str =
    include_str!("../src/operations/splittoning/fixtures/authentic_platinotype_v1.hex");

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
fn native_fixture_and_opaque_history_round_trip() {
    let bytes = decode_hex(DEFAULT_V1_FIXTURE_HEX);
    assert_eq!(bytes.len(), SPLIT_TONING_PARAMETER_BYTES);
    let history = SplitToningHistory::decode(1, &bytes).expect("decode native v1");
    assert_eq!(
        history.current().expect("current v1"),
        SplitToningParametersV1::defaults()
    );
    assert_eq!(history.payload().expect("encode native v1"), bytes);
    assert_eq!(
        SplitToningHistory::decode(1, &[0; 20]),
        Err(SplitToningError::InvalidPayloadLength {
            expected: 24,
            actual: 20,
        })
    );

    let unknown_bytes = [1_u8, 3, 3, 7, 0, 9];
    let unknown =
        SplitToningHistory::decode(12, &unknown_bytes).expect("retain unknown history bytes");
    assert_eq!(unknown.version(), 12);
    assert_eq!(
        unknown.payload().expect("clone unknown bytes"),
        unknown_bytes
    );
    assert_eq!(unknown.current(), Err(SplitToningError::OpaqueVersion(12)));
}

#[test]
fn authentic_platinotype_fixture_admits_balance_100_and_tones_all_shadows() {
    assert_eq!(SPLIT_TONING_BALANCE_UI_MIN, 0.0);
    assert_eq!(SPLIT_TONING_BALANCE_UI_MAX, 1.0);
    assert_eq!(SPLIT_TONING_BALANCE_HISTORY_MIN, 0.0);
    assert_eq!(SPLIT_TONING_BALANCE_HISTORY_MAX, 100.0);

    let bytes = decode_hex(AUTHENTIC_PLATINOTYPE_FIXTURE_HEX);
    let parameters = SplitToningParametersV1::from_bytes(&bytes).expect("platinotype fixture");
    assert_eq!(parameters.balance, 100.0);
    let plan = SplitToningPlan::compile(parameters).expect("admit native preset");
    let input = [0.5_f32, 0.5, 0.5, 0.73];
    let output = plan
        .execute(
            SplitToningRaster::new(&input, 1, 1, SplitToningFormat::RgbaF32x4),
            input.len() * size_of::<f32>(),
            || false,
        )
        .expect("execute all-shadow branch");

    let bits = output
        .iter()
        .map(|value| value.to_bits())
        .collect::<Vec<_>>();
    assert_eq!(bits, [0x3f35_c28f, 0x3ec3_126f, 0x3e94_7ae2, 0]);
}

#[test]
fn metadata_and_source_map_keep_shared_surfaces_deferred() {
    assert_eq!(SPLIT_TONING_METADATA.parameter_version, 1);
    assert!(!SPLIT_TONING_METADATA.default_enabled);
    assert_eq!(SPLIT_TONING_METADATA.default_colorspace, "rgb");
    assert!(SPLIT_TONING_METADATA.allow_tiling);
    assert!(SPLIT_TONING_METADATA.supports_shared_blending_native);
    assert!(!SPLIT_TONING_METADATA.shared_blending_integrated);
    assert_eq!(SPLIT_TONING_METADATA.legacy_order, 62.0);
    assert_eq!(SPLIT_TONING_METADATA.v50_raw_order, 67.0);
    assert_eq!(SPLIT_TONING_METADATA.v50_jpeg_order, 67.0);
    assert_eq!(SPLIT_TONING_METADATA.generated_inventory_order, 99);

    let source_map =
        include_str!("../../../architecture/rusttable-splittoning-cpu-source-map.toml");
    assert!(source_map.contains("production_registration = \"deferred"));
    assert!(source_map.contains("no fictitious v2 is authored"));
    assert!(source_map.contains("UI annotation displays balance over 0..=1"));
    assert!(source_map.contains("authentic platinotype balance=100.0f"));
    for deferred in [
        "shared masks/blending",
        "GPU",
        "GTK/color-picker UI",
        "history import",
    ] {
        assert!(
            source_map.contains(deferred),
            "missing deferred responsibility: {deferred}"
        );
    }
}

#[test]
fn cpu_preserves_shadow_midtone_and_native_fourth_lane_semantics() {
    let plan = SplitToningPlan::compile(SplitToningParametersV1::defaults())
        .expect("compile default plan");
    let input = [
        0.2_f32, 0.2, 0.2, 0.8, // shadow
        0.5, 0.5, 0.5, 0.8, // protected mid-tone
        0.8, 0.8, 0.8, 0.8, // highlight
    ];
    let output = plan
        .execute(
            SplitToningRaster::new(&input, 3, 1, SplitToningFormat::RgbaF32x4),
            input.len() * size_of::<f32>(),
            || false,
        )
        .expect("execute default plan");

    // Default compress maps to 0.15, so l=0.2 receives a 0.3 shadow mix.
    for (actual, expected) in output[..4].iter().zip([0.23_f32, 0.17, 0.17, 0.56]) {
        assert!(
            (*actual - expected).abs() <= 2.0e-6,
            "{actual} != {expected}"
        );
    }
    assert_eq!(&output[4..8], &input[4..8]);
    assert!(output[9] > output[8]);
    assert!(output[8] > output[10]);
    assert!((output[11] - 0.56_f32).abs() <= 2.0e-6);
}

#[test]
fn checked_execution_is_transactional_on_every_failure_class() {
    assert_eq!(
        SplitToningPlan::compile(SplitToningParametersV1::new(
            f32::NAN,
            0.5,
            0.2,
            0.5,
            0.5,
            33.0,
        )),
        Err(SplitToningError::NonFiniteParameter("shadow_hue"))
    );
    assert_eq!(
        SplitToningPlan::compile(SplitToningParametersV1::new(0.0, 0.5, 0.2, 0.5, 0.5, 101.0,)),
        Err(SplitToningError::ParameterOutOfRange("compress"))
    );
    assert_eq!(
        SplitToningPlan::compile(SplitToningParametersV1::new(0.0, 0.5, 0.2, 0.5, 100.1, 0.0)),
        Err(SplitToningError::ParameterOutOfRange("balance"))
    );
    assert_eq!(
        SplitToningPlan::compile(SplitToningParametersV1::new(
            0.0,
            0.5,
            0.2,
            0.5,
            f32::NAN,
            0.0
        )),
        Err(SplitToningError::NonFiniteParameter("balance"))
    );

    let plan = SplitToningPlan::compile(SplitToningParametersV1::defaults())
        .expect("compile default plan");
    let valid = [0.2_f32, 0.2, 0.2, 0.8];
    assert_eq!(
        plan.execute(
            SplitToningRaster::new(&valid[..3], 1, 1, SplitToningFormat::RgbF32x3),
            usize::MAX,
            || false,
        ),
        Err(SplitToningError::UnsupportedFormat)
    );
    assert_eq!(
        plan.execute(
            SplitToningRaster::new(&valid[..3], 1, 1, SplitToningFormat::RgbaF32x4),
            usize::MAX,
            || false,
        ),
        Err(SplitToningError::InputLengthMismatch {
            expected: 4,
            actual: 3,
        })
    );
    assert_eq!(
        plan.execute(
            SplitToningRaster::new(&valid, 1, 1, SplitToningFormat::RgbaF32x4),
            15,
            || false,
        ),
        Err(SplitToningError::OutputMemoryBudgetExceeded {
            required: 16,
            budget: 15,
        })
    );
    let invalid = [0.2_f32, f32::NAN, 0.2, 0.8];
    assert_eq!(
        plan.execute(
            SplitToningRaster::new(&invalid, 1, 1, SplitToningFormat::RgbaF32x4),
            usize::MAX,
            || false,
        ),
        Err(SplitToningError::NonFiniteInput { index: 1 })
    );

    let many = vec![0.2_f32; 257 * 4];
    let polls = Cell::new(0_u32);
    let mut destination = vec![6.0_f32, 5.0, 4.0];
    let error = plan
        .execute_and_publish(
            SplitToningRaster::new(&many, 257, 1, SplitToningFormat::RgbaF32x4),
            &mut destination,
            usize::MAX,
            || {
                let next = polls.get() + 1;
                polls.set(next);
                next >= 3
            },
        )
        .expect_err("cancel before publication");
    assert_eq!(error, SplitToningError::Cancelled);
    assert_eq!(destination, [6.0_f32, 5.0, 4.0]);
}
