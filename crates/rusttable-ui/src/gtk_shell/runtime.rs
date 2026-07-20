//! GTK4 realization of Darktable's top-level desktop layout.
//!
//! The structure mirrors the slot model in Darktable's `src/gui/gtk.h`, its
//! lighttable/darkroom view switcher, module-group panel, and filmstrip. It
//! deliberately uses GTK widgets directly instead of a framework adapter.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use rusttable_core::PhotoId;

use super::{
    DarkroomWorkspaceViewModel, LibraryBrowserModel, ModuleControlKind, ModulePanelViewModel,
    PanelSlot, ShellLayout, ShellRegion, WorkspaceRole,
};
use crate::presentation::{PhotoDetailViewModel, PhotoWorkspaceViewModel};

type PhotoSelectedHandler = Box<dyn Fn(PhotoId)>;

/// Reusable GTK4 window with Darktable-style lighttable and darkroom modes.
#[derive(Clone)]
pub struct GtkShell {
    window: gtk4::ApplicationWindow,
    layout: ShellLayout,
    workspace: gtk4::Stack,
    lighttable: gtk4::FlowBox,
    darkroom_detail: gtk4::Box,
    filmstrip: gtk4::FlowBox,
    left_modules: gtk4::Box,
    right_modules: gtk4::Box,
    photo_selected: Rc<RefCell<Option<PhotoSelectedHandler>>>,
}

impl GtkShell {
    /// Creates the standard `RustTable` desktop shell for an activated GTK app.
    ///
    /// GTK itself requires an initialized main-thread application. The pure
    /// [`ShellLayout`] API can be used in tests without that runtime setup.
    #[must_use]
    pub fn new(application: &gtk4::Application) -> Self {
        Self::with_layout(application, ShellLayout::default())
    }

    /// Creates the shell with an explicit initial workspace.
    #[must_use]
    pub fn with_layout(application: &gtk4::Application, layout: ShellLayout) -> Self {
        let window = gtk4::ApplicationWindow::builder()
            .application(application)
            .default_width(1_440)
            .default_height(920)
            .title("RustTable")
            .build();
        let (workspace, lighttable, darkroom_detail) = workspace_stack(layout.initial_workspace());
        let header = header_bar(&workspace);
        let (left_panel, left_modules) = left_panel();
        let (right_panel, right_modules) = right_panel();
        let center = central_workspace(&workspace);
        let split = gtk4::Paned::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .start_child(&left_panel)
            .end_child(&center)
            .resize_start_child(false)
            .shrink_start_child(false)
            .position(280)
            .build();
        let workspace_with_right_panel = gtk4::Paned::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .start_child(&split)
            .end_child(&right_panel)
            .resize_end_child(false)
            .shrink_end_child(false)
            .position(1_120)
            .build();
        let filmstrip = filmstrip();
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content.append(&workspace_with_right_panel);
        content.append(&filmstrip.0);

        window.set_titlebar(Some(&header));
        window.set_child(Some(&content));

        Self {
            window,
            layout,
            workspace,
            lighttable,
            darkroom_detail,
            filmstrip: filmstrip.1,
            left_modules,
            right_modules,
            photo_selected: Rc::new(RefCell::new(None)),
        }
    }

    /// Presents the application window without taking ownership of GTK's loop.
    pub fn present(&self) {
        self.window.present();
    }

    /// Returns the stable layout used to construct this runtime shell.
    #[must_use]
    pub const fn layout(&self) -> ShellLayout {
        self.layout
    }

    /// Exposes the application window for application actions and persistence.
    #[must_use]
    pub fn window(&self) -> &gtk4::ApplicationWindow {
        &self.window
    }

    /// Installs the controller callback invoked when the user selects a photo.
    ///
    /// The shell moves to darkroom before calling the handler, so controllers
    /// receive the selected photo with the appropriate workspace already shown.
    pub fn set_photo_selected_handler<F>(&self, handler: F)
    where
        F: Fn(PhotoId) + 'static,
    {
        self.photo_selected.replace(Some(Box::new(handler)));
    }

