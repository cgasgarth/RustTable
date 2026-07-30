use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use rusttable_app::gtk_controller::{GtkCatalogController, GtkCatalogState};
use rusttable_app::gtk_preview_controller::{GtkPreviewController, GtkPreviewState};
use rusttable_app::macos::{MacApplicationBridge, MacOpenTarget};
use rusttable_app::workspace::run_raster_import;
use rusttable_import::RasterImportCancellation;
use rusttable_testkit::fixtures::deterministic_compressed_raf;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct AcceptanceDirectory(PathBuf);

impl AcceptanceDirectory {
    fn new() -> Self {
        let number = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rusttable-native-open-raf-{}-{number}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("acceptance directory");
        Self(path)
    }

    fn source(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn catalog(&self) -> PathBuf {
        self.0.join("catalog.redb")
    }
}

impl Drop for AcceptanceDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn native_open_raf_reaches_import_and_gtk_preview() {
    let fixture = deterministic_compressed_raf();
    let directory = AcceptanceDirectory::new();
    let source = directory.source(fixture.source_name());
    fs::write(&source, fixture.bytes()).expect("deterministic RAF fixture");
    let canonical_source = fs::canonicalize(&source).expect("canonical RAF fixture");

    let mut bridge = MacApplicationBridge::default();
    assert!(bridge.mark_ready().is_none());
    let delivery = bridge.receive_paths([canonical_source.clone()]);
    let request = delivery.request().expect("native open request");
    assert!(matches!(request.targets(), [MacOpenTarget::Image(path)] if path == &canonical_source));

    let batch = run_raster_import(
        &directory.catalog(),
        request.image_paths().map(PathBuf::from).collect(),
        &RasterImportCancellation::default(),
        &|_| {},
    );
    let receipt = batch.receipts().next().expect("RAF import receipt");
    assert_eq!(
        receipt.status,
        rusttable_import::RasterImportStatus::Imported
    );
    let photo_id = receipt.photo_id.expect("imported photo");

    let mut catalog = GtkCatalogController::load_catalog_at(directory.catalog());
    assert!(matches!(catalog.state(), GtkCatalogState::Ready(_)));
    assert!(catalog.select_photo(photo_id));
    let preview = GtkPreviewController::new().render_selected(&catalog);
    let GtkPreviewState::Ready(preview) = preview else {
        panic!("native-open RAF must render a GTK preview");
    };
    assert_eq!(preview.photo_id(), photo_id);
    assert!(!preview.pixels().is_empty());
}
