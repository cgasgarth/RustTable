//! Darktable-shaped GTK4 darkroom composition.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::prelude::*;
use rusttable_core::{PhotoId, Revision};
use rusttable_display_profile::{DisplayProfileReceipt, DisplayProfileSnapshot};

use super::darkroom_modules::{
    DarkroomModuleActionHandler, DarkroomModulesViewModel, reference_modules,
};
#[path = "darkroom_interaction.rs"]
mod darkroom_interaction;
#[path = "darkroom_viewport.rs"]
mod darkroom_viewport;
#[path = "darkroom_controls/panel_widgets.rs"]
mod panel_widgets;
pub(super) use super::{DARKROOM_GEOMETRY, ThemeRole, apply_theme_role};
use super::{ExposurePanel, PhotoPreview};
use crate::presentation::PhotoDetailViewModel;
use crate::viewport_presentation::{
    DarkroomViewportCommand, DarkroomViewportState, ViewportGeneration,
};
use darkroom_interaction::{
    FilmstripState, HistogramView, connect_filmstrip_button, install_filmstrip_keyboard,
    sync_filmstrip_buttons,
};
pub(super) use darkroom_viewport::chrome_toggle;
use darkroom_viewport::{ViewportControls, darkroom_page, sync_viewport_controls};
use panel_widgets::{left_panel, render_typed_modules_into, right_panel};

#[path = "profile_diagnostics.rs"]
pub(super) mod profile_diagnostics;
use profile_diagnostics::{
    ProfileDiagnosticRequest, ProfileDiagnosticSurface, project_profile_diagnostic,
};

/// Stable widget identifiers for the initial darkroom surface.
pub const DARKROOM_WIDGET_IDS: [&str; 13] = [
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
    "exposure",
];

/// Stable left-to-right focus order for the darkroom rail controls.
pub const DARKROOM_RAIL_FOCUS_ORDER: [&str; 9] = [
    "darkroom-navigation",
    "darkroom-snapshots",
    "darkroom-history",
    "darkroom-image-information",
    "darkroom-module-search",
    "group-active",
    "group-favorites",
    "group-technical",
    "group-grading",
];

