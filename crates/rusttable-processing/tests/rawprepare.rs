#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::similar_names
)]

// The raw operation remains deliberately absent from shared registry and
// pixelpipe hubs. Compile the operation-local leaf directly for its focused
// contract tests.
#[path = "../src/operations/rawprepare/mod.rs"]
mod rawprepare;

use std::cell::Cell;

use rawprepare::codec::{
    RAWPREPARE_HISTORY_V1_PARAMETER_BYTES, RAWPREPARE_HISTORY_V2_PARAMETER_BYTES,
    RAWPREPARE_NATIVE_V1_PARAMETER_BYTES, RAWPREPARE_NATIVE_V2_PARAMETER_BYTES,
    RawPrepareFlatField, RawPrepareHistory, RawPrepareParametersV1, RawPrepareParametersV2,
};
use rawprepare::execution::{
    DT_IMAGE_HDR, DT_IMAGE_RAW, DT_IMAGE_S_RAW, RawPrepareCfa, RawPrepareCrop, RawPrepareError,
    RawPrepareGainMap, RawPrepareImageMetadata, RawPrepareMemoryBudget, RawPreparePlan,
    RawPrepareSampleFormat,
};
use rawprepare::{RAWPREPARE_RUST_ID, capabilities, rawprepare_descriptor};
use rusttable_processing::RasterDimensions;

fn fixture(name: &str) -> Vec<u8> {
    let source = match name {
        "v2" => include_str!("fixtures/rawprepare/v2.hex"),
        _ => panic!("unknown rawprepare fixture"),
    };
    source
        .split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).expect("hex fixture"))
        .collect()
}

fn params(crop: RawPrepareCrop, black: [u16; 4], white: u16) -> RawPrepareParametersV2 {
    RawPrepareParametersV2::new(
        crop.left(),
        crop.top(),
        crop.right(),
        crop.bottom(),
        black,
        white,
        RawPrepareFlatField::Off,
    )
}

fn bayer_metadata(
    dimensions: RasterDimensions,
    flags: u32,
    format: RawPrepareSampleFormat,
    channels: u8,
    crop: RawPrepareCrop,
) -> RawPrepareImageMetadata {
    RawPrepareImageMetadata::new(
        dimensions,
        flags,
        format,
        channels,
        RawPrepareCfa::bayer(0x1234, 0, 0),
        crop,
        [0, 100, 200, 300],
        1000,
    )
}

#[test]
fn native_and_history_abi_sizes_are_distinct_and_v2_tail_round_trips() {
    assert_eq!(RAWPREPARE_NATIVE_V1_PARAMETER_BYTES, 24);
    assert_eq!(RAWPREPARE_NATIVE_V2_PARAMETER_BYTES, 32);
    assert_eq!(RAWPREPARE_HISTORY_V1_PARAMETER_BYTES, 296);
    assert_eq!(RAWPREPARE_HISTORY_V2_PARAMETER_BYTES, 40);

    let bytes = fixture("v2");
    let v2 = RawPrepareParametersV2::from_bytes(&bytes).expect("v2 fixture");
    assert_eq!(v2.left(), 1);
    assert_eq!(v2.top(), 2);
    assert_eq!(v2.right(), 3);
    assert_eq!(v2.bottom(), 4);
    assert_eq!(v2.raw_black_level_separate(), [100, 200, 300, 400]);
    assert_eq!(v2.raw_white_point(), 4000);
    assert_eq!(v2.flat_field(), RawPrepareFlatField::Embedded);
    assert_eq!(
        v2.opaque_tail(),
        &[0xde, 0xad, 0xbe, 0xef, 0, 0x11, 0x22, 0x33]
    );
    assert_eq!(v2.to_bytes(), bytes.as_slice());
}

