#![forbid(unsafe_code)]

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk4::prelude::*;
use rusttable_core::PhotoId;
use rusttable_ui::gtk_shell::{DARKTABLE_DESKTOP_SPEC, GtkShell, WorkspaceRole};
use rusttable_ui::{
    CollectionControlState, CollectionFilterState, CollectionProperty, HistogramData,
    LighttableColorLabel, LighttablePhotoState, LighttableRating, LighttableToolbarState,
    PhotoCardViewModel, PhotoDetailViewModel, PhotoFactViewModel, PhotoWorkspaceViewModel,
    PresentationText, PreviewDimensions, Rgba8PreviewMetadata, ViewportGeneration,
};

fn main() {
    gtk4::init().expect("GTK must initialize for the app-shell runtime smoke");
    app_shell_transition_paints_darkroom_titles();
    println!("Darkroom app-shell runtime smoke passed");
}

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
    shell.window().set_default_size(1_228, 768);
    let root: gtk4::Widget = shell.window().clone().upcast();
    let rail = find_widget(&root, "darkroom-left-panel").expect("darkroom left rail");
    let left_stack = find_widget(&root, "left-panel-stack").expect("left panel stack");
    let left_split = find_widget(&root, "desktop-left-split")
        .expect("desktop left split")
        .downcast::<gtk4::Paned>()
        .expect("desktop left split is a paned");
    let photo_id = PhotoId::new(949).expect("test photo id");
    let workspace = test_workspace(photo_id);

    shell.present();
    let transitioned = Rc::new(Cell::new(false));
    gtk4::glib::idle_add_local_once({
        let shell = shell.clone();
        let transitioned = Rc::clone(&transitioned);
        move || {
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
            gtk4::glib::idle_add_local_once(move || {
                shell.show_workspace(WorkspaceRole::Darkroom);
                left_split.set_position(180);
                transitioned.set(true);
            });
        }
    });
    settle_gtk_until(
        || {
            transitioned.get()
                && rail.is_mapped()
                && rail.allocated_width() > 1
                && left_stack.allocated_width() > 0
                && left_stack.allocated_height() > 0
        },
        || {
            format!(
                "stack={}x{}, rail={}x{}",
                left_stack.allocated_width(),
                left_stack.allocated_height(),
                rail.allocated_width(),
                rail.allocated_height()
            )
        },
    );
    assert!(
        left_stack.allocated_width() <= 182,
        "active darkroom rail must honor the 180px divider, got {}px",
        left_stack.allocated_width()
    );
    assert_darkroom_titles_are_allocated(&shell);
    assert_darkroom_chrome_matches_runtime_geometry(&shell);
    assert_lighttable_preview_geometry(&shell, photo_id);
}

