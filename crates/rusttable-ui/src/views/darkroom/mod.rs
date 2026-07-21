//! Darktable-shaped GTK4 darkroom composition.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::prelude::*;
use rusttable_core::{PhotoId, Revision};
use rusttable_display_profile::{DisplayProfileReceipt, DisplayProfileSnapshot};

use crate::iop::modules::{
    DarkroomModuleActionHandler, DarkroomModuleGroup, DarkroomModulesViewModel, reference_modules,
};
mod interaction;
mod panel_widgets;
pub(crate) mod status;
mod surfaces;
mod viewport;
pub(super) use crate::gui::{DARKROOM_GEOMETRY, ThemeRole, apply_theme_role};
use crate::iop::exposure::ExposurePanel;
use crate::libs::histogram::{HistogramData, HistogramError, HistogramSample};
use crate::presentation::{
    DarkroomControlValue, DarkroomHistoryViewModel, DarkroomPanelActionHandler,
    DarkroomPanelProjection, DarkroomPanelTarget, DarkroomSnapshotsViewModel, PhotoDetailViewModel,
};
use crate::raw_denoise::{RawDenoiseAction, RawDenoisePanel, RawDenoiseViewModel};
use crate::rgb_denoise::{RgbDenoiseAction, RgbDenoisePanel, RgbDenoiseViewModel};
use crate::viewport_presentation::{
    DarkroomViewportCommand, DarkroomViewportState, ViewportGeneration,
};
use crate::widgets::preview::PhotoPreview;
use crate::{MaskManagerAction, MaskManagerPanel, MaskManagerSnapshot};
use crate::{MultiscaleRetouchAction, MultiscaleRetouchPanel, MultiscaleRetouchSnapshot};
use interaction::{
    FilmstripState, HistogramView, connect_filmstrip_button, install_filmstrip_keyboard,
    sync_filmstrip_buttons,
};
use panel_widgets::{left_panel, render_typed_modules_into, right_panel};
use status::DarkroomStatusSurface;
pub(super) use viewport::chrome_toggle;
use viewport::{ViewportControls, darkroom_page, sync_viewport_controls};

use crate::libs::profiles::diagnostics::{
    ProfileDiagnosticRequest, ProfileDiagnosticSurface, project_profile_diagnostic,
};

/// Stable widget identifiers for the initial darkroom surface.
pub const DARKROOM_WIDGET_IDS: [&str; 22] = [
    "darkroom-page",
    "darkroom-toolbar-top",
    "darkroom-photo-preview",
    "darkroom-toolbar-bottom",
    "darkroom-left-panel",
    "darkroom-navigation",
    "darkroom-snapshots",
    "darkroom-history",
    "darkroom-image-information",
    "darkroom-right-panel",
    "darkroom-histogram",
    "darkroom-module-groups",
    "darkroom-module-search",
    "darkroom-left-module-scroll",
    "darkroom-right-module-scroll",
    "darkroom-right-modules",
    "exposure",
    "darkroom-left-panel-toggle",
    "darkroom-right-panel-toggle",
    "darkroom-filmstrip-toggle",
    "darkroom-status-bar",
    "darkroom-job-status",
];

/// Focus order for layout controls that collapse darkroom regions in place.
pub const DARKROOM_LAYOUT_FOCUS_ORDER: [&str; 3] = [
    "darkroom-left-panel-toggle",
    "darkroom-right-panel-toggle",
    "darkroom-filmstrip-toggle",
];

/// Side and bottom regions that can be collapsed without changing the selected edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DarkroomPanelVisibility {
    Left,
    Right,
    Filmstrip,
}

/// Typed layout intent emitted by the darkroom chrome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DarkroomPanelVisibilityAction {
    panel: DarkroomPanelVisibility,
    visible: bool,
}

impl DarkroomPanelVisibilityAction {
    #[must_use]
    pub const fn new(panel: DarkroomPanelVisibility, visible: bool) -> Self {
        Self { panel, visible }
    }

    #[must_use]
    pub const fn panel(self) -> DarkroomPanelVisibility {
        self.panel
    }

