#![forbid(unsafe_code)]

use std::cell::{Cell, RefCell};
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use gtk4::prelude::*;
use rusttable_app::gtk_controller::GtkDarkroomEditController;
use rusttable_app::gtk_controller::colorzones_edit::ColorZonesEditAction;
use rusttable_catalog::EditRepository;
use rusttable_catalog_store::RedbCatalogRepository;
use rusttable_core::{Edit, EditId, OperationId, PhotoId, Revision};
use rusttable_processing::builtin_registry;
use rusttable_ui::gtk_shell::{
    DARKROOM_PANEL_WIDTHS, DARKTABLE_DESKTOP_SPEC, GtkShell, WorkspaceRole,
};
use rusttable_ui::iop::colorzones::{ColorZonesGtkActionHandler, ColorZonesGtkHandlerOutcome};
use rusttable_ui::{
    CollectionControlState, CollectionFilterState, CollectionProperty, HistogramData,
    LighttableColorLabel, LighttablePhotoState, LighttableRating, LighttableToolbarState,
    PhotoCardViewModel, PhotoDetailViewModel, PhotoFactViewModel, PhotoWorkspaceViewModel,
    PresentationText, PreviewDimensions, Rgba8PreviewMetadata, ViewportGeneration,
};

#[path = "darkroom_shell_runtime_smoke/render.rs"]
mod render;

use render::{find_widget, find_widget_with_prefix, named_controller, render_widget};

static SMOKE_CATALOG_ID: AtomicU64 = AtomicU64::new(0);

struct SmokeCatalog {
    path: PathBuf,
}

impl SmokeCatalog {
    fn seed(edit: &Edit) -> Self {
        let id = SMOKE_CATALOG_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rusttable-darkroom-shell-colorzones-{}-{id}.redb",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let mut repository =
            RedbCatalogRepository::open(&path).expect("open Color Zones smoke catalog");
        repository
            .commit_new(edit)
            .expect("seed canonical Color Zones edit");
        drop(repository);
        Self { path }
    }

    fn load_edit(&self, edit_id: EditId) -> Edit {
        RedbCatalogRepository::open(&self.path)
            .expect("reopen Color Zones smoke catalog")
            .find_by_edit_id(edit_id)
            .expect("read Color Zones smoke edit")
            .expect("persisted Color Zones smoke edit")
    }
}

impl Drop for SmokeCatalog {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn main() {
    gtk4::init().expect("GTK must initialize for the app-shell runtime smoke");
    prohibit_macos_test_activation();
    app_shell_transition_paints_darkroom_titles();
    println!("Darkroom app-shell runtime smoke passed");
}

#[cfg(target_os = "macos")]
fn prohibit_macos_test_activation() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    let marker = MainThreadMarker::new().expect("custom GTK smoke must start on the main thread");
    let application = NSApplication::sharedApplication(marker);
    application.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    assert_eq!(
        application.activationPolicy(),
        NSApplicationActivationPolicy::Prohibited,
        "automated GTK smoke must use the non-activating macOS policy"
    );
}

#[cfg(not(target_os = "macos"))]
fn prohibit_macos_test_activation() {}

fn app_shell_transition_paints_darkroom_titles() {
    let application = gtk4::Application::new(
        Some("com.cgasgarth.rusttable.test.darkroom-shell-runtime"),
        gtk4::gio::ApplicationFlags::default(),
    );
    application
        .register(None::<&gtk4::gio::Cancellable>)
        .expect("test GTK application must start before constructing windows");
    let display = gtk4::gdk::Display::default().expect("test display");
    rusttable_ui::install_darktable_theme(&display);
    let shell = GtkShell::new(&application);
    #[cfg(target_os = "macos")]
    assert!(
        shell.window().is_decorated(),
        "ordinary macOS RustTable windows must retain the title bar and traffic lights"
    );
    shell.window().set_default_size(1_228, 768);
    let root: gtk4::Widget = shell.window().clone().upcast();
    let rail = find_widget(&root, "darkroom-left-panel").expect("darkroom left rail");
    let workspace_stack = find_widget(&root, "center-workspace")
        .expect("center workspace")
        .downcast::<gtk4::Stack>()
        .expect("center workspace is a stack");
    let left_stack = find_widget(&root, "left-panel-stack")
        .expect("left panel stack")
        .downcast::<gtk4::Stack>()
        .expect("left panel stack is a stack");
    let left_split = find_widget(&root, "desktop-left-split")
        .expect("desktop left split")
        .downcast::<gtk4::Paned>()
        .expect("desktop left split is a paned");
    let lighttable_page = find_widget(&root, "lighttable-page").expect("lighttable page");
    let photo_id = PhotoId::new(949).expect("test photo id");
    let workspace = test_workspace(photo_id);
    let colorzones_operation_id = OperationId::new(0xc701).expect("Color Zones operation id");
    let colorzones_operation = builtin_registry()
        .materialize_operation("rusttable.colorzones", colorzones_operation_id)
        .expect("canonical Color Zones operation");
    let colorzones_edit = Edit::from_parts(
        EditId::new(0xc702).expect("Color Zones edit id"),
        photo_id,
        Revision::ZERO,
        Revision::from_u64(7),
        [colorzones_operation],
    )
    .expect("canonical Color Zones edit");
    let colorzones_edit_id = colorzones_edit.id();
    let colorzones_catalog = SmokeCatalog::seed(&colorzones_edit);
    let darkroom_edit_controller = Rc::new(RefCell::new(GtkDarkroomEditController::new(Some(
        colorzones_catalog.path.clone(),
    ))));
    let modules = darkroom_edit_controller
        .borrow_mut()
        .select_photo(photo_id)
        .expect("project canonical Color Zones instance")
        .clone();
    let colorzones_widget_id = modules
        .module_target("colorzones", Some(colorzones_operation_id))
        .expect("projected Color Zones instance")
        .widget_id();
    let published_revision = Rc::new(Cell::new(Revision::ZERO));
    let colorzones_handler: ColorZonesGtkActionHandler = Rc::new({
        let controller = Rc::clone(&darkroom_edit_controller);
        let action_shell = shell.clone();
        let published_revision = Rc::clone(&published_revision);
        move |settled| {
            let action = ColorZonesEditAction::from(settled);
            match controller.borrow_mut().apply_colorzones(&action) {
                Ok(outcome) => {
                    action_shell.update_darkroom_module_stack_snapshot(
                        outcome.modules(),
                        outcome.revision(),
                    );
                    if outcome.processing_changed() {
                        let generation = ViewportGeneration::new(2);
                        action_shell.begin_darkroom_selection(photo_id, generation);
                        let metadata = thumbnail_metadata();
                        let histogram =
                            HistogramData::from_rgba8(metadata.dimensions(), metadata.pixels())
                                .expect("edited Color Zones histogram");
                        action_shell
                            .set_darkroom_preview_result_for_edit(
                                generation,
                                &metadata,
                                Ok(histogram),
                                colorzones_edit_id,
                                outcome.revision(),
                            )
                            .expect("edited Color Zones selected preview publishes");
                        action_shell
                            .set_darkroom_preview_thumbnail_for_edit(
                                generation,
                                &metadata,
                                colorzones_edit_id,
                                outcome.revision(),
                            )
                            .expect("edited Color Zones navigation preview publishes");
                        published_revision.set(outcome.revision());
                    }
                    ColorZonesGtkHandlerOutcome::Commit {
                        revision: outcome.revision(),
                    }
                }
                Err(_) => ColorZonesGtkHandlerOutcome::Rollback,
            }
        }
    });
    shell.set_colorzones_action_handler(Some(colorzones_handler));
    shell.set_darkroom_module_stack(&modules, None);

    shell.show_workspace(WorkspaceRole::Lighttable);
    map_inert_test_window(shell.window());
    settle_gtk_until(
        || {
            lighttable_page.is_mapped()
                && lighttable_page.allocated_width() > 1
                && lighttable_page.allocated_height() > 0
        },
        || {
            format!(
                "initial lighttable={}x{}, mapped={}",
                lighttable_page.allocated_width(),
                lighttable_page.allocated_height(),
                lighttable_page.is_mapped()
            )
        },
    );
    shell.set_photo_workspace(&workspace);
    shell.set_collection_filter_state(&test_collection(photo_id));
    assert!(shell.open_photo(photo_id), "selected photo opens darkroom");
    shell.begin_darkroom_selection(photo_id, ViewportGeneration::new(1));
    let metadata = thumbnail_metadata();
    shell
        .set_photo_thumbnail(photo_id, &metadata)
        .expect("bounded navigation and filmstrip thumbnail");
    let histogram = HistogramData::from_rgba8(metadata.dimensions(), metadata.pixels())
        .expect("test histogram");
    shell
        .set_darkroom_preview_result(ViewportGeneration::new(1), &metadata, Ok(histogram))
        .expect("darkroom preview publishes");
    shell.show_workspace(WorkspaceRole::Lighttable);
    shell.show_workspace(WorkspaceRole::Darkroom);
    left_split.set_position(i32::from(DARKROOM_PANEL_WIDTHS.left_px));
    settle_gtk_until(
        || {
            rail.is_mapped()
                && rail.allocated_width() > 1
                && left_stack.allocated_width() > 0
                && left_stack.allocated_height() > 0
        },
        || {
            format!(
                "workspace={:?}, left={:?}, stack={}x{}, rail={}x{}, mapped={}",
                workspace_stack.visible_child_name(),
                left_stack.visible_child_name(),
                left_stack.allocated_width(),
                left_stack.allocated_height(),
                rail.allocated_width(),
                rail.allocated_height(),
                rail.is_mapped()
            )
        },
    );
    assert!(
        !shell.window().is_active(),
        "automated GTK smoke must not activate or steal focus"
    );
    assert!(
        left_stack.allocated_width() <= i32::from(DARKROOM_PANEL_WIDTHS.left_px) + 2,
        "active darkroom rail must honor the native divider, got {}px",
        left_stack.allocated_width()
    );
    assert_darkroom_titles_are_allocated(&shell);
    assert_mounted_colorzones_geometry_and_paint(
        &shell,
        &root,
        &colorzones_widget_id,
        &colorzones_catalog,
        colorzones_edit_id,
        &published_revision,
    );
    assert_darkroom_chrome_matches_runtime_geometry(&shell);
    assert_lighttable_preview_geometry(&shell, photo_id);
}