fn assert_lighttable_preview_geometry(shell: &GtkShell, photo_id: PhotoId) {
    let root: gtk4::Widget = shell.window().clone().upcast();
    find_widget(&root, "view-lighttable")
        .expect("header Lighttable selector")
        .downcast::<gtk4::Button>()
        .expect("header Lighttable selector is a button")
        .emit_clicked();
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
    find_widget(&root, "view-darkroom")
        .expect("header Darkroom selector")
        .downcast::<gtk4::Button>()
        .expect("header Darkroom selector is a button")
        .emit_clicked();
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

fn assert_darkroom_titles_are_allocated(shell: &GtkShell) {
    let root: gtk4::Widget = shell.window().clone().upcast();
    let rail = find_widget(&root, "darkroom-left-panel").expect("darkroom left rail");
    let visible_split = find_widget(&root, "desktop-left-split").expect("desktop left split");
    let rendered = render_widget(&visible_split);
    for (id, expected) in [
        ("darkroom-navigation", "navigation"),
        ("darkroom-snapshots", "snapshots"),
        ("darkroom-history", "history"),
        ("darkroom-image-information", "image information"),
    ] {
        let expander = find_widget(&rail, id)
            .expect("darkroom section")
            .downcast::<gtk4::Expander>()
            .expect("darkroom section expander");
        let title_row = expander.label_widget().expect("darkroom title row");
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
        for suffix in ["info", "actions"] {
            let affordance = find_widget(&title_row, &format!("{id}-{suffix}"))
                .expect("accordion affordance")
                .downcast::<gtk4::Button>()
                .expect("accordion affordance button");
            assert!(
                affordance.is_visible()
                    && !affordance.is_sensitive()
                    && affordance.allocated_width() > 0
                    && affordance.allocated_height() > 0,
                "{id}-{suffix} must keep visible neutral geometry"
            );
            assert!(
                affordance
                    .child()
                    .is_some_and(|child| child.is::<gtk4::Image>()),
                "{id}-{suffix} must render a symbolic icon"
            );
            let icon = affordance.child().expect("accordion symbolic icon");
            let icon_bounds = icon
                .compute_bounds(&visible_split)
                .expect("accordion symbolic icon bounds");
            assert!(
                rendered.pixels_with_channel_at_least(icon_bounds, 80) >= 2,
                "{id}-{suffix} must paint its neutral symbolic icon"
            );
        }
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
    let navigation = find_widget(root, "darkroom-navigation-preview").expect("navigation preview");
    let crop = find_widget(root, "darkroom-navigation-crop").expect("navigation crop indicator");
    let visible_split = find_widget(root, "desktop-left-split").expect("visible desktop split");
    let projection = find_widget(root, "darkroom-viewport-projection")
        .expect("inactive viewport projection watermark");
    assert!(navigation.is_visible() && crop.is_visible());
    assert!(
        navigation.allocated_width() >= 120 && navigation.allocated_height() >= 80,
        "navigation preview must keep useful geometry: {}x{}",
        navigation.allocated_width(),
        navigation.allocated_height()
    );
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
        ("darkroom-pipeline-state", "revision 0 · RAW"),
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
    assert!(guide_toggle.is_active());
    guide_toggle.set_active(false);
    assert!(!guide.is_visible());
    guide_toggle.set_active(true);
    assert!(guide.is_visible());
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
        for suffix in ["info", "actions"] {
            let affordance = find_widget(module.upcast_ref(), &format!("{id}-{suffix}"))
                .expect("module title action");
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

struct RenderedWidget {
    bytes: Vec<u8>,
    width: usize,
}

impl RenderedWidget {
    fn bright_pixels(&self, bounds: gtk4::graphene::Rect) -> usize {
        self.pixels_with_channel_at_least(bounds, 128)
    }

    fn pixels_with_channel_at_least(&self, bounds: gtk4::graphene::Rect, threshold: u8) -> usize {
        let left = bounds.x();
        let top = bounds.y();
        let right = left + bounds.width();
        let bottom = top + bounds.height();
        let (pixels, remainder) = self.bytes.as_chunks::<4>();
        assert!(
            remainder.is_empty(),
            "render texture must contain RGBA pixels"
        );
        pixels
            .iter()
            .enumerate()
            .filter(|(index, pixel)| {
                let x = u16::try_from(index % self.width).expect("render x fits u16");
                let y = u16::try_from(index / self.width).expect("render y fits u16");
                let x = f32::from(x);
                let y = f32::from(y);
                x >= left
                    && x < right
                    && y >= top
                    && y < bottom
                    && pixel[..3]
                        .iter()
                        .copied()
                        .max()
                        .is_some_and(|channel| channel >= threshold)
            })
            .count()
    }

    fn pixels_with_channel_at_most(&self, bounds: gtk4::graphene::Rect, threshold: u8) -> usize {
        let left = bounds.x();
        let top = bounds.y();
        let right = left + bounds.width();
        let bottom = top + bounds.height();
        let (pixels, remainder) = self.bytes.as_chunks::<4>();
        assert!(
            remainder.is_empty(),
            "render texture must contain RGBA pixels"
        );
        pixels
            .iter()
            .enumerate()
            .filter(|(index, pixel)| {
                let x = u16::try_from(index % self.width).expect("render x fits u16");
                let y = u16::try_from(index / self.width).expect("render y fits u16");
                let x = f32::from(x);
                let y = f32::from(y);
                x >= left
                    && x < right
                    && y >= top
                    && y < bottom
                    && pixel[..3].iter().all(|channel| *channel <= threshold)
            })
            .count()
    }
}

fn render_widget(widget: &gtk4::Widget) -> RenderedWidget {
    let allocated_width = widget.allocated_width();
    let allocated_height = widget.allocated_height();
    let width = usize::try_from(allocated_width).expect("positive widget width");
    let height = usize::try_from(allocated_height).expect("positive widget height");
    assert!(width > 0 && height > 0, "widget must be allocated");
    let paintable = gtk4::WidgetPaintable::new(Some(widget));
    let snapshot = gtk4::Snapshot::new();
    paintable.snapshot(
        &snapshot,
        f64::from(allocated_width),
        f64::from(allocated_height),
    );
    let node = snapshot.to_node().expect("render node for mapped rail");
    let renderer = gtk4::gsk::CairoRenderer::new();
    renderer
        .realize(None::<&gtk4::gdk::Surface>)
        .expect("Cairo renderer");
    let width_f32 = f32::from(u16::try_from(allocated_width).expect("render width fits u16"));
    let height_f32 = f32::from(u16::try_from(allocated_height).expect("render height fits u16"));
    let viewport = gtk4::graphene::Rect::new(0.0, 0.0, width_f32, height_f32);
    let texture = renderer.render_texture(&node, Some(&viewport));
    let mut bytes = vec![0; width * height * 4];
    texture.download(&mut bytes, width * 4);
    RenderedWidget { bytes, width }
}

fn find_widget(root: &gtk4::Widget, name: &str) -> Option<gtk4::Widget> {
    if root.widget_name() == name {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(current) = child {
        if let Some(found) = find_widget(&current, name) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

fn find_widget_with_prefix(root: &gtk4::Widget, prefix: &str) -> Option<gtk4::Widget> {
    if root.widget_name().starts_with(prefix) {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(current) = child {
        if let Some(found) = find_widget_with_prefix(&current, prefix) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}
