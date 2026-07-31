#![allow(clippy::cast_precision_loss, clippy::float_cmp, clippy::similar_names)]

#[path = "../src/operations/lut3d/mod.rs"]
mod lut3d;

use std::cell::Cell;
use std::fmt::Write as _;
use std::path::Path;

use lut3d::{
    FrameDimensions, LUT3D_CLUT_LEVEL, LUT3D_COMPRESSED_CLUT_BYTES, LUT3D_MAX_KEYPOINTS,
    LUT3D_MAX_LUTNAME, LUT3D_MAX_PATHNAME, LUT3D_SCHEMA_VERSION, LUT3D_V1_PARAMETER_BYTES,
    LUT3D_V2_PARAMETER_BYTES, LUT3D_V3_PARAMETER_BYTES, Lut3d, Lut3dCodecError, Lut3dColorspace,
    Lut3dExecutionError, Lut3dHistory, Lut3dInterpolation, Lut3dParameters, Lut3dParseError,
    Lut3dPlan, Lut3dProfileContext,
};

const IDENTITY_CUBE: &str = include_str!("fixtures/lut3d/identity_2.cube");
const BLUE_FAST_3DL: &str = include_str!("fixtures/lut3d/blue_fast_4.3dl");
const BLUE_FAST_5_3DL: &str = include_str!("fixtures/lut3d/blue_fast_5_header_tail.3dl");

fn identity_profile() -> Lut3dProfileContext {
    Lut3dProfileContext::from_builtin(
        Lut3dColorspace::Srgb,
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    )
    .expect("identity profile evidence is valid")
}

fn cube_with_values(values: [[f32; 3]; 8]) -> Lut3d {
    let mut contents = String::from("LUT_3D_SIZE 2\n");
    for value in values {
        let _ = writeln!(contents, "{} {} {}", value[0], value[1], value[2]);
    }
    Lut3d::parse_cube(&contents).expect("generated cube is valid")
}

#[allow(clippy::too_many_arguments)]
fn weighted4(
    a: [f32; 3],
    wa: f32,
    b: [f32; 3],
    wb: f32,
    c: [f32; 3],
    wc: f32,
    d: [f32; 3],
    wd: f32,
) -> [f32; 3] {
    [
        wa * a[0] + wb * b[0] + wc * c[0] + wd * d[0],
        wa * a[1] + wb * b[1] + wc * c[1] + wd * d[1],
        wa * a[2] + wb * b[2] + wc * c[2] + wd * d[2],
    ]
}

fn pyramid_reference(values: [[f32; 3]; 8], input: [f32; 3]) -> [f32; 3] {
    let [p000, p100, p010, p110, p001, p101, p011, p111] = values;
    let [dr, dg, db] = input;
    if dg > dr && db > dr {
        [
            p000[0]
                + (p111[0] - p011[0]) * dr
                + (p010[0] - p000[0]) * dg
                + (p001[0] - p000[0]) * db
                + (p011[0] - p001[0] - p010[0] + p000[0]) * dg * db,
            p000[1]
                + (p111[1] - p011[1]) * dr
                + (p010[1] - p000[1]) * dg
                + (p001[1] - p000[1]) * db
                + (p011[1] - p001[1] - p010[1] + p000[1]) * dg * db,
            p000[2]
                + (p111[2] - p011[2]) * dr
                + (p010[2] - p000[2]) * dg
                + (p001[2] - p000[2]) * db
                + (p011[2] - p001[2] - p010[2] + p000[2]) * dg * db,
        ]
    } else if dr > dg && db > dg {
        [
            p000[0]
                + (p100[0] - p000[0]) * dr
                + (p111[0] - p101[0]) * dg
                + (p001[0] - p000[0]) * db
                + (p101[0] - p001[0] - p100[0] + p000[0]) * dr * db,
            p000[1]
                + (p100[1] - p000[1]) * dr
                + (p111[1] - p101[1]) * dg
                + (p001[1] - p000[1]) * db
                + (p101[1] - p001[1] - p100[1] + p000[1]) * dr * db,
            p000[2]
                + (p100[2] - p000[2]) * dr
                + (p111[2] - p101[2]) * dg
                + (p001[2] - p000[2]) * db
                + (p101[2] - p001[2] - p100[2] + p000[2]) * dr * db,
        ]
    } else {
        [
            p000[0]
                + (p100[0] - p000[0]) * dr
                + (p010[0] - p000[0]) * dg
                + (p111[0] - p110[0]) * db
                + (p110[0] - p100[0] - p010[0] + p000[0]) * dr * dg,
            p000[1]
                + (p100[1] - p000[1]) * dr
                + (p010[1] - p000[1]) * dg
                + (p111[1] - p110[1]) * db
                + (p110[1] - p100[1] - p010[1] + p000[1]) * dr * dg,
            p000[2]
                + (p100[2] - p000[2]) * dr
                + (p010[2] - p000[2]) * dg
                + (p111[2] - p110[2]) * db
                + (p110[2] - p100[2] - p010[2] + p000[2]) * dr * dg,
        ]
    }
}

