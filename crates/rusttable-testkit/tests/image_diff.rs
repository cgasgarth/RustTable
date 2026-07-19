use rusttable_testkit::image_diff::{
    AlphaMode, ArtifactKind, CanonicalProfile, DiffPolicy, ImageBuffer, ImageInput,
    MatrixProfileConverter, ToleranceClass, TransferFunction, ciede2000, compare, normalize,
};

fn canonical(width: u32, height: u32, pixels: Vec<f32>) -> ImageBuffer {
    ImageBuffer::canonical_rgba(width, height, CanonicalProfile::Srgb, pixels)
}

#[test]
fn exact_identity_has_zero_metrics_and_stable_json() {
    let image = ImageBuffer::rgba(2, 1, vec![0.0, 0.5, 1.0, 1.0, 0.2, 0.3, 0.4, 1.0]);
    let policy = DiffPolicy::for_class(ToleranceClass::Exact);
    let receipt = compare(&image, &image, &policy).expect("identity comparison");
    assert!(receipt.passed);
    assert_eq!(receipt.metrics.changed_pixel_count, 0);
    assert_eq!(receipt.metrics.outlier_count, 0);
    assert_eq!(
        receipt.stable_json().expect("stable receipt"),
        receipt.stable_json().expect("repeat receipt")
    );
}

#[test]
fn pointwise_reports_one_pixel_and_bounded_artifacts() {
    let source = canonical(2, 1, vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0]);
    let mut reference = source.clone();
    reference.pixels[4] = 0.5;
    let mut policy = DiffPolicy::for_class(ToleranceClass::Pointwise);
    policy.include_heatmap = true;
    policy.include_blink = true;
    let receipt = compare(&source, &reference, &policy).expect("defect comparison");
    assert!(!receipt.passed);
    assert_eq!(receipt.metrics.changed_pixel_count, 1);
    assert_eq!(receipt.outliers.len(), 1);
    assert_eq!(receipt.outliers[0].x, 1);
    assert_eq!(receipt.artifacts[0].kind, ArtifactKind::HeatmapRgba8);
    assert_eq!(receipt.artifacts[1].kind, ArtifactKind::BlinkRgba32);
    assert_eq!(receipt.artifact_payloads()[0].bytes.len(), 2 * 4);
    assert!(receipt.artifact_payloads()[0].validate().is_ok());
    assert!(receipt.artifact_payloads()[1].validate().is_ok());
    let planes = receipt.artifact_payloads()[1]
        .blink_planes()
        .expect("blink planes");
    assert_eq!(planes.source.len(), source.pixels.len());
    assert_eq!(planes.reference.len(), source.pixels.len());
}

