use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rusttable_app::{BasicEditCommand, CatalogPreviewError, CatalogPreviewService, PreviewService};
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
fn transient_basic_edit_preview_changes_pixels_without_mutating_catalog() {
    let fixture = qualified_fixture();
    let workspace = TempWorkspace::new();
    let source = workspace.source_root.join("imports/preview.png");
    fs::create_dir_all(source.parent().expect("source has a parent")).expect("source directory");
    fs::copy(fixture, &source).expect("fixture copy");
    persist_import_and_neutral_edit(&workspace.catalog, &source);

    let repository = RedbCatalogRepository::open(&workspace.catalog).expect("open catalog");
    let current = repository
        .list()
        .expect("list current edits")
        .into_iter()
        .find(|edit| edit.photo_id() == workspace.photo_id)
        .expect("import has a current basic edit");
    let persisted_before = repository.list().expect("snapshot persisted edits");
    let command = BasicEditCommand::from_edit(&current).expect("current edit is basic-editable");
    let values = command.values();
    let transient = command
        .build_replacement(1.0, values.rgb_red(), values.rgb_green(), values.rgb_blue())
        .expect("non-default basic edit");

    let service = CatalogPreviewService::new(preview_service());
    let baseline = service
        .render_edit(&workspace.source_root, &current, &repository)
        .expect("current edit preview");
    let transient_output = service
        .render_edit(&workspace.source_root, &transient, &repository)
        .expect("transient CPU preview");

    assert_eq!(
        baseline.image().dimensions(),
        transient_output.image().dimensions()
    );
    assert_ne!(baseline.image().pixels(), transient_output.image().pixels());

    drop(repository);
    let persisted_after = RedbCatalogRepository::open(&workspace.catalog)
        .expect("reopen catalog after transient preview")
        .list()
        .expect("list edits after transient preview");
    assert_eq!(persisted_after, persisted_before);
    assert_eq!(
        persisted_after
            .iter()
            .find(|edit| edit.id() == current.id()),
        Some(&current)
    );
}

#[test]
fn transient_preview_rejects_a_draft_for_the_wrong_photo() {
    let fixture = qualified_fixture();
    let workspace = TempWorkspace::new();
    let source = workspace.source_root.join("imports/preview.png");
    fs::create_dir_all(source.parent().expect("source has a parent")).expect("source directory");
    fs::copy(fixture, &source).expect("fixture copy");
    persist_import_and_neutral_edit(&workspace.catalog, &source);

    let repository = RedbCatalogRepository::open(&workspace.catalog).expect("open catalog");
    let current = repository
        .list()
        .expect("list current edits")
        .into_iter()
        .next()
        .expect("import has a current edit");
    let wrong_photo = PhotoId::new(9_999).expect("wrong photo ID");
    let wrong_photo_draft = Edit::from_parts(
        EditId::new(9_998).expect("draft edit ID"),
        wrong_photo,
        current.base_photo_revision(),
        current.revision(),
        current.operations().cloned(),
    )
    .expect("wrong-photo draft remains structurally valid");

    let result = CatalogPreviewService::new(preview_service()).render_edit(
        &workspace.source_root,
        &wrong_photo_draft,
        &repository,
    );

    assert!(matches!(
        result,
        Err(CatalogPreviewError::UnknownPhoto { photo_id }) if photo_id == wrong_photo
    ));
}

fn persist_import_and_neutral_edit(catalog: &Path, source: &Path) {
    let photo_id = PhotoId::new(101).expect("photo ID");
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
    .expect("PNG import");
    repository
        .commit_new(&neutral_edit(EditId::new(201).expect("edit ID"), photo_id))
        .expect("neutral basic edit");
}

fn neutral_edit(edit_id: EditId, photo_id: PhotoId) -> Edit {
    Edit::new(
        edit_id,
        photo_id,
        Revision::ZERO,
        [
            operation(301, "rusttable.exposure", [("stops", 0.0)]),
            operation(
                302,
                "rusttable.rgb_gain",
                [("red", 1.0), ("green", 1.0), ("blue", 1.0)],
            ),
        ],
    )
    .expect("neutral edit")
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

fn qualified_fixture() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest = FixtureManifest::parse(
        &fs::read_to_string(root.join("fixtures/manifest.toml")).expect("fixture manifest"),
    )
    .expect("valid fixture manifest");
    let entry = manifest
        .fixture("corpus.raster.png.16-alpha")
        .expect("registered PNG fixture");
    let fixture = entry.canonical_path(&root).expect("canonical fixture path");
    let bytes = fs::read(&fixture).expect("fixture bytes");
    assert_eq!(
        u64::try_from(bytes.len()).expect("fixture length"),
        entry.size
    );
    assert_eq!(sha256_hex(&bytes), entry.sha256);
    qualify_binary(entry, &bytes).expect("qualified PNG fixture");
    let probe = FileImageInput::new(decode_limits())
        .probe_path(&fixture)
        .expect("production decoder probe");
    assert_eq!(probe.dimensions().width(), 4);
    assert_eq!(probe.dimensions().height(), 3);
    fixture
}

fn preview_service() -> PreviewService {
    PreviewService::new(
        decode_limits(),
        PreviewBounds::new(1_536, 1_536).expect("preview bounds"),
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
    photo_id: PhotoId,
}

impl TempWorkspace {
    fn new() -> Self {
        let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "rusttable-transient-edit-preview-{}-{sequence}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temporary workspace");
        Self {
            source_root: root.join("source-root"),
            catalog: root.join("catalog.redb"),
            root,
            photo_id: PhotoId::new(101).expect("photo ID"),
        }
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
