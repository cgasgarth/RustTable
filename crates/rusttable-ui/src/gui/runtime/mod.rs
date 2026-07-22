//! GTK4 realization of Darktable's top-level desktop layout.
//!
//! The structure mirrors the slot model in Darktable's `src/gui/gtk.h`, its
//! lighttable/darkroom view switcher, module-group panel, and filmstrip. It
//! deliberately uses GTK widgets directly instead of a framework adapter.

mod layout;
mod lighttable;
mod preview;
mod selection;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use crate::gui::display_profile::DisplayProfileBanner;
use gtk4::gdk;
use gtk4::prelude::*;
use rusttable_core::{PhotoId, Revision};
use rusttable_i18n::{Direction, I18n, MessageArgs, MessageId};

use self::layout::{
    desktop_body, mode_panel_stack, render_modules, right_panel, synchronize_panel_stacks,
    workspace_stack,
};
use super::{
    CollectionControlAction, CollectionControlState, CollectionControls, CollectionFilterState,
    DARKTABLE_DESKTOP_SPEC, DarkroomPanelVisibility, DarkroomView, DarkroomWorkspaceViewModel,
    ExportPanel, ImportAction, LighttableLayout, LighttableLayoutAction, LighttablePanel,
    LighttablePhotoState, LighttableToolbar, LighttableToolbarAction, LighttableToolbarState,
    LighttableZoom, PhotoPreview, ShellLayout, ThemeRole, WorkspaceRole, apply_theme_role,
};
use super::{
    LighttableInteractionState, LighttableSelectionAction, NavigationDirection, SelectionModifiers,
    header::HeaderChrome, left_panel::LeftPanel,
};
use crate::ai_batch::AiBatchPanel;
use crate::ai_models::AiModelsPanel;
use crate::camera::{CameraAction, CameraPanel, CameraViewModel};
use crate::external_editor::{ExternalEditorAction, ExternalEditorPanel, ExternalEditorViewModel};
use crate::import::{
    ImportDialog, ImportSessionAction, ImportSessionPanel, ImportSessionViewModel,
};
use crate::input_mapping::InputMappingEditor;
use crate::libs::profiles::diagnostics::ProfileDiagnosticRequest;
use crate::presentation::{
    DarkroomHistoryViewModel, DarkroomPanelActionHandler, DarkroomPanelProjection,
    DarkroomPanelTarget, DarkroomSnapshotsViewModel, PhotoDetailViewModel, PhotoWorkspaceViewModel,
};
use crate::viewport_presentation::{
    DisplayPresentationFrame, PresentationStatus, ViewportGeneration,
};

use self::lighttable::{PhotoTilePair, WorkspaceRenderHandle};

pub(super) type PhotoSelectedHandler = Box<dyn Fn(PhotoId, SelectionModifiers)>;

type DarkroomRuntime = (
    DarkroomView,
    PhotoPreview,
    Rc<RefCell<Option<DarkroomWorkspaceViewModel>>>,
);

/// Reusable GTK4 window with Darktable-style lighttable and darkroom modes.
#[derive(Clone)]
pub struct GtkShell {
    window: gtk4::ApplicationWindow,
    layout: ShellLayout,
    workspace: gtk4::Stack,
    lighttable: gtk4::FlowBox,
    lighttable_empty_state: gtk4::Stack,
    pub(crate) darkroom: DarkroomView,
    darkroom_preview: PhotoPreview,
    export_panel: ExportPanel,
    external_editor_panel: ExternalEditorPanel,
    filmstrip: gtk4::FlowBox,
    filmstrip_root: gtk4::Box,
    lighttable_layout_controls: super::LighttableLayoutControls,
    left_panel_stack: gtk4::Stack,
    right_panel_stack: gtk4::Stack,
    left_modules: gtk4::Box,
    right_modules: gtk4::Box,
    darkroom_workspace: Rc<RefCell<Option<DarkroomWorkspaceViewModel>>>,
    import_buttons: Vec<gtk4::Button>,
    collection_controls: CollectionControls,
    lighttable_toolbar: LighttableToolbar,
    input_mapping_editor: InputMappingEditor,
    pub(super) ai_models_panel: AiModelsPanel,
    pub(super) ai_batch_panel: AiBatchPanel,
    camera_panel: CameraPanel,
    import_session_panel: ImportSessionPanel,
    import_dialog: ImportDialog,
    i18n: Rc<RefCell<I18n>>,
    display_profile_banner: DisplayProfileBanner,
    lighttable_workspace: Rc<RefCell<Option<PhotoWorkspaceViewModel>>>,
    lighttable_filter: Rc<RefCell<Option<BTreeSet<PhotoId>>>>,
    lighttable_interaction: Rc<RefCell<LighttableInteractionState>>,
    lighttable_generation: Rc<Cell<u64>>,
    photo_selected: Rc<RefCell<Option<PhotoSelectedHandler>>>,
    photo_tiles: Rc<RefCell<BTreeMap<PhotoId, PhotoTilePair>>>,
    photo_details: Rc<RefCell<BTreeMap<PhotoId, PhotoDetailViewModel>>>,
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

