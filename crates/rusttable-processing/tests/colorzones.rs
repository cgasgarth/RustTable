#![allow(
    clippy::cast_precision_loss,
    clippy::float_cmp,
    reason = "source-derived ABI and migration vectors intentionally preserve exact f32 values"
)]

use std::fmt::Write as _;

use rusttable_color::ColorEncoding;
use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
    PhotoId, Revision,
};
use rusttable_processing::operations::colorzones::{
    COLORZONES_CHANNELS, COLORZONES_COMPATIBILITY_ID, COLORZONES_DEFAULT_ENABLED,
    COLORZONES_LEGACY_BANDS, COLORZONES_MAX_NODES, COLORZONES_RUST_ID, COLORZONES_SCHEMA_VERSION,
    COLORZONES_V1_BANDS, COLORZONES_V1_PARAMETER_BYTES, COLORZONES_V2_PARAMETER_BYTES,
    COLORZONES_V3_PARAMETER_BYTES, COLORZONES_V4_PARAMETER_BYTES, COLORZONES_V5_PARAMETER_BYTES,
    ColorZonesChannel, ColorZonesCodecError, ColorZonesConfig, ColorZonesCurveType,
    ColorZonesHistory, ColorZonesMode, ColorZonesNode, ColorZonesParameterError,
    ColorZonesParametersV1, ColorZonesParametersV2, ColorZonesParametersV3, ColorZonesParametersV4,
    ColorZonesParametersV5, ColorZonesPlan, ColorZonesSplinesVersion, migrate_v1_to_v5,
    migrate_v2_to_v5, migrate_v3_to_v5, migrate_v4_to_v5,
};
use rusttable_processing::{
    CompiledPipeline, FiniteF32, LinearRgb, OperationCompileError, PipelineStepIndex,
    ProcessingOperation, ProcessingOperationKind, RasterDimensions, WorkingRgbImage,
    builtin_registry, colorzones_descriptor,
    descriptor::{OperationFlags, ParameterDefault, ParameterKind, RoiKind},
    evaluate,
};
use sha2::{Digest, Sha256};

fn decode_hex(source: &str) -> Vec<u8> {
    let compact: String = source
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect();
    let (chunks, remainder) = compact.as_bytes().as_chunks::<2>();
    let bytes = chunks
        .iter()
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).expect("fixture hex is ASCII"), 16)
                .expect("fixture contains checked hexadecimal")
        })
        .collect();
    assert!(
        remainder.is_empty(),
        "fixture must contain complete hexadecimal bytes"
    );
    bytes
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn zero_curves() -> [[ColorZonesNode; COLORZONES_MAX_NODES]; COLORZONES_CHANNELS] {
    [[ColorZonesNode::new(0.0, 0.0); COLORZONES_MAX_NODES]; COLORZONES_CHANNELS]
}

fn equalizer<const BANDS: usize>(
    channel_stride: f32,
    node_stride: f32,
    offset: f32,
) -> [[f32; BANDS]; COLORZONES_CHANNELS] {
    std::array::from_fn(|channel| {
        std::array::from_fn(|node| {
            offset + channel as f32 * channel_stride + node as f32 * node_stride
        })
    })
}

fn assert_bytes_4(bytes: &[u8], offset: usize, expected: [u8; 4]) {
    assert_eq!(
        &bytes[offset..offset + 4],
        expected.as_slice(),
        "unexpected word at native byte offset {offset}"
    );
}

fn assert_zero_tail(parameters: &ColorZonesParametersV5, active: usize) {
    for curve in &parameters.curves {
        for node in &curve[active..] {
            assert_eq!(node.x.to_bits(), 0, "inactive x tail must be canonical +0");
            assert_eq!(node.y.to_bits(), 0, "inactive y tail must be canonical +0");
        }
    }
}

fn assert_history_current(version: u16, payload: &[u8], expected: &ColorZonesParametersV5) {
    let history = ColorZonesHistory::decode(version, payload).expect("known native history");
    assert_eq!(history.version(), version);
    assert_eq!(history.payload().as_slice(), payload);
    assert_eq!(
        history
            .current()
            .expect("known history migrates")
            .to_bytes(),
        expected.to_bytes()
    );
}

fn expect_invalid_length<T>(
    result: Result<T, ColorZonesCodecError>,
    expected: usize,
    actual: usize,
) {
    match result {
        Err(ColorZonesCodecError::InvalidLength {
            expected: found_expected,
            actual: found_actual,
        }) => {
            assert_eq!(found_expected, expected);
            assert_eq!(found_actual, actual);
        }
        Err(other) => panic!("unexpected codec error: {other}"),
        Ok(_) => panic!("payload length {actual} unexpectedly decoded"),
    }
}

fn zero_bytes(length: usize) -> Vec<u8> {
    vec![0; length]
}

