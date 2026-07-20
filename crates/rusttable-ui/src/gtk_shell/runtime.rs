//! GTK4 realization of Darktable's top-level desktop layout.
//!
//! The structure mirrors the slot model in Darktable's `src/gui/gtk.h`, its
//! lighttable/darkroom view switcher, module-group panel, and filmstrip. It
//! deliberately uses GTK widgets directly instead of a framework adapter.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use crate::display_profile::DisplayProfileBanner;
use gtk4::prelude::*;
use rusttable_core::PhotoId;
use rusttable_i18n::{Direction, I18n, MessageArgs, MessageId};

use super::{
    CollectionControlAction, CollectionControlState, CollectionControls, CollectionFilterState,
    DARKTABLE_DESKTOP_SPEC, DarkroomWorkspaceViewModel, ExportPanel, LIGHTTABLE_TOOLBAR,
    LibraryBrowserModel, ModuleControlKind, ModulePanelViewModel, PanelSlot, PhotoPreview,
    ShellLayout, ShellRegion, ThemeRole, WorkspaceRole, apply_theme_role,
};
use crate::input_mapping::InputMappingEditor;
use crate::presentation::{PhotoDetailViewModel, PhotoWorkspaceViewModel};

type PhotoSelectedHandler = Box<dyn Fn(PhotoId)>;

/// Reusable GTK4 window with Darktable-style lighttable and darkroom modes.
#[derive(Clone)]
pub struct GtkShell {
    window: gtk4::ApplicationWindow,
    layout: ShellLayout,
    workspace: gtk4::Stack,
    lighttable: gtk4::FlowBox,
    lighttable_empty_state: gtk4::Stack,
    darkroom_preview: PhotoPreview,
    export_panel: ExportPanel,
    filmstrip: gtk4::FlowBox,
    left_modules: gtk4::Box,
    right_modules: gtk4::Box,
    collection_controls: CollectionControls,
    input_mapping_editor: InputMappingEditor,
    i18n: Rc<RefCell<I18n>>,
    display_profile_banner: DisplayProfileBanner,
    lighttable_workspace: Rc<RefCell<Option<PhotoWorkspaceViewModel>>>,
    photo_selected: Rc<RefCell<Option<PhotoSelectedHandler>>>,
}

impl GtkShell {
    /// Creates the standard `RustTable` desktop shell for an activated GTK app.
    ///
    /// GTK itself requires an initialized main-thread application. The pure
    /// [`ShellLayout`] API can be used in tests without that runtime setup.
    #[must_use]
    pub fn new(application: &gtk4::Application) -> Self {
        Self::with_i18n(application, I18n::default())
    }

    /// Creates the shell with an explicit locale service.
    #[must_use]
    pub fn with_i18n(application: &gtk4::Application, i18n: I18n) -> Self {
        Self::with_layout_and_i18n(application, ShellLayout::default(), i18n)
    }

    /// Creates the shell with an explicit initial workspace.
    #[must_use]
    pub fn with_layout(application: &gtk4::Application, layout: ShellLayout) -> Self {
        Self::with_layout_and_i18n(application, layout, I18n::default())
    }

