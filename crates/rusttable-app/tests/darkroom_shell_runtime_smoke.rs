#![forbid(unsafe_code)]

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk4::prelude::*;
use rusttable_core::PhotoId;
use rusttable_ui::gtk_shell::{GtkShell, WorkspaceRole};
use rusttable_ui::{
    CollectionControlState, CollectionFilterState, CollectionProperty, LighttableColorLabel,
    LighttablePhotoState, LighttableRating, LighttableToolbarState, PhotoCardViewModel,
    PhotoDetailViewModel, PhotoFactViewModel, PhotoWorkspaceViewModel, PresentationText,
    PreviewDimensions, Rgba8PreviewMetadata, ViewportGeneration,
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
            shell
                .set_photo_thumbnail(photo_id, &thumbnail_metadata())
                .expect("bounded navigation and filmstrip thumbnail");
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
}

fn assert_navigation_rendering(root: &gtk4::Widget) {
    let navigation = find_widget(root, "darkroom-navigation-preview").expect("navigation preview");
    let crop = find_widget(root, "darkroom-navigation-crop").expect("navigation crop indicator");
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
    let rendered = render_widget(root);
    let crop_bounds = crop.compute_bounds(root).expect("navigation crop bounds");
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
    let panel_bounds = right_panel
        .compute_bounds(root)
        .expect("right panel bounds");
    for id in [
        "rgb-denoise-model",
        "rgb-denoise-provider",
        "raw-denoise-model",
    ] {
        let field = find_widget(root, id).expect("narrow right-rail field");
        let bounds = field.compute_bounds(root).expect("right-rail field bounds");
        assert!(
            bounds.width() >= 40.0,
            "{id} must retain a usable value width"
        );
        assert!(
            bounds.x() + bounds.width() <= panel_bounds.x() + panel_bounds.width() + 1.0,
            "{id} must stay inside the narrow right rail: {bounds:?} vs {panel_bounds:?}"
        );
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
    assert!(
        viewport.allocated_width() >= 600,
        "resize must preserve the canvas"
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