    #[allow(clippy::too_many_lines)]
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
        let import_dialog = ImportDialog::new(&window);
        let panel_width = i32::from(DARKTABLE_DESKTOP_SPEC.layout.side_panel_widths.preferred_px);
        let (darkroom, darkroom_preview, darkroom_workspace) = build_darkroom(panel_width);
        let darkroom_left_modules = darkroom.left_modules().clone();
        let darkroom_right_modules = darkroom.right_modules().clone();
        let (workspace, lighttable, lighttable_empty_state, lighttable_layout_controls) =
            workspace_stack(layout.initial_workspace(), &initial_i18n, darkroom.page());
        let input_mapping_editor = InputMappingEditor::new(application);
        let ai_models_panel = AiModelsPanel::new();
        let display_profile_banner = DisplayProfileBanner::new();
        let header = HeaderChrome::new(&workspace, &initial_i18n, &display_profile_banner);
        initialize_profile_diagnostics(&darkroom);
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
            ai_batch_panel,
            camera_panel,
            import_session_panel,
        ) = right_panel();
        let (left_panel, right_panel) = build_mode_panels(
            &workspace,
            &lighttable_left_panel,
            &lighttable_right_panel,
            &darkroom,
            layout.initial_workspace(),
        );
        let (content, filmstrip, filmstrip_root) =
            desktop_body(&workspace, &left_panel, &right_panel, &initial_i18n);

        let shell = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        apply_theme_role(&shell, ThemeRole::Shell);
        shell.append(header.widget());
        shell.append(&content);
        window.set_child(Some(&shell));

        let shell = Self {
            window,
            layout,
            workspace,
            lighttable,
            lighttable_empty_state,
            darkroom,
            darkroom_preview,
            export_panel,
            external_editor_panel,
            filmstrip,
            filmstrip_root,
            lighttable_layout_controls,
            left_panel_stack: left_panel.clone(),
            right_panel_stack: right_panel.clone(),
            left_modules: darkroom_left_modules,
            right_modules: darkroom_right_modules,
            darkroom_workspace,
            import_buttons: vec![
                header.import_button().clone(),
                lighttable_left_panel.import_button().clone(),
            ],
            collection_controls,
            lighttable_toolbar,
            input_mapping_editor,
            ai_models_panel,
            ai_batch_panel,
            camera_panel,
            import_session_panel,
            import_dialog,
            i18n: Rc::clone(&i18n),
            display_profile_banner,
            lighttable_workspace: Rc::new(RefCell::new(None)),
            lighttable_filter: Rc::new(RefCell::new(None)),
            lighttable_interaction: Rc::new(RefCell::new(LighttableInteractionState::new(6))),
            lighttable_generation: Rc::new(Cell::new(0)),
            photo_selected: Rc::new(RefCell::new(None)),
            photo_tiles: Rc::new(RefCell::new(BTreeMap::new())),
            photo_details: Rc::new(RefCell::new(BTreeMap::new())),
        };
        shell.install_lighttable_keyboard();
        let filmstrip_root = shell.filmstrip_root.clone();
        let interaction = Rc::clone(&shell.lighttable_interaction);
        let left_panel = shell.left_panel_stack.clone();
        let right_panel = shell.right_panel_stack.clone();
        let darkroom = shell.darkroom.clone();
        shell
            .workspace
            .connect_visible_child_name_notify(move |workspace| {
                let darkroom_visible = workspace.visible_child_name().as_deref()
                    == Some(WorkspaceRole::Darkroom.stack_name());
                filmstrip_root.set_visible(if darkroom_visible {
                    darkroom.filmstrip_visible()
                } else {
                    interaction.borrow().layout().shows_filmstrip()
                });
                let state = interaction.borrow();
                left_panel.set_visible(if darkroom_visible {
                    darkroom.left_panel_visible()
                } else {
                    state.left_panel_visible()
                });
                right_panel.set_visible(if darkroom_visible {
                    darkroom.right_panel_visible()
                } else {
                    state.right_panel_visible()
                });
            });
        let workspace = shell.workspace.clone();
        let left_panel = shell.left_panel_stack.clone();
        let right_panel = shell.right_panel_stack.clone();
        let filmstrip_root = shell.filmstrip_root.clone();
        shell.darkroom.connect_panel_visibility(move |action| {
            if workspace.visible_child_name().as_deref()
                != Some(WorkspaceRole::Darkroom.stack_name())
            {
                return;
            }
            match action.panel() {
                DarkroomPanelVisibility::Left => left_panel.set_visible(action.visible()),
                DarkroomPanelVisibility::Right => right_panel.set_visible(action.visible()),
                DarkroomPanelVisibility::Filmstrip => filmstrip_root.set_visible(action.visible()),
            }
        });
        let darkroom_status = shell.darkroom.clone();
        shell
            .export_panel
            .connect_status(move |status| darkroom_status.set_background_job_status(status));
        let shell_for_layout = shell.clone();
        shell
            .lighttable_layout_controls
            .connect_action(move |action| match action {
                LighttableLayoutAction::SetLayout(layout) => {
                    shell_for_layout.set_lighttable_layout(layout);
                }
                LighttableLayoutAction::SetPanelVisibility { panel, visible } => {
                    shell_for_layout.set_lighttable_panel_visibility(panel, visible);
                }
            });
        shell.left_panel_stack.set_visible(true);
        shell.right_panel_stack.set_visible(true);
        shell
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

    /// Starts a generation-tagged darkroom selection before its worker-rendered preview arrives.
    pub fn begin_darkroom_selection(&self, photo_id: PhotoId, generation: ViewportGeneration) {
        self.darkroom
            .set_viewport_selection(photo_id, Revision::ZERO, generation);
    }

    #[must_use]
    pub fn darkroom_panel_target(&self) -> Option<DarkroomPanelTarget> {
        let viewport = self.darkroom.viewport_state();
        viewport.photo_id().map(|photo_id| {
            DarkroomPanelTarget::new(
                photo_id,
                viewport.generation(),
                viewport.edit_revision().unwrap_or(Revision::ZERO),
            )
        })
    }

    /// Projects a controller-owned selected-photo operation stack into GTK.
    pub fn set_darkroom_module_stack(
        &self,
        modules: &super::DarkroomModulesViewModel,
        action_handler: Option<super::DarkroomModuleActionHandler>,
    ) {
        self.darkroom.set_module_stack(modules, action_handler);
    }

    /// Projects a controller-owned darkroom status or typed error.
    pub fn set_darkroom_status(&self, text: &str) {
        self.darkroom.set_status(text);
    }

    pub fn set_history_projection(
        &self,
        projection: &DarkroomPanelProjection<DarkroomHistoryViewModel>,
        handler: Option<DarkroomPanelActionHandler>,
    ) {
        self.darkroom.set_history_projection(projection, handler);
    }

    pub fn set_snapshots_projection(
        &self,
        projection: &DarkroomPanelProjection<DarkroomSnapshotsViewModel>,
        handler: Option<DarkroomPanelActionHandler>,
    ) {
        self.darkroom.set_snapshots_projection(projection, handler);
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
        if state.controls().generation() < self.lighttable_generation.get() {
            return;
        }
        self.lighttable_generation
            .set(state.controls().generation());
        self.collection_controls.set_state(state.controls());
        self.lighttable_toolbar.set_state(state.toolbar());
        self.lighttable_interaction
            .borrow_mut()
            .reconcile_selection(
                state
                    .matching_photo_ids()
                    .iter()
                    .copied()
                    .filter(|photo_id| {
                        state
                            .photo_state(*photo_id)
                            .is_some_and(LighttablePhotoState::selected)
                    }),
            );
        self.lighttable_filter.replace(Some(
            state
                .matching_photo_ids()
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
        ));
        self.refresh_lighttable(state.matching_photo_ids());
    }

    /// Changes the lighttable density and rerenders the active collection.
    pub fn set_lighttable_zoom(&self, zoom: LighttableZoom) {
        let _ = self
            .lighttable_interaction
            .borrow_mut()
            .apply(LighttableSelectionAction::SetZoom(zoom));
        let workspace = self.lighttable_workspace.borrow();
        let Some(view_model) = workspace.as_ref() else {
            return;
        };
        let filter = self.lighttable_filter.borrow();
        self.workspace_render_handle()
            .render(view_model, filter.as_ref());
    }

    /// Switches the visible lighttable surface and reconciles its culling set.
    pub fn set_lighttable_layout(&self, layout: LighttableLayout) {
        let _ = self
            .lighttable_interaction
            .borrow_mut()
            .apply(LighttableSelectionAction::SetLayout(layout));
        self.lighttable_layout_controls.set_layout(layout);
        let workspace = self.lighttable_workspace.borrow();
        let Some(view_model) = workspace.as_ref() else {
            self.filmstrip_root.set_visible(layout.shows_filmstrip());
            self.sync_lighttable_panels();
            return;
        };
        let filter = self.lighttable_filter.borrow();
        self.workspace_render_handle()
            .render(view_model, filter.as_ref());
        self.sync_lighttable_panels();
    }

    /// Applies a Darktable-style side-panel toggle without changing selection or layout.
    pub fn set_lighttable_panel_visibility(&self, panel: LighttablePanel, visible: bool) {
        let action = match panel {
            LighttablePanel::Left => LighttableSelectionAction::SetLeftPanelVisible(visible),
            LighttablePanel::Right => LighttableSelectionAction::SetRightPanelVisible(visible),
        };
        let _ = self.lighttable_interaction.borrow_mut().apply(action);
        self.lighttable_layout_controls
            .set_panel_visibility(panel, visible);
        self.sync_lighttable_panels();
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
            generation: self.lighttable_generation.clone(),
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
            generation: self.lighttable_generation.clone(),
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
        self.import_dialog.connect_action(callback);
        let dialog = self.import_dialog.clone();
        for button in &self.import_buttons {
            let dialog = dialog.clone();
            button.connect_clicked(move |_| dialog.present());
        }
    }

    /// Projects the catalog-owned source identity into the import row model.
    pub fn set_import_existing_paths(&self, paths: impl IntoIterator<Item = std::path::PathBuf>) {
        self.import_dialog.set_existing_paths(paths);
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
        F: Fn(PhotoId, SelectionModifiers) + 'static,
    {
        self.photo_selected.replace(Some(Box::new(handler)));
    }

    /// Renders product presentation data into the GTK lighttable and filmstrip.
    ///
    /// The title and secondary information come directly from the typed Rust
    /// view model. Each native card switches to darkroom and reports the typed
    /// [`PhotoId`] through [`Self::set_photo_selected_handler`].
    pub fn set_lighttable_workspace(&self, view_model: &PhotoWorkspaceViewModel) {
        self.lighttable_generation.set(0);
        self.lighttable_filter.replace(None);
        self.lighttable_workspace.replace(Some(view_model.clone()));
        self.workspace_render_handle().render(view_model, None);
    }

    /// Renders only the photos selected by the active collection rule.
    pub fn set_lighttable_workspace_filtered(
        &self,
        view_model: &PhotoWorkspaceViewModel,
        matching_photo_ids: impl IntoIterator<Item = PhotoId>,
    ) {
        self.lighttable_generation.set(0);
        let matching_photo_ids = matching_photo_ids.into_iter().collect::<BTreeSet<_>>();
        self.lighttable_filter
            .replace(Some(matching_photo_ids.clone()));
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
        self.darkroom.set_detail(view_model.detail());
        self.darkroom.set_status(&format!(
            "selected · {}",
            view_model.detail().title().as_str()
        ));
        if let Some(projection) = view_model.snapshots_projection() {
            self.darkroom.set_snapshots_projection(projection, None);
        }
        if let Some(projection) = view_model.history_projection() {
            self.darkroom.set_history_projection(projection, None);
        }
        self.darkroom_workspace.replace(Some(view_model.clone()));
        self.darkroom_preview.set_detail(view_model.detail());
        render_modules(&self.left_modules, view_model.left_modules(), None);
        render_modules(
            &self.right_modules,
            view_model.right_modules(),
            Some(self.darkroom.module_group_state().get()),
        );
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
        let result = self.darkroom_preview.set_presentation(frame);
        self.darkroom.set_status(&frame.status().label());
        result
    }

    /// Projects a pending/failure/fallback status without touching the source edit.
    pub fn set_darkroom_presentation_status(&self, status: PresentationStatus) {
        self.darkroom_preview.set_presentation_status(status);
        self.darkroom.set_status(&status.label());
    }

    /// Switches the central workspace without starting or owning a GTK loop.
    pub fn show_workspace(&self, role: WorkspaceRole) {
        self.workspace.set_visible_child_name(role.stack_name());
        self.sync_lighttable_panels();
    }

    fn sync_lighttable_panels(&self) {
        let darkroom_visible = self.workspace.visible_child_name().as_deref()
            == Some(WorkspaceRole::Darkroom.stack_name());
        let state = self.lighttable_interaction.borrow();
        self.left_panel_stack.set_visible(if darkroom_visible {
            self.darkroom.left_panel_visible()
        } else {
            state.left_panel_visible()
        });
        self.right_panel_stack.set_visible(if darkroom_visible {
            self.darkroom.right_panel_visible()
        } else {
            state.right_panel_visible()
        });
        self.filmstrip_root.set_visible(if darkroom_visible {
            self.darkroom.filmstrip_visible()
        } else {
            state.layout().shows_filmstrip()
        });
    }

    pub(super) fn workspace_render_handle(&self) -> WorkspaceRenderHandle {
        WorkspaceRenderHandle {
            lighttable: self.lighttable.clone(),
            lighttable_empty_state: self.lighttable_empty_state.clone(),
            filmstrip: self.filmstrip.clone(),
            filmstrip_root: self.filmstrip_root.clone(),
            darkroom_preview: self.darkroom_preview.clone(),
            darkroom: self.darkroom.clone(),
            workspace: self.workspace.clone(),
            photo_selected: Rc::clone(&self.photo_selected),
            export_panel: self.export_panel.clone(),
            external_editor_panel: self.external_editor_panel.clone(),
            photo_tiles: Rc::clone(&self.photo_tiles),
            interaction: Rc::clone(&self.lighttable_interaction),
            photo_details: Rc::clone(&self.photo_details),
            lighttable_workspace: Rc::clone(&self.lighttable_workspace),
            lighttable_filter: Rc::clone(&self.lighttable_filter),
        }
    }

    fn install_lighttable_keyboard(&self) {
        let controller = gtk4::EventControllerKey::new();
        controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let render = self.workspace_render_handle();
        controller.connect_key_pressed(move |_, key, _, modifiers| {
            handle_lighttable_key(&render, key, modifiers)
        });
        self.lighttable.add_controller(controller);
        let controller = gtk4::EventControllerKey::new();
        controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let render = self.workspace_render_handle();
        controller.connect_key_pressed(move |_, key, _, modifiers| {
            handle_lighttable_key(&render, key, modifiers)
        });
        self.filmstrip.add_controller(controller);
    }
}