fn map_inert_test_window(window: &gtk4::ApplicationWindow) {
    // Geometry and paint assertions need a mapped native surface. Mapping it
    // transparently with `set_visible` preserves that coverage without the
    // focus request made by `Window::present`.
    window.set_focusable(false);
    window.set_opacity(1.0 / 255.0);
    window.set_visible(true);
}

fn assert_lighttable_preview_geometry(shell: &GtkShell, photo_id: PhotoId) {
    let root: gtk4::Widget = shell.window().clone().upcast();
    let lighttable_selector = find_widget(&root, "view-lighttable")
        .expect("header Lighttable selector")
        .downcast::<gtk4::Label>()
        .expect("header Lighttable selector is a direct label link");
    shell.show_workspace(WorkspaceRole::Lighttable);
    assert!(
        lighttable_selector.has_css_class("active"),
        "header Lighttable selector reflects the visible workspace"
    );
    let lighttable_grid = find_widget(&root, "lighttable-grid").expect("Lighttable grid");
    let thumbnail_name = format!("photo-thumbnail-{photo_id}");
    settle_gtk_until(
        || {
            find_widget(&lighttable_grid, &thumbnail_name)
                .is_some_and(|thumbnail| thumbnail.allocated_height() >= 400)
        },
        || {
            find_widget(&lighttable_grid, &thumbnail_name).map_or_else(
                || "full-preview thumbnail missing".to_owned(),
                |thumbnail| {
                    format!(
                        "full-preview thumbnail={}x{}",
                        thumbnail.allocated_width(),
                        thumbnail.allocated_height()
                    )
                },
            )
        },
    );
    settle_next_gtk_frame();
    settle_gtk_until(
        || {
            find_widget(&lighttable_grid, &thumbnail_name)
                .is_some_and(|thumbnail| thumbnail.allocated_height() >= 400)
        },
        || "full-preview thumbnail did not stabilize after the header switch".to_owned(),
    );
    shell
        .set_photo_thumbnail(photo_id, &thumbnail_metadata())
        .expect("ready thumbnail publishes on the initial preview layout");
    settle_next_gtk_frame();
    let thumbnail = find_widget(&lighttable_grid, &thumbnail_name).expect("full-preview thumbnail");
    let width = thumbnail.allocated_width();
    let height = thumbnail.allocated_height();
    assert!(
        width >= 600 && height >= 400,
        "full preview must occupy the center canvas, got {width}x{height}"
    );
    let picture = find_widget(&thumbnail, &format!("{thumbnail_name}-image"))
        .expect("full-preview picture")
        .downcast::<gtk4::Picture>()
        .expect("full-preview image is a GTK picture");
    assert_eq!(
        picture.content_fit(),
        gtk4::ContentFit::Contain,
        "full preview must preserve the rendered image aspect ratio"
    );
    let paintable = picture.paintable().expect("ready full-preview texture");
    assert!(
        paintable.intrinsic_width() * 20 == paintable.intrinsic_height() * 32,
        "full preview texture must preserve source geometry, got {}x{}",
        paintable.intrinsic_width(),
        paintable.intrinsic_height()
    );
    assert_lighttable_footer_and_chrome(&root);
    let darkroom_selector = find_widget(&root, "view-darkroom")
        .expect("header Darkroom selector")
        .downcast::<gtk4::Label>()
        .expect("header Darkroom selector is a direct label link");
    shell.show_workspace(WorkspaceRole::Darkroom);
    assert!(
        darkroom_selector.has_css_class("active"),
        "header Darkroom selector reflects the visible workspace"
    );
    settle_gtk_until(
        || find_widget(&root, "darkroom-viewport").is_some_and(|viewport| viewport.is_mapped()),
        || "darkroom viewport did not remap".to_owned(),
    );
    settle_next_gtk_frame();
}

