use rusttable_color::{
    Adaptation, AdaptationMethod, AlphaTransform, BlackPointCompensation,
    BuiltinColorTransformPlanner, ColorEncoding, ColorRole, ColorTransformPlanner,
    ColorTransformRequest, ExtendedRange, Lut1D, Lut1DError, Lut3D, Lut3DError, LutInterpolation,
    LutPacking, Matrix3, MatrixError, Pcs, Precision, ProfileClass, ProfileId, ProfileModel,
    RenderingIntent, TransferFunction, TransformPlan, TransformStep, WhitePoint,
};

fn request(source: ColorEncoding, target: ColorEncoding) -> ColorTransformRequest {
    ColorTransformRequest::new(
        source,
        target,
        ColorRole::Working,
        RenderingIntent::Relative,
        BlackPointCompensation::Disabled,
        AdaptationMethod::Bradford,
        Precision::F32,
        AlphaTransform::Preserve,
        ExtendedRange::Extended,
        1,
    )
    .expect("test request is valid")
}

#[test]
fn published_spaces_have_explicit_independent_identities() {
    assert_eq!(ColorEncoding::SrgbD65, ColorEncoding::Srgb);
    assert_ne!(ColorEncoding::SrgbD65, ColorEncoding::LinearSrgbD65);
    assert_eq!(ColorEncoding::DisplayP3D65, ColorEncoding::DisplayP3);
    assert_eq!(ColorEncoding::XyzD50.white_point(), Some(WhitePoint::D50));
    assert_eq!(
        ColorEncoding::AcesCgD60.white_point(),
        Some(WhitePoint::D60)
    );
    assert!(ColorEncoding::Unspecified.transfer().is_none());
}

#[test]
fn transfer_vectors_preserve_negative_and_hdr_values() {
    let transfer = TransferFunction::Srgb;
    for value in [-0.25_f32, 0.0, 0.003, 0.18, 1.0, 4.0] {
        let encoded = transfer
            .encode(transfer.decode(value).expect("decode"))
            .expect("encode");
        assert!((encoded - value).abs() < 0.000_01_f32.max(value.abs() * 0.000_01));
    }
}

#[test]
fn bradford_adaptation_roundtrips_d50_white() {
    let forward = Adaptation::between(WhitePoint::D50, WhitePoint::D65, AdaptationMethod::Bradford)
        .expect("forward adaptation");
    let reverse = Adaptation::between(WhitePoint::D65, WhitePoint::D50, AdaptationMethod::Bradford)
        .expect("reverse adaptation");
    let actual = reverse
        .matrix()
        .apply(forward.matrix().apply(WhitePoint::D50.xyz()));
    for (actual, expected) in actual.into_iter().zip(WhitePoint::D50.xyz()) {
        assert!((actual - expected).abs() < 0.000_2);
    }
}

#[test]
fn profile_id_is_content_addressed_without_retaining_bytes() {
    let parser = rusttable_color::ProfileParserVersion::new(1).expect("parser version");
    let first = ProfileId::from_content(
        b"profile bytes",
        ProfileClass::Display,
        ProfileModel::Matrix,
        Pcs::XyzD50,
        parser,
    )
    .expect("profile id");
    let second = ProfileId::from_content(
        b"profile bytes",
        ProfileClass::Display,
        ProfileModel::Matrix,
        Pcs::XyzD50,
        parser,
    )
    .expect("profile id");
    assert_eq!(first, second);
    assert_ne!(first.sha256(), [0; 32]);
    assert_eq!(first.size(), 13);
}

#[test]
fn lut_bounds_and_sample_shapes_are_checked() {
    assert_eq!(
        Lut1D::new(vec![[0.0; 3]], LutInterpolation::Linear),
        Err(Lut1DError::TooFewSamples)
    );
    let one_d = Lut1D::new(vec![[0.0; 3], [1.0; 3]], LutInterpolation::Linear).expect("1D LUT");
    assert_eq!(one_d.sample_count(), 2);
    assert!(matches!(
        Lut3D::new(
            2,
            vec![[0.0; 3]; 7],
            LutPacking::RgbInterleaved,
            LutInterpolation::Tetrahedral
        ),
        Err(Lut3DError::SampleCountMismatch { .. })
    ));
    let three_d = Lut3D::new(
        2,
        vec![[0.0; 3]; 8],
        LutPacking::RgbInterleaved,
        LutInterpolation::Tetrahedral,
    )
    .expect("3D LUT");
    let plan = TransformPlan::new(
        request(ColorEncoding::SrgbD65, ColorEncoding::SrgbD65),
        vec![TransformStep::Lut1D(one_d), TransformStep::Lut3D(three_d)],
    )
    .expect("LUT plan");
    assert_eq!(plan.resource_estimate(), 10);
}

#[test]
fn invalid_scalars_matrices_and_gamma_are_rejected() {
    assert!(matches!(Matrix3::new([0.0; 9]), Err(MatrixError::Singular)));
    assert!(matches!(
        Matrix3::new([f32::NAN; 9]),
        Err(MatrixError::NonFinite)
    ));
    assert!(TransferFunction::gamma(0.0).is_err());
    assert!(WhitePoint::custom(0.0, 0.3).is_err());
}

#[test]
fn planner_is_deterministic_and_canonical() {
    let planner = BuiltinColorTransformPlanner;
    let request = request(ColorEncoding::SrgbD65, ColorEncoding::DisplayP3D65);
    let first = planner.plan(&request).expect("first plan");
    let second = planner.plan(&request).expect("second plan");
    assert_eq!(first, second);
    assert_eq!(
        first.canonical_bytes().expect("bytes"),
        second.canonical_bytes().expect("bytes")
    );
    assert_eq!(
        first.identity().expect("hash"),
        second.identity().expect("hash")
    );
    assert!(!first.is_identity());
    let receipt = first.receipt().expect("receipt");
    assert_eq!(receipt.source(), ColorEncoding::SrgbD65);
    assert_eq!(receipt.target(), ColorEncoding::DisplayP3D65);
    assert_eq!(
        receipt.step_count(),
        u16::try_from(first.steps().len()).expect("bounded step count")
    );
}

#[test]
fn request_and_plan_codecs_reject_unknown_schema() {
    let request = request(ColorEncoding::SrgbD65, ColorEncoding::SrgbD65);
    let bytes = request.canonical_bytes().expect("request bytes");
    assert_eq!(
        rusttable_color::decode_request(&bytes).expect("request decode"),
        request
    );
    let plan = BuiltinColorTransformPlanner
        .plan(&request)
        .expect("identity plan");
    let plan_bytes = plan.canonical_bytes().expect("plan bytes");
    assert_eq!(
        rusttable_color::decode_plan(&plan_bytes).expect("plan decode"),
        plan
    );
}