    #[must_use]
    pub const fn visible(self) -> bool {
        self.visible
    }
}

/// Stable left-to-right focus order for the darkroom rail controls.
pub const DARKROOM_RAIL_FOCUS_ORDER: [&str; 15] = [
    "darkroom-navigation",
    "darkroom-snapshots",
    "darkroom-history",
    "darkroom-image-information",
    "darkroom-module-search",
    "group-active",
    "group-favorites",
    "group-basic",
    "group-tone",
    "group-color",
    "group-correct",
    "group-effects",
    "group-grading",
    "group-technical",
    "group-deprecated",
];

/// Stable identifiers for the searchable, grouped module-stack controls.
pub const DARKROOM_MODULE_WIDGET_IDS: [&str; 8] = [
    "darkroom-module-search",
    "group-active",
    "group-favorites",
    "group-technical",
    "group-grading",
    "group-deprecated",
    "exposure-presets",
    "exposure-reset",
];

type DarkroomModuleGroupHandler = Box<dyn Fn(DarkroomModuleGroup)>;
pub(super) type DarkroomPanelVisibilityHandler = Box<dyn Fn(DarkroomPanelVisibilityAction)>;
type DarkroomFilmstripHandler = Box<dyn Fn(PhotoId, ViewportGeneration)>;

/// Stable identifiers for the darkroom viewport controls and filmstrip boundary.
pub const DARKROOM_VIEWPORT_WIDGET_IDS: [&str; 14] = [
    "darkroom-viewport",
    "darkroom-soft-proof",
    "darkroom-gamut-check",
    "darkroom-zoom",
    "darkroom-fit",
    "darkroom-before-after",
    "darkroom-viewport-projection",
    "darkroom-viewport-overlay",
    "darkroom-overlay-before",
    "darkroom-overlay-soft-proof",
    "darkroom-overlay-gamut",
    "darkroom-overlay-histogram-sample",
    "darkroom-filmstrip-boundary",
    "darkroom-image-canvas",
];

/// Focus order for all controls introduced by the darkroom viewport batch.
pub const DARKROOM_VIEWPORT_FOCUS_ORDER: [&str; 5] = [
    "darkroom-soft-proof",
    "darkroom-gamut-check",
    "darkroom-zoom",
    "darkroom-fit",
    "darkroom-before-after",
];

/// Application-owned receiver for viewport commands. The orchestrator supplies the renderer or
/// controller; the GTK view only emits typed, generation-tagged intent.
pub type DarkroomViewportActionHandler = Box<dyn Fn(DarkroomViewportCommand)>;

/// Native GTK widgets owned by the darkroom view.
#[derive(Clone)]
pub struct DarkroomView {
    page: gtk4::Box,
    preview: PhotoPreview,
    viewport_state: Rc<RefCell<DarkroomViewportState>>,
    viewport_controls: ViewportControls,
    viewport_handler: Rc<RefCell<Option<DarkroomViewportActionHandler>>>,
    left_panel: gtk4::Box,
    left_modules: gtk4::Box,
    right_panel: gtk4::Box,
    right_modules: gtk4::Box,
    exposure: ExposurePanel,
    rgb_denoise: RgbDenoisePanel,
    raw_denoise: RawDenoisePanel,
    mask_manager: MaskManagerPanel,
    multiscale_retouch: MultiscaleRetouchPanel,
    rail_status: DarkroomRailStatus,
    histogram: HistogramView,
    histogram_generation: Rc<Cell<Option<ViewportGeneration>>>,
    module_search: gtk4::SearchEntry,
    module_group: Rc<Cell<DarkroomModuleGroup>>,
    module_group_handler: Rc<RefCell<Option<DarkroomModuleGroupHandler>>>,
    typed_modules: Rc<RefCell<Option<DarkroomModulesViewModel>>>,
    module_action_handler: Rc<RefCell<Option<DarkroomModuleActionHandler>>>,
    filmstrip_state: Rc<RefCell<FilmstripState>>,
    filmstrip_handler: Rc<RefCell<Option<DarkroomFilmstripHandler>>>,
    filmstrip_widget: Rc<RefCell<Option<gtk4::FlowBox>>>,
    profile_diagnostic: ProfileDiagnosticSurface,
    status_surface: DarkroomStatusSurface,
    left_panel_visible: Rc<Cell<bool>>,
    right_panel_visible: Rc<Cell<bool>>,
    filmstrip_visible: Rc<Cell<bool>>,
    panel_visibility_handler: Rc<RefCell<Option<DarkroomPanelVisibilityHandler>>>,
}