fn assert_lighttable_footer_and_chrome(root: &gtk4::Widget) {
    for id in [
        "lighttable-footer-rating-1",
        "lighttable-footer-rating-5",
        "lighttable-footer-color-0",
        "lighttable-footer-color-4",
        "lighttable-layout-preview",
    ] {
        let control = find_widget(root, id).expect("lighttable footer control");
        assert!(
            control.is_visible() && control.allocated_width() > 0,
            "{id} must be visible in the bottom composition"
        );
    }
    let footer_organization =
        find_widget(root, "lighttable-footer-organization").expect("footer organization controls");
    let footer_bounds = footer_organization
        .compute_bounds(root)
        .expect("footer organization bounds");
    assert!(
        footer_bounds.x() < 360.0 && footer_bounds.width() >= 150.0,
        "rating and color controls must occupy the footer start: {footer_bounds:?}"
    );
    assert!(
        render_widget(root).bright_pixels(footer_bounds) >= 30,
        "rating stars and color swatches must paint in the footer"
    );
    assert!(
        find_widget(root, "right-module-search").is_none(),
        "lighttable must not paint a floating right-rail search entry"
    );
    for id in [
        "lighttable-import",
        "lighttable-copy-import",
        "lighttable-import-parameters",
        "lighttable-display-controls",
    ] {
        let control = find_widget(root, id).expect("implemented lighttable chrome");
        assert!(
            control.is_visible() && control.allocated_width() > 0,
            "{id} must occupy truthful Lighttable chrome geometry"
        );
    }
    assert!(
        find_widget(root, "lighttable-import")
            .expect("add-to-library action")
            .is_sensitive(),
        "implemented add-to-library action must remain available"
    );
    for id in ["lighttable-copy-import", "lighttable-import-parameters"] {
        assert!(
            !find_widget(root, id)
                .expect("truthful import placeholder")
                .is_sensitive(),
            "{id} must not imply unavailable import behavior"
        );
    }
    for id in ["lighttable-rating-1", "lighttable-color-0"] {
        assert!(
            find_widget(root, id).is_none(),
            "{id} must not duplicate the footer organization controls"
        );
    }
    for id in ["header-import", "header-preferences"] {
        assert!(
            find_widget(root, id).is_none(),
            "{id} must not drift into the persistent product header"
        );
    }
}

fn test_workspace(photo_id: PhotoId) -> PhotoWorkspaceViewModel {
    let title = PresentationText::new("Alex_Benes.RAF").expect("test title");
    PhotoWorkspaceViewModel::new(
        vec![PhotoCardViewModel::new(
            photo_id,
            title.clone(),
            Some(PresentationText::new("RAW · 6048 × 4038").expect("secondary metadata")),
        )],
        vec![PhotoDetailViewModel::new(
            photo_id,
            title,
            vec![
                PhotoFactViewModel::new(
                    PresentationText::new("Camera").expect("test fact label"),
                    PresentationText::new("Fujifilm X-T5").expect("test fact value"),
                ),
                PhotoFactViewModel::new(
                    PresentationText::new("Exposure").expect("test fact label"),
                    PresentationText::new("1/90").expect("test fact value"),
                ),
                PhotoFactViewModel::new(
                    PresentationText::new("Aperture").expect("test fact label"),
                    PresentationText::new("f/8.0").expect("test fact value"),
                ),
                PhotoFactViewModel::new(
                    PresentationText::new("Focal length").expect("test fact label"),
                    PresentationText::new("10.3 mm").expect("test fact value"),
                ),
                PhotoFactViewModel::new(
                    PresentationText::new("ISO").expect("test fact label"),
                    PresentationText::new("200").expect("test fact value"),
                ),
                PhotoFactViewModel::new(
                    PresentationText::new("Format").expect("test fact label"),
                    PresentationText::new("RAW").expect("test fact value"),
                ),
                PhotoFactViewModel::new(
                    PresentationText::new("Dimensions").expect("test fact label"),
                    PresentationText::new("6048 × 4038").expect("test fact value"),
                ),
                PhotoFactViewModel::new(
                    PresentationText::new("File size").expect("test fact label"),
                    PresentationText::new("23.6 MB").expect("test fact value"),
                ),
            ],
        )],
    )
    .expect("test workspace")
}

fn test_collection(photo_id: PhotoId) -> CollectionFilterState {
    CollectionFilterState::new(
        CollectionControlState::new(CollectionProperty::Filename, 1),
        vec![photo_id],
    )
    .with_lighttable_state(
        [LighttablePhotoState::new(
            photo_id,
            true,
            LighttableRating::Three,
            [LighttableColorLabel::Red, LighttableColorLabel::Blue],
        )],
        LighttableToolbarState::new(1),
    )
}

fn thumbnail_metadata() -> Rgba8PreviewMetadata {
    let dimensions = PreviewDimensions::new(32, 20).expect("thumbnail dimensions");
    let mut pixels = Vec::with_capacity(32 * 20 * 4);
    for y in 0..20_u8 {
        for x in 0..32_u8 {
            pixels.extend_from_slice(&[x.saturating_mul(7), y.saturating_mul(11), 180, 255]);
        }
    }
    Rgba8PreviewMetadata::new(
        dimensions,
        PresentationText::new("thumbnail ready").expect("thumbnail status"),
        pixels,
    )
    .expect("thumbnail metadata")
}

fn direct_child_of(widget: &gtk4::Widget, ancestor: &gtk4::Widget) -> gtk4::Widget {
    let mut current = widget.clone();
    loop {
        let parent = current
            .parent()
            .expect("Color Zones control remains inside its editor");
        if parent == ancestor.clone() {
            return current;
        }
        current = parent;
    }
}

fn settle_gtk_until(done: impl Fn() -> bool, state: impl Fn() -> String) {
    let context = gtk4::glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() && Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(done(), "GTK shell transition timed out: {}", state());
    while context.pending() {
        context.iteration(false);
    }
}

fn settle_next_gtk_frame() {
    let elapsed = Rc::new(Cell::new(false));
    gtk4::glib::timeout_add_local_once(Duration::from_millis(20), {
        let elapsed = Rc::clone(&elapsed);
        move || elapsed.set(true)
    });
    settle_gtk_until(
        || elapsed.get(),
        || "GTK did not deliver the next frame interval".to_owned(),
    );
}

