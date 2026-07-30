use rusttable_color::{
    Adaptation, AdaptationMethod, BuiltinSpace, ColorMathError, Matrix3, MatrixError,
    NonFinitePolicy, TransferDirection, TransferFunction, TransferFunctionError, TransferPolicy,
    TransferRange, WhitePoint, clip_to_unit_gamut, is_in_unit_gamut, jzczhz_to_xyz_d65, lab_to_lch,
    lab_to_xyz, lch_to_lab, relative_luminance, xyz_d65_to_jzczhz, xyz_luminance, xyz_to_lab,
};

fn close(actual: f32, expected: f32, tolerance: f32) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "actual={actual:?}, expected={expected:?}, tolerance={tolerance:?}"
    );
}

fn close3(actual: [f32; 3], expected: [f32; 3], tolerance: f32) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        close(actual, expected, tolerance);
    }
}

#[test]
fn transfer_reference_vectors_match_published_functions() {
    close(TransferFunction::Linear.encode(0.18).unwrap(), 0.18, 0.0);
    close(
        TransferFunction::Srgb.encode(0.18).unwrap(),
        0.461_356_13,
        2.0e-7,
    );
    close(
        TransferFunction::Rec709.encode(0.18).unwrap(),
        0.409_007_73,
        2.0e-7,
    );
    close(
        TransferFunction::Rec2020.encode(0.18).unwrap(),
        0.408_848_1,
        2.0e-7,
    );
    close(
        TransferFunction::gamma(2.2).unwrap().encode(0.18).unwrap(),
        0.458_656_46,
        2.0e-7,
    );
    // ITU-R BT.2100-3 Tables 4 and 5: 100 cd/m² PQ and 18% HLG scene light.
    close(
        TransferFunction::Pq.encode(0.01).unwrap(),
        0.508_078_4,
        7.0e-7,
    );
    close(
        TransferFunction::Hlg.encode(0.18).unwrap(),
        0.672_358_16,
        3.0e-7,
    );
}

#[test]
fn transfer_policy_is_explicit_for_domain_clamp_and_non_finite_values() {
    assert_eq!(
        TransferFunction::Pq.encode_with_policy(-0.1, TransferPolicy::strict_unit()),
        Err(TransferFunctionError::OutOfRange)
    );
    close(
        TransferFunction::Pq
            .encode_with_policy(-0.1, TransferPolicy::clamped_unit())
            .unwrap(),
        7.309_559e-7,
        0.0,
    );
    assert_eq!(
        TransferFunction::Pq.encode_with_policy(0.5, TransferPolicy::extended()),
        Err(TransferFunctionError::UnsupportedExtendedRange)
    );
    assert_eq!(
        TransferFunction::Srgb.decode_with_policy(
            f32::INFINITY,
            TransferPolicy::new(TransferRange::Extended, NonFinitePolicy::Reject),
        ),
        Err(TransferFunctionError::NonFinite)
    );
    let preserved = TransferFunction::Srgb
        .decode_with_policy(
            f32::NEG_INFINITY,
            TransferPolicy::new(TransferRange::Extended, NonFinitePolicy::Preserve),
        )
        .unwrap();
    assert_eq!(preserved.to_bits(), f32::NEG_INFINITY.to_bits());
    let canonical = TransferFunction::Srgb
        .decode_with_policy(
            f32::from_bits(0x7fc0_1234),
            TransferPolicy::new(TransferRange::Extended, NonFinitePolicy::CanonicalizeNaN),
        )
        .unwrap();
    assert_eq!(canonical.to_bits(), 0x7fc0_0000);
}

