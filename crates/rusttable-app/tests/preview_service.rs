use std::path::PathBuf;

use rusttable_app::PreviewService;
use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
    PhotoId, Revision,
};
use rusttable_image::DecodeLimits;
use rusttable_import::{FileSourceSnapshotReader, ImportSourceLimits, SourceSnapshotReader};
use rusttable_render::PreviewBounds;

#[test]
fn renders_the_committed_png_fixture_through_the_production_cpu_path() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/corpus/assets/raster-png-16-alpha.png");
    let edit = Edit::new(
        EditId::new(1).expect("valid edit ID"),
        PhotoId::new(1).expect("valid photo ID"),
        Revision::ZERO,
        [],
    )
    .expect("valid empty edit");
    let service = PreviewService::new(
        DecodeLimits::new(32 * 1024 * 1024, 4096, 4096, 16_777_216, 64 * 1024 * 1024)
            .expect("valid limits"),
        PreviewBounds::new(64, 64).expect("valid bounds"),
    );

    let source_limits = ImportSourceLimits::new(32 * 1024 * 1024).expect("source cap");
    let snapshot = FileSourceSnapshotReader
        .read_snapshot(&source, source_limits)
        .expect("fixture snapshot");
    let bytes = snapshot.materialize(source_limits).expect("fixture bytes");
    let output = service
        .render_bytes(&bytes, &edit)
        .expect("fixture renders");

    assert!(output.image().dimensions().width() <= 64);
    assert!(output.image().dimensions().height() <= 64);
    assert!(!output.image().pixels().is_empty());
}

#[test]
fn applies_registered_edits_through_the_production_cpu_pixelpipe() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/corpus/assets/raster-png-16-alpha.png");
    let empty_edit = edit([]);
    let adjusted_edit = edit([Operation::new(
        OperationId::new(2).expect("valid operation ID"),
        OperationKey::new("rusttable.exposure").expect("valid operation key"),
        true,
        [(
            ParameterName::new("stops").expect("valid parameter name"),
            ParameterValue::Scalar(FiniteF64::new(1.0).expect("finite parameter")),
        )],
    )
    .expect("valid operation")]);
    let service = PreviewService::new(
        DecodeLimits::new(32 * 1024 * 1024, 4096, 4096, 16_777_216, 64 * 1024 * 1024)
            .expect("valid limits"),
        PreviewBounds::new(64, 64).expect("valid bounds"),
    );
    let source_limits = ImportSourceLimits::new(32 * 1024 * 1024).expect("source cap");
    let snapshot = FileSourceSnapshotReader
        .read_snapshot(&source, source_limits)
        .expect("fixture snapshot");
    let bytes = snapshot.materialize(source_limits).expect("fixture bytes");

    let empty = service
        .render_bytes(&bytes, &empty_edit)
        .expect("empty fixture renders");
    let adjusted = service
        .render_bytes(&bytes, &adjusted_edit)
        .expect("adjusted fixture renders");

    assert_ne!(adjusted.image().pixels(), empty.image().pixels());
    assert_eq!(adjusted.provenance().source_edit_id(), adjusted_edit.id());
}

#[test]
fn geometry_frame_replacement_reaches_preview_with_transformed_alpha() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/corpus/assets/raster-png-16-alpha.png");
    let scalar =
        |value| ParameterValue::Scalar(FiniteF64::new(value).expect("finite geometry parameter"));
    let geometry_edit = edit([Operation::new(
        OperationId::new(3).expect("valid operation ID"),
        OperationKey::new("rusttable.enlargecanvas").expect("valid operation key"),
        true,
        [
            (
                ParameterName::new("percent_left").expect("parameter"),
                scalar(50.0),
            ),
            (
                ParameterName::new("percent_right").expect("parameter"),
                scalar(50.0),
            ),
            (
                ParameterName::new("percent_top").expect("parameter"),
                scalar(50.0),
            ),
            (
                ParameterName::new("percent_bottom").expect("parameter"),
                scalar(50.0),
            ),
            (
                ParameterName::new("color").expect("parameter"),
                ParameterValue::Integer(2),
            ),
        ],
    )
    .expect("valid enlarge-canvas operation")]);
    let service = PreviewService::new(
        DecodeLimits::new(32 * 1024 * 1024, 4096, 4096, 16_777_216, 64 * 1024 * 1024)
            .expect("valid limits"),
        PreviewBounds::new(64, 64).expect("valid bounds"),
    );
    let source_limits = ImportSourceLimits::new(32 * 1024 * 1024).expect("source cap");
    let snapshot = FileSourceSnapshotReader
        .read_snapshot(&source, source_limits)
        .expect("fixture snapshot");
    let bytes = snapshot.materialize(source_limits).expect("fixture bytes");

    let output = service
        .render_bytes(&bytes, &geometry_edit)
        .expect("geometry preview renders");

    assert_eq!(output.image().dimensions().width(), 8);
    assert_eq!(output.image().dimensions().height(), 5);
    assert_eq!(&output.image().pixels()[..4], &[0, 0, 255, 255]);
    assert_eq!(output.provenance().source_edit_id(), geometry_edit.id());
}

fn edit<const N: usize>(operations: [Operation; N]) -> Edit {
    Edit::new(
        EditId::new(1).expect("valid edit ID"),
        PhotoId::new(1).expect("valid photo ID"),
        Revision::ZERO,
        operations,
    )
    .expect("valid edit")
}