fn handle_lighttable_key(
    render: &WorkspaceRenderHandle,
    key: gdk::Key,
    modifiers: gdk::ModifierType,
) -> gtk4::glib::Propagation {
    let modifiers = SelectionModifiers::new(
        modifiers.contains(gdk::ModifierType::CONTROL_MASK)
            || modifiers.contains(gdk::ModifierType::SUPER_MASK),
        modifiers.contains(gdk::ModifierType::SHIFT_MASK),
    );
    let action = match key {
        gdk::Key::Left => Some(LighttableSelectionAction::Move {
            direction: NavigationDirection::Previous,
            modifiers,
        }),
        gdk::Key::Right => Some(LighttableSelectionAction::Move {
            direction: NavigationDirection::Next,
            modifiers,
        }),
        gdk::Key::Up => Some(LighttableSelectionAction::Move {
            direction: NavigationDirection::RowPrevious,
            modifiers,
        }),
        gdk::Key::Down => Some(LighttableSelectionAction::Move {
            direction: NavigationDirection::RowNext,
            modifiers,
        }),
        gdk::Key::Escape => Some(LighttableSelectionAction::Clear),
        gdk::Key::Return | gdk::Key::KP_Enter => {
            render.open_focused();
            return gtk4::glib::Propagation::Stop;
        }
        gdk::Key::minus | gdk::Key::KP_Subtract => Some(LighttableSelectionAction::SetZoom(
            render.interaction.borrow().zoom().smaller(),
        )),
        gdk::Key::plus | gdk::Key::KP_Add => Some(LighttableSelectionAction::SetZoom(
            render.interaction.borrow().zoom().larger(),
        )),
        _ => None,
    };
    let Some(action) = action else {
        return gtk4::glib::Propagation::Proceed;
    };
    let zoom_changed = matches!(action, LighttableSelectionAction::SetZoom(_));
    let selected = match action {
        LighttableSelectionAction::Move {
            direction,
            modifiers,
        } => render.move_focus(direction, modifiers),
        action => render.interaction.borrow_mut().apply(action),
    };
    if zoom_changed {
        render.rerender_current();
    } else {
        render.sync_selection_styles();
        if selected.is_some() {
            render.focus_selected();
        }
    }
    gtk4::glib::Propagation::Stop
}

