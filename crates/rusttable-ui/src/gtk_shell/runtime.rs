//! GTK4 realization of Darktable's top-level desktop layout.
//!
//! The structure mirrors the slot model in Darktable's `src/gui/gtk.h`, its
//! lighttable/darkroom view switcher, module-group panel, and filmstrip. It
//! deliberately uses GTK widgets directly instead of a framework adapter.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use crate::display_profile::DisplayProfileBanner;
use gtk4::prelude::*;
use rusttable_core::PhotoId;
use rusttable_i18n::{Direction, I18n, MessageArgs, MessageId};

use super::runtime_layout::{
    desktop_body, mode_panel_stack, render_modules, right_panel, synchronize_panel_stacks,
    workspace_stack,
};
use super::thumbnail::{ThumbnailPair, ThumbnailSurface};
use super::{
    CollectionControlAction, CollectionControlState, CollectionControls, CollectionFilterState,
    DARKTABLE_DESKTOP_SPEC, DarkroomView, DarkroomWorkspaceViewModel, ExportPanel, ImportAction,
    LibraryBrowserModel, LighttableContentState, LighttableToolbar, LighttableToolbarAction,
    LighttableToolbarState, PhotoPreview, ShellLayout, THUMBNAIL_METRICS, ThemeRole, WorkspaceRole,
    apply_theme_role,
};
use super::{header::HeaderChrome, left_panel::LeftPanel};
use crate::ai_batch::AiBatchPanel;
use crate::ai_models::AiModelsPanel;
use crate::camera::{CameraAction, CameraPanel, CameraViewModel};
use crate::external_editor::{ExternalEditorAction, ExternalEditorPanel, ExternalEditorViewModel};
use crate::import::{ImportSessionAction, ImportSessionPanel, ImportSessionViewModel};
use crate::input_mapping::InputMappingEditor;
use crate::neural_restore::NeuralRestorePanel;
use crate::presentation::{PhotoDetailViewModel, PhotoWorkspaceViewModel};
use crate::viewport_presentation::{DisplayPresentationFrame, PresentationStatus};

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
    external_editor_panel: ExternalEditorPanel,
    filmstrip: gtk4::FlowBox,
    left_modules: gtk4::Box,
    right_modules: gtk4::Box,
    import_buttons: Vec<gtk4::Button>,
    collection_controls: CollectionControls,
    lighttable_toolbar: LighttableToolbar,
    input_mapping_editor: InputMappingEditor,
    pub(super) ai_models_panel: AiModelsPanel,
    pub(super) neural_restore_panel: NeuralRestorePanel,
    pub(super) ai_batch_panel: AiBatchPanel,
    camera_panel: CameraPanel,
    import_session_panel: ImportSessionPanel,
    i18n: Rc<RefCell<I18n>>,
    display_profile_banner: DisplayProfileBanner,
    lighttable_workspace: Rc<RefCell<Option<PhotoWorkspaceViewModel>>>,
    photo_selected: Rc<RefCell<Option<PhotoSelectedHandler>>>,
    photo_tiles: Rc<RefCell<BTreeMap<PhotoId, PhotoTilePair>>>,
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
        let panel_width = i32::from(DARKTABLE_DESKTOP_SPEC.layout.side_panel_widths.preferred_px);
        let darkroom = DarkroomView::new(panel_width);
        let darkroom_preview = darkroom.preview().clone();
        let (workspace, lighttable, lighttable_empty_state) =
            workspace_stack(layout.initial_workspace(), &initial_i18n, darkroom.page());
        let input_mapping_editor = InputMappingEditor::new(application);
        let ai_models_panel = AiModelsPanel::new();
        let display_profile_banner = DisplayProfileBanner::new();
        let header = HeaderChrome::new(&workspace, &initial_i18n, &display_profile_banner);
        let lighttable_toolbar = header.lighttable_toolbar().clone();
        header.preferences_button().connect_clicked({
            let editor = input_mapping_editor.clone();
            let ai_models = ai_models_panel.clone();
            let window = window.clone();
            move |_| {
                editor.present();
                ai_models.present(&window);
            }
        });
        let collection_controls = CollectionControls::with_i18n(
            I18n::new(initial_i18n.locale().clone()).unwrap_or_default(),
        );
        let lighttable_left_panel = LeftPanel::new(&collection_controls, &initial_i18n);
        let (
            lighttable_right_panel,
            export_panel,
            external_editor_panel,
            neural_restore_panel,
            ai_batch_panel,
            camera_panel,
            import_session_panel,
        ) = right_panel();
        let left_panel = mode_panel_stack(
            "left-panel-stack",
            lighttable_left_panel.widget(),
            darkroom.left_panel(),
            layout.initial_workspace(),
        );
        let right_panel = mode_panel_stack(
            "right-panel-stack",
            &lighttable_right_panel,
            darkroom.right_panel(),
            layout.initial_workspace(),
        );
        synchronize_panel_stacks(&workspace, &left_panel, &right_panel);
        let (content, filmstrip) =
            desktop_body(&workspace, &left_panel, &right_panel, &initial_i18n);

        let shell = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        apply_theme_role(&shell, ThemeRole::Shell);
        shell.append(header.widget());
        shell.append(&content);
        window.set_child(Some(&shell));

        Self {
            window,
            layout,
            workspace,
            lighttable,
            lighttable_empty_state,
            darkroom_preview,
            export_panel,
            external_editor_panel,
            filmstrip,
            left_modules: darkroom.left_modules().clone(),
            right_modules: darkroom.right_modules().clone(),
            import_buttons: vec![
                header.import_button().clone(),
                lighttable_left_panel.import_button().clone(),
            ],
            collection_controls,
            lighttable_toolbar,
            input_mapping_editor,
            ai_models_panel,
            neural_restore_panel,
            ai_batch_panel,
            camera_panel,
            import_session_panel,
            i18n: Rc::clone(&i18n),
            display_profile_banner,
            lighttable_workspace: Rc::new(RefCell::new(None)),
            photo_selected: Rc::new(RefCell::new(None)),
            photo_tiles: Rc::new(RefCell::new(BTreeMap::new())),
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

    /// Returns the service-safe external-editor workflow module.
    #[must_use]
    pub fn external_editor_panel(&self) -> &ExternalEditorPanel {
        &self.external_editor_panel
    }

    /// Projects external-editor presets, qualification, and durable job state into GTK.
    pub fn set_external_editor_state(&self, state: &ExternalEditorViewModel) {
        self.external_editor_panel.set_state(state);
    }

    /// Connects typed external-editor commands to the application service boundary.
    pub fn connect_external_editor_action<F>(&self, handler: F)
    where
        F: Fn(ExternalEditorAction) + 'static,
    {
        self.external_editor_panel.connect_action(handler);
    }

    /// Updates the send-to module with the application's revision-pinned selection count.
    pub fn set_external_editor_selection(&self, count: usize) {
        self.external_editor_panel.set_selection(count);
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
        self.lighttable_toolbar.set_state(state.toolbar());
        self.refresh_lighttable(state.matching_photo_ids());
    }

    /// Projects the typed lighttable toolbar state into the persistent header row.
    pub fn set_lighttable_toolbar_state(&self, state: &LighttableToolbarState) {
        self.lighttable_toolbar.set_state(state);
    }

    /// Connects the persistent header controls to the application collection controller.
    pub fn connect_lighttable_toolbar_action<F>(&self, callback: F)
    where
        F: Fn(LighttableToolbarAction) -> CollectionFilterState + 'static,
    {
        let refresh = CollectionRefreshHandle {
            controls: self.collection_controls.clone(),
            toolbar: self.lighttable_toolbar.clone(),
            render: self.workspace_render_handle(),
            lighttable_workspace: Rc::clone(&self.lighttable_workspace),
        };
        self.lighttable_toolbar.connect_action(move |action| {
            refresh.apply(&callback(action));
        });
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
            toolbar: self.lighttable_toolbar.clone(),
            render: self.workspace_render_handle(),
            lighttable_workspace: Rc::clone(&self.lighttable_workspace),
        };
        self.collection_controls.connect_action(move |action| {
            refresh.apply(&callback(action));
        });
    }

    /// Connects both visible import buttons to one typed application callback.
    pub fn connect_import_action<F>(&self, callback: F)
    where
        F: Fn(ImportAction) + 'static,
    {
        let callback = Rc::new(callback);
        for button in &self.import_buttons {
            let callback = Rc::clone(&callback);
            button.connect_clicked(move |_| callback(ImportAction::ChooseFiles));
        }
    }

    /// Projects #469 camera discovery/session/capture state into the shell.
    pub fn set_camera_state(&self, state: &CameraViewModel) {
        self.camera_panel.set_state(state);
    }

    /// Sends camera actions to an application-owned `rusttable-camera` service port.
    pub fn connect_camera_action<F>(&self, callback: F)
    where
        F: Fn(CameraAction) + 'static,
    {
        self.camera_panel.connect_action(callback);
    }

    /// Projects import-session review/progress/recovery state into the shell.
    pub fn set_import_session_state(&self, state: &ImportSessionViewModel) {
        self.import_session_panel.set_state(state);
    }

    /// Sends import-session actions to an application-owned import service adapter.
    pub fn connect_import_session_action<F>(&self, callback: F)
    where
        F: Fn(ImportSessionAction) + 'static,
    {
        self.import_session_panel.connect_action(callback);
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

    /// Installs a background-rendered thumbnail into the synchronized grid and filmstrip tiles.
    ///
    /// # Errors
    ///
    /// Returns a typed texture error when validated dimensions exceed GTK's representation.
    pub fn set_photo_thumbnail(
        &self,
        photo_id: PhotoId,
        metadata: &crate::presentation::Rgba8PreviewMetadata,
    ) -> Result<(), super::PhotoPreviewTextureError> {
        let tiles = self.photo_tiles.borrow();
        let Some(tile) = tiles.get(&photo_id) else {
            return Ok(());
        };
        tile.thumbnails.set_rgba8(metadata)
    }

    /// Projects a bounded background-rendering failure onto both thumbnail surfaces.
    pub fn set_photo_thumbnail_failed(&self, photo_id: PhotoId) {
        if let Some(tile) = self.photo_tiles.borrow().get(&photo_id) {
            tile.thumbnails.set_failed();
        }
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

    /// Installs a generation-checked frame from the color presentation service.
    ///
    /// # Errors
    ///
    /// Returns a texture error when the frame dimensions cannot be represented by GTK.
    pub fn set_darkroom_presentation(
        &self,
        frame: &DisplayPresentationFrame,
    ) -> Result<(), super::PhotoPreviewTextureError> {
        self.darkroom_preview.set_presentation(frame)
    }

    /// Projects a pending/failure/fallback status without touching the source edit.
    pub fn set_darkroom_presentation_status(&self, status: PresentationStatus) {
        self.darkroom_preview.set_presentation_status(status);
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
            external_editor_panel: self.external_editor_panel.clone(),
            photo_tiles: Rc::clone(&self.photo_tiles),
        }
    }
}