#[allow(clippy::too_many_lines)] // Keep the source-ordered mounted editor contract in one runtime assertion.
fn assert_mounted_colorzones_geometry_and_paint(
    shell: &GtkShell,
    root: &gtk4::Widget,
    widget_id: &str,
    catalog: &SmokeCatalog,
    edit_id: EditId,
    published_revision: &Cell<Revision>,
) {
    let basic = find_widget(root, "group-basic")
        .expect("basic module group")
        .downcast::<gtk4::ToggleButton>()
        .expect("basic module group toggle");
    basic.set_active(true);
    settle_gtk_until(
        || find_widget(root, widget_id).is_none(),
        || "Color Zones remained visible outside its groups".to_owned(),
    );

    let search = find_widget(root, "darkroom-module-search")
        .expect("production module search")
        .downcast::<gtk4::SearchEntry>()
        .expect("module search entry");
    search.set_text("  COLOR ZONES  ");
    settle_gtk_until(
        || find_widget(root, widget_id).is_some_and(|module| module.is_mapped()),
        || "production search did not expose the mounted Color Zones instance".to_owned(),
    );

    let color = find_widget(root, "group-color")
        .expect("color module group")
        .downcast::<gtk4::ToggleButton>()
        .expect("color module group toggle");
    color.set_active(true);
    settle_gtk_until(
        || {
            search.text().is_empty()
                && find_widget(root, widget_id).is_some_and(|module| module.is_mapped())
        },
        || "Color group did not clear search and retain Color Zones".to_owned(),
    );

    let module = find_widget(root, widget_id)
        .expect("group-filtered Color Zones module")
        .downcast::<gtk4::Expander>()
        .expect("Color Zones module expander");
    assert!(
        module.is_sensitive(),
        "mounted canonical Color Zones instance must be supported"
    );
    let title_root = module.label_widget().expect("Color Zones source header");
    assert_eq!(title_root.widget_name(), format!("{widget_id}-title"));
    assert!(title_root.has_css_class("dt_module_header"));
    assert!(find_widget(&title_root, &format!("{widget_id}-enabled")).is_some());
    assert!(find_widget(&title_root, &format!("{widget_id}-reset")).is_some());
    let content = module.child().expect("Color Zones module body");
    for omitted in [
        format!("{widget_id}-header"),
        format!("{widget_id}-enabled"),
        format!("{widget_id}-reset"),
        format!("{widget_id}-status-row"),
        format!("{widget_id}-status"),
        format!("{widget_id}-recover"),
        format!("{widget_id}-partial-warning"),
        format!("{widget_id}-presets"),
    ] {
        assert!(
            find_widget(&content, &omitted).is_none(),
            "Color Zones body must not duplicate or invent {omitted}"
        );
    }
    assert!(
        find_widget(root, &format!("{widget_id}-presets")).is_none(),
        "unimplemented presets must not appear as a placeholder"
    );
    module.set_expanded(true);

    let editor_id = format!("{widget_id}-colorzones-editor");
    let ordered_ids = [
        format!("{widget_id}-channel-tabs"),
        format!("{widget_id}-graph"),
        format!("{widget_id}-bottom-strip"),
        format!("{widget_id}-edit-by-area"),
        format!("{widget_id}-select-by"),
        format!("{widget_id}-mode"),
        format!("{widget_id}-strength"),
        format!("{widget_id}-interpolator"),
    ];
    settle_gtk_until(
        || {
            find_widget(root, &editor_id).is_some_and(|editor| {
                editor.is_mapped()
                    && editor.allocated_width() > 0
                    && editor.allocated_height() > 0
                    && ordered_ids.iter().all(|id| {
                        find_widget(&editor, id).is_some_and(|widget| {
                            widget.is_mapped()
                                && widget.allocated_width() > 0
                                && widget.allocated_height() > 0
                        })
                    })
            })
        },
        || format!("expanded Color Zones editor {editor_id} did not receive mapped allocations"),
    );
    settle_next_gtk_frame();

    let editor = find_widget(root, &editor_id).expect("mounted Color Zones editor");
    let editor_box = editor
        .clone()
        .downcast::<gtk4::Box>()
        .expect("Color Zones editor is a vertical source box");
    assert_eq!(editor_box.orientation(), gtk4::Orientation::Vertical);
    assert_eq!(editor_box.spacing(), 0);
    let widgets = ordered_ids
        .iter()
        .map(|id| find_widget(&editor, id).unwrap_or_else(|| panic!("missing {id}")))
        .collect::<Vec<_>>();
    let bounds = widgets
        .iter()
        .map(|widget| {
            widget
                .compute_bounds(&editor)
                .expect("Color Zones child bounds")
        })
        .collect::<Vec<_>>();
    for (pair, ids) in bounds.windows(2).zip(ordered_ids.windows(2)) {
        assert!(
            pair[0].y() + pair[0].height() <= pair[1].y() + 1.0,
            "Color Zones controls must retain source vertical order: {} at {:?}, {} at {:?}",
            ids[0],
            pair[0],
            ids[1],
            pair[1]
        );
    }

    let notebook = find_widget(&editor, &format!("{widget_id}-channel-tabs"))
        .expect("Color Zones output tabs")
        .downcast::<gtk4::Notebook>()
        .expect("Color Zones output tabs type");
    assert!(!notebook.is_scrollable());
    assert_eq!(notebook.n_pages(), 3);
    let tab_widths = (0..notebook.n_pages())
        .map(|index| {
            let page = notebook
                .nth_page(Some(index))
                .expect("Color Zones output page");
            let notebook_page = notebook.page(&page);
            assert!(notebook_page.is_tab_expand() && notebook_page.is_tab_fill());
            let label = notebook
                .tab_label(&page)
                .expect("Color Zones output label")
                .downcast::<gtk4::Label>()
                .expect("Color Zones output label type");
            assert_eq!(label.ellipsize(), gtk4::pango::EllipsizeMode::End);
            let label_text = label.text();
            assert_eq!(label.tooltip_text().as_deref(), Some(label_text.as_str()));
            label.allocated_width()
        })
        .collect::<Vec<_>>();
    assert!(
        tab_widths
            .iter()
            .all(|width| width.abs_diff(tab_widths[0]) <= 1),
        "the three non-scrollable output tabs must divide the row equally: {tab_widths:?}"
    );

    let graph = &widgets[1];
    assert_eq!(
        graph.allocated_height(),
        200,
        "Color Zones graph must retain its native default height"
    );
    let graph_width = u16::try_from(graph.allocated_width()).expect("graph width fits u16");
    let graph_height = u16::try_from(graph.allocated_height()).expect("graph height fits u16");
    let graph_bounds =
        gtk4::graphene::Rect::new(0.0, 0.0, f32::from(graph_width), f32::from(graph_height));
    assert!(
        render_widget(graph).pixels_differing_from_first(graph_bounds, 8) >= 200,
        "Color Zones graph must paint coarse non-background field and curve evidence"
    );

    let strip = &widgets[2];
    assert!(strip.hexpands() && strip.vexpands());
    let edit_by_area = widgets[3]
        .clone()
        .downcast::<gtk4::CheckButton>()
        .expect("compact Color Zones edit-by-area control");
    assert_eq!(edit_by_area.halign(), gtk4::Align::Start);
    assert!(!edit_by_area.hexpands());
    for id in &ordered_ids[4..] {
        let control = find_widget(&editor, id).expect("full-width Color Zones Bauhaus control");
        let direct = direct_child_of(&control, &editor);
        assert!(direct.has_css_class("dt_bauhaus"), "{id} is Bauhaus");
        assert!(direct.hexpands(), "{id} expands across the module body");
        assert_eq!(direct.halign(), gtk4::Align::Fill);
        assert!(
            direct.allocated_width() >= editor.allocated_width() - 2,
            "{id} must occupy the full Color Zones body width"
        );
    }
    let mix_value = find_widget(&editor, "bauhaus-slider-value")
        .expect("closed Color Zones mix value")
        .downcast::<gtk4::Label>()
        .expect("closed Color Zones mix value type");
    assert_eq!(mix_value.text(), "+0.00%");
    let strip_width = u16::try_from(strip.allocated_width()).expect("strip width fits u16");
    let strip_height = u16::try_from(strip.allocated_height()).expect("strip height fits u16");
    let strip_bounds =
        gtk4::graphene::Rect::new(0.0, 0.0, f32::from(strip_width), f32::from(strip_height));
    assert!(
        render_widget(strip).pixels_differing_from_first(strip_bounds, 8) >= 50,
        "Color Zones bottom strip must paint coarse non-background gradient evidence"
    );

    for omitted in [
        format!("{widget_id}-picker"),
        format!("{widget_id}-color-picker"),
        format!("{widget_id}-display-selection"),
        format!("{widget_id}-show-selection"),
        format!("{widget_id}-mask-display"),
    ] {
        assert!(
            find_widget(&editor, &omitted).is_none(),
            "unported Color Zones control {omitted} must remain omitted"
        );
    }

    let mode = find_widget(&editor, &format!("{widget_id}-mode-selection"))
        .expect("visible Color Zones process mode")
        .downcast::<gtk4::DropDown>()
        .expect("visible Color Zones process mode type");
    assert_eq!(mode.selected(), 0, "canonical process mode starts smooth");

    let graph = graph
        .clone()
        .downcast::<gtk4::DrawingArea>()
        .expect("mounted Color Zones graph type");
    let motion = named_controller(&graph, "dt-colorzones-motion")
        .expect("mounted graph motion controller")
        .downcast::<gtk4::EventControllerMotion>()
        .expect("mounted graph motion controller type");
    let click = named_controller(&graph, "dt-colorzones-click")
        .expect("mounted graph primary controller")
        .downcast::<gtk4::GestureClick>()
        .expect("mounted graph primary controller type");
    let point = |x: f64, y: f64| {
        (
            5.0 + (f64::from(graph.allocated_width()) - 10.0) * x,
            5.0 + (f64::from(graph.allocated_height()) - 10.0) * (1.0 - y),
        )
    };
    let (node_x, node_y) = point(0.25, 0.5);
    let (drag_x, drag_y) = point(0.2, 0.6);
    motion.emit_by_name::<()>("motion", &[&node_x, &node_y]);
    click.emit_by_name::<()>("pressed", &[&1_i32, &node_x, &node_y]);
    motion.emit_by_name::<()>("motion", &[&drag_x, &drag_y]);
    settle_gtk_until(
        || catalog.load_edit(edit_id).revision() == Revision::from_u64(7),
        || "live Color Zones graph motion unexpectedly committed".to_owned(),
    );
    assert_eq!(
        published_revision.get(),
        Revision::ZERO,
        "live graph motion must not publish an image revision"
    );
    click.emit_by_name::<()>("released", &[&1_i32, &drag_x, &drag_y]);
    let expected_revision = Revision::from_u64(8);
    settle_gtk_until(
        || {
            catalog.load_edit(edit_id).revision() == expected_revision
                && published_revision.get() == expected_revision
                && shell.darkroom_panel_target().is_some_and(|target| {
                    target.generation() == ViewportGeneration::new(2)
                        && target.edit_revision() == expected_revision
                })
        },
        || {
            format!(
                "Color Zones edit publication: catalog={}, published={}, target={:?}",
                catalog.load_edit(edit_id).revision(),
                published_revision.get(),
                shell.darkroom_panel_target()
            )
        },
    );
    assert_eq!(
        catalog.load_edit(edit_id).revision(),
        expected_revision,
        "one settled Color Zones graph edit must atomically advance catalog history once"
    );
    assert_eq!(
        published_revision.get(),
        expected_revision,
        "the same production handler must publish the selected edited preview"
    );

    find_widget(root, "group-active")
        .expect("active module group")
        .downcast::<gtk4::ToggleButton>()
        .expect("active module group toggle")
        .set_active(true);
    settle_gtk_until(
        || find_widget(root, "exposure").is_some(),
        || "active module group did not restore shared runtime assertions".to_owned(),
    );
}