fn build_mode_panels(
    workspace: &gtk4::Stack,
    lighttable_left_panel: &LeftPanel,
    lighttable_right_panel: &gtk4::Box,
    darkroom: &DarkroomView,
    initial_workspace: WorkspaceRole,
) -> (gtk4::Stack, gtk4::Stack) {
    let left_panel = mode_panel_stack(
        "left-panel-stack",
        lighttable_left_panel.widget(),
        darkroom.left_panel(),
        initial_workspace,
    );
    let right_panel = mode_panel_stack(
        "right-panel-stack",
        lighttable_right_panel,
        darkroom.right_panel(),
        initial_workspace,
    );
    synchronize_panel_stacks(workspace, &left_panel, &right_panel);
    (left_panel, right_panel)
}

fn build_darkroom(panel_width: i32) -> DarkroomRuntime {
    let darkroom = DarkroomView::new(panel_width);
    let darkroom_preview = darkroom.preview().clone();
    let darkroom_workspace = Rc::new(RefCell::new(None::<DarkroomWorkspaceViewModel>));
    connect_darkroom_module_group(&darkroom, &darkroom_workspace);
    (darkroom, darkroom_preview, darkroom_workspace)
}

fn initialize_profile_diagnostics(darkroom: &DarkroomView) {
    darkroom.set_profile_diagnostic_state(None, None, ProfileDiagnosticRequest::new());
}