impl DarkroomView {
    /// Builds the initial Darktable darkroom around the immutable preview boundary.
    #[must_use]
    pub fn new(panel_width: i32) -> Self {
        debug_assert_eq!(DARKROOM_RAIL_FOCUS_ORDER.len(), 15);
        debug_assert_eq!(DARKROOM_MODULE_WIDGET_IDS.len(), 8);
        let preview = PhotoPreview::new();
        let viewport_state = Rc::new(RefCell::new(DarkroomViewportState::default()));
        let viewport_handler = Rc::new(RefCell::new(None));
        let left_panel_visible = Rc::new(Cell::new(true));
        let right_panel_visible = Rc::new(Cell::new(true));
        let filmstrip_visible = Rc::new(Cell::new(true));
        let panel_visibility_handler = Rc::new(RefCell::new(None));
        let (page, viewport_controls, status_surface) = darkroom_page(
            &preview,
            &viewport_state,
            &viewport_handler,
            &left_panel_visible,
            &right_panel_visible,
            &filmstrip_visible,
            &panel_visibility_handler,
        );
        let profile_diagnostic =
            ProfileDiagnosticSurface::new("darkroom-profile-diagnostic", "Darkroom profile status");
        status_surface.append(profile_diagnostic.widget());
        let (left_panel, left_modules, rail_status) = left_panel(panel_width);
        let (
            right_panel,
            right_modules,
            exposure,
            rgb_denoise,
            raw_denoise,
            mask_manager,
            multiscale_retouch,
            histogram,
            module_search,
            module_group,
            module_group_handler,
        ) = right_panel(panel_width);
        let histogram = HistogramView::new(histogram);
        let histogram_generation = Rc::new(Cell::new(None));
        let typed_modules = Rc::new(RefCell::new(reference_modules().ok()));
        let module_action_handler = Rc::new(RefCell::new(None));
        let filmstrip_state = Rc::new(RefCell::new(FilmstripState::default()));
        let filmstrip_handler = Rc::new(RefCell::new(None));
        let filmstrip_widget = Rc::new(RefCell::new(None));
        let view = Self {
            page,
            preview,
            viewport_state,
            viewport_controls,
            viewport_handler,
            left_panel,
            left_modules,
            right_panel,
            right_modules,
            exposure,
            rgb_denoise,
            raw_denoise,
            mask_manager,
            multiscale_retouch,
            rail_status,
            histogram,
            histogram_generation,
            module_search,
            module_group,
            module_group_handler,
            typed_modules,
            module_action_handler,
            filmstrip_state,
            filmstrip_handler,
            filmstrip_widget,
            profile_diagnostic,
            status_surface,
            left_panel_visible,
            right_panel_visible,
            filmstrip_visible,
            panel_visibility_handler,
        };
        let overlay_controls = view.viewport_controls.clone();
        view.histogram.connect_sample(move |sample| {
            overlay_controls.set_histogram_sample(sample);
        });
        view.install_module_search();
        view.render_typed_modules();
        view
    }

    #[must_use]
    pub fn page(&self) -> &gtk4::Box {
        &self.page
    }

    #[must_use]
    pub fn left_panel_visible(&self) -> bool {
        self.left_panel_visible.get()
    }

    #[must_use]
    pub fn right_panel_visible(&self) -> bool {
        self.right_panel_visible.get()
    }

    #[must_use]
    pub fn filmstrip_visible(&self) -> bool {
        self.filmstrip_visible.get()
    }

    /// Connects darkroom-local panel toggles to the shell's layout owner.
    pub fn connect_panel_visibility<F>(&self, handler: F)
    where
        F: Fn(DarkroomPanelVisibilityAction) + 'static,
    {
        self.panel_visibility_handler
            .replace(Some(Box::new(handler)));
    }