#[test]
fn codec_preserves_v3_offsets_and_native_defaults() {
    assert_eq!(LUT3D_MAX_PATHNAME, 512);
    assert_eq!(LUT3D_MAX_LUTNAME, 128);
    assert_eq!(LUT3D_CLUT_LEVEL, 48);
    assert_eq!(LUT3D_MAX_KEYPOINTS, 2048);
    assert_eq!(LUT3D_COMPRESSED_CLUT_BYTES, 12_288);
    assert_eq!(LUT3D_V1_PARAMETER_BYTES, 520);
    assert_eq!(LUT3D_V2_PARAMETER_BYTES, 12_944);
    assert_eq!(LUT3D_V3_PARAMETER_BYTES, 12_940);

    let params = Lut3dParameters::default();
    let bytes = params.to_bytes();
    assert_eq!(bytes.len(), LUT3D_V3_PARAMETER_BYTES);
    assert_eq!(&bytes[..512], &[0; 512]);
    assert_eq!(&bytes[512..516], &0_i32.to_le_bytes());
    assert_eq!(&bytes[516..520], &0_i32.to_le_bytes());
    assert_eq!(&bytes[520..524], &0_i32.to_le_bytes());
    assert!(bytes[524..].iter().all(|byte| *byte == 0));
    assert_eq!(Lut3dParameters::from_v3_bytes(&bytes).unwrap(), params);
}

#[test]
fn codec_migrates_v1_and_v2_and_keeps_unknown_opaque() {
    let mut v1 = [0_u8; LUT3D_V1_PARAMETER_BYTES];
    v1[..512].fill(b'x');
    v1[..5].copy_from_slice(b"abcde");
    v1[512..516].copy_from_slice(&5_i32.to_le_bytes());
    v1[516..520].copy_from_slice(&2_i32.to_le_bytes());
    let migrated = Lut3dHistory::decode(1, &v1).unwrap();
    let current = migrated.current().unwrap();
    assert_eq!(&current.filepath[..5], b"abcde");
    assert_eq!(current.filepath[511], 0);
    assert_eq!(current.colorspace, Lut3dColorspace::LinearProphoto);
    assert_eq!(current.interpolation, Lut3dInterpolation::Pyramid);
    assert_eq!(current.nb_keypoints, 0);
    assert!(current.c_clut.iter().all(|byte| *byte == 0));
    assert!(current.lutname.iter().all(|byte| *byte == 0));

    let mut v3 = Lut3dParameters::default();
    v3.filepath[0] = b'x';
    v3.nb_keypoints = 7;
    v3.c_clut[123] = 0xa5;
    v3.lutname[0] = b'n';
    let mut v2 = vec![0_u8; LUT3D_V2_PARAMETER_BYTES];
    v2[..LUT3D_V3_PARAMETER_BYTES].copy_from_slice(&v3.to_bytes());
    v2[LUT3D_V3_PARAMETER_BYTES..].copy_from_slice(&0xdead_beef_u32.to_le_bytes());
    let migrated_v2 = Lut3dHistory::decode(2, &v2).unwrap();
    assert_eq!(migrated_v2.current().unwrap(), &v3);
    assert_eq!(migrated_v2.payload(), v3.to_bytes());

    let opaque_bytes = vec![1, 2, 3, 4, 5];
    let opaque = Lut3dHistory::decode(99, &opaque_bytes).unwrap();
    assert_eq!(opaque.version(), 99);
    assert_eq!(opaque.payload(), opaque_bytes);
    assert!(matches!(
        opaque.current(),
        Err(Lut3dCodecError::UnsupportedVersion(99))
    ));
    assert_eq!(LUT3D_SCHEMA_VERSION, 3);
}

