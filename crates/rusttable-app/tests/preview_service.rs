use std::path::PathBuf;

use rusttable_app::PreviewService;
use rusttable_core::{Edit, EditId, PhotoId, Revision};
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