fn assert_darkroom_titles_are_allocated(shell: &GtkShell) {
    let root: gtk4::Widget = shell.window().clone().upcast();
    let rail = find_widget(&root, "darkroom-left-panel").expect("darkroom left rail");
    let visible_split = find_widget(&root, "desktop-left-split").expect("desktop left split");
    let rendered = render_widget(&visible_split);
    for (id, expected) in [
        ("darkroom-snapshots", "snapshots"),
        ("darkroom-history", "history"),
        ("darkroom-image-information", "image information"),
    ] {
        let section = find_widget(&rail, id).expect("darkroom section");
        let title_row = darkroom_section_title_row(&section);
        let title = find_widget(&title_row, &format!("{id}-label"))
            .expect("darkroom title")
            .downcast::<gtk4::Label>()
            .expect("darkroom title label");
        assert!(
            title_row.allocated_width() > 0
                && title_row.allocated_height() > 0
                && title.allocated_width() > 0
                && title.allocated_height() > 0
                && title.is_visible(),
            "nonzero title allocation required for {id}: row {}x{}, title {}x{}",
            title_row.allocated_width(),
            title_row.allocated_height(),
            title.allocated_width(),
            title.allocated_height()
        );
        assert_eq!(title.text().as_str(), expected, "title text for {id}");
        let action_id = format!("{id}-actions");
        let affordance = find_widget(&title_row, &action_id)
            .expect("accordion affordance")
            .downcast::<gtk4::Button>()
            .expect("accordion affordance button");
        assert!(
            affordance.is_visible()
                && !affordance.is_sensitive()
                && affordance.allocated_width() > 0
                && affordance.allocated_height() > 0,
            "{action_id} must keep visible neutral geometry"
        );
        assert!(
            affordance
                .child()
                .is_some_and(|child| child.is::<gtk4::Image>()),
            "{action_id} must render a symbolic icon"
        );
        let icon = affordance.child().expect("accordion symbolic icon");
        let icon_bounds = icon
            .compute_bounds(&visible_split)
            .expect("accordion symbolic icon bounds");
        assert!(
            rendered.pixels_with_channel_at_least(icon_bounds, 80) >= 2,
            "{action_id} must paint its neutral symbolic icon"
        );
        let bounds = title
            .compute_bounds(&visible_split)
            .expect("title bounds within visible desktop split");
        let bright_pixels = rendered.bright_pixels(bounds);
        assert!(
            bright_pixels >= 8,
            "title must paint readable pixels for {id}: {bright_pixels} bright pixels in {bounds:?}"
        );
    }
}