    /// Projects the selected edit into the darkroom status row.
    pub fn set_status(&self, text: &str) {
        self.status_surface.set_status(text);
    }

    /// Projects an existing export/background-job status without owning export work.
    pub fn set_background_job_status(&self, text: &str) {
        self.status_surface.set_job_status(text);
    }

    #[must_use]
    pub fn preview(&self) -> &PhotoPreview {
        &self.preview
    }

    /// Returns the current display-free viewport state.
    #[must_use]
    pub fn viewport_state(&self) -> DarkroomViewportState {
        *self.viewport_state.borrow()
    }

    /// Starts a new generation for the selected catalog photo/edit projection.
    pub fn set_viewport_selection(
        &self,
        photo_id: PhotoId,
        edit_revision: Revision,
        generation: ViewportGeneration,
    ) {
        self.viewport_state
            .borrow_mut()
            .select(photo_id, edit_revision, generation);
        self.filmstrip_state.borrow_mut().set_generation(generation);
        self.histogram_generation.set(Some(generation));
        self.histogram.loading(generation);
        self.viewport_controls.clear_histogram_sample();
        self.sync_viewport_projection();
    }

    /// Restores truthful no-photo state and resets transient viewport controls.
    pub fn clear_viewport_selection(&self) {
        self.viewport_state.borrow_mut().clear_selection();
        self.filmstrip_state.borrow_mut().clear_selection();
        self.histogram_generation.set(None);
        self.histogram.clear();
        self.viewport_controls.clear_histogram_sample();
        self.sync_viewport_projection();
    }

    /// Connects typed toolbar and navigation commands to the application orchestrator.
    pub fn connect_viewport_action<F>(&self, handler: F)
    where
        F: Fn(DarkroomViewportCommand) + 'static,
    {
        self.viewport_handler.replace(Some(Box::new(handler)));
    }

    /// Reapplies the current projection after the orchestrator installs a new texture.
    pub fn sync_viewport_projection(&self) {
        sync_viewport_controls(&self.viewport_controls, &self.preview, &self.viewport_state);
    }

    /// Projects the same typed profile decision used by the header into the darkroom status row.
    pub(crate) fn set_profile_diagnostic_state(
        &self,
        snapshot: Option<&DisplayProfileSnapshot>,
        receipt: Option<DisplayProfileReceipt>,
        request: ProfileDiagnosticRequest,
    ) {
        let projection = project_profile_diagnostic(snapshot, receipt, request);
        self.profile_diagnostic.set_projection(&projection);
    }

    /// Publishes validated RGB histogram bins without coupling GTK to the pixelpipe.
    ///
    /// Returns `false` for malformed, non-finite, negative, mismatched, or oversized input and
    /// leaves the visible surface in the explicit unavailable state.
    #[must_use]
    pub fn set_histogram(&self, red: &[f32], green: &[f32], blue: &[f32]) -> bool {
        let state = self.viewport_state.borrow();
        let Some(generation) = state.photo_id().map(|_| state.generation()) else {
            self.histogram.clear();
            return false;
        };
        self.set_histogram_for_generation(red, green, blue, generation)
    }

    /// Publishes histogram bins only for the currently selected viewport generation.
    #[must_use]
    pub fn set_histogram_for_generation(
        &self,
        red: &[f32],
        green: &[f32],
        blue: &[f32],
        generation: ViewportGeneration,
    ) -> bool {
        if self.histogram_generation.get() != Some(generation) {
            self.histogram
                .stale(self.viewport_state.borrow().generation(), generation);
            return false;
        }
        let Some(data) = HistogramData::from_rgb_bin_values(red, green, blue) else {
            self.histogram.failure(
                generation,
                HistogramError::IncorrectSampleLength {
                    expected: red.len(),
                    actual: green.len().max(blue.len()),
                },
            );
            return false;
        };
        self.histogram.set_data(generation, data);
        true
    }