fn connect_darkroom_module_group(
    darkroom: &DarkroomView,
    workspace: &Rc<RefCell<Option<DarkroomWorkspaceViewModel>>>,
) {
    let right_modules = darkroom.right_modules().clone();
    let workspace = Rc::clone(workspace);
    darkroom.connect_module_group(move |group| {
        if let Some(view_model) = workspace.borrow().as_ref() {
            render_modules(&right_modules, view_model.right_modules(), Some(group));
        }
    });
}

#[cfg(test)]
mod tests;

#[derive(Clone)]
struct CollectionRefreshHandle {
    controls: CollectionControls,
    toolbar: LighttableToolbar,
    render: WorkspaceRenderHandle,
    lighttable_workspace: Rc<RefCell<Option<PhotoWorkspaceViewModel>>>,
    generation: Rc<Cell<u64>>,
}

impl CollectionRefreshHandle {
    fn apply(&self, state: &CollectionFilterState) {
        if state.controls().generation() < self.generation.get() {
            return;
        }
        self.generation.set(state.controls().generation());
        self.controls.set_state(state.controls());
        self.toolbar.set_state(state.toolbar());
        self.render.lighttable_filter.replace(Some(
            state
                .matching_photo_ids()
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
        ));
        let workspace = self.lighttable_workspace.borrow();
        let Some(view_model) = workspace.as_ref() else {
            return;
        };
        let matching_photo_ids = state
            .matching_photo_ids()
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        self.render.interaction.borrow_mut().reconcile_selection(
            state
                .matching_photo_ids()
                .iter()
                .copied()
                .filter(|photo_id| {
                    state
                        .photo_state(*photo_id)
                        .is_some_and(LighttablePhotoState::selected)
                }),
        );
        self.render.render(view_model, Some(&matching_photo_ids));
    }
}