#[test]
fn transfer_roundtrips_are_monotonic_and_batch_friendly() {
    let extended = [
        TransferFunction::Linear,
        TransferFunction::Srgb,
        TransferFunction::Rec709,
        TransferFunction::Rec2020,
        TransferFunction::gamma(2.4).unwrap(),
    ];
    for function in extended {
        let mut previous = f32::NEG_INFINITY;
        for step in -256_i16..=256_i16 {
            let input = f32::from(step) / 64.0;
            let encoded = function.encode(input).unwrap();
            assert!(
                encoded >= previous,
                "{function:?} is not monotonic at {input}"
            );
            previous = encoded;
            close(function.decode(encoded).unwrap(), input, 2.0e-5);
        }
    }
    for function in [TransferFunction::Pq, TransferFunction::Hlg] {
        let mut previous = -1.0;
        for step in 0_u16..=512_u16 {
            let input = f32::from(step) / 512.0;
            let encoded = function.encode(input).unwrap();
            assert!(
                encoded >= previous,
                "{function:?} is not monotonic at {input}"
            );
            previous = encoded;
            close(function.decode(encoded).unwrap(), input, 1.2e-4);
        }
    }

    let input = [-0.25, 0.0, 0.18, 1.0, 4.0];
    let mut encoded = [0.0; 5];
    TransferFunction::Srgb
        .apply_slice(
            &input,
            &mut encoded,
            TransferDirection::Encode,
            TransferPolicy::extended(),
        )
        .unwrap();
    let mut decoded = [0.0; 5];
    TransferFunction::Srgb
        .apply_slice(
            &encoded,
            &mut decoded,
            TransferDirection::Decode,
            TransferPolicy::extended(),
        )
        .unwrap();
    close3(
        [decoded[0], decoded[2], decoded[4]],
        [-0.25, 0.18, 4.0],
        2.0e-5,
    );
    assert_eq!(
        TransferFunction::Srgb.apply_slice(
            &input,
            &mut decoded[..4],
            TransferDirection::Encode,
            TransferPolicy::extended(),
        ),
        Err(TransferFunctionError::LengthMismatch)
    );
}

#[test]
fn matrices_validate_scale_relative_singularity_and_checked_batches() {
    let scaled = Matrix3::new([1.0e-8, 0.0, 0.0, 0.0, 2.0e-8, 0.0, 0.0, 0.0, 3.0e-8])
        .expect("well-conditioned scale must remain invertible");
    let inverse = scaled.inverse().unwrap();
    close3(
        inverse
            .apply_checked(scaled.apply_checked([1.0, -2.0, 3.0]).unwrap())
            .unwrap(),
        [1.0, -2.0, 3.0],
        3.0e-6,
    );

    assert!(matches!(
        Matrix3::new([1.0, 2.0, 3.0, 2.0, 4.0, 6.0, 0.0, 1.0, 0.0]),
        Err(MatrixError::Singular)
    ));
    assert_eq!(
        Matrix3::identity().apply_checked([f32::NAN, 0.0, 0.0]),
        Err(MatrixError::NonFinite)
    );

    let input = [[1.0, 2.0, 3.0], [-1.0, 0.5, 4.0]];
    let mut output = [[0.0; 3]; 2];
    Matrix3::identity()
        .apply_slice(&input, &mut output)
        .unwrap();
    assert_eq!(output, input);
    assert_eq!(
        Matrix3::identity().apply_slice(&input, &mut output[..1]),
        Err(MatrixError::LengthMismatch)
    );
}

#[test]
fn bradford_matches_reference_matrix_and_rejects_false_identity() {
    let d65_to_d50 =
        Adaptation::between(WhitePoint::D65, WhitePoint::D50, AdaptationMethod::Bradford).unwrap();
    close3(
        d65_to_d50.matrix().apply_checked([0.25, 0.4, 0.1]).unwrap(),
        [0.266_141_98, 0.401_873_35, 0.078_898_74],
        3.0e-6,
    );
    assert!(
        Adaptation::between(WhitePoint::D65, WhitePoint::D50, AdaptationMethod::Identity).is_err()
    );
    assert!(WhitePoint::custom(0.7, 0.4).is_err());
}