    /// Publishes the worker-computed histogram result for the selected preview generation.
    #[must_use]
    pub fn set_histogram_result(
        &self,
        generation: ViewportGeneration,
        result: Result<HistogramData, HistogramError>,
    ) -> bool {
        if self.histogram_generation.get() != Some(generation) {
            self.histogram
                .stale(self.viewport_state.borrow().generation(), generation);
            return false;
        }
        match result {
            Ok(data) => self.histogram.set_data(generation, data),
            Err(error) => self.histogram.failure(generation, error),
        }
        true
    }

    /// Returns the histogram sample currently selected by a click in the right rail.
    #[must_use]
    pub fn histogram_sample(&self) -> Option<HistogramSample> {
        self.histogram.selected_sample()
    }

    /// Returns whether a rendered histogram is currently available for the selected preview.
    #[must_use]
    pub fn histogram_available(&self) -> bool {
        self.histogram.is_ready()
    }

    /// Reconciles the filmstrip order and selected photo with a new viewport generation.
    pub fn set_filmstrip_items(
        &self,
        photo_ids: impl IntoIterator<Item = PhotoId>,
        selected: Option<PhotoId>,
        generation: ViewportGeneration,
    ) {
        self.filmstrip_state
            .borrow_mut()
            .set_items(photo_ids, selected, generation);
        self.sync_filmstrip_selection();
    }

    /// Returns the selected filmstrip photo, if the selection is still in the ordered strip.
    #[must_use]
    pub fn filmstrip_selection(&self) -> Option<PhotoId> {
        self.filmstrip_state.borrow().selected()
    }

    /// Connects filmstrip selection to the application-owned photo/detail controller.
    pub fn connect_filmstrip_selection<F>(&self, handler: F)
    where
        F: Fn(PhotoId, ViewportGeneration) + 'static,
    {
        self.filmstrip_handler.replace(Some(Box::new(handler)));
    }

    /// Attaches darkroom keyboard and click routing to the shell-owned filmstrip `FlowBox`.
    ///
    /// The existing filmstrip remains the single visual owner. This method only adds a
    /// generation-tagged darkroom selection boundary and synchronizes its selected styling.
    pub fn install_filmstrip_interaction(&self, filmstrip: &gtk4::FlowBox) {
        self.filmstrip_widget.replace(Some(filmstrip.clone()));
        let current = self.filmstrip_state.borrow();
        let selected = current.selected();
        let generation = current.generation();
        drop(current);
        self.filmstrip_state.borrow_mut().set_items(
            interaction::filmstrip_ids(filmstrip),
            selected,
            generation,
        );
        install_filmstrip_keyboard(
            filmstrip,
            &self.filmstrip_state,
            &self.internal_filmstrip_handler(),
        );
        let handler = self.internal_filmstrip_handler();
        let buttons = interaction::filmstrip_buttons(filmstrip);
        for (photo_id, button) in buttons {
            connect_filmstrip_button(
                &button,
                photo_id,
                filmstrip,
                &self.filmstrip_state,
                &handler,
            );
        }
        self.sync_filmstrip_selection();
    }

    fn internal_filmstrip_handler(&self) -> interaction::FilmstripHandler {
        let handler = Rc::clone(&self.filmstrip_handler);
        let filmstrip = Rc::clone(&self.filmstrip_widget);
        Rc::new(RefCell::new(Some(Box::new(move |selection| {
            if let Some(filmstrip) = filmstrip.borrow().as_ref() {
                sync_filmstrip_buttons(filmstrip, Some(selection.photo_id));
            }
            if let Some(handler) = handler.borrow().as_ref() {
                handler(selection.photo_id, selection.generation);
            }
        }))))
    }

    fn sync_filmstrip_selection(&self) {
        if let Some(filmstrip) = self.filmstrip_widget.borrow().as_ref() {
            sync_filmstrip_buttons(filmstrip, self.filmstrip_selection());
        }
    }

    #[must_use]
    pub fn left_panel(&self) -> &gtk4::Box {
        &self.left_panel
    }

    #[must_use]
    pub fn left_modules(&self) -> &gtk4::Box {
        &self.left_modules
    }

    #[must_use]
    pub fn right_panel(&self) -> &gtk4::Box {
        &self.right_panel
    }