fn darkroom_section_title_row(section: &gtk4::Widget) -> gtk4::Widget {
    section
        .clone()
        .downcast::<gtk4::Expander>()
        .expect("collapsible darkroom section expander")
        .label_widget()
        .expect("darkroom title row")
}

fn assert_darkroom_chrome_matches_runtime_geometry(shell: &GtkShell) {
    let root: gtk4::Widget = shell.window().clone().upcast();
    assert_toolbar_and_status_geometry(&root);
    assert_navigation_rendering(&root);
    assert_right_rail_geometry(&root);
    assert_filmstrip_rendering(&root);
    assert_right_rail_resize(&root);
    assert_frame_edge_controls(&root);
}

fn assert_navigation_rendering(root: &gtk4::Widget) {
    let module = find_widget(root, "darkroom-navigation").expect("navigation module");
    let navigation = find_widget(root, "darkroom-navigation-preview").expect("navigation preview");
    let crop = find_widget(root, "darkroom-navigation-crop").expect("navigation crop indicator");
    let resize =
        find_widget(&module, "darkroom-module-resize-handle").expect("navigation resize overlay");
    let zoom = find_widget(&module, "darkroom-navigation-zoom").expect("navigation zoom overlay");
    let visible_split = find_widget(root, "desktop-left-split").expect("visible desktop split");
    let projection = find_widget(root, "darkroom-viewport-projection")
        .expect("inactive viewport projection watermark");
    assert!(!module.is::<gtk4::Expander>());
    assert!(
        find_widget(&module, "darkroom-navigation-title").is_none(),
        "non-expandable Darktable navigation must not add a title row"
    );
    assert!(navigation.is_visible() && crop.is_visible());
    assert!(
        navigation.allocated_width() >= 200 && (180..=210).contains(&navigation.allocated_height()),
        "navigation preview must keep configured source geometry: {}x{}",
        navigation.allocated_width(),
        navigation.allocated_height()
    );
    for overlay in [&resize, &zoom] {
        let bounds = overlay
            .compute_bounds(&navigation)
            .expect("navigation overlay bounds");
        assert!(
            bounds.y() >= 0.0
                && f64::from(bounds.y() + bounds.height())
                    <= f64::from(navigation.allocated_height()),
            "navigation chrome must overlay the preview instead of adding a row: {bounds:?}"
        );
    }
    assert!(
        !projection.is_visible(),
        "default fit/edited/normal state must not paint a viewport watermark"
    );
    settle_next_gtk_frame();
    let rendered = render_widget(&visible_split);
    let crop_bounds = crop
        .compute_bounds(&visible_split)
        .expect("navigation crop bounds");
    assert!(
        rendered.bright_pixels(crop_bounds) >= 40,
        "navigation crop frame must paint over the thumbnail"
    );
}

fn assert_toolbar_and_status_geometry(root: &gtk4::Widget) {
    assert!(
        find_widget(root, "header-profile-diagnostic").is_none(),
        "display-profile prose must not leak into the global header"
    );
    let header_clip = find_widget(root, "header-clip").expect("bounded product header");
    assert_eq!(
        header_clip.allocated_height(),
        i32::from(DARKTABLE_DESKTOP_SPEC.layout.header_height_px),
        "stacked brand labels must not grow the shared header past the Darktable contract"
    );

    let viewport = find_widget(root, "darkroom-viewport").expect("darkroom viewport");
    let top = find_widget(root, "darkroom-toolbar-top").expect("legacy top toolbar");
    let bottom = find_widget(root, "darkroom-toolbar-bottom").expect("bottom toolbar");
    let status = find_widget(root, "darkroom-status-bar").expect("darkroom status bar");
    let job = find_widget(root, "darkroom-job-status").expect("darkroom job status");
    let profile =
        find_widget(root, "darkroom-profile-diagnostic").expect("retained profile diagnostic");
    assert!(
        !top.is_visible(),
        "the canvas must not reserve a top toolbar row"
    );
    assert!(bottom.is_visible() && bottom.is_ancestor(&status));
    assert!(
        !job.is_visible(),
        "idle export prose must stay out of the status bar"
    );
    assert!(
        !profile.is_visible(),
        "profile diagnostics belong in the header icon tooltip"
    );

    let viewport_bounds = viewport.compute_bounds(root).expect("viewport bounds");
    let status_bounds = status.compute_bounds(root).expect("status bounds");
    assert!(
        status_bounds.y() >= viewport_bounds.y() + viewport_bounds.height() - 1.0,
        "viewport controls and status must sit below the canvas"
    );

    let status_text = find_widget(root, "darkroom-status")
        .expect("centered image status")
        .downcast::<gtk4::Label>()
        .expect("image status label");
    assert_eq!(status_text.text(), "1/90 · f/8.0 · 10.3 mm · ISO 200");
    assert!(!status_text.text().contains("MB"));
    for (id, expected) in [
        ("darkroom-module-order", "module order"),
        ("darkroom-pipeline-state", "revision 8 · RAW"),
    ] {
        let label = find_widget(root, id)
            .expect("pipeline affordance")
            .downcast::<gtk4::Label>()
            .expect("pipeline affordance label");
        assert!(label.is_visible());
        assert_eq!(label.text(), expected);
    }
    let guide = find_widget(root, "darkroom-composition-guide").expect("composition guide");
    let guide_toggle = find_widget(root, "darkroom-guides-toggle")
        .expect("composition guide toggle")
        .downcast::<gtk4::ToggleButton>()
        .expect("composition guide toggle button");
    assert!(
        guide.is_visible()
            && guide.is_mapped()
            && guide.allocated_width() == viewport.allocated_width()
            && guide.allocated_height() == viewport.allocated_height(),
        "composition guide must cover the image viewport"
    );
    assert!(!guide_toggle.is_active());
    guide_toggle.set_active(true);
    assert!(guide.is_visible());
    guide_toggle.set_active(false);
    assert!(
        guide.is_visible(),
        "the mapped drawing surface remains present while inactive guides draw nothing"
    );
}