    /// Renders product presentation data into the GTK lighttable and filmstrip.
    ///
    /// The title and secondary information come directly from the typed Rust
    /// view model. Each native card switches to darkroom and reports the typed
    /// [`PhotoId`] through [`Self::set_photo_selected_handler`].
    pub fn set_lighttable_workspace(&self, view_model: &PhotoWorkspaceViewModel) {
        clear_children(&self.lighttable);
        clear_children(&self.filmstrip);
        let browser = LibraryBrowserModel::from_workspace(view_model);

        if browser.photos().len() == 0 {
            return;
        }

        for photo in browser.photos() {
            let Some(detail) = view_model.detail(photo.id()) else {
                continue;
            };
            let detail = detail.clone();
            let card = lighttable_card(photo.title(), photo.secondary());
            let filmstrip_item = filmstrip_item(photo.title());
            self.connect_photo_selection(&card, photo.id(), detail.clone());
            self.connect_photo_selection(&filmstrip_item, photo.id(), detail);
            self.lighttable.insert(&card, -1);
            self.filmstrip.insert(&filmstrip_item, -1);
        }
    }

    /// Compatibility spelling for updating the lighttable presentation model.
    pub fn set_photo_workspace(&self, view_model: &PhotoWorkspaceViewModel) {
        self.set_lighttable_workspace(view_model);
    }

    /// Updates the darkroom image detail and its controller-owned module panels.
    ///
    /// This surface deliberately accepts only `rusttable-ui` presentation
    /// types, keeping the UI crate independent from application composition.
    pub fn set_darkroom_workspace(&self, view_model: &DarkroomWorkspaceViewModel) {
        show_photo_detail(&self.darkroom_detail, view_model.detail());
        render_modules(&self.left_modules, view_model.left_modules());
        render_modules(&self.right_modules, view_model.right_modules());
        self.show_workspace(WorkspaceRole::Darkroom);
    }

    /// Switches the central workspace without starting or owning a GTK loop.
    pub fn show_workspace(&self, role: WorkspaceRole) {
        self.workspace.set_visible_child_name(role.stack_name());
    }

    fn connect_photo_selection(
        &self,
        button: &gtk4::Button,
        photo_id: PhotoId,
        detail: PhotoDetailViewModel,
    ) {
        let photo_detail = self.darkroom_detail.clone();
        let workspace = self.workspace.clone();
        let handler = Rc::clone(&self.photo_selected);
        button.connect_clicked(move |_| {
            show_photo_detail(&photo_detail, &detail);
            workspace.set_visible_child_name(WorkspaceRole::Darkroom.stack_name());
            if let Some(handler) = handler.borrow().as_ref() {
                handler(photo_id);
            }
        });
    }
}

fn header_bar(workspace: &gtk4::Stack) -> gtk4::HeaderBar {
    let header = gtk4::HeaderBar::new();
    header.set_widget_name(ShellRegion::Header.identifier());
    header.set_show_title_buttons(true);

    let brand = gtk4::Label::new(Some("RustTable"));
    brand.set_widget_name(PanelSlot::HeaderLeft.identifier());
    brand.add_css_class("title-3");
    header.pack_start(&brand);

    let tools = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    tools.set_widget_name(PanelSlot::HeaderCenter.identifier());
    tools.append(&gtk4::Button::with_label("import"));
    tools.append(&gtk4::Button::with_label("preferences"));
    header.set_title_widget(Some(&tools));

    let modes = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    modes.set_widget_name(PanelSlot::HeaderRight.identifier());
    for role in [WorkspaceRole::Lighttable, WorkspaceRole::Darkroom] {
        let button = gtk4::Button::with_label(role.title());
        let stack = workspace.clone();
        button.connect_clicked(move |_| stack.set_visible_child_name(role.stack_name()));
        modes.append(&button);
    }
    header.pack_end(&modes);
    header
}

fn left_panel() -> (gtk4::Box, gtk4::Box) {
    let panel = panel_column(ShellRegion::LeftPanel, 280);
    let top = panel_slot(PanelSlot::LeftTop);
    top.append(&panel_heading("navigation"));
    top.append(&gtk4::Button::with_label("recently used collections"));
    let center = panel_slot(PanelSlot::LeftCenter);
    let bottom = panel_slot(PanelSlot::LeftBottom);
    bottom.append(&gtk4::Label::new(Some("background jobs")));
    append_panel_slots(&panel, &top, &center, &bottom);
    (panel, center)
}