    #[must_use]
    pub fn right_modules(&self) -> &gtk4::Box {
        &self.right_modules
    }

    /// Installs a controller-owned typed module stack in both side rails.
    ///
    /// The snapshot is copied so a later controller update cannot invalidate
    /// GTK callbacks. Each callback still carries the module revision that
    /// created its widget.
    pub fn set_module_stack(
        &self,
        modules: &DarkroomModulesViewModel,
        action_handler: Option<DarkroomModuleActionHandler>,
    ) {
        self.typed_modules.replace(Some(modules.clone()));
        if let Some(exposure) = modules.module("exposure") {
            let exposure_ev = exposure
                .controls()
                .control("exposure-stops")
                .and_then(|control| match control.value() {
                    DarkroomControlValue::Slider(value) => Some(value),
                    _ => None,
                })
                .unwrap_or(0.0);
            let black_level = exposure
                .controls()
                .control("exposure-black")
                .and_then(|control| match control.value() {
                    DarkroomControlValue::Slider(value) => Some(value),
                    _ => None,
                })
                .unwrap_or(0.0);
            let _ = self.exposure.set_module_projection(
                exposure.revision(),
                exposure.enabled(),
                exposure.expanded(),
                exposure_ev,
                black_level,
            );
            self.exposure
                .set_module_action_handler(action_handler.clone(), exposure.revision());
        } else {
            self.exposure
                .set_module_action_handler(None, Revision::ZERO);
        }
        self.module_action_handler.replace(action_handler);
        self.render_typed_modules();
    }

    /// Returns the searchable module entry for shell-level focus and tests.
    #[must_use]
    pub fn module_search(&self) -> &gtk4::SearchEntry {
        &self.module_search
    }

    #[must_use]
    pub fn exposure(&self) -> &ExposurePanel {
        &self.exposure
    }

    #[must_use]
    pub fn mask_manager(&self) -> &MaskManagerPanel {
        &self.mask_manager
    }

    #[must_use]
    pub fn multiscale_retouch(&self) -> &MultiscaleRetouchPanel {
        &self.multiscale_retouch
    }

    pub fn set_mask_manager_state(&self, state: &MaskManagerSnapshot) {
        self.mask_manager.set_state(state);
    }

    pub fn connect_mask_manager_action<F>(&self, handler: F)
    where
        F: Fn(MaskManagerAction) + 'static,
    {
        self.mask_manager.connect_action(handler);
    }

    pub fn set_multiscale_retouch_state(&self, state: &MultiscaleRetouchSnapshot) {
        self.multiscale_retouch.set_state(state);
    }

    pub fn connect_multiscale_retouch_action<F>(&self, handler: F)
    where
        F: Fn(MultiscaleRetouchAction) + 'static,
    {
        self.multiscale_retouch.connect_action(handler);
    }

    pub(crate) fn module_group_state(&self) -> Rc<Cell<DarkroomModuleGroup>> {
        Rc::clone(&self.module_group)
    }

    pub(crate) fn connect_module_group<F>(&self, handler: F)
    where
        F: Fn(DarkroomModuleGroup) + 'static,
    {
        let typed_modules = Rc::clone(&self.typed_modules);
        let module_action_handler = Rc::clone(&self.module_action_handler);
        let left_modules = self.left_modules.clone();
        let right_modules = self.right_modules.clone();
        let exposure = self.exposure.clone();
        let rgb_denoise = self.rgb_denoise.clone();
        let raw_denoise = self.raw_denoise.clone();
        let mask_manager = self.mask_manager.clone();
        let multiscale_retouch = self.multiscale_retouch.clone();
        let search = self.module_search.clone();
        self.module_group_handler
            .replace(Some(Box::new(move |group| {
                render_typed_modules_into(
                    &left_modules,
                    &right_modules,
                    &exposure,
                    &rgb_denoise,
                    &raw_denoise,
                    &mask_manager,
                    &multiscale_retouch,
                    &typed_modules,
                    &module_action_handler,
                    group,
                    search.text().as_str(),
                );
                handler(group);
            })));
    }

