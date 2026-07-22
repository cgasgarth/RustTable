#![forbid(unsafe_code)]

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk4::prelude::*;
use rusttable_core::PhotoId;
use rusttable_ui::gtk_shell::{GtkShell, WorkspaceRole};
use rusttable_ui::{
    PhotoCardViewModel, PhotoDetailViewModel, PhotoWorkspaceViewModel, PresentationText,
    ViewportGeneration,
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
    let title = PresentationText::new("Alex_Benes.RAF").expect("test title");
    let workspace = PhotoWorkspaceViewModel::new(
        vec![PhotoCardViewModel::new(photo_id, title.clone(), None)],
        vec![PhotoDetailViewModel::new(photo_id, title, Vec::new())],
    )
    .expect("test workspace");

    shell.present();
    let transitioned = Rc::new(Cell::new(false));
    gtk4::glib::idle_add_local_once({
        let shell = shell.clone();
        let transitioned = Rc::clone(&transitioned);
        move || {
            shell.set_photo_workspace(&workspace);
            assert!(shell.open_photo(photo_id), "selected photo opens darkroom");
            shell.begin_darkroom_selection(photo_id, ViewportGeneration::new(1));
            shell.show_workspace(WorkspaceRole::Lighttable);
            gtk4::glib::idle_add_local_once(move || {
                shell.show_workspace(WorkspaceRole::Darkroom);
                left_split.set_position(150);
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
        left_stack.allocated_width() <= 152,
        "active darkroom rail must honor the 150px divider, got {}px",
        left_stack.allocated_width()
    );
    assert_darkroom_titles_are_allocated(&shell);
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

struct RenderedWidget {
    bytes: Vec<u8>,
    width: usize,
}

impl RenderedWidget {
    fn bright_pixels(&self, bounds: gtk4::graphene::Rect) -> usize {
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
                        .is_some_and(|channel| channel >= 128)
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