#[test]
fn cube_reader_preserves_comments_domains_and_r_fastest_order() {
    let lut = Lut3d::parse_cube(IDENTITY_CUBE).unwrap();
    assert_eq!(lut.level(), 2);
    assert_eq!(lut.values()[0], [0.0, 0.0, 0.0]);
    assert_eq!(lut.values()[1], [1.0, 0.0, 0.0]);
    assert_eq!(lut.values()[2], [0.0, 1.0, 0.0]);
    assert_eq!(lut.values()[4], [0.0, 0.0, 1.0]);
    assert_eq!(lut.values()[7], [1.0, 1.0, 1.0]);

    let out_of_range = Lut3d::parse_cube(
        "DOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1\nLUT_3D_SIZE 2\n-2 3 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n",
    )
    .unwrap();
    assert_eq!(out_of_range.values()[0], [-2.0, 3.0, 0.0]);
    assert!(matches!(
        Lut3d::from_file(Path::new("unsupported.png")),
        Err(Lut3dParseError::UnsupportedFormat)
    ));
    assert!(matches!(
        Lut3d::from_file(Path::new("missing.cube")),
        Err(Lut3dParseError::Io(_))
    ));
}

#[test]
fn cube_reader_rejects_unsafe_or_unsupported_records() {
    let cases = [
        ("LUT_3D_SIZE 1\n", "level"),
        ("LUT_1D_SIZE 2\n", "1d"),
        ("DOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 0 1\n", "domain"),
        ("LUT_3D_SIZE 2\n0 0 nan\n", "nan"),
        ("LUT_3D_SIZE 2\n0 0 inf\n", "infinity"),
        ("0 0 0\n", "size"),
    ];
    for (contents, label) in cases {
        assert!(
            Lut3d::parse_cube(contents).is_err(),
            "{label} must fail closed"
        );
    }

    let mut extra = String::from("LUT_3D_SIZE 2\n");
    for _ in 0..9 {
        extra.push_str("0 0 0\n");
    }
    assert!(matches!(
        Lut3d::parse_cube(&extra),
        Err(Lut3dParseError::ExtraRecords(_))
    ));
}

#[test]
fn three_dl_reader_remaps_blue_fast_records_and_normalizes() {
    let lut = Lut3d::parse_3dl(BLUE_FAST_3DL).unwrap();
    assert_eq!(lut.level(), 4);
    let normalizer = 1.0 / 511.0;
    assert_eq!(lut.values()[0], [128.0 * normalizer; 3]);
    assert_eq!(
        lut.values()[1],
        [192.0 * normalizer, 128.0 * normalizer, 128.0 * normalizer]
    );
    assert_eq!(
        lut.values()[4],
        [128.0 * normalizer, 192.0 * normalizer, 128.0 * normalizer]
    );
    assert_eq!(
        lut.values()[16],
        [128.0 * normalizer, 128.0 * normalizer, 192.0 * normalizer]
    );

    assert!(matches!(
        Lut3d::parse_3dl("0 64 100 100\n"),
        Err(Lut3dParseError::InvalidMaximum(100))
    ));
    let level_five = Lut3d::parse_3dl(BLUE_FAST_5_3DL).unwrap();
    assert_eq!(level_five.level(), 5);
    let normalizer = 1.0 / 255.0;
    assert_eq!(level_five.values()[1], [64.0 * normalizer, 0.0, 0.0]);
    assert_eq!(level_five.values()[5], [0.0, 64.0 * normalizer, 0.0]);
    assert_eq!(level_five.values()[25], [0.0, 0.0, 64.0 * normalizer]);
    assert_eq!(level_five.values()[124], [255.0 * normalizer; 3]);
    assert!(Lut3d::parse_3dl("0 128 255\n").is_err());
    assert!(Lut3d::parse_3dl("0 128 255 511\n-1 0 0\n").is_err());
    assert!(Lut3d::parse_3dl("0 128 255 511\nnope 0 0\n").is_err());
    assert!(Lut3d::parse_3dl("0 128 255 511\n0 0 0 0\n").is_err());
}