fn right_panel() -> (gtk4::Box, gtk4::Box) {
    let panel = panel_column(ShellRegion::RightPanel, 320);
    let top = panel_slot(PanelSlot::RightTop);
    top.append(&panel_heading("module groups"));
    let group_selector = gtk4::DropDown::from_strings(&["favorites", "active", "tone", "color"]);
    top.append(&group_selector);
    let center = panel_slot(PanelSlot::RightCenter);
    let bottom = panel_slot(PanelSlot::RightBottom);
    bottom.append(&gtk4::SearchEntry::new());
    append_panel_slots(&panel, &top, &center, &bottom);
    (panel, center)
}

fn central_workspace(workspace: &gtk4::Stack) -> gtk4::Box {
    let center = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    center.set_hexpand(true);
    center.set_vexpand(true);
    let top_tools = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    top_tools.set_widget_name(PanelSlot::CenterTop.identifier());
    for label in ["grid", "zoomable", "culling", "overlay"] {
        top_tools.append(&gtk4::Button::with_label(label));
    }
    let bottom_tools = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    bottom_tools.set_widget_name(PanelSlot::CenterBottom.identifier());
    for label in ["fit", "100%", "before/after", "soft proof"] {
        bottom_tools.append(&gtk4::Button::with_label(label));
    }
    center.append(&top_tools);
    center.append(workspace);
    center.append(&bottom_tools);
    center
}

fn workspace_stack(initial_workspace: WorkspaceRole) -> (gtk4::Stack, gtk4::FlowBox, gtk4::Box) {
    let workspace = gtk4::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        .transition_type(gtk4::StackTransitionType::Crossfade)
        .build();
    workspace.set_widget_name(ShellRegion::Workspace.identifier());

    let lighttable = gtk4::FlowBox::builder()
        .max_children_per_line(6)
        .selection_mode(gtk4::SelectionMode::None)
        .valign(gtk4::Align::Start)
        .build();
    lighttable.set_widget_name("lighttable-grid");
    let lighttable_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    lighttable_page.set_margin_top(16);
    lighttable_page.set_margin_bottom(16);
    lighttable_page.set_margin_start(16);
    lighttable_page.set_margin_end(16);
    lighttable_page.append(&panel_heading("lighttable"));
    let lighttable_scroll = gtk4::ScrolledWindow::builder()
        .child(&lighttable)
        .hexpand(true)
        .vexpand(true)
        .build();
    lighttable_page.append(&lighttable_scroll);

    let darkroom_detail = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    darkroom_detail.set_widget_name("darkroom-image-details");
    let darkroom_frame = gtk4::Frame::builder()
        .label("darkroom image preview")
        .child(&darkroom_detail)
        .hexpand(true)
        .vexpand(true)
        .build();
    let darkroom_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    darkroom_page.set_margin_top(16);
    darkroom_page.set_margin_bottom(16);
    darkroom_page.set_margin_start(16);
    darkroom_page.set_margin_end(16);
    darkroom_page.append(&panel_heading("darkroom"));
    darkroom_page.append(&darkroom_frame);

    workspace.add_titled(
        &lighttable_page,
        Some(WorkspaceRole::Lighttable.stack_name()),
        "lighttable",
    );
    workspace.add_titled(
        &darkroom_page,
        Some(WorkspaceRole::Darkroom.stack_name()),
        "darkroom",
    );
    workspace.set_visible_child_name(initial_workspace.stack_name());
    (workspace, lighttable, darkroom_detail)
}

fn filmstrip() -> (gtk4::Box, gtk4::FlowBox) {
    let strip = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    strip.set_widget_name(ShellRegion::Filmstrip.identifier());
    strip.set_margin_top(6);
    strip.set_margin_bottom(6);
    strip.set_margin_start(12);
    strip.set_margin_end(12);
    strip.append(&gtk4::Label::new(Some("filmstrip")));
    let photos = gtk4::FlowBox::builder()
        .max_children_per_line(12)
        .selection_mode(gtk4::SelectionMode::None)
        .build();
    photos.set_widget_name(PanelSlot::Bottom.identifier());
    strip.append(&photos);
    (strip, photos)
}