fn assert_right_rail_geometry(root: &gtk4::Widget) {
    let histogram = find_widget(root, "darkroom-histogram").expect("histogram");
    let groups = find_widget(root, "darkroom-module-groups-scroll").expect("module groups");
    let search = find_widget(root, "darkroom-module-search").expect("module search");
    let modules = find_widget(root, "darkroom-right-module-scroll").expect("module scroll");
    let histogram_y = histogram
        .compute_bounds(root)
        .expect("histogram bounds")
        .y();
    let groups_y = groups.compute_bounds(root).expect("group bounds").y();
    let search_y = search.compute_bounds(root).expect("search bounds").y();
    let modules_y = modules.compute_bounds(root).expect("module bounds").y();
    assert!(histogram_y < groups_y && groups_y < search_y && search_y < modules_y);
    for id in [
        "darkroom-left-panel-toggle",
        "darkroom-right-panel-toggle",
        "group-active",
        "group-favorites",
    ] {
        let button = find_widget(root, id).expect("icon button");
        let icon = button.first_child().expect("symbolic icon");
        assert!(
            icon.is_visible() && icon.is_mapped(),
            "{id} icon must be mapped"
        );
        assert!(
            icon.allocated_width() > 0 && icon.allocated_height() > 0,
            "{id} icon must have a positive allocation"
        );
        let image = icon
            .downcast::<gtk4::Image>()
            .expect("icon button must use a GTK symbolic image");
        let icon_name = image.icon_name().expect("symbolic image must name an icon");
        let theme = gtk4::IconTheme::for_display(
            &gtk4::gdk::Display::default().expect("test display remains active"),
        );
        assert!(
            theme.has_icon(&icon_name),
            "{id} must use an installed symbolic icon, got {icon_name}"
        );
    }
    let soft_proof = find_widget(root, "darkroom-soft-proof").expect("soft-proof control");
    let soft_proof_glyph = soft_proof.first_child().expect("soft-proof symbolic glyph");
    assert!(soft_proof_glyph.is_visible() && soft_proof_glyph.is_mapped());
    assert!(soft_proof_glyph.allocated_width() > 0 && soft_proof_glyph.allocated_height() > 0);
    for id in [
        "darkroom-histogram-empty",
        "darkroom-histogram-loading",
        "darkroom-histogram-failure",
        "darkroom-histogram-stale",
    ] {
        let label = find_widget(&histogram, id)
            .expect("histogram state")
            .downcast::<gtk4::Label>()
            .expect("histogram state label");
        assert!(
            label.text().is_empty(),
            "{id} must not expose diagnostic prose"
        );
    }
}

fn assert_filmstrip_rendering(root: &gtk4::Widget) {
    let filmstrip_item = find_widget_with_prefix(root, "filmstrip-photo-").expect("filmstrip item");
    assert!(filmstrip_item.has_css_class("dt_selected"));
    let filmstrip_metadata =
        find_widget_with_prefix(root, "filmstrip-metadata-").expect("filmstrip metadata");
    let filmstrip_rating = find_widget_with_prefix(root, "filmstrip-rating-")
        .expect("filmstrip rating")
        .downcast::<gtk4::Label>()
        .expect("filmstrip rating label");
    let filmstrip_format = find_widget_with_prefix(root, "filmstrip-format-")
        .expect("filmstrip format")
        .downcast::<gtk4::Label>()
        .expect("filmstrip format label");
    assert_eq!(filmstrip_rating.text(), "★★★☆☆");
    assert_eq!(filmstrip_format.text(), "RAW");
    assert!(find_widget_with_prefix(root, "filmstrip-red-tag-").is_some());
    assert!(find_widget_with_prefix(root, "filmstrip-blue-tag-").is_some());
    let selection_pointer = find_widget_with_prefix(root, "filmstrip-selection-pointer-")
        .expect("selected filmstrip pointer");
    assert!(selection_pointer.is_visible());
    assert!(
        selection_pointer.allocated_width() >= 18 && selection_pointer.allocated_height() >= 8,
        "selected filmstrip pointer must keep chevron geometry: {}x{}",
        selection_pointer.allocated_width(),
        selection_pointer.allocated_height()
    );
    let visible_split = find_widget(root, "desktop-left-split").expect("visible center split");
    settle_next_gtk_frame();
    let rendered = render_widget(&visible_split);
    let metadata_bounds = filmstrip_metadata
        .compute_bounds(&visible_split)
        .expect("filmstrip metadata bounds");
    assert!(
        rendered.bright_pixels(metadata_bounds) >= 8,
        "filmstrip metadata must paint visible pixels"
    );
    let item_bounds = filmstrip_item
        .compute_bounds(&visible_split)
        .expect("selected filmstrip bounds");
    let selection_marker =
        gtk4::graphene::Rect::new(item_bounds.x(), item_bounds.y(), 3.0, item_bounds.height());
    assert!(
        rendered.bright_pixels(selection_marker) >= 20,
        "selected filmstrip item must paint one light frame"
    );
    let pointer_bounds = selection_pointer
        .compute_bounds(&visible_split)
        .expect("selected filmstrip pointer bounds");
    assert!(
        rendered.bright_pixels(pointer_bounds) >= 20,
        "selected filmstrip item must paint a top-center pointer chevron"
    );
    let boundary = find_widget(root, "darkroom-filmstrip-boundary").expect("filmstrip boundary");
    let boundary_bounds = boundary
        .compute_bounds(&visible_split)
        .expect("filmstrip boundary bounds");
    assert!(
        rendered.pixels_with_channel_at_most(boundary_bounds, 110) >= 100,
        "filmstrip boundary must render as a dark, compact separator"
    );
}

fn assert_right_rail_resize(root: &gtk4::Widget) {
    let viewport = find_widget(root, "darkroom-viewport").expect("darkroom viewport");
    let histogram = find_widget(root, "darkroom-histogram").expect("histogram");
    let histogram_chart =
        find_widget(root, "darkroom-histogram-chart").expect("rendered histogram chart");
    let right_panel = find_widget(root, "darkroom-right-panel").expect("right panel");
    let right_split = find_widget(root, "desktop-right-split")
        .expect("right split")
        .downcast::<gtk4::Paned>()
        .expect("right split is a paned");
    let split_width = right_split.allocated_width();
    right_split.set_position(split_width.saturating_sub(300));
    settle_gtk_until(
        || histogram.allocated_width() >= 280,
        || {
            format!(
                "expanded histogram={}x{}",
                histogram.allocated_width(),
                histogram.allocated_height()
            )
        },
    );
    assert!((120..=180).contains(&histogram.allocated_height()));
    assert_histogram_chart_paints(root, &histogram_chart);
    for id in [
        "exposure",
        "rgb-denoise",
        "raw-denoise",
        "mask-manager",
        "multiscale-retouch",
    ] {
        let module = find_widget(root, id)
            .expect("implemented processing module")
            .downcast::<gtk4::Expander>()
            .expect("implemented processing module is an expander");
        assert!(
            module.is_visible() && !module.is_expanded(),
            "{id} must use the compact collapsed module-stack presentation"
        );
        let action_ids = if id == "exposure" {
            vec![
                "exposure-enabled".to_owned(),
                "exposure-presets".to_owned(),
                "exposure-reset".to_owned(),
                "exposure-multi".to_owned(),
            ]
        } else {
            vec![format!("{id}-actions")]
        };
        for action_id in action_ids {
            let affordance =
                find_widget(module.upcast_ref(), &action_id).expect("module title action");
            assert!(affordance.is_visible() && affordance.allocated_width() > 0);
        }
    }
    right_split.set_position(split_width.saturating_sub(180));
    settle_gtk_until(
        || histogram.allocated_width() <= 190,
        || {
            format!(
                "narrow panel={}, histogram={}x{}, split={}@{}",
                right_panel.allocated_width(),
                histogram.allocated_width(),
                histogram.allocated_height(),
                right_split.allocated_width(),
                right_split.position()
            )
        },
    );
    assert!((120..=180).contains(&histogram.allocated_height()));
    assert_histogram_chart_paints(root, &histogram_chart);
    assert!(
        viewport.allocated_width() >= 600,
        "resize must preserve the canvas"
    );
}

