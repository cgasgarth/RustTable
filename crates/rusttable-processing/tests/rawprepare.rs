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
    migrate_native_v1_to_v2,
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
        "v1" => include_str!("fixtures/rawprepare/v1.hex"),
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

const fn bayer_metadata(
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
    assert_eq!(RAWPREPARE_NATIVE_V1_PARAMETER_BYTES, 28);
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
    let native_bytes: [u8; RAWPREPARE_NATIVE_V1_PARAMETER_BYTES] =
        fixture("v1").try_into().expect("native v1 fixture size");
    let native = RawPrepareParametersV1::from_native_bytes(&native_bytes).expect("native v1");
    assert!(RawPrepareParametersV1::from_bytes(&native_bytes[..24]).is_err());
    assert_eq!(native.raw_white_point(), 777);
    assert_eq!(native.to_native_bytes(), native_bytes);

    let migrated_native = migrate_native_v1_to_v2(&native_bytes).expect("native migration");
    assert_eq!(
        migrated_native,
        [
            0xfb, 0xff, 0xff, 0xff, 0x02, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x04, 0x00,
            0x00, 0x00, 0x0b, 0x00, 0x16, 0x00, 0x21, 0x00, 0x2c, 0x00, 0x09, 0x03, 0xde, 0xad,
            0x00, 0x00, 0x00, 0x00,
        ]
    );
    let migrated_native =
        RawPrepareParametersV2::from_native_bytes(&migrated_native).expect("native v2");
    assert_eq!(migrated_native.left(), -5);
    assert_eq!(migrated_native.raw_black_level_separate(), [11, 22, 33, 44]);
    assert_eq!(migrated_native.raw_white_point(), 777);
    assert_eq!(migrated_native.flat_field(), RawPrepareFlatField::Off);

    let mut v1_bytes = vec![0_u8; RAWPREPARE_HISTORY_V1_PARAMETER_BYTES];
    v1_bytes[..native_bytes.len()].copy_from_slice(&native_bytes);
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
fn rawprepare_route_maps_only_the_supported_source_shape() {
    let dimensions = RasterDimensions::new(4, 4).expect("dimensions");
    let crop = RawPrepareCrop::new(0, 0, 0, 0);
    let raw = bayer_metadata(
        dimensions,
        DT_IMAGE_RAW,
        RawPrepareSampleFormat::U16,
        1,
        crop,
    );
    assert_eq!(
        rawprepare::rawprepare_route(&raw),
        rawprepare::RawPrepareRoute::RawPrepare(rawprepare::RawPrepareSourceOperation::RawPrepare,)
    );
    assert_eq!(
        rawprepare::RAWPREPARE_SOURCE_REGISTRATION.operation,
        rawprepare::RawPrepareSourceOperation::RawPrepare
    );

    let sraw = RawPrepareImageMetadata::new(
        dimensions,
        DT_IMAGE_S_RAW,
        RawPrepareSampleFormat::F32,
        4,
        RawPrepareCfa::None,
        crop,
        [0; 4],
        1000,
    );
    assert_eq!(
        rawprepare::rawprepare_route(&sraw),
        rawprepare::RawPrepareRoute::Rejected(rawprepare::RawPrepareRouteRejection::Sraw,)
    );

    let float_raw = bayer_metadata(
        dimensions,
        DT_IMAGE_RAW,
        RawPrepareSampleFormat::F32,
        1,
        crop,
    );
    assert_eq!(
        rawprepare::rawprepare_route(&float_raw),
        rawprepare::RawPrepareRoute::Rejected(rawprepare::RawPrepareRouteRejection::FloatRaw,)
    );
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
fn four_channel_hdr_normalized_inputs_fail_closed_before_cpu_normalization() {
    let dimensions = RasterDimensions::new(2, 2).expect("dimensions");
    let crop = RawPrepareCrop::new(0, 0, 0, 0);
    let parameters = params(crop, [0; 4], 1000);
    for raw_white_point in [1, 0x3F80_0000] {
        let metadata = RawPrepareImageMetadata::new(
            dimensions,
            DT_IMAGE_S_RAW | DT_IMAGE_HDR,
            RawPrepareSampleFormat::F32,
            4,
            RawPrepareCfa::None,
            crop,
            [0; 4],
            raw_white_point,
        );
        assert!(matches!(
            RawPreparePlan::new(&metadata, &parameters),
            Err(RawPrepareError::AlreadyNormalized)
        ));
    }
}

#[test]
fn raw_and_sraw_layouts_follow_distinct_native_flag_branches() {
    let dimensions = RasterDimensions::new(2, 2).expect("dimensions");
    let crop = RawPrepareCrop::new(0, 0, 0, 0);
    let parameters = params(crop, [0; 4], 1000);
    let raw = bayer_metadata(
        dimensions,
        DT_IMAGE_RAW,
        RawPrepareSampleFormat::U16,
        1,
        crop,
    );
    assert_eq!(
        RawPreparePlan::new(&raw, &parameters)
            .expect("RAW branch")
            .input_kind(),
        rawprepare::RawPrepareInputKind::MosaicU16
    );

    let sraw = RawPrepareImageMetadata::new(
        dimensions,
        DT_IMAGE_S_RAW,
        RawPrepareSampleFormat::F32,
        4,
        RawPrepareCfa::None,
        crop,
        [0; 4],
        1000,
    );
    assert_eq!(
        RawPreparePlan::new(&sraw, &parameters)
            .expect("SRAW branch")
            .input_kind(),
        rawprepare::RawPrepareInputKind::FourChannelF32
    );

    let impossible_flags = RawPrepareImageMetadata::new(
        dimensions,
        DT_IMAGE_RAW | DT_IMAGE_S_RAW,
        RawPrepareSampleFormat::U16,
        1,
        RawPrepareCfa::bayer(0x1234, 0, 0),
        crop,
        [0; 4],
        1000,
    );
    assert!(matches!(
        RawPreparePlan::new(&impossible_flags, &parameters),
        Err(RawPrepareError::UnsupportedLayout)
    ));

    let raw_flag_four_channel = RawPrepareImageMetadata::new(
        dimensions,
        DT_IMAGE_RAW,
        RawPrepareSampleFormat::F32,
        4,
        RawPrepareCfa::None,
        crop,
        [0; 4],
        1000,
    );
    assert!(matches!(
        RawPreparePlan::new(&raw_flag_four_channel, &parameters),
        Err(RawPrepareError::UnsupportedLayout)
    ));

    let sraw_flag_mosaic = bayer_metadata(
        dimensions,
        DT_IMAGE_S_RAW,
        RawPrepareSampleFormat::U16,
        1,
        crop,
    );
    assert!(matches!(
        RawPreparePlan::new(&sraw_flag_mosaic, &parameters),
        Err(RawPrepareError::UnsupportedLayout)
    ));
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
    .with_gain_maps(maps);
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
fn gain_map_coordinates_use_scaled_full_buffer_dimensions_across_tiles() {
    let plan_dimensions = RasterDimensions::new(8, 8).expect("plan dimensions");
    let maps = (0..4)
        .map(|filter| {
            RawPrepareGainMap::new(
                (filter >> 1) as u32,
                (filter & 1) as u32,
                8,
                8,
                2,
                2,
                1.0,
                1.0,
                0.0,
                0.0,
                vec![1.0, 3.0, 1.0, 3.0],
            )
        })
        .collect::<Vec<_>>();
    let crop = RawPrepareCrop::new(0, 0, 0, 0);
    let metadata = bayer_metadata(
        plan_dimensions,
        DT_IMAGE_RAW,
        RawPrepareSampleFormat::U16,
        1,
        crop,
    )
    .with_gain_maps(maps);
    let parameters =
        RawPrepareParametersV2::new(0, 0, 0, 0, [0; 4], 1000, RawPrepareFlatField::Embedded);
    let plan = RawPreparePlan::new(&metadata, &parameters).expect("gain-map plan");

    let scaled_full_input = RasterDimensions::new(4, 4).expect("scaled full input");
    let full_tile = rawprepare::RawPrepareTile::new_with_full_input(
        scaled_full_input,
        scaled_full_input,
        0,
        0,
        scaled_full_input,
        1.0,
        1.0,
        crop,
    )
    .expect("scaled full tile");
    let full_output = plan
        .execute_u16(&[1000; 16], full_tile, || false)
        .expect("full-frame execute");
    let tile = rawprepare::RawPrepareTile::new_with_full_input(
        RasterDimensions::new(2, 2).expect("tile input"),
        scaled_full_input,
        2,
        2,
        RasterDimensions::new(2, 2).expect("tile output"),
        1.0,
        1.0,
        crop,
    )
    .expect("non-origin tile");
    let output = plan
        .execute_u16(&[1000; 4], tile, || false)
        .expect("execute");

    for y in 0..2 {
        for x in 0..2 {
            let tile_index = y * 2 + x;
            let full_index = (y + 2) * 4 + x + 2;
            assert!((output[tile_index] - full_output[full_index]).abs() < 1e-6);
        }
    }
    // The non-origin tile uses the full scaled 4-pixel buffer for
    // interpolation: global x=2 maps to 0.5, giving a gain of 2 rather than
    // the tile-local coordinate 1.0 and gain 3.
    assert!((output[0] - 2.0).abs() < 1e-6);
    assert!((output[1] - 2.5).abs() < 1e-6);
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
    // The scalar ImagePredicate is the truthful fail-closed RAW branch;
    // SRAW's native 4->4 shape stays deferred until a union contract exists.
    assert_eq!(descriptor.io.input.channels, 1);
    assert_eq!(descriptor.io.output.channels, 1);
    assert_eq!(
        descriptor.capability.required_features,
        vec!["raw-image-metadata".to_owned()]
    );
    assert_eq!(
        descriptor.capability.required_formats,
        vec!["raw-u16x1".to_owned(), "raw-f32x1".to_owned()]
    );
    let capabilities = capabilities();
    assert!(capabilities.cpu);
    assert!(!capabilities.gpu);
    assert!(!capabilities.import_materialization);
    assert!(!capabilities.production_routing);
    assert!(capabilities.source_routing);
    assert!(!capabilities.ui);
}