    fn render_typed_modules(&self) {
        render_typed_modules_into(
            &self.left_modules,
            &self.right_modules,
            &self.exposure,
            &self.rgb_denoise,
            &self.raw_denoise,
            &self.mask_manager,
            &self.multiscale_retouch,
            &self.typed_modules,
            &self.module_action_handler,
            self.module_group.get(),
            self.module_search.text().as_str(),
        );
    }

    fn install_module_search(&self) {
        let typed_modules = Rc::clone(&self.typed_modules);
        let module_action_handler = Rc::clone(&self.module_action_handler);
        let left_modules = self.left_modules.clone();
        let right_modules = self.right_modules.clone();
        let exposure = self.exposure.clone();
        let rgb_denoise = self.rgb_denoise.clone();
        let raw_denoise = self.raw_denoise.clone();
        let mask_manager = self.mask_manager.clone();
        let multiscale_retouch = self.multiscale_retouch.clone();
        let group = Rc::clone(&self.module_group);
        self.module_search.connect_search_changed(move |search| {
            render_typed_modules_into(
                &left_modules,
                &right_modules,
                &exposure,
                &rgb_denoise,
                &raw_denoise,
                &mask_manager,
                &multiscale_retouch,
                &typed_modules,
                &module_action_handler,
                group.get(),
                search.text().as_str(),
            );
        });
    }

    /// Projects RGB denoise service state into the darkroom processing rail.
    pub fn set_rgb_denoise_state(&self, state: &RgbDenoiseViewModel) {
        self.rgb_denoise.set_state(state);
    }

    /// Connects RGB denoise controls to the application-owned service controller.
    pub fn connect_rgb_denoise_action<F>(&self, handler: F)
    where
        F: Fn(RgbDenoiseAction) + 'static,
    {
        self.rgb_denoise.connect_action(handler);
    }

    /// Projects RAW denoise service state into the darkroom processing rail.
    pub fn set_raw_denoise_state(&self, state: &RawDenoiseViewModel) {
        self.raw_denoise.set_state(state);
    }

    /// Connects RAW denoise controls to the application-owned service controller.
    pub fn connect_raw_denoise_action<F>(&self, handler: F)
    where
        F: Fn(RawDenoiseAction) + 'static,
    {
        self.raw_denoise.connect_action(handler);
    }

    /// Projects a selected image into the side-rail states without inventing unavailable data.
    pub fn set_detail(&self, detail: &PhotoDetailViewModel) {
        let viewport = self.viewport_state.borrow();
        self.rail_status
            .navigation
            .set_text("filmstrip navigation ready");
        let target = DarkroomPanelTarget::new(
            detail.id(),
            viewport.generation(),
            viewport.edit_revision().unwrap_or(Revision::ZERO),
        );
        self.rail_status.set_detail(detail, target);
        self.set_history_projection(
            &DarkroomPanelProjection::<DarkroomHistoryViewModel>::empty(),
            None,
        );
        self.set_snapshots_projection(
            &DarkroomPanelProjection::<DarkroomSnapshotsViewModel>::empty(),
            None,
        );
    }

    /// Projects the controller-owned snapshot state into the left rail.
    pub fn set_snapshots_projection(
        &self,
        projection: &DarkroomPanelProjection<DarkroomSnapshotsViewModel>,
        handler: Option<DarkroomPanelActionHandler>,
    ) {
        self.rail_status
            .set_snapshots_projection(projection, handler);
    }

    /// Projects the controller-owned edit history into the left rail.
    pub fn set_history_projection(
        &self,
        projection: &DarkroomPanelProjection<DarkroomHistoryViewModel>,
        handler: Option<DarkroomPanelActionHandler>,
    ) {
        self.rail_status.set_history_projection(projection, handler);
    }

    /// Restores the explicit no-selection state of every side-rail surface.
    pub fn clear_detail(&self) {
        self.rail_status
            .navigation
            .set_text("select a photo to navigate");
        self.rail_status.clear_detail();
        self.histogram_generation.set(None);
        self.histogram.clear();
        self.viewport_controls.clear_histogram_sample();
        self.preview.clear_selection();
    }
}