#[allow(clippy::too_many_lines)] // Keep the native frame geometry and interaction contract auditable together.
#[allow(clippy::cast_precision_loss, clippy::float_cmp)] // GTK allocates geometry as f32; exact edge assertions are intentional.
fn assert_frame_edge_controls(root: &gtk4::Widget) {
    let border = i32::from(DARKTABLE_DESKTOP_SPEC.layout.outer_border_px);
    let frame = find_widget(root, "workspace-frame").expect("workspace frame");
    let frame_width = frame.allocated_width();
    let frame_height = frame.allocated_height();
    assert!(frame_width > 0 && frame_height > 0);

    for (toggle_id, horizontal) in [
        ("workspace-left-edge-toggle", false),
        ("workspace-right-edge-toggle", false),
        ("workspace-top-edge-toggle", true),
        ("workspace-bottom-edge-toggle", true),
    ] {
        let toggle = find_widget(root, toggle_id)
            .expect("frame panel affordance")
            .downcast::<gtk4::Button>()
            .expect("frame panel affordance is a button");
        assert!(
            toggle.is_visible()
                && toggle.is_mapped()
                && toggle.allocated_width() > 0
                && toggle.allocated_height() > 0,
            "{toggle_id} must paint on the workspace frame"
        );
        if horizontal {
            assert_eq!(
                toggle.allocated_height(),
                border,
                "{toggle_id} must consume exactly the horizontal frame edge"
            );
            assert_eq!(toggle.allocated_width(), 28);
        } else {
            assert_eq!(
                toggle.allocated_width(),
                border,
                "{toggle_id} must consume exactly the vertical frame edge"
            );
            assert_eq!(toggle.allocated_height(), 28);
        }
        let child = toggle.first_child().expect("frame toggle glyph");
        assert!(
            child.is_visible()
                && child.is_mapped()
                && child.allocated_width() > 0
                && child.allocated_height() > 0,
            "{toggle_id} must paint its directional glyph"
        );
    }

    for (toggle_id, expected_width, expected_height) in [
        ("workspace-left-edge-toggle", 4, 10),
        ("workspace-right-edge-toggle", 4, 10),
        ("workspace-top-edge-toggle", 10, 4),
        ("workspace-bottom-edge-toggle", 10, 4),
    ] {
        let glyph = find_widget(root, toggle_id)
            .expect("frame toggle")
            .first_child()
            .expect("frame toggle glyph");
        assert_eq!(glyph.allocated_width(), expected_width);
        assert_eq!(glyph.allocated_height(), expected_height);
    }

    let left_bounds = find_widget(root, "workspace-left-edge-toggle")
        .expect("left frame toggle")
        .compute_bounds(&frame)
        .expect("left frame bounds");
    let right_bounds = find_widget(root, "workspace-right-edge-toggle")
        .expect("right frame toggle")
        .compute_bounds(&frame)
        .expect("right frame bounds");
    let top_bounds = find_widget(root, "workspace-top-edge-toggle")
        .expect("top frame toggle")
        .compute_bounds(&frame)
        .expect("top frame bounds");
    let bottom_bounds = find_widget(root, "workspace-bottom-edge-toggle")
        .expect("bottom frame toggle")
        .compute_bounds(&frame)
        .expect("bottom frame bounds");
    assert_eq!(left_bounds.x(), 0.0);
    assert_eq!(right_bounds.x() + right_bounds.width(), frame_width as f32);
    assert_eq!(top_bounds.y(), 0.0);
    assert_eq!(
        bottom_bounds.y() + bottom_bounds.height(),
        frame_height as f32
    );
    assert!(
        ((top_bounds.x() * 2.0 + top_bounds.width()) - frame_width as f32).abs() <= 1.0,
        "top control must stay horizontally centered"
    );
    assert!(
        ((bottom_bounds.x() * 2.0 + bottom_bounds.width()) - frame_width as f32).abs() <= 1.0,
        "bottom control must stay horizontally centered"
    );

    for (toggle_id, panel_id) in [
        ("workspace-left-edge-toggle", "darkroom-left-panel"),
        ("workspace-right-edge-toggle", "darkroom-right-panel"),
    ] {
        let toggle = find_widget(root, toggle_id)
            .expect("outer panel affordance")
            .downcast::<gtk4::Button>()
            .expect("outer panel affordance is a button");
        let panel = find_widget(root, panel_id).expect("darkroom rail");
        assert!(
            toggle.is_visible()
                && toggle.is_mapped()
                && toggle.allocated_width() > 0
                && toggle.allocated_height() > 0,
            "{toggle_id} must paint on the outer workspace edge"
        );
        toggle.emit_clicked();
        settle_gtk_until(
            || !panel.is_visible(),
            || format!("{panel_id} did not collapse"),
        );
        toggle.emit_clicked();
        settle_gtk_until(
            || panel.is_visible() && panel.is_mapped(),
            || format!("{panel_id} did not expand"),
        );
    }

    let top = find_widget(root, "workspace-top-edge-toggle")
        .expect("top panel affordance")
        .downcast::<gtk4::Button>()
        .expect("top panel affordance is a button");
    let header = find_widget(root, "header-clip").expect("bounded header panel");
    top.emit_clicked();
    settle_gtk_until(|| !header.is_visible(), || "header did not collapse".into());
    top.emit_clicked();
    settle_gtk_until(
        || header.is_visible() && header.is_mapped(),
        || "header did not expand".into(),
    );

    let bottom = find_widget(root, "workspace-bottom-edge-toggle")
        .expect("bottom panel affordance")
        .downcast::<gtk4::Button>()
        .expect("bottom panel affordance is a button");
    let filmstrip = find_widget(root, "filmstrip").expect("filmstrip panel");
    bottom.emit_clicked();
    settle_gtk_until(
        || !filmstrip.is_visible(),
        || "filmstrip did not collapse".into(),
    );
    bottom.emit_clicked();
    settle_gtk_until(
        || filmstrip.is_visible() && filmstrip.is_mapped(),
        || "filmstrip did not expand".into(),
    );
}

fn assert_histogram_chart_paints(root: &gtk4::Widget, chart: &gtk4::Widget) {
    assert!(
        chart.is_visible() && chart.is_mapped(),
        "ready histogram chart must stay mapped after rail resize"
    );
    let _ = root;
    settle_next_gtk_frame();
    let rendered = render_widget(chart);
    let chart_width = u16::try_from(chart.allocated_width()).expect("histogram width fits u16");
    let chart_height = u16::try_from(chart.allocated_height()).expect("histogram height fits u16");
    let bounds =
        gtk4::graphene::Rect::new(0.0, 0.0, f32::from(chart_width), f32::from(chart_height));
    assert!(
        rendered.pixels_with_channel_at_least(bounds, 60) >= 80,
        "histogram graph must rerender visible channel traces inside {bounds:?}"
    );
}