fn panel_column(region: ShellRegion, width: i32) -> gtk4::Box {
    let panel = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    panel.set_widget_name(region.identifier());
    panel.set_width_request(width);
    panel.set_margin_top(8);
    panel.set_margin_bottom(8);
    panel.set_margin_start(8);
    panel.set_margin_end(8);
    panel
}

fn panel_slot(slot: PanelSlot) -> gtk4::Box {
    let slot_widget = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    slot_widget.set_widget_name(slot.identifier());
    slot_widget
}

fn append_panel_slots(panel: &gtk4::Box, top: &gtk4::Box, center: &gtk4::Box, bottom: &gtk4::Box) {
    let scrolling_center = center.clone();
    let scroll = gtk4::ScrolledWindow::builder()
        .child(&scrolling_center)
        .hexpand(true)
        .vexpand(true)
        .build();
    panel.append(&top.clone());
    panel.append(&scroll);
    panel.append(&bottom.clone());
}

fn render_modules<'a>(
    container: &gtk4::Box,
    modules: impl ExactSizeIterator<Item = &'a ModulePanelViewModel>,
) {
    clear_children(container);
    for module in modules {
        container.append(&module_expander(module));
    }
}

fn module_expander(module: &ModulePanelViewModel) -> gtk4::Expander {
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    for control in module.controls() {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let label = gtk4::Label::new(Some(control.label().as_str()));
        label.set_halign(gtk4::Align::Start);
        label.set_hexpand(true);
        row.append(&label);
        let widget: gtk4::Widget = match control.kind() {
            ModuleControlKind::Slider => {
                gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, 1.0, 0.01).upcast()
            }
            ModuleControlKind::Toggle => gtk4::Switch::new().upcast(),
            ModuleControlKind::Choice => gtk4::DropDown::from_strings(&["default"]).upcast(),
        };
        row.append(&widget);
        content.append(&row);
    }
    gtk4::Expander::builder()
        .label(module.title().as_str())
        .expanded(true)
        .child(&content)
        .build()
}

fn lighttable_card(title: &str, secondary: Option<&str>) -> gtk4::Button {
    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    card.set_margin_top(8);
    card.set_margin_bottom(8);
    card.set_margin_start(8);
    card.set_margin_end(8);
    let title_label = gtk4::Label::new(Some(title));
    title_label.set_halign(gtk4::Align::Start);
    title_label.set_wrap(true);
    card.append(&title_label);
    if let Some(secondary) = secondary {
        let secondary_label = gtk4::Label::new(Some(secondary));
        secondary_label.set_halign(gtk4::Align::Start);
        secondary_label.add_css_class("dim-label");
        secondary_label.set_wrap(true);
        card.append(&secondary_label);
    }
    let button = gtk4::Button::new();
    button.set_widget_name("lighttable-photo-card");
    button.set_child(Some(&card));
    button
}

fn filmstrip_item(title: &str) -> gtk4::Button {
    let button = gtk4::Button::with_label(title);
    button.set_widget_name("filmstrip-photo");
    button
}

fn show_photo_detail(container: &gtk4::Box, detail: &PhotoDetailViewModel) {
    clear_children(container);
    container.append(&panel_heading(detail.title().as_str()));
    for fact in detail.facts() {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let label = gtk4::Label::new(Some(fact.label().as_str()));
        label.set_halign(gtk4::Align::Start);
        label.add_css_class("dim-label");
        let value = gtk4::Label::new(Some(fact.value().as_str()));
        value.set_halign(gtk4::Align::Start);
        row.append(&label);
        row.append(&value);
        container.append(&row);
    }
}

fn panel_heading(title: &str) -> gtk4::Label {
    let label = gtk4::Label::new(Some(title));
    label.set_halign(gtk4::Align::Start);
    label.add_css_class("title-3");
    label
}

fn clear_children(container: &impl IsA<gtk4::Widget>) {
    while let Some(child) = container.first_child() {
        child.unparent();
    }
}