#[derive(Clone)]
struct DarkroomRailStatus {
    navigation: gtk4::Label,
    snapshots_body: gtk4::Box,
    history_body: gtk4::Box,
    image_information_body: gtk4::Box,
}

type DarkroomPanelBuild = (
    gtk4::Box,
    gtk4::Box,
    ExposurePanel,
    RgbDenoisePanel,
    RawDenoisePanel,
    MaskManagerPanel,
    MultiscaleRetouchPanel,
    gtk4::Stack,
    gtk4::SearchEntry,
    Rc<Cell<DarkroomModuleGroup>>,
    Rc<RefCell<Option<DarkroomModuleGroupHandler>>>,
);

#[cfg(test)]
mod tests {
    use super::{
        DARKROOM_MODULE_WIDGET_IDS, DARKROOM_RAIL_FOCUS_ORDER, DARKROOM_VIEWPORT_FOCUS_ORDER,
        DARKROOM_VIEWPORT_WIDGET_IDS, DARKROOM_WIDGET_IDS, DarkroomModuleGroup,
    };
    use crate::gtk_shell::DARKROOM_OPERATION_FOCUS_ORDER;

    #[test]
    fn darkroom_contract_has_stable_unique_roles_and_initial_exposure() {
        let unique = DARKROOM_WIDGET_IDS
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), DARKROOM_WIDGET_IDS.len());
        assert_eq!(DARKROOM_WIDGET_IDS[0], "darkroom-page");
        assert_eq!(DARKROOM_WIDGET_IDS.last(), Some(&"darkroom-job-status"));
        assert!(DARKROOM_WIDGET_IDS.contains(&"exposure"));
        assert!(DARKROOM_WIDGET_IDS.contains(&"darkroom-left-module-scroll"));
        assert!(DARKROOM_WIDGET_IDS.contains(&"darkroom-right-module-scroll"));
        assert_eq!(DARKROOM_OPERATION_FOCUS_ORDER[0], "module-disclosure");
        assert_eq!(
            DARKROOM_OPERATION_FOCUS_ORDER.last(),
            Some(&"module-control")
        );
        assert_eq!(DARKROOM_RAIL_FOCUS_ORDER[0], "darkroom-navigation");
        assert_eq!(DARKROOM_RAIL_FOCUS_ORDER.last(), Some(&"group-deprecated"));
        assert_eq!(DARKROOM_MODULE_WIDGET_IDS[0], "darkroom-module-search");
        assert_eq!(DARKROOM_MODULE_WIDGET_IDS.last(), Some(&"exposure-reset"));
        assert_eq!(
            DARKROOM_MODULE_WIDGET_IDS
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            DARKROOM_MODULE_WIDGET_IDS.len()
        );
    }

    #[test]
    fn viewport_controls_have_unique_accessible_focus_contract() {
        let unique = DARKROOM_VIEWPORT_WIDGET_IDS
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), DARKROOM_VIEWPORT_WIDGET_IDS.len());
        assert_eq!(DARKROOM_VIEWPORT_FOCUS_ORDER[0], "darkroom-soft-proof");
        assert!(
            DARKROOM_VIEWPORT_FOCUS_ORDER
                .iter()
                .all(|id| unique.contains(id))
        );
    }

    #[test]
    fn module_groups_have_stable_semantics_and_truthful_filtering() {
        let modules = crate::reference_modules().expect("registry modules");
        let exposure = modules.module("exposure").expect("exposure");
        let lens = modules.module("lenscorrection").expect("lens correction");
        let grading = modules.module("graduatednd").expect("graduated ND");
        let grain = modules.module("grain").expect("grain");
        let hidden = modules.module("finalscale").expect("hidden final scale");
        assert!(DarkroomModuleGroup::Active.matches(exposure));
        assert!(DarkroomModuleGroup::Technical.matches(lens));
        assert!(DarkroomModuleGroup::Grading.matches(grading));
        assert!(DarkroomModuleGroup::Favorites.matches(grain));
        assert!(!DarkroomModuleGroup::Technical.matches(exposure));
        assert!(!DarkroomModuleGroup::Correct.matches(hidden));
    }
}