#[test]
fn dimensions_alpha_and_nonfinite_values_fail_closed() {
    let source = canonical(1, 1, vec![0.0, 0.0, 0.0, 1.0]);
    let dimensions = canonical(2, 1, vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    assert!(
        compare(
            &source,
            &dimensions,
            &DiffPolicy::for_class(ToleranceClass::Exact)
        )
        .is_err()
    );

    let mut alpha = source.clone();
    alpha.alpha = AlphaMode::Premultiplied;
    assert!(
        compare(
            &source,
            &alpha,
            &DiffPolicy::for_class(ToleranceClass::Exact)
        )
        .is_err()
    );

    let mut nonfinite = source.clone();
    nonfinite.pixels[0] = f32::NAN;
    let receipt = compare(
        &source,
        &nonfinite,
        &DiffPolicy::for_class(ToleranceClass::Exact),
    )
    .expect("nonfinite receipt");
    assert!(!receipt.passed);
    assert_eq!(receipt.metrics.nonfinite_mismatch_count, 1);
}

#[test]
fn built_in_policies_round_trip_and_schema_v1_is_rejected() {
    for class in [
        ToleranceClass::Exact,
        ToleranceClass::Transfer,
        ToleranceClass::Pointwise,
        ToleranceClass::Neighborhood,
        ToleranceClass::LegacyGpu,
    ] {
        let policy = DiffPolicy::for_class(class);
        policy.validate().expect("built-in policy");
        let json = serde_json::to_string(&policy).expect("policy JSON");
        let round_trip: DiffPolicy = serde_json::from_str(&json).expect("policy round trip");
        assert_eq!(policy, round_trip);
    }

    let image = canonical(1, 1, vec![0.0, 0.0, 0.0, 1.0]);
    let receipt = compare(
        &image,
        &image,
        &DiffPolicy::for_class(ToleranceClass::Exact),
    )
    .expect("receipt");
    let mut old = receipt.stable_json().expect("receipt JSON");
    old = old.replacen("\"schema_version\":3", "\"schema_version\":2", 1);
    assert!(rusttable_testkit::image_diff::DiffReceipt::from_json(&old).is_err());
}

#[test]
fn ciede2000_matches_independent_published_vector() {
    let actual = ciede2000([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485]);
    assert!((actual - 2.0425).abs() < 1.0e-3);
}

#[test]
fn normalization_keeps_alpha_linear_and_unpremultiplies_rgb_only() {
    let input = ImageInput {
        width: 2,
        height: 1,
        channels: 4,
        stride: 8,
        alpha: AlphaMode::Premultiplied,
        profile: CanonicalProfile::Srgb,
        transfer: TransferFunction::Srgb,
        unpremultiply_epsilon: 1.0e-8,
        pixels: vec![0.25, 0.125, 0.0, 0.5, 0.7, 0.8, 0.9, 0.0],
    };
    let normalized = normalize(&input, CanonicalProfile::Srgb, None).expect("normalize");
    assert_eq!(normalized.alpha, AlphaMode::Straight);
    assert!((normalized.pixels[0] - 0.214_041_14).abs() < 1.0e-6);
    assert!((normalized.pixels[1] - 0.050_876_09).abs() < 1.0e-6);
    assert!((normalized.pixels[3] - 0.5).abs() < 1.0e-6);
    assert_eq!(&normalized.pixels[4..8], &[0.0, 0.0, 0.0, 0.0]);
}

#[test]
fn comparison_rejects_noncanonical_or_mismatched_profiles() {
    let mut source = canonical(1, 1, vec![0.1, 0.2, 0.3, 1.0]);
    let mut reference = source.clone();
    reference.profile = CanonicalProfile::DisplayP3;
    assert!(
        compare(
            &source,
            &reference,
            &DiffPolicy::for_class(ToleranceClass::Exact)
        )
        .is_err()
    );

    source.alpha = AlphaMode::Premultiplied;
    assert!(
        compare(
            &source,
            &source,
            &DiffPolicy::for_class(ToleranceClass::Exact)
        )
        .is_err()
    );

    let input = ImageInput {
        width: 1,
        height: 1,
        channels: 4,
        stride: 4,
        alpha: AlphaMode::Straight,
        profile: CanonicalProfile::DisplayP3,
        transfer: TransferFunction::Linear,
        unpremultiply_epsilon: 1.0e-8,
        pixels: vec![0.2, 0.3, 0.4, 1.0],
    };
    assert!(normalize(&input, CanonicalProfile::Srgb, None).is_err());
    let converted = normalize(
        &input,
        CanonicalProfile::Srgb,
        Some(&MatrixProfileConverter),
    )
    .expect("explicit profile conversion");
    assert_eq!(converted.profile, CanonicalProfile::Srgb);
}

#[test]
fn neighborhood_matching_is_symmetric_and_does_not_use_finite_neighbors() {
    let source = canonical(
        3,
        1,
        vec![0.0, 0.0, 0.0, 1.0, 0.2, 0.2, 0.2, 1.0, 0.4, 0.4, 0.4, 1.0],
    );
    let reference = canonical(
        3,
        1,
        vec![0.2, 0.2, 0.2, 1.0, 0.4, 0.4, 0.4, 1.0, 0.0, 0.0, 0.0, 1.0],
    );
    let policy = DiffPolicy::for_class(ToleranceClass::Neighborhood);
    let forward = compare(&source, &reference, &policy).expect("forward neighborhood");
    let reverse = compare(&reference, &source, &policy).expect("reverse neighborhood");
    assert_eq!(forward.passed, reverse.passed);
    assert_eq!(forward.metrics, reverse.metrics);
}

#[test]
fn alpha_is_compared_as_linear_coverage_and_infinities_are_explicit() {
    let source = canonical(1, 1, vec![0.2, 0.3, 0.4, 0.25]);
    let mut reference = source.clone();
    reference.pixels[3] = 0.5;
    let receipt = compare(
        &source,
        &reference,
        &DiffPolicy::for_class(ToleranceClass::Exact),
    )
    .expect("alpha comparison");
    assert!((receipt.metrics.maximum_absolute_error - 0.25).abs() < 1.0e-6);

    let mut infinity = source.clone();
    infinity.pixels[0] = f32::INFINITY;
    let rejected = compare(
        &infinity,
        &infinity,
        &DiffPolicy::for_class(ToleranceClass::Exact),
    )
    .expect("infinity receipt");
    assert!(!rejected.passed);
    let mut policy = DiffPolicy::for_class(ToleranceClass::Exact);
    policy.allow_matching_infinities = true;
    let accepted = compare(&infinity, &infinity, &policy).expect("permitted infinity receipt");
    assert!(accepted.passed);
}

#[test]
fn alpha_weight_is_applied_once_and_reports_raw_alpha_metrics() {
    let source = canonical(1, 1, vec![0.0, 0.0, 0.0, 0.0]);
    let reference = canonical(1, 1, vec![0.0, 0.0, 0.0, 1.0]);
    let mut policy = DiffPolicy::for_class(ToleranceClass::Pointwise);
    policy.alpha_weight = 2.0;
    let receipt = compare(&source, &reference, &policy).expect("weighted comparison");
    assert!(receipt.metrics.maximum_rgb_absolute_error.abs() < f32::EPSILON);
    assert!((receipt.metrics.maximum_alpha_absolute_error - 1.0).abs() < f32::EPSILON);
    assert!((receipt.metrics.weighted_maximum_absolute_error - 2.0).abs() < f32::EPSILON);
    assert!((receipt.metrics.alpha_rmse - 1.0).abs() < 1.0e-6);
    assert!((receipt.metrics.rmse - (4.0_f32 / 7.0).sqrt()).abs() < 1.0e-6);
}

#[test]
fn normalization_rejects_invalid_gamma_and_nonfinite_samples() {
    let mut input = ImageInput::rgba(1, 1, vec![0.2, 0.3, 0.4, 1.0]);
    input.transfer = TransferFunction::Gamma(0.0);
    assert!(normalize(&input, CanonicalProfile::Srgb, None).is_err());
    input.transfer = TransferFunction::Linear;
    input.pixels[0] = f32::NAN;
    assert!(normalize(&input, CanonicalProfile::Srgb, None).is_err());
}

#[test]
fn artifact_budget_fails_before_evidence_allocation() {
    let image = ImageBuffer::rgba(2, 1, vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0]);
    let mut policy = DiffPolicy::for_class(ToleranceClass::Pointwise);
    policy.include_heatmap = true;
    policy.include_blink = true;
    policy.artifact_budget_bytes = 1;
    let error = compare(&image, &image, &policy).expect_err("artifact budget");
    assert!(error.to_string().contains("artifact"));
}