#[derive(Clone)]
struct CollectionRefreshHandle {
    controls: CollectionControls,
    toolbar: LighttableToolbar,
    render: WorkspaceRenderHandle,
    lighttable_workspace: Rc<RefCell<Option<PhotoWorkspaceViewModel>>>,
}

impl CollectionRefreshHandle {
    fn apply(&self, state: &CollectionFilterState) {
        self.controls.set_state(state.controls());
        self.toolbar.set_state(state.toolbar());
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
    external_editor_panel: ExternalEditorPanel,
    photo_tiles: Rc<RefCell<BTreeMap<PhotoId, PhotoTilePair>>>,
}

#[derive(Clone)]
struct PhotoTilePair {
    thumbnails: ThumbnailPair,
    lighttable_button: gtk4::Button,
    filmstrip_button: gtk4::Button,
}

#[derive(Clone)]
struct PhotoSelectionContext {
    darkroom_preview: PhotoPreview,
    workspace: gtk4::Stack,
    photo_selected: Rc<RefCell<Option<PhotoSelectedHandler>>>,
    export_panel: ExportPanel,
    external_editor_panel: ExternalEditorPanel,
    photo_tiles: Rc<RefCell<BTreeMap<PhotoId, PhotoTilePair>>>,
}

impl WorkspaceRenderHandle {
    fn render(
        &self,
        view_model: &PhotoWorkspaceViewModel,
        matching_photo_ids: Option<&BTreeSet<PhotoId>>,
    ) {
        clear_children(&self.lighttable);
        clear_children(&self.filmstrip);
        self.photo_tiles.borrow_mut().clear();
        let browser = LibraryBrowserModel::from_workspace(view_model);
        let mut rendered_photos = 0;
        let selection = PhotoSelectionContext {
            darkroom_preview: self.darkroom_preview.clone(),
            workspace: self.workspace.clone(),
            photo_selected: Rc::clone(&self.photo_selected),
            export_panel: self.export_panel.clone(),
            external_editor_panel: self.external_editor_panel.clone(),
            photo_tiles: Rc::clone(&self.photo_tiles),
        };

        for photo in browser.photos() {
            if matching_photo_ids.is_some_and(|ids| !ids.contains(&photo.id())) {
                continue;
            }
            let Some(detail) = view_model.detail(photo.id()) else {
                continue;
            };
            let detail = detail.clone();
            let (card, card_thumbnail) = lighttable_card(
                photo.id(),
                photo.title(),
                photo.secondary(),
                photo.indicators(),
            );
            let (filmstrip_item, filmstrip_thumbnail) = filmstrip_item(photo.id(), photo.title());
            connect_photo_selection(&card, photo.id(), detail.clone(), &selection);
            connect_photo_selection(&filmstrip_item, photo.id(), detail, &selection);
            self.lighttable.insert(&card, -1);
            self.filmstrip.insert(&filmstrip_item, -1);
            self.photo_tiles.borrow_mut().insert(
                photo.id(),
                PhotoTilePair {
                    thumbnails: ThumbnailPair::new(card_thumbnail, filmstrip_thumbnail),
                    lighttable_button: card,
                    filmstrip_button: filmstrip_item,
                },
            );
            rendered_photos += 1;
        }
        self.lighttable_empty_state.set_visible_child_name(
            LighttableContentState::from_rendered_count(rendered_photos).stack_name(),
        );
    }
}

fn connect_photo_selection(
    button: &gtk4::Button,
    photo_id: PhotoId,
    detail: PhotoDetailViewModel,
    context: &PhotoSelectionContext,
) {
    let photo_preview = context.darkroom_preview.clone();
    let workspace = context.workspace.clone();
    let handler = Rc::clone(&context.photo_selected);
    let export_panel = context.export_panel.clone();
    let external_editor_panel = context.external_editor_panel.clone();
    let selected_button = button.clone();
    let photo_tiles = Rc::clone(&context.photo_tiles);
    button.connect_clicked(move |_| {
        for (id, pair) in photo_tiles.borrow().iter() {
            for button in [&pair.lighttable_button, &pair.filmstrip_button] {
                if *id == photo_id {
                    button.add_css_class(ThemeRole::SelectedPhoto.class_name());
                } else {
                    button.remove_css_class(ThemeRole::SelectedPhoto.class_name());
                }
            }
        }
        selected_button.add_css_class(ThemeRole::SelectedPhoto.class_name());
        export_panel.set_selected(true);
        external_editor_panel.set_selection(1);
        show_photo_detail(&photo_preview, &detail);
        workspace.set_visible_child_name(WorkspaceRole::Darkroom.stack_name());
        if let Some(handler) = handler.borrow().as_ref() {
            handler(photo_id);
        }
    });
}

fn lighttable_card(
    photo_id: PhotoId,
    title: &str,
    secondary: Option<&str>,
    indicators: crate::presentation::ThumbnailIndicators,
) -> (gtk4::Button, ThumbnailSurface) {
    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    card.set_margin_top(4);
    card.set_margin_bottom(4);
    card.set_margin_start(4);
    card.set_margin_end(4);
    let thumbnail = ThumbnailSurface::new(
        &format!("photo-thumbnail-{photo_id}"),
        &format!("Thumbnail for {title}"),
        i32::from(THUMBNAIL_METRICS.grid_width_px),
        i32::from(THUMBNAIL_METRICS.grid_height_px),
    );
    apply_theme_role(thumbnail.widget(), ThemeRole::ThumbnailImage);
    let thumbnail_overlay = gtk4::Overlay::new();
    thumbnail_overlay.set_child(Some(thumbnail.widget()));
    let badges = thumbnail_badges(indicators);
    badges.set_halign(gtk4::Align::End);
    badges.set_valign(gtk4::Align::Start);
    thumbnail_overlay.add_overlay(&badges);
    card.append(&thumbnail_overlay);
    let title_label = gtk4::Label::new(Some(title));
    title_label.set_halign(gtk4::Align::Start);
    title_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    title_label.set_max_width_chars(22);
    title_label.set_single_line_mode(true);
    card.append(&title_label);
    if let Some(secondary) = secondary {
        let secondary_label = gtk4::Label::new(Some(secondary));
        secondary_label.set_halign(gtk4::Align::Start);
        secondary_label.add_css_class("dim-label");
        secondary_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        secondary_label.set_max_width_chars(22);
        secondary_label.set_single_line_mode(true);
        card.append(&secondary_label);
    }
    let button = gtk4::Button::new();
    button.set_widget_name(&format!("photo-{photo_id}"));
    apply_theme_role(&button, ThemeRole::PhotoCard);
    button.set_child(Some(&card));
    button.set_tooltip_text(Some(title));
    (button, thumbnail)
}

fn thumbnail_badges(indicators: crate::presentation::ThumbnailIndicators) -> gtk4::Box {
    let badges = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
    badges.set_widget_name("thumbnail-indicators");
    badges.add_css_class("dt_thumbnail_indicators");
    for (visible, text, name) in [
        (indicators.grouped, "G", "grouped photo"),
        (indicators.local_copy, "C", "local copy"),
        (indicators.altered, "●", "altered edit"),
    ] {
        if visible {
            let badge = gtk4::Label::new(Some(text));
            badge.set_tooltip_text(Some(name));
            badges.append(&badge);
        }
    }
    badges
}

fn filmstrip_item(photo_id: PhotoId, title: &str) -> (gtk4::Button, ThumbnailSurface) {
    let thumbnail = ThumbnailSurface::new(
        &format!("filmstrip-thumbnail-{photo_id}"),
        &format!("Filmstrip thumbnail for {title}"),
        i32::from(THUMBNAIL_METRICS.filmstrip_width_px),
        i32::from(THUMBNAIL_METRICS.filmstrip_height_px),
    );
    let button = gtk4::Button::new();
    button.set_widget_name(&format!("filmstrip-photo-{photo_id}"));
    apply_theme_role(&button, ThemeRole::PhotoCard);
    button.add_css_class("dt_filmstrip_item");
    button.set_tooltip_text(Some(title));
    button.set_child(Some(thumbnail.widget()));
    (button, thumbnail)
}

fn show_photo_detail(preview: &PhotoPreview, detail: &PhotoDetailViewModel) {
    preview.set_detail(detail);
}

fn clear_children(container: &impl IsA<gtk4::Widget>) {
    while let Some(child) = container.first_child() {
        child.unparent();
    }
}