fn overwrite_i32(bytes: &mut [u8], offset: usize, value: i32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn overwrite_f32_bits(bytes: &mut [u8], offset: usize, bits: u32) {
    bytes[offset..offset + 4].copy_from_slice(&bits.to_le_bytes());
}

fn adversarial_words(length: usize) -> Vec<u8> {
    let words = [
        0x8000_0000_u32, // -0.0
        0x7fc1_2345,     // quiet NaN with payload
        0x7f80_0000,     // +infinity
        0xff80_0000,     // -infinity
        0x7fff_ffff,     // invalid enum/count when interpreted as i32
        0x3f12_3456,
    ];
    (0..length / 4)
        .flat_map(|index| words[index % words.len()].to_le_bytes())
        .collect()
}

#[test]
fn native_abi_sizes_offsets_and_little_endian_words_are_exact() {
    assert_eq!(COLORZONES_COMPATIBILITY_ID, "colorzones");
    assert_eq!(COLORZONES_RUST_ID, "rusttable.colorzones");
    assert_eq!(COLORZONES_SCHEMA_VERSION, 5);
    assert_eq!(COLORZONES_CHANNELS, 3);
    assert_eq!(COLORZONES_MAX_NODES, 20);
    assert_eq!(COLORZONES_V1_BANDS, 6);
    assert_eq!(COLORZONES_LEGACY_BANDS, 8);
    assert_eq!(COLORZONES_V1_PARAMETER_BYTES, 148);
    assert_eq!(COLORZONES_V2_PARAMETER_BYTES, 196);
    assert_eq!(COLORZONES_V3_PARAMETER_BYTES, 200);
    assert_eq!(COLORZONES_V4_PARAMETER_BYTES, 516);
    assert_eq!(COLORZONES_V5_PARAMETER_BYTES, 520);

    let mut v1_x = [[0.0; COLORZONES_V1_BANDS]; COLORZONES_CHANNELS];
    let mut v1_y = [[0.0; COLORZONES_V1_BANDS]; COLORZONES_CHANNELS];
    v1_x[0][0] = f32::from_bits(0x3f12_3456);
    v1_x[2][5] = f32::from_bits(0xbf65_4321);
    v1_y[0][0] = f32::from_bits(0x3eab_cdef);
    v1_y[2][5] = f32::from_bits(0xbe01_0203);
    let v1 = ColorZonesParametersV1::new(0x0102_0304, v1_x, v1_y).to_bytes();
    assert_eq!(v1.len(), COLORZONES_V1_PARAMETER_BYTES);
    assert_bytes_4(&v1, 0, [0x04, 0x03, 0x02, 0x01]);
    assert_bytes_4(&v1, 4, 0x3f12_3456_u32.to_le_bytes());
    assert_bytes_4(&v1, 72, 0xbf65_4321_u32.to_le_bytes());
    assert_bytes_4(&v1, 76, 0x3eab_cdef_u32.to_le_bytes());
    assert_bytes_4(&v1, 144, 0xbe01_0203_u32.to_le_bytes());

    let mut v2_x = [[0.0; COLORZONES_LEGACY_BANDS]; COLORZONES_CHANNELS];
    let mut v2_y = [[0.0; COLORZONES_LEGACY_BANDS]; COLORZONES_CHANNELS];
    v2_x[0][0] = f32::from_bits(0x3f21_4365);
    v2_x[2][7] = f32::from_bits(0xbf76_5432);
    v2_y[0][0] = f32::from_bits(0x3e12_efcd);
    v2_y[2][7] = f32::from_bits(0xbe03_0201);
    let v2_parameters = ColorZonesParametersV2::new(-0x0102_0304, v2_x, v2_y);
    let v2 = v2_parameters.to_bytes();
    assert_eq!(v2.len(), COLORZONES_V2_PARAMETER_BYTES);
    assert_bytes_4(&v2, 0, (-0x0102_0304_i32).to_le_bytes());
    assert_bytes_4(&v2, 4, 0x3f21_4365_u32.to_le_bytes());
    assert_bytes_4(&v2, 96, 0xbf76_5432_u32.to_le_bytes());
    assert_bytes_4(&v2, 100, 0x3e12_efcd_u32.to_le_bytes());
    assert_bytes_4(&v2, 192, 0xbe03_0201_u32.to_le_bytes());

    let v3 = ColorZonesParametersV3::new(
        v2_parameters.channel,
        v2_parameters.equalizer_x,
        v2_parameters.equalizer_y,
        f32::from_bits(0xc123_4567),
    )
    .to_bytes();
    assert_eq!(v3.len(), COLORZONES_V3_PARAMETER_BYTES);
    assert_bytes_4(&v3, 196, 0xc123_4567_u32.to_le_bytes());

    let mut curves = zero_curves();
    curves[0][0] = ColorZonesNode::new(f32::from_bits(0x3f10_2030), f32::from_bits(0x3f40_5060));
    curves[1][0].x = f32::from_bits(0xbf11_2233);
    curves[2][0].x = f32::from_bits(0x3e44_5566);
    curves[2][19].y = f32::from_bits(0xbe77_6655);
    let v4_parameters = ColorZonesParametersV4::new(
        2,
        curves,
        [2, 3, 20],
        [0, 1, 2],
        f32::from_bits(0xc042_1234),
        1,
    );
    let v4 = v4_parameters.to_bytes();
    assert_eq!(v4.len(), COLORZONES_V4_PARAMETER_BYTES);
    assert_bytes_4(&v4, 4, 0x3f10_2030_u32.to_le_bytes());
    assert_bytes_4(&v4, 8, 0x3f40_5060_u32.to_le_bytes());
    assert_bytes_4(&v4, 164, 0xbf11_2233_u32.to_le_bytes());
    assert_bytes_4(&v4, 324, 0x3e44_5566_u32.to_le_bytes());
    assert_bytes_4(&v4, 480, 0xbe77_6655_u32.to_le_bytes());
    assert_bytes_4(&v4, 484, 2_i32.to_le_bytes());
    assert_bytes_4(&v4, 492, 20_i32.to_le_bytes());
    assert_bytes_4(&v4, 496, 0_i32.to_le_bytes());
    assert_bytes_4(&v4, 504, 2_i32.to_le_bytes());
    assert_bytes_4(&v4, 508, 0xc042_1234_u32.to_le_bytes());
    assert_bytes_4(&v4, 512, 1_i32.to_le_bytes());

    let v5 = ColorZonesParametersV5::new(
        v4_parameters.channel,
        v4_parameters.curves,
        v4_parameters.curve_num_nodes,
        v4_parameters.curve_type,
        v4_parameters.strength,
        v4_parameters.mode,
        0x0102_0304,
    )
    .to_bytes();
    assert_eq!(v5.len(), COLORZONES_V5_PARAMETER_BYTES);
    assert_eq!(&v5[..COLORZONES_V4_PARAMETER_BYTES], v4.as_slice());
    assert_bytes_4(&v5, 516, [0x04, 0x03, 0x02, 0x01]);
}

#[test]
fn v5_defaults_match_hue_splines_v2_reset_and_checked_projection() {
    let defaults = ColorZonesParametersV5::defaults();
    assert_eq!(defaults.channel, 2);
    assert_eq!(defaults.curve_num_nodes, [2, 2, 2]);
    assert_eq!(defaults.curve_type, [1, 1, 1]);
    assert_eq!(defaults.strength.to_bits(), 0);
    assert_eq!(defaults.mode, 0);
    assert_eq!(defaults.splines_version, 1);
    for curve in &defaults.curves {
        assert_eq!(curve[0].x, 0.25);
        assert_eq!(curve[0].y, 0.5);
        assert_eq!(curve[1].x, 0.75);
        assert_eq!(curve[1].y, 0.5);
    }
    assert_zero_tail(&defaults, 2);
    assert_eq!(
        ColorZonesParametersV5::from_bytes(&defaults.to_bytes())
            .expect("native defaults decode")
            .to_bytes(),
        defaults.to_bytes()
    );

    let config = ColorZonesConfig::defaults();
    assert_eq!(config.channel(), ColorZonesChannel::Hue);
    assert_eq!(config.strength().to_bits(), 0);
    assert_eq!(config.mode(), ColorZonesMode::Smooth);
    assert_eq!(config.splines_version(), ColorZonesSplinesVersion::V2);
    for channel in [
        ColorZonesChannel::Lightness,
        ColorZonesChannel::Chroma,
        ColorZonesChannel::Hue,
    ] {
        let curve = config.curve(channel);
        assert_eq!(curve.curve_type(), ColorZonesCurveType::Catmull);
        assert_eq!(curve.node_count(), 2);
        assert_eq!(curve.points()[0].x(), 0.25);
        assert_eq!(curve.points()[0].y(), 0.5);
        assert_eq!(curve.points()[1].x(), 0.75);
        assert_eq!(curve.points()[1].y(), 0.5);
    }
    assert_eq!(config.curves().len(), COLORZONES_CHANNELS);
}

#[test]
fn v1_migration_duplicates_edges_offsets_inner_neighbors_and_zeroes_tail() {
    let x = equalizer::<COLORZONES_V1_BANDS>(0.2, 0.02, 0.1);
    let y = equalizer::<COLORZONES_V1_BANDS>(0.1, -0.01, 0.7);
    let source = ColorZonesParametersV1::new(1, x, y);
    let migrated = migrate_v1_to_v5(source);

    assert_eq!(migrated.channel, source.channel);
    assert_eq!(migrated.curve_num_nodes, [8, 8, 8]);
    assert_eq!(migrated.curve_type, [1, 1, 1]);
    assert_eq!(migrated.strength.to_bits(), 0);
    assert_eq!(migrated.mode, 0);
    assert_eq!(migrated.splines_version, 0);
    for channel in 0..COLORZONES_CHANNELS {
        let curve = &migrated.curves[channel];
        assert_eq!(curve[0].x, source.equalizer_x[channel][0]);
        assert_eq!(curve[0].y, source.equalizer_y[channel][0]);
        assert_eq!(curve[1].x, source.equalizer_x[channel][0] + 0.001_f32);
        assert_eq!(curve[1].y, source.equalizer_y[channel][0]);
        for node in 1..COLORZONES_V1_BANDS - 1 {
            assert_eq!(curve[node + 1].x, source.equalizer_x[channel][node]);
            assert_eq!(curve[node + 1].y, source.equalizer_y[channel][node]);
        }
        assert_eq!(
            curve[6].x,
            source.equalizer_x[channel][COLORZONES_V1_BANDS - 1] - 0.001_f32
        );
        assert_eq!(
            curve[6].y,
            source.equalizer_y[channel][COLORZONES_V1_BANDS - 1]
        );
        assert_eq!(
            curve[7].x,
            source.equalizer_x[channel][COLORZONES_V1_BANDS - 1]
        );
        assert_eq!(
            curve[7].y,
            source.equalizer_y[channel][COLORZONES_V1_BANDS - 1]
        );
    }
    assert_zero_tail(&migrated, COLORZONES_LEGACY_BANDS);
    assert_history_current(1, &source.to_bytes(), &migrated);
}

#[test]
fn v2_and_v3_migrations_transpose_no_axes_preserve_strength_and_zero_tail() {
    let x = equalizer::<COLORZONES_LEGACY_BANDS>(0.25, 0.015, -0.2);
    let y = equalizer::<COLORZONES_LEGACY_BANDS>(-0.2, 0.025, 0.8);

    let source_v2 = ColorZonesParametersV2::new(2, x, y);
    let migrated_v2 = migrate_v2_to_v5(source_v2);
    let negative_zero = f32::from_bits(0x8000_0000);
    let source_v3 = ColorZonesParametersV3::new(0, x, y, negative_zero);
    let migrated_v3 = migrate_v3_to_v5(source_v3);

    for (source_x, source_y, migrated) in [
        (&source_v2.equalizer_x, &source_v2.equalizer_y, &migrated_v2),
        (&source_v3.equalizer_x, &source_v3.equalizer_y, &migrated_v3),
    ] {
        assert_eq!(migrated.curve_num_nodes, [8, 8, 8]);
        assert_eq!(migrated.curve_type, [1, 1, 1]);
        assert_eq!(migrated.mode, 0);
        assert_eq!(migrated.splines_version, 0);
        for channel in 0..COLORZONES_CHANNELS {
            for node in 0..COLORZONES_LEGACY_BANDS {
                assert_eq!(migrated.curves[channel][node].x, source_x[channel][node]);
                assert_eq!(migrated.curves[channel][node].y, source_y[channel][node]);
            }
        }
        assert_zero_tail(migrated, COLORZONES_LEGACY_BANDS);
    }
    assert_eq!(migrated_v2.strength.to_bits(), 0);
    assert_eq!(migrated_v3.strength.to_bits(), negative_zero.to_bits());
    assert_history_current(2, &source_v2.to_bytes(), &migrated_v2);
    assert_history_current(3, &source_v3.to_bytes(), &migrated_v3);
}

#[test]
fn v4_migration_copies_all_516_bytes_and_appends_splines_v1() {
    let mut curves = zero_curves();
    for (channel, curve) in curves.iter_mut().enumerate() {
        for (node, point) in curve.iter_mut().enumerate() {
            point.x = channel as f32 * 10.0 + node as f32 * 0.25;
            point.y = 100.0 - channel as f32 * 5.0 - node as f32 * 0.5;
        }
    }
    curves[0][19].x = f32::from_bits(0x7fc1_2345);
    curves[1][18].y = f32::INFINITY;
    let source = ColorZonesParametersV4::new(
        1,
        curves,
        [2, 8, 20],
        [0, 1, 2],
        f32::from_bits(0x8000_0000),
        1,
    );
    let migrated = migrate_v4_to_v5(source);
    let source_bytes = source.to_bytes();
    let migrated_bytes = migrated.to_bytes();

    assert_eq!(
        &migrated_bytes[..COLORZONES_V4_PARAMETER_BYTES],
        source_bytes.as_slice()
    );
    assert_eq!(
        &migrated_bytes[COLORZONES_V4_PARAMETER_BYTES..],
        0_i32.to_le_bytes().as_slice()
    );
    assert_history_current(4, &source_bytes, &migrated);
}

#[test]
fn raw_codecs_round_trip_every_bit_before_semantic_validation() {
    macro_rules! assert_raw_round_trip {
        ($type:ty, $length:expr) => {{
            let bytes = adversarial_words($length);
            let decoded = <$type>::from_bytes(&bytes).expect("raw native payload");
            assert_eq!(decoded.to_bytes().as_slice(), bytes.as_slice());
        }};
    }

    assert_raw_round_trip!(ColorZonesParametersV1, COLORZONES_V1_PARAMETER_BYTES);
    assert_raw_round_trip!(ColorZonesParametersV2, COLORZONES_V2_PARAMETER_BYTES);
    assert_raw_round_trip!(ColorZonesParametersV3, COLORZONES_V3_PARAMETER_BYTES);
    assert_raw_round_trip!(ColorZonesParametersV4, COLORZONES_V4_PARAMETER_BYTES);

    let mut bytes = ColorZonesParametersV5::defaults().to_bytes();
    overwrite_i32(&mut bytes, 0, 73);
    overwrite_f32_bits(&mut bytes, 4, 0x8000_0000);
    overwrite_f32_bits(&mut bytes, 8, 0x7fc1_2345);
    overwrite_f32_bits(&mut bytes, 12, 0x7f80_0000);
    overwrite_i32(&mut bytes, 496, -17);
    overwrite_i32(&mut bytes, 512, 91);
    overwrite_i32(&mut bytes, 516, i32::MIN);
    let decoded = ColorZonesParametersV5::from_bytes(&bytes).expect("lossless raw v5 decode");
    assert_eq!(decoded.to_bytes(), bytes);
    assert_eq!(decoded.curves[0][0].x.to_bits(), 0x8000_0000);
    assert_eq!(decoded.curves[0][0].y.to_bits(), 0x7fc1_2345);
    assert_eq!(decoded.curves[0][1].x.to_bits(), 0x7f80_0000);
    assert_eq!(
        ColorZonesConfig::try_from(&decoded),
        Err(ColorZonesParameterError::InvalidEnum {
            parameter: "channel",
            value: 73,
        })
    );
    let history =
        ColorZonesHistory::decode(COLORZONES_SCHEMA_VERSION, &bytes).expect("raw v5 history");
    assert_eq!(history.payload().as_slice(), bytes.as_slice());
    assert_eq!(
        history.current().expect("v5 remains current").to_bytes(),
        bytes
    );
}

#[test]
fn semantic_projection_validates_only_active_nodes_and_never_ui_clamps() {
    let invalid_enums = [
        (
            {
                let mut parameters = ColorZonesParametersV5::defaults();
                parameters.channel = 9;
                parameters
            },
            ColorZonesParameterError::InvalidEnum {
                parameter: "channel",
                value: 9,
            },
        ),
        (
            {
                let mut parameters = ColorZonesParametersV5::defaults();
                parameters.mode = -7;
                parameters
            },
            ColorZonesParameterError::InvalidEnum {
                parameter: "mode",
                value: -7,
            },
        ),
        (
            {
                let mut parameters = ColorZonesParametersV5::defaults();
                parameters.splines_version = 7;
                parameters
            },
            ColorZonesParameterError::InvalidEnum {
                parameter: "splines_version",
                value: 7,
            },
        ),
    ];
    for (parameters, expected) in invalid_enums {
        assert_eq!(ColorZonesConfig::try_from(&parameters), Err(expected));
    }

    let mut invalid_curve_type = ColorZonesParametersV5::defaults();
    invalid_curve_type.curve_type[1] = 12;
    assert_eq!(
        ColorZonesConfig::try_from(&invalid_curve_type),
        Err(ColorZonesParameterError::InvalidCurveType {
            channel: 1,
            value: 12,
        })
    );

    for count in [0, 21, -1] {
        let mut parameters = ColorZonesParametersV5::defaults();
        parameters.curve_num_nodes[2] = count;
        assert_eq!(
            ColorZonesConfig::try_from(&parameters),
            Err(ColorZonesParameterError::InvalidNodeCount { channel: 2, count })
        );
    }
    let mut single_node_v2 = ColorZonesParametersV5::defaults();
    single_node_v2.curve_num_nodes = [1, 1, 1];
    let single_node_v2 = ColorZonesConfig::try_from(&single_node_v2)
        .expect("native spline v2 accepts a constant one-node curve");
    assert!(
        single_node_v2
            .curves()
            .iter()
            .all(|curve| curve.node_count() == 1)
    );

    let mut single_node_v1 = ColorZonesParametersV5::defaults();
    single_node_v1.splines_version = 0;
    single_node_v1.curve_num_nodes[2] = 1;
    assert_eq!(
        ColorZonesConfig::try_from(&single_node_v1),
        Err(ColorZonesParameterError::InvalidNodeCount {
            channel: 2,
            count: 1,
        })
    );

    let mut maximum_nodes = ColorZonesParametersV5::defaults();
    maximum_nodes.curve_num_nodes = [20, 20, 20];
    assert!(ColorZonesConfig::try_from(&maximum_nodes).is_ok());

    let mut invalid_active_x = ColorZonesParametersV5::defaults();
    invalid_active_x.curves[0][1].x = f32::from_bits(0x7fc1_2345);
    assert_eq!(
        ColorZonesConfig::try_from(&invalid_active_x),
        Err(ColorZonesParameterError::NonFiniteActiveNode {
            channel: 0,
            node: 1,
            coordinate: "x",
        })
    );
    let mut invalid_active_y = ColorZonesParametersV5::defaults();
    invalid_active_y.curves[2][0].y = f32::NEG_INFINITY;
    assert_eq!(
        ColorZonesConfig::try_from(&invalid_active_y),
        Err(ColorZonesParameterError::NonFiniteActiveNode {
            channel: 2,
            node: 0,
            coordinate: "y",
        })
    );
    let mut invalid_strength = ColorZonesParametersV5::defaults();
    invalid_strength.strength = f32::INFINITY;
    assert_eq!(
        ColorZonesConfig::try_from(&invalid_strength),
        Err(ColorZonesParameterError::NonFiniteStrength)
    );

    let mut ignored_tail = ColorZonesParametersV5::defaults();
    ignored_tail.curves[0][2] = ColorZonesNode::new(f32::NAN, f32::INFINITY);
    ignored_tail.curves[1][19] =
        ColorZonesNode::new(f32::NEG_INFINITY, f32::from_bits(0x7fc5_4321));
    let ignored_tail_bytes = ignored_tail.to_bytes();
    let ignored_tail =
        ColorZonesParametersV5::from_bytes(&ignored_tail_bytes).expect("raw inactive tails decode");
    assert_eq!(ignored_tail.to_bytes(), ignored_tail_bytes);
    assert!(ColorZonesConfig::try_from(&ignored_tail).is_ok());

    let mut unbounded = ColorZonesParametersV5::defaults();
    unbounded.curves[0][0] = ColorZonesNode::new(1_000.0, -20_000.0);
    unbounded.curves[0][1] = ColorZonesNode::new(-1_000.0, 20_000.0);
    unbounded.strength = -10_000.0;
    let config = ColorZonesConfig::try_from(&unbounded)
        .expect("finite persisted values are not range-checked or sorted");
    let lightness = config.curve(ColorZonesChannel::Lightness);
    assert_eq!(lightness.points()[0].x(), 1_000.0);
    assert_eq!(lightness.points()[0].y(), -20_000.0);
    assert_eq!(lightness.points()[1].x(), -1_000.0);
    assert_eq!(lightness.points()[1].y(), 20_000.0);
    assert_eq!(config.strength(), -10_000.0);
}

#[test]
fn every_known_version_requires_exact_length_and_unknown_history_is_opaque() {
    macro_rules! assert_exact_lengths {
        ($type:ty, $version:expr, $expected:expr) => {{
            let short = zero_bytes($expected - 1);
            let long = zero_bytes($expected + 1);
            expect_invalid_length(<$type>::from_bytes(&short), $expected, $expected - 1);
            expect_invalid_length(<$type>::from_bytes(&long), $expected, $expected + 1);
            expect_invalid_length(
                ColorZonesHistory::decode($version, &short),
                $expected,
                $expected - 1,
            );
            expect_invalid_length(
                ColorZonesHistory::decode($version, &long),
                $expected,
                $expected + 1,
            );
        }};
    }

    assert_exact_lengths!(ColorZonesParametersV1, 1, COLORZONES_V1_PARAMETER_BYTES);
    assert_exact_lengths!(ColorZonesParametersV2, 2, COLORZONES_V2_PARAMETER_BYTES);
    assert_exact_lengths!(ColorZonesParametersV3, 3, COLORZONES_V3_PARAMETER_BYTES);
    assert_exact_lengths!(ColorZonesParametersV4, 4, COLORZONES_V4_PARAMETER_BYTES);
    assert_exact_lengths!(
        ColorZonesParametersV5,
        COLORZONES_SCHEMA_VERSION,
        COLORZONES_V5_PARAMETER_BYTES
    );

    let opaque_bytes = [0x00, 0x80, 0xff, 0x7f, 0xde, 0xad, 0xbe, 0xef, 0x45, 0x23];
    let opaque =
        ColorZonesHistory::decode(77, &opaque_bytes).expect("future history remains opaque");
    assert_eq!(opaque.version(), 77);
    assert_eq!(opaque.payload().as_slice(), opaque_bytes.as_slice());
    assert_eq!(
        opaque.current(),
        Err(ColorZonesCodecError::UnsupportedVersion(77))
    );
}

#[test]
fn audited_benchmark_v5_fixtures_retain_exact_native_payloads() {
    let fixtures = [
        (
            include_str!("fixtures/colorzones-benchmark-3.4legacy-v5.hex"),
            "900b1fde6b7bb3eeef5d973a9378764b8e5853ad4dbcb418c36593efea287108",
        ),
        (
            include_str!("fixtures/colorzones-benchmark-4.2-v5.hex"),
            "2214a0605315ae2401c39be002784c359007e9b4c6bd625c2c276165eea75016",
        ),
    ];

    for (fixture, expected_sha256) in fixtures {
        let bytes = decode_hex(fixture);
        assert_eq!(bytes.len(), COLORZONES_V5_PARAMETER_BYTES);
        assert_eq!(sha256_hex(&bytes), expected_sha256);

        let parameters =
            ColorZonesParametersV5::from_bytes(&bytes).expect("audited native v5 fixture");
        assert_eq!(parameters.to_bytes().as_slice(), bytes.as_slice());
        assert_eq!(parameters.channel, 2);
        assert_eq!(parameters.curve_num_nodes, [2, 2, 2]);
        assert_eq!(parameters.curve_type, [1, 1, 1]);
        assert_eq!(parameters.strength.to_bits(), 0);
        assert_eq!(parameters.mode, 0);
        assert_eq!(parameters.splines_version, 1);
        assert!(ColorZonesConfig::try_from(&parameters).is_ok());

        let history =
            ColorZonesHistory::decode(COLORZONES_SCHEMA_VERSION, &bytes).expect("audited history");
        assert_eq!(history.version(), COLORZONES_SCHEMA_VERSION);
        assert_eq!(history.payload().as_slice(), bytes.as_slice());
        assert_eq!(
            history.current().expect("v5 is current").to_bytes(),
            parameters.to_bytes()
        );
    }
}

fn editable_operation(
    id: u128,
    enabled: bool,
    parameters: impl IntoIterator<Item = (ParameterName, ParameterValue)>,
) -> Operation {
    Operation::new(
        OperationId::new(id).expect("operation ID"),
        OperationKey::new(COLORZONES_RUST_ID).expect("Color Zones operation key"),
        enabled,
        parameters,
    )
    .expect("canonical Color Zones operation")
}

fn replacing_parameter(operation: &Operation, name: &str, value: ParameterValue) -> Operation {
    let name = ParameterName::new(name).expect("test parameter name");
    let mut replacement = Some(value);
    editable_operation(
        operation.id().get(),
        operation.is_enabled(),
        operation.parameters().map(|(candidate, current)| {
            if *candidate == name {
                (
                    candidate.clone(),
                    replacement.take().expect("parameter is replaced once"),
                )
            } else {
                (candidate.clone(), current.clone())
            }
        }),
    )
}

#[test]
fn descriptor_parameter_ids_follow_native_v5_declaration_order_exactly() {
    let mut expected = vec!["channel".to_owned()];
    for curve in 0..COLORZONES_CHANNELS {
        for node in 0..COLORZONES_MAX_NODES {
            expected.push(format!("curve_{curve}_node_{node}_x"));
            expected.push(format!("curve_{curve}_node_{node}_y"));
        }
    }
    expected.extend((0..COLORZONES_CHANNELS).map(|curve| format!("curve_{curve}_num_nodes")));
    expected.extend((0..COLORZONES_CHANNELS).map(|curve| format!("curve_{curve}_type")));
    expected.extend([
        "strength".to_owned(),
        "mode".to_owned(),
        "splines_version".to_owned(),
    ]);

    assert_eq!(
        colorzones_descriptor()
            .parameters
            .into_iter()
            .map(|parameter| parameter.id)
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn canonical_descriptor_and_cpu_registry_binding_match_native_v5_contract() {
    let descriptor = colorzones_descriptor();
    assert_eq!(
        descriptor.id.compatibility_name,
        COLORZONES_COMPATIBILITY_ID
    );
    assert_eq!(descriptor.id.rust_id, COLORZONES_RUST_ID);
    assert_eq!(descriptor.id.schema_version, COLORZONES_SCHEMA_VERSION);
    assert_eq!(descriptor.id.parameter_version, COLORZONES_SCHEMA_VERSION);
    const { assert!(!COLORZONES_DEFAULT_ENABLED) };
    assert_eq!(descriptor.roi, RoiKind::Identity);
    assert_eq!(descriptor.parameters.len(), 130);
    assert!(descriptor.flags.contains(OperationFlags::STYLE_ELIGIBLE));
    assert!(descriptor.flags.contains(OperationFlags::BLENDING));
    assert!(descriptor.flags.contains(OperationFlags::TILEABLE));
    assert!(descriptor.flags.contains(OperationFlags::DETERMINISTIC_CPU));
    assert!(!descriptor.flags.contains(OperationFlags::DETERMINISTIC_GPU));
    assert!(!descriptor.flags.contains(OperationFlags::MANDATORY));
    assert_eq!(descriptor.migration.source_versions, [1, 2, 3, 4, 5]);
    assert_eq!(descriptor.migration.target_version, 5);
    assert_eq!(descriptor.io.input.encodings, [ColorEncoding::LabD50]);

    let channel = descriptor
        .parameters
        .iter()
        .find(|parameter| parameter.id == "channel")
        .expect("selection channel descriptor");
    assert_eq!(
        channel.kind,
        ParameterKind::Enum {
            tags: vec![
                "lightness".to_owned(),
                "chroma".to_owned(),
                "hue".to_owned()
            ]
        }
    );
    assert_eq!(channel.default, ParameterDefault::Enum("hue".to_owned()));
    let first_x = descriptor
        .parameters
        .iter()
        .find(|parameter| parameter.id == "curve_0_node_0_x")
        .expect("first curve point descriptor");
    assert_eq!(first_x.default, ParameterDefault::Scalar(0.25));
    let inactive_x = descriptor
        .parameters
        .iter()
        .find(|parameter| parameter.id == "curve_2_node_19_x")
        .expect("inactive curve tail descriptor");
    assert_eq!(inactive_x.default, ParameterDefault::Scalar(0.0));

    let registry = builtin_registry();
    let definition = registry
        .definition(COLORZONES_RUST_ID)
        .expect("Color Zones registry definition");
    assert!(definition.availability().is_available());
    assert!(!definition.ui_availability().is_available());
    assert!(definition.cpu().is_some());
    assert!(definition.gpu().is_none());
    assert_eq!(
        definition
            .migrations()
            .iter()
            .map(|migration| (migration.from_version(), migration.to_version()))
            .collect::<Vec<_>>(),
        [(1, 5), (2, 5), (3, 5), (4, 5)]
    );
    assert!(definition.evidence_ids().len() >= 8);
    assert!(
        registry
            .capability(
                COLORZONES_RUST_ID,
                &rusttable_processing::DeviceCapabilitySnapshot::cpu_only(),
                ColorEncoding::LabD50,
                Some("preview"),
            )
            .is_some_and(|capability| capability.available)
    );

    let order = registry
        .definitions_in_declaration_order()
        .into_iter()
        .map(|definition| definition.descriptor().id.compatibility_name.as_str())
        .collect::<Vec<_>>();
    let vibrance = order
        .iter()
        .position(|name| *name == "vibrance")
        .expect("Vibrance order entry");
    let colorzones = order
        .iter()
        .position(|name| *name == "colorzones")
        .expect("Color Zones order entry");
    let bloom = order
        .iter()
        .position(|name| *name == "bloom")
        .expect("Bloom order entry");
    assert_eq!(colorzones, vibrance + 1);
    assert!(colorzones < bloom);

    let materialized = registry
        .materialize_operation(
            COLORZONES_RUST_ID,
            OperationId::new(900).expect("operation ID"),
        )
        .expect("editable Color Zones defaults");
    let prepared = registry
        .prepare_cpu(&materialized)
        .expect("canonical Color Zones CPU factory");
    let ProcessingOperationKind::ColorZones { plan } = prepared.operation().kind() else {
        panic!("Color Zones factory compiled the wrong operation kind");
    };
    assert_eq!(plan.config(), &ColorZonesConfig::defaults());

    let mut pixels = [LinearRgb::new(
        FiniteF32::new(0.25).expect("finite red"),
        FiniteF32::new(0.5).expect("finite green"),
        FiniteF32::new(0.75).expect("finite blue"),
    )];
    prepared
        .execute(
            PipelineStepIndex::new(0),
            &mut pixels,
            RasterDimensions::new(1, 1).expect("dimensions"),
            0,
        )
        .expect("registered Color Zones CPU execution");
}

#[test]
fn canonical_evaluator_executes_the_committed_colorzones_plan() {
    let scalar = |value| {
        ParameterValue::Scalar(FiniteF64::new(value).expect("finite Color Zones test parameter"))
    };
    let operation = editable_operation(
        904,
        true,
        [
            ("channel", ParameterValue::Integer(0)),
            ("mode", ParameterValue::Integer(1)),
            ("curve_0_num_nodes", ParameterValue::Integer(2)),
            ("curve_0_node_0_x", scalar(0.0)),
            ("curve_0_node_0_y", scalar(0.6)),
            ("curve_0_node_1_x", scalar(1.0)),
            ("curve_0_node_1_y", scalar(0.6)),
        ]
        .into_iter()
        .map(|(name, value)| (ParameterName::new(name).expect("parameter name"), value)),
    );
    let edit = Edit::from_parts(
        EditId::new(31).expect("edit ID"),
        PhotoId::new(41).expect("photo ID"),
        Revision::ZERO,
        Revision::from_u64(1),
        [operation],
    )
    .expect("Color Zones edit");
    let pipeline = CompiledPipeline::compile(&edit).expect("Color Zones pipeline");
    let dimensions = RasterDimensions::new(2, 1).expect("dimensions");
    let input = WorkingRgbImage::new(
        dimensions,
        vec![
            LinearRgb::new(
                FiniteF32::new(0.2).expect("red"),
                FiniteF32::new(0.4).expect("green"),
                FiniteF32::new(0.6).expect("blue"),
            ),
            LinearRgb::new(
                FiniteF32::new(0.8).expect("red"),
                FiniteF32::new(0.3).expect("green"),
                FiniteF32::new(0.1).expect("blue"),
            ),
        ],
    )
    .expect("working image");

    let output = evaluate(&pipeline, &input).expect("canonical Color Zones evaluation");
    assert_eq!(output.dimensions(), dimensions);
    assert_ne!(output.pixel_slice(), input.pixel_slice());
}

#[test]
fn editable_compiler_validates_types_ranges_counts_and_normalizes_inactive_tails() {
    let registry = builtin_registry();
    let defaults = registry
        .materialize_operation(
            COLORZONES_RUST_ID,
            OperationId::new(901).expect("operation ID"),
        )
        .expect("editable Color Zones defaults");
    let with_tail = replacing_parameter(
        &defaults,
        "curve_1_node_19_x",
        ParameterValue::Scalar(FiniteF64::new(0.875).expect("finite tail")),
    );
    let compiled_defaults = ProcessingOperation::compile(&defaults).expect("default operation");
    let compiled_tail = ProcessingOperation::compile(&with_tail).expect("normalized tail");
    let ProcessingOperationKind::ColorZones { plan: default_plan } = compiled_defaults.kind()
    else {
        panic!("default operation kind");
    };
    let ProcessingOperationKind::ColorZones { plan: tail_plan } = compiled_tail.kind() else {
        panic!("tail operation kind");
    };
    assert_eq!(default_plan, tail_plan);
    assert!(
        tail_plan
            .config()
            .curves()
            .iter()
            .all(|curve| curve.node_count() == 2)
    );

    let wrong_type = replacing_parameter(&defaults, "curve_0_node_0_x", ParameterValue::Bool(true));
    assert!(matches!(
        ProcessingOperation::compile(&wrong_type),
        Err(OperationCompileError::WrongParameterType { parameter, .. })
            if parameter.as_str() == "curve_0_node_0_x"
    ));

    let out_of_range = replacing_parameter(
        &defaults,
        "strength",
        ParameterValue::Scalar(FiniteF64::new(200.001).expect("finite strength")),
    );
    assert!(matches!(
        ProcessingOperation::compile(&out_of_range),
        Err(OperationCompileError::InvalidParameters { .. })
    ));

    let zero_count =
        replacing_parameter(&defaults, "curve_2_num_nodes", ParameterValue::Integer(0));
    assert!(matches!(
        ProcessingOperation::compile(&zero_count),
        Err(OperationCompileError::InvalidParameters { .. })
    ));

    let missing_new_active_point = editable_operation(
        902,
        false,
        [(
            ParameterName::new("curve_0_num_nodes").expect("parameter name"),
            ParameterValue::Integer(3),
        )],
    );
    assert!(matches!(
        ProcessingOperation::compile(&missing_new_active_point),
        Err(OperationCompileError::MissingParameter { parameter, .. })
            if parameter.as_str() == "curve_0_node_2_x"
    ));

    let v1_capacity = replacing_parameter(
        &replacing_parameter(&defaults, "splines_version", ParameterValue::Integer(0)),
        "curve_0_num_nodes",
        ParameterValue::Integer(20),
    );
    assert!(matches!(
        ProcessingOperation::compile(&v1_capacity),
        Err(OperationCompileError::InvalidParameters { .. })
    ));

    let disabled = editable_operation(903, false, std::iter::empty());
    let compiled = ProcessingOperation::compile(&disabled).expect("default-disabled operation");
    assert!(!compiled.is_enabled());
    assert_eq!(
        compiled.kind(),
        &ProcessingOperationKind::ColorZones {
            plan: ColorZonesPlan::new(ColorZonesConfig::defaults()).expect("default plan")
        }
    );
}
