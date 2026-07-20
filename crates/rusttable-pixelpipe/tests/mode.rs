use rusttable_color::{ColorEncoding, Precision};
use rusttable_image::{ImageDimensions, Orientation, PixelFormat, Roi};
use rusttable_pixelpipe::{
    AnalysisRequest, Background, BasicStackFixture, CacheKey, ColorIdentity, DegradationPolicy,
    EmbeddedPreviewProvenance, ModeFinding, ModeOperationCapability, ModePlanner,
    ModePlanningError, ModeQuality, ModeRequest, OperationInclusion, OutputSpec,
    PipelineGeneration, PipelinePurpose, PipelineSnapshot, PipelineSnapshotInput, SourceDescriptor,
    SourceIdentity, TargetIdentity,
};
use rusttable_processing::operation_stack::{OperationStackSnapshot, OperationStackTemplate};

fn snapshot() -> PipelineSnapshot {
    let dimensions = ImageDimensions::new(4, 3).expect("dimensions");
    let color = ColorIdentity::new(ColorEncoding::SrgbD65, 1).expect("color");
    let source = SourceDescriptor::new(
        SourceIdentity::new([4; 32]),
        dimensions,
        Orientation::Normal,
        Roi::full(dimensions),
        PixelFormat::rgba8(),
        color,
    )
    .expect("source");
    let output = OutputSpec::new(
        dimensions,
        Roi::full(dimensions),
        PixelFormat::rgba8(),
        color,
        Background::transparent(),
    )
    .expect("output");
    PipelineSnapshot::new(
        PipelineSnapshotInput::new(
            PipelineGeneration::new(1).expect("generation"),
            source,
            OperationStackSnapshot::new(OperationStackTemplate::raster_basic()),
            output,
            PipelinePurpose::Preview,
            rusttable_pixelpipe::ImplementationIdentity::new("rusttable.mode.test", 1, "test")
                .expect("implementation"),
        )
        .expect("input"),
    )
    .expect("snapshot")
}

fn output(width: u32, height: u32) -> OutputSpec {
    let dimensions = ImageDimensions::new(width, height).expect("dimensions");
    OutputSpec::new(
        dimensions,
        Roi::full(dimensions),
        PixelFormat::rgba8(),
        ColorIdentity::new(ColorEncoding::SrgbD65, 1).expect("color"),
        Background::transparent(),
    )
    .expect("output")
}

#[test]
fn interactive_preview_records_only_declared_approximation() {
    let request = ModeRequest::preview(output(4, 3), TargetIdentity::preview_surface("main"))
        .with_quality(ModeQuality::Interactive);
    let plan = ModePlanner
        .plan(
            &snapshot(),
            request,
            BasicStackFixture::raster().operations(),
        )
        .expect("plan");
    assert_eq!(plan.approximations().len(), 1);
    assert_eq!(plan.approximations()[0].1.as_str(), "exposure.preview-fast");
    assert!(plan.receipt().degraded());
}

#[test]
fn full_and_export_are_exact_and_have_distinct_mode_identity() {
    let full = ModePlanner
        .plan(
            &snapshot(),
            ModeRequest::full(output(4, 3), TargetIdentity::consumer("canvas")),
            BasicStackFixture::raster().operations(),
        )
        .expect("full");
    let export = ModePlanner
        .plan(
            &snapshot(),
            ModeRequest::export(output(4, 3), TargetIdentity::export_destination("png")),
            BasicStackFixture::raster().operations(),
        )
        .expect("export");
    assert!(full.approximations().is_empty());
    assert!(export.approximations().is_empty());
    assert!(!full.receipt().degraded());
    assert_ne!(full.identity(), export.identity());
    assert_ne!(
        CacheKey::from_mode_plan(&full),
        CacheKey::from_mode_plan(&export)
    );
}

#[test]
fn thumbnail_is_bounded_and_explicitly_allows_embedded_preview_only() {
    let thumbnail = ModePlanner
        .plan(
            &snapshot(),
            ModeRequest::thumbnail(output(2, 2), TargetIdentity::consumer("thumb")),
            BasicStackFixture::raster().operations(),
        )
        .expect("thumbnail");
    assert!(thumbnail.approximations().is_empty());
    let oversized = ModePlanner.plan(
        &snapshot(),
        ModeRequest::thumbnail(output(8, 8), TargetIdentity::consumer("thumb")),
        BasicStackFixture::raster().operations(),
    );
    assert!(matches!(oversized, Err(ModePlanningError::Request(_))));
    let embedded = ModeRequest::thumbnail(output(2, 2), TargetIdentity::consumer("thumb"))
        .with_embedded_preview(EmbeddedPreviewProvenance::new([9; 32]));
    assert!(
        ModePlanner
            .plan(
                &snapshot(),
                embedded,
                BasicStackFixture::raster().operations()
            )
            .is_ok()
    );
}

#[test]
fn unsupported_masks_and_exact_operations_are_typed() {
    let result = ModePlanner.plan(
        &snapshot(),
        ModeRequest::preview(output(4, 3), TargetIdentity::consumer("main"))
            .with_analysis(AnalysisRequest::Required),
        BasicStackFixture::raster().operations(),
    );
    assert!(matches!(
        result,
        Err(ModePlanningError::Finding(ModeFinding::AnalysisUnsupported))
    ));
    let unsupported = ModeOperationCapability::new(99, OperationInclusion::Processing, true)
        .exact(false)
        .for_purposes([PipelinePurpose::Full]);
    let result = ModePlanner.plan(
        &snapshot(),
        ModeRequest::full(output(4, 3), TargetIdentity::consumer("main")),
        &[unsupported],
    );
    assert!(matches!(
        result,
        Err(ModePlanningError::Finding(
            ModeFinding::ExactUnavailable { .. }
        ))
    ));
}

#[test]
fn request_identity_changes_for_precision_interpolation_and_degradation() {
    let base = ModeRequest::preview(output(4, 3), TargetIdentity::consumer("main"));
    let first = ModePlanner
        .plan(
            &snapshot(),
            base.clone(),
            BasicStackFixture::raster().operations(),
        )
        .expect("base");
    let second = ModePlanner
        .plan(
            &snapshot(),
            base.with_precision(Precision::F64)
                .with_degradation(DegradationPolicy::None),
            BasicStackFixture::raster().operations(),
        )
        .expect("changed");
    assert_ne!(first.identity(), second.identity());
}