#[test]
fn all_interpolation_branches_and_edges_follow_native_equations() {
    let values = [
        [0.1, 1.1, 2.1],
        [3.2, 4.2, 5.2],
        [6.3, 7.3, 8.3],
        [9.4, 10.4, 11.4],
        [12.5, 13.5, 14.5],
        [15.6, 16.6, 17.6],
        [18.7, 19.7, 20.7],
        [21.8, 22.8, 23.8],
    ];
    let lut = cube_with_values(values);
    let tetra_inputs = [
        [0.8, 0.5, 0.2],
        [0.8, 0.2, 0.5],
        [0.5, 0.2, 0.8],
        [0.2, 0.5, 0.8],
        [0.2, 0.8, 0.5],
        [0.5, 0.8, 0.2],
    ];
    for input in tetra_inputs {
        let [dr, dg, db] = input;
        let expected = if dr > dg {
            if dg > db {
                weighted4(
                    values[0],
                    1.0 - dr,
                    values[1],
                    dr - dg,
                    values[3],
                    dg - db,
                    values[7],
                    db,
                )
            } else if dr > db {
                weighted4(
                    values[0],
                    1.0 - dr,
                    values[1],
                    dr - db,
                    values[5],
                    db - dg,
                    values[7],
                    dg,
                )
            } else {
                weighted4(
                    values[0],
                    1.0 - db,
                    values[4],
                    db - dr,
                    values[5],
                    dr - dg,
                    values[7],
                    dg,
                )
            }
        } else if db > dg {
            weighted4(
                values[0],
                1.0 - db,
                values[4],
                db - dg,
                values[6],
                dg - dr,
                values[7],
                dr,
            )
        } else if db > dr {
            weighted4(
                values[0],
                1.0 - dg,
                values[2],
                dg - db,
                values[6],
                db - dr,
                values[7],
                dr,
            )
        } else {
            weighted4(
                values[0],
                1.0 - dg,
                values[2],
                dg - dr,
                values[3],
                dr - db,
                values[7],
                db,
            )
        };
        let actual = lut.sample(
            [input[0], input[1], input[2], 0.37],
            Lut3dInterpolation::Tetrahedral,
        );
        assert_eq!(&actual[..3], &expected);
        assert_eq!(actual[3], 0.37);
    }

    for input in [
        [0.2, 0.6, 0.8],
        [0.8, 0.2, 0.6],
        [0.6, 0.4, 0.2],
        [0.5, 0.5, 0.5],
    ] {
        let actual = lut.sample(
            [input[0], input[1], input[2], 0.9],
            Lut3dInterpolation::Pyramid,
        );
        assert_eq!(&actual[..3], &pyramid_reference(values, input));
        assert_eq!(actual[3], 0.9);
    }

    let tri_input = [0.25, 0.5, 0.75];
    let p000 = values[0];
    let p100 = values[1];
    let p010 = values[2];
    let p110 = values[3];
    let p001 = values[4];
    let p101 = values[5];
    let p011 = values[6];
    let p111 = values[7];
    let blend = |a: [f32; 3], b: [f32; 3], amount: f32| {
        [
            a[0] * (1.0 - amount) + b[0] * amount,
            a[1] * (1.0 - amount) + b[1] * amount,
            a[2] * (1.0 - amount) + b[2] * amount,
        ]
    };
    let plane0 = blend(
        blend(p000, p100, tri_input[0]),
        blend(p010, p110, tri_input[0]),
        tri_input[1],
    );
    let plane1 = blend(
        blend(p001, p101, tri_input[0]),
        blend(p011, p111, tri_input[0]),
        tri_input[1],
    );
    let expected = blend(plane0, plane1, tri_input[2]);
    let actual = lut.sample(
        [tri_input[0], tri_input[1], tri_input[2], 0.1],
        Lut3dInterpolation::Trilinear,
    );
    assert_eq!(&actual[..3], &expected);

    let identity = Lut3d::parse_cube(IDENTITY_CUBE).unwrap();
    let clipped = identity.sample([-1.0, 2.0, f32::NAN, 0.42], Lut3dInterpolation::Trilinear);
    assert_eq!(clipped, [0.0, 1.0, 0.0, 0.42]);
    let top = identity.sample([1.0, 1.0, 1.0, 0.42], Lut3dInterpolation::Tetrahedral);
    assert_eq!(top, [1.0, 1.0, 1.0, 0.42]);
}