#[test]
fn v1_migration_copies_native_fields_and_forces_flat_field_off() {
    let mut v1_bytes = vec![0_u8; RAWPREPARE_HISTORY_V1_PARAMETER_BYTES];
    v1_bytes[0..4].copy_from_slice(&(-5_i32).to_le_bytes());
    v1_bytes[4..8].copy_from_slice(&2_i32.to_le_bytes());
    v1_bytes[8..12].copy_from_slice(&3_i32.to_le_bytes());
    v1_bytes[12..16].copy_from_slice(&4_i32.to_le_bytes());
    for (index, value) in [11_u16, 22, 33, 44].into_iter().enumerate() {
        let offset = 16 + index * 2;
        v1_bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    v1_bytes[24..26].copy_from_slice(&777_u16.to_le_bytes());
    v1_bytes[280..].fill(0xa5);
    let history = RawPrepareHistory::decode(1, &v1_bytes).expect("v1 history");
    let migrated = history.migrate_to_v2().expect("migration");
    assert_eq!(migrated.left(), -5);
    assert_eq!(migrated.top(), 2);
    assert_eq!(migrated.raw_black_level_separate(), [11, 22, 33, 44]);
    assert_eq!(migrated.raw_white_point(), 777);
    assert_eq!(migrated.flat_field(), RawPrepareFlatField::Off);
    assert!(migrated.opaque_tail().iter().all(|byte| *byte == 0));

    let v1 = RawPrepareParametersV1::from_bytes(&v1_bytes).expect("v1 payload");
    assert_eq!(v1.to_bytes(), v1_bytes.as_slice());
}

#[test]
fn unknown_history_is_opaque_and_cannot_execute() {
    let history = RawPrepareHistory::decode(99, &[1, 2, 3]).expect("opaque version");
    assert_eq!(history.version(), 99);
    assert_eq!(history.payload(), vec![1, 2, 3]);
    assert!(history.migrate_to_v2().is_err());
}

#[test]
fn bayer_crop_normalization_uses_source_parity_and_publishes_phase() {
    let crop = RawPrepareCrop::new(1, 1, 0, 0);
    let metadata = bayer_metadata(
        RasterDimensions::new(4, 4).expect("dimensions"),
        DT_IMAGE_RAW,
        RawPrepareSampleFormat::U16,
        1,
        crop,
    );
    let plan =
        RawPreparePlan::new(&metadata, &params(crop, [0, 100, 200, 300], 1000)).expect("plan");
    assert_eq!(
        plan.output_dimensions(),
        RasterDimensions::new(3, 3).unwrap()
    );
    assert_eq!(plan.output_cfa(), RawPrepareCfa::bayer(0x1234, 1, 1));

    let tile = plan.full_frame_tile().expect("full frame");
    let input: Vec<u16> = (0..16).collect();
    let output = plan.execute_u16(&input, tile, || false).expect("execute");
    // Output (0,0) refers to source (1,1), so `_BL` selects filter 3.
    assert_eq!(output[0], (5.0 - 300.0) / 700.0);
    // Output (1,0) refers to source (2,1), so `_BL` selects filter 2.
    assert_eq!(output[1], (6.0 - 200.0) / 800.0);
}

#[test]
fn xtrans_crop_rotates_the_native_six_by_six_table() {
    let mut pattern = [[0_u8; 6]; 6];
    for (y, row) in pattern.iter_mut().enumerate() {
        for (x, value) in row.iter_mut().enumerate() {
            *value = (y * 6 + x) as u8;
        }
    }
    let cfa = RawPrepareCfa::xtrans(pattern, 0, 0);
    let shifted = cfa.xtrans_table_after_crop(2, 3).expect("xtrans table");
    assert_eq!(shifted[0][0], pattern[3][2]);
    assert_eq!(shifted[5][5], pattern[2][1]);
    assert_eq!(cfa.after_crop(2, 3), RawPrepareCfa::xtrans(pattern, 2, 3));
}

#[test]
fn four_channel_cpu_path_normalizes_fourth_lane_in_source_order() {
    let dimensions = RasterDimensions::new(2, 2).expect("dimensions");
    let crop = RawPrepareCrop::new(0, 0, 0, 0);
    let metadata = RawPrepareImageMetadata::new(
        dimensions,
        DT_IMAGE_S_RAW,
        RawPrepareSampleFormat::F32,
        4,
        RawPrepareCfa::None,
        crop,
        [1000; 4],
        50000,
    );
    let plan = RawPreparePlan::new(&metadata, &params(crop, [1000; 4], 50000)).expect("plan");
    let input = [
        [0.5, 0.6, 0.7, 0.8],
        [0.1, 0.2, 0.3, 0.4],
        [0.2, 0.3, 0.4, 0.5],
        [0.3, 0.4, 0.5, 0.6],
    ];
    let output = plan
        .execute_four_channel(&input, plan.full_frame_tile().unwrap(), || false)
        .expect("execute");
    let black = 1000.0 / 65535.0;
    let white = 50000.0 / 65535.0;
    let divisor = white - black;
    assert_eq!(
        plan.alpha_behavior(),
        rawprepare::RawPrepareAlphaBehavior::CpuNormalizesFourthChannel
    );
    assert!((output[0][3] - (0.8 - black) / divisor).abs() < 1e-6);
    assert_ne!(output[0][3], input[0][3]);
    assert_eq!(
        RawPreparePlan::gpu_alpha_behavior(),
        rawprepare::RawPrepareAlphaBehavior::GpuFourthChannelPreservedDeferred
    );
}

#[test]
fn valid_bayer_gain_maps_apply_bilinear_gain_and_malformed_maps_fail_closed() {
    let dimensions = RasterDimensions::new(4, 4).expect("dimensions");
    let maps = (0..4)
        .map(|filter| {
            RawPrepareGainMap::new(
                (filter >> 1) as u32,
                (filter & 1) as u32,
                4,
                4,
                2,
                2,
                1.0,
                1.0,
                0.0,
                0.0,
                vec![2.0; 4],
            )
        })
        .collect::<Vec<_>>();
    let crop = RawPrepareCrop::new(0, 0, 0, 0);
    let metadata = bayer_metadata(
        dimensions,
        DT_IMAGE_RAW,
        RawPrepareSampleFormat::U16,
        1,
        crop,
    )
    .with_gain_maps(maps.clone());
    let parameters =
        RawPrepareParametersV2::new(0, 0, 0, 0, [0; 4], 1000, RawPrepareFlatField::Embedded);
    let plan = RawPreparePlan::new(&metadata, &parameters).expect("gain-map plan");
    let output = plan
        .execute_u16(&[1000; 16], plan.full_frame_tile().unwrap(), || false)
        .expect("execute");
    assert!(output.iter().all(|value| (*value - 2.0).abs() < 1e-6));

    let malformed = RawPrepareGainMap::new(0, 0, 4, 4, 1, 2, 1.0, 1.0, 0.0, 0.0, vec![1.0, 1.0]);
    assert!(
        rawprepare::RawPrepareGainMapSet::try_new(
            dimensions,
            &[
                malformed.clone(),
                malformed.clone(),
                malformed.clone(),
                malformed
            ]
        )
        .is_err()
    );
}

#[test]
fn unsupported_and_normalized_inputs_fail_closed() {
    let crop = RawPrepareCrop::new(0, 0, 0, 0);
    let dimensions = RasterDimensions::new(4, 4).expect("dimensions");
    let parameters = params(crop, [0; 4], 1000);
    let jpeg = bayer_metadata(dimensions, 0, RawPrepareSampleFormat::U16, 1, crop);
    assert!(matches!(
        RawPreparePlan::new(&jpeg, &parameters),
        Err(RawPrepareError::UnsupportedCamera)
    ));
    let normalized = bayer_metadata(
        dimensions,
        DT_IMAGE_RAW,
        RawPrepareSampleFormat::F32,
        1,
        crop,
    );
    assert!(matches!(
        RawPreparePlan::new(&normalized, &parameters),
        Err(RawPrepareError::AlreadyNormalized)
    ));
    let hdr_normalized = RawPrepareImageMetadata::new(
        dimensions,
        DT_IMAGE_RAW | DT_IMAGE_HDR,
        RawPrepareSampleFormat::F32,
        1,
        RawPrepareCfa::bayer(1, 0, 0),
        crop,
        [0; 4],
        1,
    );
    assert!(matches!(
        RawPreparePlan::new(&hdr_normalized, &parameters),
        Err(RawPrepareError::AlreadyNormalized)
    ));
}

#[test]
fn cancellation_and_memory_budget_never_publish_partial_output() {
    let crop = RawPrepareCrop::new(0, 0, 0, 0);
    let metadata = bayer_metadata(
        RasterDimensions::new(4, 4).expect("dimensions"),
        DT_IMAGE_RAW,
        RawPrepareSampleFormat::U16,
        1,
        crop,
    );
    let parameters = params(crop, [0; 4], 1000);
    let plan =
        RawPreparePlan::new_with_budget(&metadata, &parameters, RawPrepareMemoryBudget::new(4))
            .expect("plan");
    assert!(matches!(
        plan.execute_u16(&[1; 16], plan.full_frame_tile().unwrap(), || false),
        Err(RawPrepareError::MemoryBudgetExceeded { .. })
    ));

    let plan = RawPreparePlan::new(&metadata, &parameters).expect("unlimited plan");
    let cancelled = Cell::new(false);
    let error = plan.execute_u16(&[1; 16], plan.full_frame_tile().unwrap(), || {
        let was_cancelled = cancelled.get();
        cancelled.set(true);
        !was_cancelled
    });
    assert!(matches!(error, Err(RawPrepareError::Cancelled)));
}

#[test]
fn descriptor_and_capabilities_do_not_overclaim_integration() {
    let descriptor = rawprepare_descriptor();
    descriptor.validate().expect("descriptor");
    assert_eq!(descriptor.id.rust_id, RAWPREPARE_RUST_ID);
    let capabilities = capabilities();
    assert!(capabilities.cpu);
    assert!(!capabilities.gpu);
    assert!(!capabilities.import_materialization);
    assert!(!capabilities.production_routing);
    assert!(!capabilities.ui);
}
