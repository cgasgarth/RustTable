use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rusttable_app::{CatalogPreviewRequest, CatalogPreviewService, PreviewService};
use rusttable_catalog::{CatalogState, EditRepository, SourcePath};
use rusttable_catalog_store::RedbCatalogRepository;
use rusttable_core::{
    AssetId, Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_image::{DecodeLimits, ImageInput};
use rusttable_image_io::FileImageInput;
use rusttable_import::{
    FileSourceSnapshotReader, ImportSourceLimits, SourceImportRequest, SourceImportService,
};
use rusttable_metadata::{ExifMetadataInput, MetadataLimits};
use rusttable_render::PreviewBounds;
use rusttable_testkit::fixtures::{FixtureManifest, qualify_binary, sha256_hex};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[test]
fn renders_a_persisted_import_and_two_operation_edit_from_a_fresh_catalog() {
    let fixture = qualified_fixture();
    let workspace = TempWorkspace::new();
    let source = workspace.source_root.join("imports/preview.png");
    fs::create_dir_all(source.parent().unwrap()).expect("source directory");
    fs::copy(fixture, &source).expect("fixture copy");

    let photo_id = PhotoId::new(101).expect("photo ID");
    let edit_id = EditId::new(201).expect("edit ID");
    persist_catalog(&workspace.catalog, &source, photo_id);
    persist_edit(&workspace.catalog, edit_id, photo_id);

    let repository = RedbCatalogRepository::open(&workspace.catalog).expect("reopen catalog");
    let output = CatalogPreviewService::new(preview_service())
        .render(
            CatalogPreviewRequest::new(&workspace.source_root, photo_id, edit_id),
            &repository,
            &repository,
        )
        .expect("persisted preview renders");

    assert_eq!(output.provenance().source_photo_id(), photo_id);
    assert_eq!(output.provenance().source_edit_id(), edit_id);
    assert!(output.image().dimensions().width() <= 64);
    assert!(output.image().dimensions().height() <= 64);
    assert!(!output.image().pixels().is_empty());
}

fn qualified_fixture() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest = FixtureManifest::parse(
        &fs::read_to_string(root.join("fixtures/manifest.toml")).expect("fixture manifest"),
    )
    .expect("valid fixture manifest");
    let entry = manifest
        .fixture("corpus.raster.png.16-alpha")
        .expect("registered parser-qualified fixture");
    let fixture = entry.canonical_path(&root).expect("canonical fixture path");
    let bytes = fs::read(&fixture).expect("fixture bytes");
    assert_eq!(u64::try_from(bytes.len()).unwrap(), entry.size);
    assert_eq!(sha256_hex(&bytes), entry.sha256);
    qualify_binary(entry, &bytes).expect("qualified PNG fixture");
    let probe = FileImageInput::new(decode_limits())
        .probe_path(&fixture)
        .expect("production decoder probe");
    assert_eq!(probe.dimensions().width(), 4);
    assert_eq!(probe.dimensions().height(), 3);
    fixture
}

fn persist_catalog(catalog: &Path, source: &Path, photo_id: PhotoId) {
    let mut state = CatalogState::new();
    let mut repository = RedbCatalogRepository::open(catalog).expect("new catalog");
    let request = SourceImportRequest::new(
        photo_id,
        AssetId::new(102).expect("asset ID"),
        SourcePath::new("imports/preview.png").expect("logical source"),
        source.to_owned(),
    );
    SourceImportService::inspect_and_register(
        &mut state,
        Revision::ZERO,
        &request,
        ImportSourceLimits::new(1024 * 1024).expect("source limit"),
        &mut repository,
        &FileSourceSnapshotReader,
        &FileImageInput::new(decode_limits()),
        &ExifMetadataInput::new(metadata_limits()),
    )
    .expect("ordinary source import");
}

fn persist_edit(catalog: &Path, edit_id: EditId, photo_id: PhotoId) {
    let edit = Edit::new(
        edit_id,
        photo_id,
        Revision::ZERO,
        [
            operation(301, "rusttable.exposure", [("stops", 0.5)]),
            operation(
                302,
                "rusttable.rgb_gain",
                [("red", 1.0), ("green", 0.75), ("blue", 0.5)],
            ),
        ],
    )
    .expect("exact immutable edit");
    let mut repository = RedbCatalogRepository::open(catalog).expect("catalog");
    repository.commit_new(&edit).expect("persist edit");
}

fn operation<const N: usize>(id: u128, key: &str, values: [(&str, f64); N]) -> Operation {
    Operation::new(
        OperationId::new(id).expect("operation ID"),
        OperationKey::new(key).expect("operation key"),
        true,
        values.map(|(name, value)| {
            (
                ParameterName::new(name).expect("parameter name"),
                ParameterValue::Scalar(FiniteF64::new(value).expect("finite parameter")),
            )
        }),
    )
    .expect("operation")
}

fn preview_service() -> PreviewService {
    PreviewService::new(
        decode_limits(),
        PreviewBounds::new(64, 64).expect("preview bounds"),
    )
}

fn decode_limits() -> DecodeLimits {
    DecodeLimits::new(1024 * 1024, 4096, 4096, 16_777_216, 64 * 1024 * 1024).expect("decode limits")
}

fn metadata_limits() -> MetadataLimits {
    MetadataLimits::new(1024 * 1024, 128 * 1024, 128, 128, 8, 4096, 64 * 1024)
        .expect("metadata limits")
}

struct TempWorkspace {
    root: PathBuf,
    source_root: PathBuf,
    catalog: PathBuf,
}

impl TempWorkspace {
    fn new() -> Self {
        let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "rusttable-catalog-preview-{}-{sequence}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temporary workspace");
        Self {
            source_root: root.join("source-root"),
            catalog: root.join("catalog.redb"),
            root,
        }
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
