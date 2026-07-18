use rusttable_testkit::image_diff::{
    AlphaMode, ArtifactKind, DiffPolicy, ImageBuffer, ToleranceClass, compare,
};

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
    let source = ImageBuffer::rgba(2, 1, vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0]);
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
}

#[test]
fn dimensions_alpha_and_nonfinite_values_fail_closed() {
    let source = ImageBuffer::rgba(1, 1, vec![0.0, 0.0, 0.0, 1.0]);
    let dimensions = ImageBuffer::rgba(2, 1, vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
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