#[test]
fn psnr_requires_an_explicit_finite_peak_for_extended_range() {
    let source = canonical(1, 1, vec![2.0, 0.0, 0.0, 1.0]);
    let mut reference = source.clone();
    reference.pixels[0] = 1.5;
    let mut no_peak = DiffPolicy::for_class(ToleranceClass::Pointwise);
    no_peak.psnr_peak = None;
    assert!(
        compare(&source, &reference, &no_peak)
            .expect("no-peak comparison")
            .metrics
            .psnr
            .is_none()
    );
    let with_peak = compare(
        &source,
        &reference,
        &DiffPolicy::for_class(ToleranceClass::Pointwise),
    )
    .expect("explicit default peak comparison");
    assert!(with_peak.metrics.psnr.is_some());
}

#[test]
fn bounded_reports_keep_only_deterministic_worst_thirty_two() {
    #[allow(clippy::cast_precision_loss)]
    let pixels = (0..100)
        .flat_map(|index| [index as f32, 0.0, 0.0, 1.0])
        .collect::<Vec<_>>();
    let source = canonical(100, 1, pixels.clone());
    let reference = canonical(100, 1, {
        let (pixels, remainder) = pixels.as_chunks::<4>();
        assert!(remainder.is_empty());
        pixels
            .iter()
            .flat_map(|pixel| [pixel[0] + 0.1, 0.0, 0.0, 1.0])
            .collect()
    });
    let receipt = compare(
        &source,
        &reference,
        &DiffPolicy::for_class(ToleranceClass::Pointwise),
    )
    .expect("bounded report");
    assert_eq!(receipt.metrics.outlier_count, 100);
    assert_eq!(receipt.outliers.len(), 32);
    assert_eq!(receipt.outliers[0].x, 8);
    let repeat = compare(
        &source,
        &reference,
        &DiffPolicy::for_class(ToleranceClass::Pointwise),
    )
    .expect("repeat bounded report");
    assert_eq!(receipt.outliers, repeat.outliers);
}