    fn with_layout_and_i18n(
        application: &gtk4::Application,
        layout: ShellLayout,
        i18n: I18n,
    ) -> Self {
        let i18n = Rc::new(RefCell::new(i18n));
        let initial_i18n = i18n.borrow();
        let window = gtk4::ApplicationWindow::builder()
            .application(application)
            .default_width(i32::from(DARKTABLE_DESKTOP_SPEC.layout.window_width_px))
            .default_height(i32::from(DARKTABLE_DESKTOP_SPEC.layout.window_height_px))
            .title(initial_i18n.text(MessageId::AppTitle, &MessageArgs::new()))
            .build();
        window.set_widget_name("rusttable-window");
        apply_theme_role(&window, ThemeRole::Shell);
        let (workspace, lighttable, lighttable_empty_state, darkroom_preview) =
            workspace_stack(layout.initial_workspace(), &initial_i18n);
        let input_mapping_editor = InputMappingEditor::new(application);
        let display_profile_banner = DisplayProfileBanner::new();
        let (header, preferences_button) =
            header_bar(&workspace, &initial_i18n, &display_profile_banner);
        preferences_button.connect_clicked({
            let editor = input_mapping_editor.clone();
            move |_| editor.present()
        });
        let collection_controls = CollectionControls::with_i18n(
            I18n::new(initial_i18n.locale().clone()).unwrap_or_default(),
        );
        let (left_panel, left_modules) = left_panel(&collection_controls, &initial_i18n);
        let (right_panel, right_modules, export_panel) = right_panel(&initial_i18n);
        let center = central_workspace(&workspace, &initial_i18n);
        let layout_metrics = DARKTABLE_DESKTOP_SPEC.layout;
        let split = gtk4::Paned::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .start_child(&left_panel)
            .end_child(&center)
            .resize_start_child(false)
            .shrink_start_child(true)
            .position(i32::from(layout_metrics.side_panel_widths.preferred_px))
            .build();
        let workspace_with_right_panel = gtk4::Paned::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .start_child(&split)
            .end_child(&right_panel)
            .resize_end_child(false)
            .shrink_end_child(true)
            .position(i32::from(layout_metrics.preferred_right_panel_position_px(
                layout_metrics.window_width_px,
            )))
            .build();
        let filmstrip = filmstrip(&initial_i18n);
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let outer_border = i32::from(layout_metrics.outer_border_px);
        content.set_margin_top(outer_border);
        content.set_margin_bottom(outer_border);
        content.set_margin_start(outer_border);
        content.set_margin_end(outer_border);
        content.append(&workspace_with_right_panel);
        content.append(&filmstrip.0);

        window.set_titlebar(Some(&header));
        window.set_child(Some(&content));

        Self {
            window,
            layout,
            workspace,
            lighttable,
            lighttable_empty_state,
            darkroom_preview,
            export_panel,
            filmstrip: filmstrip.1,
            left_modules,
            right_modules,
            collection_controls,
            input_mapping_editor,
            i18n: Rc::clone(&i18n),
            display_profile_banner,
            lighttable_workspace: Rc::new(RefCell::new(None)),
            photo_selected: Rc::new(RefCell::new(None)),
        }
    }

    /// Applies a locale and GTK text direction to the live shell controls.
    pub fn set_locale(&self, i18n: I18n) {
        let direction = i18n.direction();
        self.window.set_direction(match direction {
            Direction::Ltr => gtk4::TextDirection::Ltr,
            Direction::Rtl => gtk4::TextDirection::Rtl,
        });
        self.collection_controls
            .set_locale(I18n::new(i18n.locale().clone()).unwrap_or_default());
        self.i18n.replace(i18n);
    }