#[test]
fn execution_requires_profile_evidence_preserves_alpha_and_cancels_transactionally() {
    let lut = Lut3d::parse_cube(IDENTITY_CUBE).unwrap();
    let plan = Lut3dPlan::new(lut, Lut3dColorspace::Srgb, Lut3dInterpolation::Tetrahedral);
    assert_eq!(plan.lut().level(), 2);
    assert_eq!(plan.colorspace(), Lut3dColorspace::Srgb);
    assert_eq!(plan.interpolation(), Lut3dInterpolation::Tetrahedral);
    let profile = identity_profile();
    assert_eq!(profile.colorspace(), Lut3dColorspace::Srgb);
    assert_eq!(
        profile.evidence(),
        lut3d::Lut3dProfileEvidence::BuiltIn(Lut3dColorspace::Srgb)
    );
    assert_eq!(profile.working_to_lut()[0], [1.0, 0.0, 0.0]);
    assert_eq!(profile.lut_to_working()[1], [0.0, 1.0, 0.0]);
    let input = vec![
        [0.1, 0.2, 0.3, 0.11],
        [0.4, 0.5, 0.6, 0.22],
        [0.7, 0.8, 0.9, 0.33],
        [1.0, 0.0, 1.0, 0.44],
    ];
    let dimensions = FrameDimensions::new(2, 2).unwrap();
    assert!(matches!(
        plan.execute(&input, dimensions, None),
        Err(Lut3dExecutionError::MissingProfileContext)
    ));
    let output = plan
        .execute(&input, dimensions, Some(&identity_profile()))
        .unwrap();
    assert_eq!(output, input);

    let polls = Cell::new(0_usize);
    let cancelled =
        plan.execute_with_cancellation(&input, dimensions, Some(&identity_profile()), || {
            let next = polls.get() + 1;
            polls.set(next);
            next >= 7
        });
    assert!(matches!(cancelled, Err(Lut3dExecutionError::Cancelled)));
    assert!(polls.get() >= 3);

    let nonfinite = [[f32::NAN, 0.0, 0.0, 1.0]];
    assert!(matches!(
        plan.execute(
            &nonfinite,
            FrameDimensions::new(1, 1).unwrap(),
            Some(&identity_profile())
        ),
        Err(Lut3dExecutionError::NonFiniteInput {
            index: 0,
            channel: 0
        })
    ));
    let wrong_profile = Lut3dProfileContext::from_builtin(
        Lut3dColorspace::LinearRec2020,
        [[1.0, 0.0, 0.0]; 3],
        [[1.0, 0.0, 0.0]; 3],
    )
    .unwrap();
    assert!(matches!(
        plan.execute(&input, dimensions, Some(&wrong_profile)),
        Err(Lut3dExecutionError::ProfileMismatch { .. })
    ));
}

#[test]
fn compressed_and_unimplemented_surfaces_remain_unavailable() {
    let params = Lut3dParameters {
        nb_keypoints: 1,
        ..Lut3dParameters::default()
    };
    let lut = Lut3d::parse_cube(IDENTITY_CUBE).unwrap();
    assert!(matches!(
        Lut3dPlan::from_parameters(lut.clone(), &params),
        Err(Lut3dExecutionError::CompressedLutUnsupported)
    ));
    let malformed_params = Lut3dParameters {
        nb_keypoints: -1,
        ..Lut3dParameters::default()
    };
    let malformed_payload = malformed_params.to_bytes();
    let malformed_history = Lut3dHistory::decode(LUT3D_SCHEMA_VERSION, &malformed_payload).unwrap();
    assert_eq!(malformed_history.payload(), malformed_payload.to_vec());
    assert!(matches!(
        Lut3dPlan::from_parameters(lut, malformed_history.current().unwrap()),
        Err(Lut3dExecutionError::InvalidCompressedKeypointCount(-1))
    ));
    assert!(matches!(
        Lut3d::parse_3dl("0 128 255 511\n0 0 0\n"),
        Err(Lut3dParseError::WrongRecordCount { .. })
    ));
    let source_map = include_str!("../src/operations/lut3d/source-map.toml");
    assert!(source_map.contains("status = \"bounded_cpu_leaf_unregistered\""));
    assert!(source_map.contains("id = \"hald-png\""));
    assert!(source_map.contains("id = \"compressed-gmic-gmz\""));
    assert!(source_map.contains("id = \"operation-integration\""));
}
