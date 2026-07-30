#[cfg(not(target_os = "macos"))]
use std::cell::RefCell;
#[cfg(not(target_os = "macos"))]
use std::fs;
#[cfg(not(target_os = "macos"))]
use std::path::PathBuf;
#[cfg(not(target_os = "macos"))]
use std::rc::Rc;
#[cfg(not(target_os = "macos"))]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(target_os = "macos"))]
use std::time::{Duration, Instant};

use gtk4::gio::prelude::ApplicationExt;
use rusttable_core::PhotoId;
use rusttable_ui::{CollectionControlAction, CollectionItem, CollectionProperty};

#[cfg(not(target_os = "macos"))]
use crate::macos::MacOpenTarget;
#[cfg(not(target_os = "macos"))]
use rusttable_testkit::fixtures::deterministic_compressed_raf;

use super::collection_bridge::apply_collection_action;
use super::{CollectionController, collection_filter_state};
#[cfg(not(target_os = "macos"))]
use super::{
    GtkCatalogController, MacApplicationBridge, apply_selection_projection, dispatch_open_request,
    thumbnails::ThumbnailLifecycle,
};

#[cfg(not(target_os = "macos"))]
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(not(target_os = "macos"))]
struct NativeOpenDirectory(PathBuf);

#[cfg(not(target_os = "macos"))]
impl NativeOpenDirectory {
    fn new() -> Self {
        let number = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rusttable-composition-native-open-{}-{number}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("native-open test directory");
        Self(path)
    }

    fn source(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn catalog(&self) -> PathBuf {
        self.0.join("catalog.redb")
    }
}

#[cfg(not(target_os = "macos"))]
impl Drop for NativeOpenDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn id(value: u128) -> PhotoId {
    PhotoId::new(value).expect("non-zero test photo identifier")
}

#[test]
fn collection_actions_project_filter_transitions_for_the_lighttable() {
    let mut controller = CollectionController::new([
        CollectionItem::new(id(1), "/photos/2026/holiday/IMG_0001.CR3"),
        CollectionItem::new(id(2), "/photos/2026/portraits/portrait.jpg"),
    ]);

    let initial = collection_filter_state(&controller.snapshot());
    assert_eq!(initial.controls().total_count(), 2);
    assert_eq!(initial.matching_photo_ids(), &[id(1), id(2)]);

    apply_collection_action(
        &mut controller,
        CollectionControlAction::SetSearchText {
            search_text: "portrait".to_owned(),
            generation: 1,
        },
    );
    let filtered = collection_filter_state(&controller.snapshot());
    assert_eq!(filtered.controls().result_count(), 1);
    assert_eq!(filtered.controls().search_text(), "portrait");
    assert_eq!(filtered.matching_photo_ids(), &[id(2)]);

    apply_collection_action(
        &mut controller,
        CollectionControlAction::SetProperty {
            property: CollectionProperty::Folders,
            generation: 2,
        },
    );
    apply_collection_action(
        &mut controller,
        CollectionControlAction::Clear { generation: 3 },
    );
    let cleared = collection_filter_state(&controller.snapshot());
    assert_eq!(cleared.controls().property(), CollectionProperty::Folders);
    assert_eq!(cleared.controls().result_count(), 2);
    assert_eq!(cleared.matching_photo_ids(), &[id(1), id(2)]);
}

#[test]
fn cold_start_binds_the_first_persisted_selection_in_collection_order() {
    let mut controller = CollectionController::new([
        CollectionItem::new(id(1), "/photos/2026/z-last.RAF"),
        CollectionItem::new(id(2), "/photos/2026/a-first.RAF"),
    ]);
    assert!(controller.select_only(id(1)));
    assert!(controller.toggle_selection(id(2)));

    assert_eq!(
        super::first_persisted_selected_photo(&controller),
        Some(id(2)),
        "cold launch must follow the visible collection order, not PhotoId order"
    );
}

#[test]
fn gtk_application_advertises_native_file_open_events() {
    let application = super::create_application();
    assert!(
        application
            .flags()
            .contains(gtk4::gio::ApplicationFlags::HANDLES_OPEN)
    );
}

// The GTK test helper cannot initialize GTK from the Darwin process main
// thread. Native-open behavior is covered on macOS by the installed-app
// Computer Use acceptance smoke; retain this GTK event-loop test elsewhere.
#[cfg(not(target_os = "macos"))]
#[gtk4::test]
fn native_open_import_dispatches_selection_through_active_projection() {
    let fixture = deterministic_compressed_raf();
    let directory = NativeOpenDirectory::new();
    let source = directory.source(fixture.source_name());
    fs::write(&source, fixture.bytes()).expect("RAF fixture");
    let source = fs::canonicalize(source).expect("canonical RAF fixture");

    let application = super::create_application();
    let shell = rusttable_ui::GtkShell::new(&application);
    let catalog = Rc::new(RefCell::new(GtkCatalogController::load_catalog_at(
        directory.catalog(),
    )));
    let collection = Rc::new(RefCell::new(catalog.borrow().collection_controller()));
    let active_shell = Rc::new(RefCell::new(Some(shell.clone())));
    let active_catalog = Rc::new(RefCell::new(Some(Rc::clone(&catalog))));
    let active_collection = Rc::new(RefCell::new((*collection.borrow()).clone()));
    let selected = Rc::new(RefCell::new(None));
    shell.set_photo_selected_handler({
        let selected = Rc::clone(&selected);
        let catalog = Rc::clone(&catalog);
        let collection = Rc::clone(&active_collection);
        let shell = shell.clone();
        move |photo_id, modifiers| {
            selected.replace(Some(photo_id));
            let _ = apply_selection_projection(&catalog, &collection, &shell, photo_id, modifiers);
        }
    });

    let mut bridge = MacApplicationBridge::default();
    assert!(bridge.mark_ready().is_none());
    let delivery = bridge.receive_paths([source.clone()]);
    let request = delivery.request().cloned().expect("native open request");
    assert!(matches!(
        request.targets(),
        [MacOpenTarget::Image(path)] if path == &source
    ));
    dispatch_open_request(
        &request,
        &active_shell,
        &active_catalog,
        &active_collection,
        &Rc::new(RefCell::new(ThumbnailLifecycle::default())),
        &crate::diagnostics::AppDiagnostics::default(),
    );

    let context = gtk4::glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    while selected.borrow().is_none() && Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    let photo_id = selected.borrow().expect("native-open photo selection");
    assert_eq!(catalog.borrow().selected_photo(), Some(photo_id));
    let collection = active_collection
        .borrow()
        .as_ref()
        .cloned()
        .expect("active collection after import");
    let snapshot = collection.snapshot();
    assert_eq!(
        snapshot
            .photo_states()
            .filter(|state| state.selected())
            .map(|state| state.photo_id())
            .collect::<Vec<_>>(),
        [photo_id]
    );
    assert_eq!(snapshot.toolbar().selected_count(), 1);
    assert!(snapshot.matching_photo_ids().any(|id| id == photo_id));
}