    /// Presents the application window without taking ownership of GTK's loop.
    pub fn present(&self) {
        if self.lighttable.first_child().is_none() {
            self.lighttable_empty_state.set_visible_child_name("empty");
        }
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

    /// Returns the reusable darkroom preview surface for rendered texture updates.
    #[must_use]
    pub fn darkroom_preview(&self) -> &PhotoPreview {
        &self.darkroom_preview
    }

    /// Returns the Darktable-shaped selected-photo PNG export module.
    #[must_use]
    pub fn export_panel(&self) -> &ExportPanel {
        &self.export_panel
    }

    /// Enables the export module only when a catalog photo is selected.
    pub fn set_export_selected(&self, selected: bool) {
        self.export_panel.set_selected(selected);
    }

    /// Installs the application-owned callback for export module actions.
    pub fn connect_export_action<F>(&self, handler: F)
    where
        F: Fn(super::ExportAction) + 'static,
    {
        self.export_panel.connect_action(handler);
    }

    /// Returns the Darktable-shaped collection rule controls in the left panel.
    #[must_use]
    pub fn collection_controls(&self) -> &CollectionControls {
        &self.collection_controls
    }

    /// Returns the GTK4 shortcut/device preferences editor.
    #[must_use]
    pub fn input_mapping_editor(&self) -> &InputMappingEditor {
        &self.input_mapping_editor
    }

    /// Returns the visible monitor-profile status surface.
    #[must_use]
    pub const fn display_profile_banner(&self) -> &DisplayProfileBanner {
        &self.display_profile_banner
    }

    /// Projects collection counts and rule values into the left-panel controls.
    pub fn set_collection_state(&self, state: &CollectionControlState) {
        self.collection_controls.set_state(state);
    }

    /// Applies a collection projection to both the controls and the lighttable.
    pub fn set_collection_filter_state(&self, state: &CollectionFilterState) {
        self.collection_controls.set_state(state.controls());
        self.refresh_lighttable(state.matching_photo_ids());
    }

    /// Connects a typed collection action to an application-owned rule controller.
    ///
    /// The callback captures only GTK child handles and the application controller. It does not
    /// capture `GtkShell`, avoiding a shell-to-handler reference cycle.
    pub fn connect_collection_action<F>(&self, callback: F)
    where
        F: Fn(CollectionControlAction) -> CollectionFilterState + 'static,
    {
        let refresh = CollectionRefreshHandle {
            controls: self.collection_controls.clone(),
            render: self.workspace_render_handle(),
            lighttable_workspace: Rc::clone(&self.lighttable_workspace),
        };
        self.collection_controls.connect_action(move |action| {
            refresh.apply(&callback(action));
        });
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
        self.lighttable_workspace.replace(Some(view_model.clone()));
        self.workspace_render_handle().render(view_model, None);
    }

    /// Renders only the photos selected by the active collection rule.
    pub fn set_lighttable_workspace_filtered(
        &self,
        view_model: &PhotoWorkspaceViewModel,
        matching_photo_ids: impl IntoIterator<Item = PhotoId>,
    ) {
        let matching_photo_ids = matching_photo_ids.into_iter().collect::<BTreeSet<_>>();
        self.lighttable_workspace.replace(Some(view_model.clone()));
        self.workspace_render_handle()
            .render(view_model, Some(&matching_photo_ids));
    }

    fn refresh_lighttable(&self, matching_photo_ids: &[PhotoId]) {
        let workspace = self.lighttable_workspace.borrow();
        let Some(view_model) = workspace.as_ref() else {
            return;
        };
        let matching_photo_ids = matching_photo_ids.iter().copied().collect::<BTreeSet<_>>();
        self.workspace_render_handle()
            .render(view_model, Some(&matching_photo_ids));
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
        self.darkroom_preview.set_detail(view_model.detail());
        render_modules(&self.left_modules, view_model.left_modules());
        render_modules(&self.right_modules, view_model.right_modules());
        self.show_workspace(WorkspaceRole::Darkroom);
    }

    /// Switches the central workspace without starting or owning a GTK loop.
    pub fn show_workspace(&self, role: WorkspaceRole) {
        self.workspace.set_visible_child_name(role.stack_name());
    }

    fn workspace_render_handle(&self) -> WorkspaceRenderHandle {
        WorkspaceRenderHandle {
            lighttable: self.lighttable.clone(),
            lighttable_empty_state: self.lighttable_empty_state.clone(),
            filmstrip: self.filmstrip.clone(),
            darkroom_preview: self.darkroom_preview.clone(),
            workspace: self.workspace.clone(),
            photo_selected: Rc::clone(&self.photo_selected),
            export_panel: self.export_panel.clone(),
        }
    }
}

#[derive(Clone)]
struct CollectionRefreshHandle {
    controls: CollectionControls,
    render: WorkspaceRenderHandle,
    lighttable_workspace: Rc<RefCell<Option<PhotoWorkspaceViewModel>>>,
}

impl CollectionRefreshHandle {
    fn apply(&self, state: &CollectionFilterState) {
        self.controls.set_state(state.controls());
        let workspace = self.lighttable_workspace.borrow();
        let Some(view_model) = workspace.as_ref() else {
            return;
        };
        let matching_photo_ids = state
            .matching_photo_ids()
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        self.render.render(view_model, Some(&matching_photo_ids));
    }
}

#[derive(Clone)]
struct WorkspaceRenderHandle {
    lighttable: gtk4::FlowBox,
    lighttable_empty_state: gtk4::Stack,
    filmstrip: gtk4::FlowBox,
    darkroom_preview: PhotoPreview,
    workspace: gtk4::Stack,
    photo_selected: Rc<RefCell<Option<PhotoSelectedHandler>>>,
    export_panel: ExportPanel,
}

impl WorkspaceRenderHandle {
    fn render(
        &self,
        view_model: &PhotoWorkspaceViewModel,
        matching_photo_ids: Option<&BTreeSet<PhotoId>>,
    ) {
        clear_children(&self.lighttable);
        clear_children(&self.filmstrip);
        let browser = LibraryBrowserModel::from_workspace(view_model);
        let mut rendered_photos = 0;

        for photo in browser.photos() {
            if matching_photo_ids.is_some_and(|ids| !ids.contains(&photo.id())) {
                continue;
            }
            let Some(detail) = view_model.detail(photo.id()) else {
                continue;
            };
            let detail = detail.clone();
            let card = lighttable_card(photo.id(), photo.title(), photo.secondary());
            let filmstrip_item = filmstrip_item(photo.id(), photo.title());
            connect_photo_selection(
                &card,
                photo.id(),
                detail.clone(),
                &self.darkroom_preview,
                &self.workspace,
                &self.photo_selected,
                &self.export_panel,
            );
            connect_photo_selection(
                &filmstrip_item,
                photo.id(),
                detail,
                &self.darkroom_preview,
                &self.workspace,
                &self.photo_selected,
                &self.export_panel,
            );
            self.lighttable.insert(&card, -1);
            self.filmstrip.insert(&filmstrip_item, -1);
            rendered_photos += 1;
        }
        self.lighttable_empty_state
            .set_visible_child_name(if rendered_photos == 0 {
                "empty"
            } else {
                "grid"
            });
    }
}

fn connect_photo_selection(
    button: &gtk4::Button,
    photo_id: PhotoId,
    detail: PhotoDetailViewModel,
    photo_preview: &PhotoPreview,
    workspace: &gtk4::Stack,
    photo_selected: &Rc<RefCell<Option<PhotoSelectedHandler>>>,
    export_panel: &ExportPanel,
) {
    let photo_preview = photo_preview.clone();
    let workspace = workspace.clone();
    let handler = Rc::clone(photo_selected);
    let export_panel = export_panel.clone();
    let selected_button = button.clone();
    button.connect_clicked(move |_| {
        selected_button.add_css_class(ThemeRole::SelectedPhoto.class_name());
        export_panel.set_selected(true);
        show_photo_detail(&photo_preview, &detail);
        workspace.set_visible_child_name(WorkspaceRole::Darkroom.stack_name());
        if let Some(handler) = handler.borrow().as_ref() {
            handler(photo_id);
        }
    });
}

fn header_bar(
    workspace: &gtk4::Stack,
    i18n: &I18n,
    display_profile_banner: &DisplayProfileBanner,
) -> (gtk4::HeaderBar, gtk4::Button) {
    let header = gtk4::HeaderBar::new();
    header.set_widget_name(ShellRegion::Header.identifier());
    apply_theme_role(&header, ThemeRole::Header);
    header.set_show_title_buttons(true);

    let brand = gtk4::Label::new(Some(&i18n.text(MessageId::AppTitle, &MessageArgs::new())));
    brand.set_widget_name(PanelSlot::HeaderLeft.identifier());
    apply_theme_role(&brand, ThemeRole::Toolbar);
    brand.add_css_class("title-3");
    header.pack_start(&brand);

    let tools = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    tools.set_widget_name(PanelSlot::HeaderCenter.identifier());
    apply_theme_role(&tools, ThemeRole::Toolbar);
    tools.append(&gtk4::Button::with_label(
        &i18n.text(MessageId::ToolbarImport, &MessageArgs::new()),
    ));
    let preferences =
        gtk4::Button::with_label(&i18n.text(MessageId::ToolbarPreferences, &MessageArgs::new()));
    tools.append(display_profile_banner.widget());
    tools.append(&preferences);
    header.set_title_widget(Some(&tools));

    let modes = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    modes.set_widget_name(PanelSlot::HeaderRight.identifier());
    apply_theme_role(&modes, ThemeRole::ViewSwitcher);
    let lighttable_button =
        gtk4::Button::with_label(&i18n.text(MessageId::WorkspaceLighttable, &MessageArgs::new()));
    lighttable_button.set_widget_name("view-lighttable");
    lighttable_button.add_css_class("active");
    let darkroom_button =
        gtk4::Button::with_label(&i18n.text(MessageId::WorkspaceDarkroom, &MessageArgs::new()));
    darkroom_button.set_widget_name("view-darkroom");
    let darkroom_stack = workspace.clone();
    let darkroom_lighttable_button = lighttable_button.clone();
    let darkroom_button_handle = darkroom_button.clone();
    darkroom_button.connect_clicked(move |_| {
        darkroom_stack.set_visible_child_name(WorkspaceRole::Darkroom.stack_name());
        darkroom_lighttable_button.remove_css_class("active");
        darkroom_button_handle.add_css_class("active");
    });
    let lighttable_stack = workspace.clone();
    let lighttable_darkroom_button = darkroom_button.clone();
    let lighttable_button_handle = lighttable_button.clone();
    lighttable_button.connect_clicked(move |_| {
        lighttable_stack.set_visible_child_name(WorkspaceRole::Lighttable.stack_name());
        lighttable_darkroom_button.remove_css_class("active");
        lighttable_button_handle.add_css_class("active");
    });
    modes.append(&lighttable_button);
    modes.append(&darkroom_button);
    let other = gtk4::Button::with_label("other ▾");
    other.set_widget_name("view-other");
    modes.append(&other);
    header.pack_end(&modes);
    (header, preferences)
}

fn left_panel(collection_controls: &CollectionControls, i18n: &I18n) -> (gtk4::Box, gtk4::Box) {
    let panel = panel_column(
        ShellRegion::LeftPanel,
        i32::from(DARKTABLE_DESKTOP_SPEC.layout.side_panel_widths.preferred_px),
    );
    apply_theme_role(&panel, ThemeRole::Panel);
    let top = panel_slot(PanelSlot::LeftTop);
    top.append(&panel_heading(i18n, MessageId::PanelNavigation));
    top.append(&module_group("import", "import", false));
    top.append(&module_group_with_child(
        "collections",
        "collections",
        collection_controls.widget(),
        true,
    ));
    let center = panel_slot(PanelSlot::LeftCenter);
    center.append(&module_group(
        "collection-filters",
        "collection filters",
        false,
    ));
    let image_information = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    image_information.append(&gtk4::Label::new(Some("selected image")));
    image_information.append(&gtk4::Label::new(Some("rating · color label · tags")));
    center.append(&module_group_with_child(
        "image-information",
        "image information",
        &image_information,
        false,
    ));
    let bottom = panel_slot(PanelSlot::LeftBottom);
    bottom.append(&module_group("scripts", "scripts", false));
    bottom.append(&gtk4::Label::new(Some(
        &i18n.text(MessageId::PanelBackgroundJobs, &MessageArgs::new()),
    )));
    append_panel_slots(&panel, &top, &center, &bottom);
    (panel, center)
}

fn right_panel(i18n: &I18n) -> (gtk4::Box, gtk4::Box, ExportPanel) {
    let panel = panel_column(
        ShellRegion::RightPanel,
        i32::from(DARKTABLE_DESKTOP_SPEC.layout.side_panel_widths.preferred_px),
    );
    apply_theme_role(&panel, ThemeRole::Panel);
    let top = panel_slot(PanelSlot::RightTop);
    top.append(&panel_heading(i18n, MessageId::PanelModuleGroups));
    let groups = [
        i18n.text(MessageId::PanelFavorites, &MessageArgs::new()),
        i18n.text(MessageId::PanelActive, &MessageArgs::new()),
        i18n.text(MessageId::PanelTone, &MessageArgs::new()),
        i18n.text(MessageId::PanelColor, &MessageArgs::new()),
    ];
    let group_labels = groups.iter().map(String::as_str).collect::<Vec<_>>();
    let group_selector = gtk4::DropDown::from_strings(&group_labels);
    group_selector.set_widget_name("right-module-group-selector");
    top.append(&group_selector);
    let export_panel = ExportPanel::new();
    let center = panel_slot(PanelSlot::RightCenter);
    for (id, label) in [
        ("selection", "selection"),
        ("actions-on-selection", "actions on selection"),
        ("history-stack", "history stack"),
        ("styles", "styles"),
        ("metadata-editor", "metadata editor"),
        ("tagging", "tagging"),
        ("geotagging", "geotagging"),
        ("neural-restore", "neural restore"),
    ] {
        center.append(&module_group(id, label, false));
    }
    center.append(&module_group_with_child(
        "export",
        "export",
        export_panel.widget(),
        true,
    ));
    let bottom = panel_slot(PanelSlot::RightBottom);
    bottom.append(&gtk4::SearchEntry::new());
    append_panel_slots(&panel, &top, &center, &bottom);
    (panel, center, export_panel)
}

fn central_workspace(workspace: &gtk4::Stack, i18n: &I18n) -> gtk4::Box {
    let center = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    center.set_hexpand(true);
    center.set_vexpand(true);
    center.set_widget_name("workspace");
    apply_theme_role(&center, ThemeRole::Workspace);
    let bottom_tools = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    bottom_tools.set_widget_name(PanelSlot::CenterBottom.identifier());
    apply_theme_role(&bottom_tools, ThemeRole::Toolbar);
    for message_id in [
        MessageId::WorkspaceFit,
        MessageId::WorkspaceBeforeAfter,
        MessageId::WorkspaceSoftProof,
    ] {
        bottom_tools.append(&gtk4::Button::with_label(
            &i18n.text(message_id, &MessageArgs::new()),
        ));
    }
    bottom_tools.insert_child_after(&gtk4::Button::with_label("100%"), None::<&gtk4::Widget>);
    center.append(workspace);
    center.append(&bottom_tools);
    center
}

fn workspace_stack(
    initial_workspace: WorkspaceRole,
    i18n: &I18n,
) -> (gtk4::Stack, gtk4::FlowBox, gtk4::Stack, PhotoPreview) {
    let workspace = gtk4::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        .transition_type(gtk4::StackTransitionType::Crossfade)
        .build();
    workspace.set_widget_name("center-workspace");
    apply_theme_role(&workspace, ThemeRole::Workspace);

    let lighttable = gtk4::FlowBox::builder()
        .max_children_per_line(6)
        .selection_mode(gtk4::SelectionMode::None)
        .valign(gtk4::Align::Start)
        .build();
    lighttable.set_widget_name("lighttable-grid");
    apply_theme_role(&lighttable, ThemeRole::Lighttable);
    let lighttable_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    lighttable_page.set_widget_name("lighttable-page");
    lighttable_page.append(&lighttable_toolbar(i18n));
    let lighttable_scroll = gtk4::ScrolledWindow::builder()
        .child(&lighttable)
        .hexpand(true)
        .vexpand(true)
        .build();
    let empty_state = empty_collection_state();
    empty_state.set_halign(gtk4::Align::Fill);
    empty_state.set_valign(gtk4::Align::Fill);
    empty_state.set_hexpand(true);
    empty_state.set_vexpand(true);
    empty_state.set_visible(true);
    let lighttable_canvas = gtk4::Stack::new();
    lighttable_canvas.set_hexpand(true);
    lighttable_canvas.set_vexpand(true);
    apply_theme_role(&lighttable_canvas, ThemeRole::Lighttable);
    lighttable_canvas.add_named(&lighttable_scroll, Some("grid"));
    lighttable_canvas.add_named(&empty_state, Some("empty"));
    lighttable_canvas.set_visible_child_name("empty");
    lighttable_page.append(&lighttable_canvas);

    let darkroom_preview = PhotoPreview::new();
    let darkroom_page = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    darkroom_page.set_margin_top(16);
    darkroom_page.set_margin_bottom(16);
    darkroom_page.set_margin_start(16);
    darkroom_page.set_margin_end(16);
    apply_theme_role(&darkroom_page, ThemeRole::Darkroom);
    darkroom_page.append(&panel_heading(i18n, MessageId::WorkspaceDarkroom));
    darkroom_page.append(darkroom_preview.widget());

    workspace.add_titled(
        &lighttable_page,
        Some(WorkspaceRole::Lighttable.stack_name()),
        &i18n.text(MessageId::WorkspaceLighttable, &MessageArgs::new()),
    );
    workspace.add_titled(
        &darkroom_page,
        Some(WorkspaceRole::Darkroom.stack_name()),
        &i18n.text(MessageId::WorkspaceDarkroom, &MessageArgs::new()),
    );
    workspace.set_visible_child_name(initial_workspace.stack_name());
    (workspace, lighttable, lighttable_canvas, darkroom_preview)
}

fn filmstrip(i18n: &I18n) -> (gtk4::Box, gtk4::FlowBox) {
    let strip = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    strip.set_widget_name(ShellRegion::Filmstrip.identifier());
    apply_theme_role(&strip, ThemeRole::Filmstrip);
    strip.set_margin_top(6);
    strip.set_margin_bottom(6);
    strip.set_margin_start(12);
    strip.set_margin_end(12);
    strip.set_height_request(i32::from(
        DARKTABLE_DESKTOP_SPEC.layout.filmstrip_heights.preferred_px,
    ));
    strip.append(&filmstrip_toolbar(i18n));
    let photos = gtk4::FlowBox::builder()
        .max_children_per_line(12)
        .selection_mode(gtk4::SelectionMode::None)
        .build();
    photos.set_widget_name(PanelSlot::Bottom.identifier());
    strip.append(&photos);
    (strip, photos)
}

fn lighttable_toolbar(i18n: &I18n) -> gtk4::Box {
    let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    toolbar.set_widget_name(LIGHTTABLE_TOOLBAR.widget_name);
    apply_theme_role(&toolbar, ThemeRole::Toolbar);

    let import =
        gtk4::Button::with_label(&i18n.text(MessageId::ToolbarImport, &MessageArgs::new()));
    import.set_widget_name("lighttable-import");
    toolbar.append(&import);

    let property = gtk4::DropDown::from_strings(&["filename", "folder", "capture date"]);
    property.set_widget_name("lighttable-filter-property");
    toolbar.append(&property);
    let filter = gtk4::SearchEntry::new();
    filter.set_widget_name(LIGHTTABLE_TOOLBAR.filter_entry_name);
    filter.set_placeholder_text(Some("filter all images"));
    filter.set_hexpand(true);
    toolbar.append(&filter);

    for (index, color) in ["★", "●", "●", "●", "●", "●"].into_iter().enumerate() {
        let button = gtk4::Button::with_label(color);
        button.set_widget_name(&format!("lighttable-rating-{index}"));
        button.add_css_class("dt_filter_button");
        toolbar.append(&button);
    }
    let sort = gtk4::DropDown::from_strings(&["filename", "capture date", "rating"]);
    sort.set_widget_name("lighttable-sort");
    toolbar.append(&sort);
    let count = gtk4::Label::new(Some("0 images selected of 0"));
    count.set_widget_name("lighttable-selection-count");
    count.add_css_class("dim-label");
    toolbar.append(&count);
    toolbar
}

fn filmstrip_toolbar(i18n: &I18n) -> gtk4::Box {
    let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    toolbar.set_widget_name("filmstrip-toolbar");
    apply_theme_role(&toolbar, ThemeRole::Toolbar);
    toolbar.append(&gtk4::Label::new(Some(
        &i18n.text(MessageId::PanelFilmstrip, &MessageArgs::new()),
    )));
    for label in ["grid", "zoom", "fit", "before/after", "soft proof"] {
        let button = gtk4::Button::with_label(label);
        button.add_css_class("dt_filmstrip_button");
        toolbar.append(&button);
    }
    toolbar
}

fn panel_column(region: ShellRegion, width: i32) -> gtk4::Box {
    let panel = gtk4::Box::new(
        gtk4::Orientation::Vertical,
        i32::from(DARKTABLE_DESKTOP_SPEC.layout.panel_module_spacing_px),
    );
    panel.set_widget_name(region.identifier());
    apply_theme_role(&panel, ThemeRole::Panel);
    panel.set_width_request(width);
    panel
}

fn panel_slot(slot: PanelSlot) -> gtk4::Box {
    let slot_widget = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    slot_widget.set_widget_name(slot.identifier());
    slot_widget.add_css_class("dt_panel_slot");
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
    for (index, module) in modules.enumerate() {
        container.append(&module_expander(module, index));
    }
}

fn module_group(id: &str, label: &str, expanded: bool) -> gtk4::Expander {
    let group_widget = gtk4::Expander::builder()
        .label(label)
        .expanded(expanded)
        .build();
    group_widget.set_widget_name(id);
    apply_theme_role(&group_widget, ThemeRole::ModuleGroup);
    group_widget
}

fn module_group_with_child<W: IsA<gtk4::Widget>>(
    id: &str,
    label: &str,
    child: &W,
    expanded: bool,
) -> gtk4::Expander {
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    content.append(child);
    let module_widget = gtk4::Expander::builder()
        .label(label)
        .expanded(expanded)
        .child(&content)
        .build();
    module_widget.set_widget_name(id);
    apply_theme_role(&module_widget, ThemeRole::ModuleGroup);
    module_widget
}

fn module_expander(module: &ModulePanelViewModel, index: usize) -> gtk4::Expander {
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
    let expander = gtk4::Expander::builder()
        .label(module.title().as_str())
        .expanded(true)
        .child(&content)
        .build();
    expander.set_widget_name(&format!("module-{index}"));
    apply_theme_role(&expander, ThemeRole::Module);
    expander
}

fn lighttable_card(photo_id: PhotoId, title: &str, secondary: Option<&str>) -> gtk4::Button {
    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    card.set_margin_top(4);
    card.set_margin_bottom(4);
    card.set_margin_start(4);
    card.set_margin_end(4);
    let image_placeholder = gtk4::Label::new(Some("RAW"));
    image_placeholder.set_widget_name("photo-thumbnail");
    apply_theme_role(&image_placeholder, ThemeRole::ThumbnailImage);
    image_placeholder.set_halign(gtk4::Align::Fill);
    image_placeholder.set_valign(gtk4::Align::Fill);
    card.append(&image_placeholder);
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
    button.set_widget_name(&format!("photo-{photo_id}"));
    apply_theme_role(&button, ThemeRole::PhotoCard);
    button.set_child(Some(&card));
    button
}

fn filmstrip_item(photo_id: PhotoId, title: &str) -> gtk4::Button {
    let button = gtk4::Button::with_label(title);
    button.set_widget_name(&format!("filmstrip-photo-{photo_id}"));
    apply_theme_role(&button, ThemeRole::PhotoCard);
    button
}

fn empty_collection_state() -> gtk4::Box {
    let root = gtk4::Box::new(gtk4::Orientation::Horizontal, 36);
    root.set_widget_name("empty-collection-state");
    root.set_halign(gtk4::Align::Center);
    root.set_valign(gtk4::Align::Center);
    root.set_margin_start(48);
    root.set_margin_end(48);
    root.set_margin_top(28);
    root.set_margin_bottom(28);
    apply_theme_role(&root, ThemeRole::EmptyState);

    let collection = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    collection.set_hexpand(true);
    collection.set_valign(gtk4::Align::Center);
    let title = gtk4::Label::new(Some("there are no images in this collection"));
    title.add_css_class("title-3");
    title.set_halign(gtk4::Align::Start);
    collection.append(&title);
    for text in [
        "if you have not imported any images yet",
        "you can do so from the import module",
        "try to relax the filter settings in the top panel",
        "or add images in the collections module",
        "try the 'no-click' workflow",
        "hover over an image and use keyboard shortcuts",
        "to apply ratings, colors, styles, etc.",
        "hover over any button for its description and shortcuts",
    ] {
        let label = gtk4::Label::new(Some(text));
        label.set_halign(gtk4::Align::Start);
        label.set_wrap(true);
        collection.append(&label);
    }

    let help = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    help.set_hexpand(true);
    help.set_valign(gtk4::Align::Center);
    let help_title = gtk4::Label::new(Some("need help?"));
    help_title.add_css_class("title-3");
    help_title.set_halign(gtk4::Align::End);
    help.append(&help_title);
    for text in [
        "click on ? then an on-screen item to open the manual page",
        "press and hold 'h' to show all active keyboard shortcuts",
        "personalize darktable",
        "click on the gear icon for global preferences",
        "set module-specific preferences through a module's menu",
        "make default raw development look more like your camera's JPEG",
        "by applying a camera-specific style",
    ] {
        let label = gtk4::Label::new(Some(text));
        label.set_halign(gtk4::Align::End);
        label.set_wrap(true);
        help.append(&label);
    }
    root.append(&collection);
    root.append(&help);
    root
}

fn show_photo_detail(preview: &PhotoPreview, detail: &PhotoDetailViewModel) {
    preview.set_detail(detail);
}

fn panel_heading(i18n: &I18n, message_id: MessageId) -> gtk4::Label {
    let label = gtk4::Label::new(Some(&i18n.text(message_id, &MessageArgs::new())));
    label.set_halign(gtk4::Align::Start);
    label.add_css_class("title-3");
    label.add_css_class("dt_section_label");
    label
}

fn clear_children(container: &impl IsA<gtk4::Widget>) {
    while let Some(child) = container.first_child() {
        child.unparent();
    }
}