/// Stable identifiers for the searchable, grouped module-stack controls.
pub const DARKROOM_MODULE_WIDGET_IDS: [&str; 7] = [
    "darkroom-module-search",
    "group-active",
    "group-favorites",
    "group-technical",
    "group-grading",
    "exposure-presets",
    "exposure-reset",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DarkroomModuleGroup {
    Active,
    Favorites,
    Technical,
    Grading,
}

impl DarkroomModuleGroup {
    pub(super) fn matches_title(self, title: &str) -> bool {
        let title = title.to_ascii_lowercase();
        match self {
            Self::Active => true,
            Self::Favorites => title.contains("favorite"),
            Self::Technical => ["balance", "denoise", "lens", "raw", "sharpen"]
                .iter()
                .any(|term| title.contains(term)),
            Self::Grading => ["color", "contrast", "curve", "exposure", "tone"]
                .iter()
                .any(|term| title.contains(term)),
        }
    }
}

type DarkroomModuleGroupHandler = Box<dyn Fn(DarkroomModuleGroup)>;
type DarkroomFilmstripHandler = Box<dyn Fn(PhotoId, ViewportGeneration)>;

/// Stable identifiers for the darkroom viewport controls and filmstrip boundary.
pub const DARKROOM_VIEWPORT_WIDGET_IDS: [&str; 9] = [
    "darkroom-viewport",
    "darkroom-soft-proof",
    "darkroom-gamut-check",
    "darkroom-zoom",
    "darkroom-fit",
    "darkroom-before-after",
    "darkroom-viewport-projection",
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
}

impl DarkroomView {
    /// Builds the initial Darktable darkroom around the immutable preview boundary.
    #[must_use]
    pub fn new(panel_width: i32) -> Self {
        debug_assert_eq!(DARKROOM_RAIL_FOCUS_ORDER.len(), 9);
        debug_assert_eq!(DARKROOM_MODULE_WIDGET_IDS.len(), 7);
        let preview = PhotoPreview::new();
        let viewport_state = Rc::new(RefCell::new(DarkroomViewportState::default()));
        let viewport_handler = Rc::new(RefCell::new(None));
        let (page, viewport_controls) = darkroom_page(&preview, &viewport_state, &viewport_handler);
        let profile_diagnostic =
            ProfileDiagnosticSurface::new("darkroom-profile-diagnostic", "Darkroom profile status");
        page.append(profile_diagnostic.widget());
        let (left_panel, left_modules, rail_status) = left_panel(panel_width);
        let (
            right_panel,
            right_modules,
            exposure,
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
        };
        view.install_module_search();
        view.render_typed_modules();
        view
    }

    #[must_use]
    pub fn page(&self) -> &gtk4::Box {
        &self.page
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
        self.histogram.unavailable();
        self.sync_viewport_projection();
    }

    /// Restores truthful no-photo state and resets transient viewport controls.
    pub fn clear_viewport_selection(&self) {
        self.viewport_state.borrow_mut().clear_selection();
        self.filmstrip_state.borrow_mut().clear_selection();
        self.histogram_generation.set(None);
        self.histogram.clear();
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
    pub(super) fn set_profile_diagnostic_state(
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
            return false;
        }
        self.histogram.set_bins(red, green, blue)
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
            darkroom_interaction::filmstrip_ids(filmstrip),
            selected,
            generation,
        );
        install_filmstrip_keyboard(
            filmstrip,
            &self.filmstrip_state,
            &self.internal_filmstrip_handler(),
        );
        let handler = self.internal_filmstrip_handler();
        let buttons = darkroom_interaction::filmstrip_buttons(filmstrip);
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

    fn internal_filmstrip_handler(&self) -> darkroom_interaction::FilmstripHandler {
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

    pub(super) fn module_group_state(&self) -> Rc<Cell<DarkroomModuleGroup>> {
        Rc::clone(&self.module_group)
    }

    pub(super) fn connect_module_group<F>(&self, handler: F)
    where
        F: Fn(DarkroomModuleGroup) + 'static,
    {
        let typed_modules = Rc::clone(&self.typed_modules);
        let module_action_handler = Rc::clone(&self.module_action_handler);
        let left_modules = self.left_modules.clone();
        let right_modules = self.right_modules.clone();
        let exposure = self.exposure.clone();
        let search = self.module_search.clone();
        self.module_group_handler
            .replace(Some(Box::new(move |group| {
                render_typed_modules_into(
                    &left_modules,
                    &right_modules,
                    &exposure,
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
        let group = Rc::clone(&self.module_group);
        self.module_search.connect_search_changed(move |search| {
            render_typed_modules_into(
                &left_modules,
                &right_modules,
                &exposure,
                &typed_modules,
                &module_action_handler,
                group.get(),
                search.text().as_str(),
            );
        });
    }

    /// Projects a selected image into the side-rail states without inventing unavailable data.
    pub fn set_detail(&self, detail: &PhotoDetailViewModel) {
        self.rail_status
            .navigation
            .set_text("filmstrip navigation ready");
        self.rail_status
            .snapshots
            .set_text("snapshot data unavailable");
        self.rail_status
            .history
            .set_text("edit history unavailable");
        self.rail_status.image_information.set_text(&format!(
            "{} · {} metadata fields",
            detail.title().as_str(),
            detail.facts().count()
        ));
        self.histogram.unavailable();
    }

    /// Restores the explicit no-selection state of every side-rail surface.
    pub fn clear_detail(&self) {
        self.rail_status
            .navigation
            .set_text("select a photo to navigate");
        self.rail_status
            .snapshots
            .set_text("select a photo to view snapshots");
        self.rail_status
            .history
            .set_text("select a photo to view edit history");
        self.rail_status
            .image_information
            .set_text("image information unavailable");
        self.histogram_generation.set(None);
        self.histogram.clear();
        self.preview.clear_selection();
    }
}

#[derive(Clone)]
struct DarkroomRailStatus {
    navigation: gtk4::Label,
    snapshots: gtk4::Label,
    history: gtk4::Label,
    image_information: gtk4::Label,
}

type DarkroomPanelBuild = (
    gtk4::Box,
    gtk4::Box,
    ExposurePanel,
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

    #[test]
    fn darkroom_contract_has_stable_unique_roles_and_initial_exposure() {
        let unique = DARKROOM_WIDGET_IDS
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), DARKROOM_WIDGET_IDS.len());
        assert_eq!(DARKROOM_WIDGET_IDS[0], "darkroom-page");
        assert_eq!(DARKROOM_WIDGET_IDS.last(), Some(&"exposure"));
        assert_eq!(DARKROOM_RAIL_FOCUS_ORDER[0], "darkroom-navigation");
        assert_eq!(DARKROOM_RAIL_FOCUS_ORDER.last(), Some(&"group-grading"));
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
        assert!(DarkroomModuleGroup::Active.matches_title("anything"));
        assert!(DarkroomModuleGroup::Favorites.matches_title("Favorite presets"));
        assert!(DarkroomModuleGroup::Technical.matches_title("Lens correction"));
        assert!(DarkroomModuleGroup::Grading.matches_title("Color balance"));
        assert!(!DarkroomModuleGroup::Technical.matches_title("Exposure"));
    }
}