#[test]
fn cie_lab_and_lch_reference_vectors_roundtrip_with_defined_neutral_hue() {
    close3(
        xyz_to_lab(WhitePoint::D50.xyz(), WhitePoint::D50).unwrap(),
        [100.0, 0.0, 0.0],
        3.0e-4,
    );
    close3(
        xyz_to_lab([0.0; 3], WhitePoint::D50).unwrap(),
        [0.0; 3],
        0.0,
    );
    let lab = [53.232_88, 80.109_33, 67.220_06];
    let lch = lab_to_lch(lab).unwrap();
    close3(lch, [53.232_88, 104.575_52, 40.000_01], 2.0e-4);
    close3(lch_to_lab(lch).unwrap(), lab, 2.0e-4);
    close3(lab_to_lch([50.0, 0.0, 0.0]).unwrap(), [50.0, 0.0, 0.0], 0.0);
    assert_eq!(
        lch_to_lab([50.0, -1.0, 30.0]),
        Err(ColorMathError::OutOfRange)
    );
    assert_eq!(
        xyz_to_lab([f32::NAN, 0.0, 0.0], WhitePoint::D50),
        Err(ColorMathError::NonFinite)
    );

    let xyz_batch = [WhitePoint::D50.xyz(), [0.0; 3]];
    let mut lab_batch = [[0.0; 3]; 2];
    rusttable_color::xyz_to_lab_slice(&xyz_batch, &mut lab_batch, WhitePoint::D50).unwrap();
    let mut restored_batch = [[0.0; 3]; 2];
    rusttable_color::lab_to_xyz_slice(&lab_batch, &mut restored_batch, WhitePoint::D50).unwrap();
    close3(restored_batch[0], xyz_batch[0], 3.0e-6);
    close3(restored_batch[1], xyz_batch[1], 0.0);

    for index in 0..1_024_u32 {
        let l = f32::from(u16::try_from(index.wrapping_mul(37) % 14_000).unwrap()) / 100.0 - 20.0;
        let a = f32::from(u16::try_from(index.wrapping_mul(73) % 40_000).unwrap()) / 100.0 - 200.0;
        let b = f32::from(u16::try_from(index.wrapping_mul(97) % 40_000).unwrap()) / 100.0 - 200.0;
        let sample = [l, a, b];
        close3(
            xyz_to_lab(
                lab_to_xyz(sample, WhitePoint::D50).unwrap(),
                WhitePoint::D50,
            )
            .unwrap(),
            sample,
            8.0e-4,
        );
    }
}

#[test]
fn jzczhz_uses_absolute_d65_xyz_and_roundtrips_selected_reference() {
    close3(xyz_d65_to_jzczhz([0.0; 3]).unwrap(), [0.0; 3], 2.0e-10);
    close3(jzczhz_to_xyz_d65([0.0; 3]).unwrap(), [0.0; 3], 2.0e-7);
    let xyz = [0.206_540_08, 0.121_972_25, 0.051_369_52];
    let jzczhz = xyz_d65_to_jzczhz(xyz).unwrap();
    close3(
        [jzczhz[0], jzczhz[1], 0.0],
        [0.005_350_48, 0.010_634_93, 0.0],
        2.0e-5,
    );
    close(jzczhz[2], 29.643_58, 0.002);
    close3(jzczhz_to_xyz_d65(jzczhz).unwrap(), xyz, 3.0e-5);
    let mut batch = [[0.0; 3]; 1];
    rusttable_color::xyz_d65_to_jzczhz_slice(&[xyz], &mut batch).unwrap();
    let mut restored = [[0.0; 3]; 1];
    rusttable_color::jzczhz_to_xyz_d65_slice(&batch, &mut restored).unwrap();
    close3(restored[0], xyz, 3.0e-5);
    assert_eq!(
        xyz_d65_to_jzczhz([-0.1, 0.2, 0.3]),
        Err(ColorMathError::OutOfRange)
    );
    assert_eq!(
        xyz_d65_to_jzczhz([f32::INFINITY, 0.2, 0.3]),
        Err(ColorMathError::NonFinite)
    );
    assert_eq!(
        jzczhz_to_xyz_d65([0.5, -0.1, 20.0]),
        Err(ColorMathError::OutOfRange)
    );
}

#[test]
fn luminance_and_minimal_gamut_primitives_are_linear_and_finite() {
    close(
        relative_luminance([1.0, 0.0, 0.0], BuiltinSpace::SrgbD65).unwrap(),
        0.212_672_9,
        1.0e-7,
    );
    close(xyz_luminance([0.4, 0.18, 0.7]).unwrap(), 0.18, 0.0);
    assert!(is_in_unit_gamut([0.0, 0.5, 1.0]).unwrap());
    assert!(!is_in_unit_gamut([-0.01, 0.5, 1.1]).unwrap());
    close3(
        clip_to_unit_gamut([-0.01, 0.5, 1.1]).unwrap(),
        [0.0, 0.5, 1.0],
        0.0,
    );
    assert_eq!(
        is_in_unit_gamut([f32::INFINITY, 0.0, 0.0]),
        Err(ColorMathError::NonFinite)
    );
    assert_eq!(
        relative_luminance([0.0; 3], BuiltinSpace::LabD50),
        Err(ColorMathError::UnsupportedColorSpace)
    );
}
