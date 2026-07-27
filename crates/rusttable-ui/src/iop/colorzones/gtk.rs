//! GTK4 leaf adapter for Darktable `src/iop/colorzones.c` lines 2619-2740 and
//! the drawing-area resize path in `src/gui/gtk.c` lines 4266-4284.
//!
//! GTK controllers replace the retained GTK3 event masks. Durable channel and
//! graph-height state are consumed here; picker and mask lifecycles, the operation
//! histogram, and global shortcut or hold-mode routing remain absent.

#![allow(
    clippy::too_many_lines,
    reason = "the source-ordered GTK hierarchy and its adjacent controller wiring stay reviewable together"
)]

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk4::{gdk, glib, prelude::*};
use rusttable_core::{OperationId, OperationOpacity, Revision};
use rusttable_processing::{
    ColorZonesChannel, ColorZonesCurveType, ColorZonesMode, ColorZonesParametersV5,
};

use crate::bauhaus::{
    combobox::{BauhausComboBox, BauhausComboBoxSpec},
    slider_input::{BauhausSlider, FullWidthSliderSpec, SliderInputSpec, full_width_slider},
    slider_popup::{
        reset_source_scroll_units, source_scroll_unit_delta as normalize_source_scroll_unit_delta,
    },
};

use super::{
    COLORZONES_GRAPH_HEIGHT_MAX, COLORZONES_GRAPH_HEIGHT_MIN, COLORZONES_GRAPH_INSET,
    ColorZonesArrowKey, ColorZonesClick, ColorZonesEditorState, ColorZonesGraphHeight,
    ColorZonesInteraction, ColorZonesInteractionError, ColorZonesModifiers,
    ColorZonesPrimaryOutcome, ColorZonesRenderModel, ColorZonesScrollOutcome,
    ColorZonesSecondaryOutcome, ColorZonesSelection,
    paint::{paint_bottom_strip, paint_graph},
};

pub const COLORZONES_OUTPUT_LABELS: [&str; 3] = ["lightness", "chroma", "hue"];
pub const COLORZONES_SELECTION_LABEL: &str = "select by";
pub const COLORZONES_SELECTION_OPTIONS: [&str; 3] = ["lightness", "chroma", "hue"];
pub const COLORZONES_SELECTION_TOOLTIP: &str =
    "choose selection criterion, will be the abscissa in the graph";
pub const COLORZONES_MODE_LABEL: &str = "process mode";
pub const COLORZONES_MODE_OPTIONS: [&str; 2] = ["smooth", "strong"];
pub const COLORZONES_MODE_TOOLTIP: &str = "choose between a smoother or stronger effect";
pub const COLORZONES_STRENGTH_LABEL: &str = "mix";
pub const COLORZONES_STRENGTH_TOOLTIP: &str = "make effect stronger or weaker";
pub const COLORZONES_EDIT_BY_AREA_LABEL: &str = "edit by area";
pub const COLORZONES_EDIT_BY_AREA_TOOLTIP: &str = "edit the curve nodes by area";
pub const COLORZONES_INTERPOLATION_LABEL: &str = "interpolation method";
pub const COLORZONES_INTERPOLATION_OPTIONS: [&str; 3] =
    ["cubic spline", "centripetal spline", "monotonic spline"];
pub const COLORZONES_INTERPOLATION_TOOLTIP: &str = "change this method if you see oscillations or cusps in the curve\n- cubic spline is better to produce smooth curves but oscillates when nodes are too close\n- centripetal is better to avoids cusps and oscillations with close nodes but is less smooth\n- monotonic is better for accuracy of pure analytical functions (log, gamma, exp)";

const COLORZONES_BOTTOM_CLICK_CONTROLLER_NAME: &str = "dt-colorzones-bottom-click";
const COLORZONES_PRIMARY_BUTTON: u32 = 1;

/// Complete UI-owned snapshot for one exact Color Zones leaf.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorZonesGtkState {
    operation_id: OperationId,
    revision: Revision,
    editor: Box<ColorZonesEditorState>,
    graph_height: ColorZonesGraphHeight,
    enabled: bool,
    opacity: OperationOpacity,
    sensitive: bool,
    materialization_required: bool,
}

impl ColorZonesGtkState {
    #[must_use]
    pub fn new(
        operation_id: OperationId,
        revision: Revision,
        editor: ColorZonesEditorState,
        enabled: bool,
        opacity: OperationOpacity,
        sensitive: bool,
        materialization_required: bool,
    ) -> Self {
        Self {
            operation_id,
            revision,
            editor: Box::new(editor),
            graph_height: ColorZonesGraphHeight::default(),
            enabled,
            opacity,
            sensitive,
            materialization_required,
        }
    }

    #[must_use]
    pub const fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn editor(&self) -> &ColorZonesEditorState {
        &self.editor
    }

    #[must_use]
    pub const fn graph_height(&self) -> ColorZonesGraphHeight {
        self.graph_height
    }

    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub const fn opacity(&self) -> OperationOpacity {
        self.opacity
    }

    #[must_use]
    pub const fn sensitive(&self) -> bool {
        self.sensitive
    }

    #[must_use]
    pub const fn materialization_required(&self) -> bool {
        self.materialization_required
    }

    /// Replaces only the UI-owned output tab while retaining persisted truth.
    #[must_use]
    pub fn with_output_channel(mut self, output_channel: ColorZonesChannel) -> Self {
        self.editor.set_output_channel(output_channel);
        self
    }

    /// Replaces only the durable UI-owned graph height while retaining edit truth.
    #[must_use]
    pub const fn with_graph_height(mut self, graph_height: ColorZonesGraphHeight) -> Self {
        self.graph_height = graph_height;
        self
    }

    #[must_use]
    const fn preferences(&self) -> ColorZonesGtkPreferences {
        ColorZonesGtkPreferences::new(self.editor.output_channel(), self.graph_height)
    }
}

/// Durable global Color Zones presentation state authored by this leaf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorZonesGtkPreferences {
    output_channel: ColorZonesChannel,
    graph_height: ColorZonesGraphHeight,
}

impl ColorZonesGtkPreferences {
    #[must_use]
    pub const fn new(
        output_channel: ColorZonesChannel,
        graph_height: ColorZonesGraphHeight,
    ) -> Self {
        Self {
            output_channel,
            graph_height,
        }
    }

    #[must_use]
    pub const fn output_channel(self) -> ColorZonesChannel {
        self.output_channel
    }

    #[must_use]
    pub const fn graph_height(self) -> ColorZonesGraphHeight {
        self.graph_height
    }
}

pub type ColorZonesGtkPreferencesHandler = Rc<dyn Fn(ColorZonesGtkPreferences) -> bool>;

/// One settled canonical parameter replacement authored by this leaf.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesSettledAction {
    target: OperationId,
    expected_revision: Revision,
    output_channel: ColorZonesChannel,
    parameters: ColorZonesParametersV5,
    enable_required: bool,
    materialization_required: bool,
}

impl ColorZonesSettledAction {
    #[must_use]
    pub const fn target(self) -> OperationId {
        self.target
    }

    #[must_use]
    pub const fn expected_revision(self) -> Revision {
        self.expected_revision
    }

    #[must_use]
    pub const fn output_channel(self) -> ColorZonesChannel {
        self.output_channel
    }

    #[must_use]
    pub const fn parameters(self) -> ColorZonesParametersV5 {
        self.parameters
    }

    #[must_use]
    pub const fn enable_required(self) -> bool {
        self.enable_required
    }

    #[must_use]
    pub const fn materialization_required(self) -> bool {
        self.materialization_required
    }
}

/// Synchronous persistence result consumed without rebuilding the module rail.
#[derive(Debug, Clone, PartialEq)]
pub enum ColorZonesGtkHandlerOutcome {
    /// Keep the local state and advance to the committed edit revision.
    Commit { revision: Revision },
    /// Restore the exact state from before the attempted action.
    Rollback,
    /// Replace target, revision, parameters and presentation state in place.
    Reconcile(ColorZonesGtkState),
}

pub type ColorZonesGtkActionHandler =
    Rc<dyn Fn(ColorZonesSettledAction) -> ColorZonesGtkHandlerOutcome>;

/// Mounted Color Zones leaf with an in-place reconciliation seam.
#[derive(Clone)]
pub struct ColorZonesGtkLeaf {
    root: gtk4::Box,
    shared: Rc<Shared>,
}

impl ColorZonesGtkLeaf {
    #[must_use]
    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    #[must_use]
    pub fn state(&self) -> ColorZonesGtkState {
        self.shared.snapshot.borrow().clone()
    }

    pub fn reconcile(&self, state: ColorZonesGtkState) {
        self.shared.reconcile(state);
    }

    /// Installs the durable presentation-state persistence boundary.
    pub fn set_preferences_handler(&self, handler: Option<ColorZonesGtkPreferencesHandler>) {
        self.shared.preferences_handler.replace(handler);
    }

    /// Routes one already-normalized discrete graph-wheel delta with explicit
    /// GTK modifiers through the production interaction and widget adapter.
    ///
    /// This is also the boundary-test seam for modifier-sensitive scrolling,
    /// because synthetic `EventControllerScroll` emission cannot carry state.
    ///
    /// # Errors
    ///
    /// Returns the interaction error for a non-finite wheel delta.
    pub fn route_graph_scroll(
        &self,
        delta_y: f64,
        modifiers: gdk::ModifierType,
    ) -> Result<ColorZonesScrollOutcome, ColorZonesInteractionError> {
        self.shared.route_graph_scroll(delta_y, modifiers)
    }

    /// Starts a smooth graph-scroll sequence at the same settlement boundary
    /// used by the production GTK scroll controller.
    pub fn begin_graph_scroll_sequence(&self) {
        self.shared.begin_graph_scroll_sequence();
    }

    /// Routes already-normalized whole units as a live smooth-scroll update.
    ///
    /// # Errors
    ///
    /// Returns the interaction error for a non-finite unit delta.
    pub fn route_graph_scroll_sequence(
        &self,
        delta_y: f64,
        modifiers: gdk::ModifierType,
    ) -> Result<ColorZonesScrollOutcome, ColorZonesInteractionError> {
        self.shared.route_graph_scroll_sequence(delta_y, modifiers)
    }

    /// Routes one raw GTK graph-scroll delta through the shared source unit
    /// accumulator and the production interaction adapter.
    ///
    /// `Ok(None)` means that a smooth fractional delta has not yet reached a
    /// whole source unit.
    ///
    /// # Errors
    ///
    /// Returns the interaction error if an emitted whole unit cannot be routed.
    pub fn route_raw_graph_scroll(
        &self,
        unit: gdk::ScrollUnit,
        delta_x: f64,
        delta_y: f64,
        modifiers: gdk::ModifierType,
    ) -> Result<Option<ColorZonesScrollOutcome>, ColorZonesInteractionError> {
        self.shared
            .route_raw_graph_scroll(unit, delta_x, delta_y, modifiers)
    }

    /// Settles all parameter changes from the active smooth-scroll sequence as
    /// one logical action and clears the source-global fractional remainders.
    pub fn end_graph_scroll_sequence(&self) {
        reset_source_scroll_units();
        self.shared.end_graph_scroll_sequence();
    }
}

/// Builds the source-ordered Color Zones GTK4 leaf.
///
/// # Panics
///
/// Panics if a validated Color Zones enum cannot be represented as a GTK selection index.
#[must_use]
pub fn build_colorzones_gtk(
    widget_id: &str,
    state: ColorZonesGtkState,
    handler: Option<ColorZonesGtkActionHandler>,
) -> ColorZonesGtkLeaf {
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root.set_widget_name(&format!("{widget_id}-colorzones-editor"));
    root.set_hexpand(true);

    let notebook = gtk4::Notebook::new();
    notebook.set_widget_name(&format!("{widget_id}-channel-tabs"));
    notebook.set_hexpand(true);
    notebook.set_vexpand(false);
    notebook.set_scrollable(false);
    notebook.set_enable_popup(false);
    for label_text in COLORZONES_OUTPUT_LABELS {
        let page = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        page.set_hexpand(true);
        page.set_vexpand(false);
        page.set_widget_name(&format!("{widget_id}-{label_text}-page"));
        let label = gtk4::Label::new(Some(label_text));
        label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        label.set_tooltip_text(Some(label_text));
        notebook.append_page(&page, Some(&label));
        let notebook_page = notebook.page(&page);
        notebook_page.set_tab_expand(true);
        notebook_page.set_tab_fill(true);
    }
    root.append(&notebook);

    let graph = gtk4::DrawingArea::new();
    graph.set_widget_name(&format!("{widget_id}-graph"));
    graph.set_hexpand(true);
    graph.set_focusable(true);
    graph.set_accessible_role(gtk4::AccessibleRole::Slider);
    graph.update_property(&[gtk4::accessible::Property::Label("Color Zones graph")]);
    root.append(&graph);

    let bottom_bar = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    bottom_bar.set_widget_name("iop-bottom-bar");
    bottom_bar.set_vexpand(false);
    let bottom = gtk4::DrawingArea::new();
    bottom.set_widget_name(&format!("{widget_id}-bottom-strip"));
    bottom.set_hexpand(true);
    bottom.set_vexpand(true);
    bottom_bar.append(&bottom);
    root.append(&bottom_bar);

    let edit_by_area = gtk4::CheckButton::with_label(COLORZONES_EDIT_BY_AREA_LABEL);
    edit_by_area.set_widget_name(&format!("{widget_id}-edit-by-area"));
    edit_by_area.set_hexpand(false);
    edit_by_area.set_halign(gtk4::Align::Start);
    edit_by_area.set_tooltip_text(Some(COLORZONES_EDIT_BY_AREA_TOOLTIP));
    if let Some(label) = edit_by_area
        .first_child()
        .and_then(|child| child.downcast::<gtk4::Label>().ok())
    {
        label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
    }
    root.append(&edit_by_area);

    let select_by = BauhausComboBox::new(
        &format!("{widget_id}-select-by"),
        BauhausComboBoxSpec::new(
            COLORZONES_SELECTION_LABEL,
            COLORZONES_SELECTION_TOOLTIP,
            &COLORZONES_SELECTION_OPTIONS,
        ),
    );
    select_by.set_selected(channel_index(state.editor().selection_channel()));
    select_by
        .widget()
        .set_tooltip_text(Some(select_by.tooltip()));
    select_by
        .dropdown()
        .update_property(&[gtk4::accessible::Property::Description(
            COLORZONES_SELECTION_TOOLTIP,
        )]);
    root.append(select_by.widget());

    let mode = BauhausComboBox::new(
        &format!("{widget_id}-mode"),
        BauhausComboBoxSpec::new(
            COLORZONES_MODE_LABEL,
            COLORZONES_MODE_TOOLTIP,
            &COLORZONES_MODE_OPTIONS,
        ),
    );
    mode.set_selected(u32::try_from(state.editor().mode().raw()).expect("mode index"));
    mode.widget().set_tooltip_text(Some(mode.tooltip()));
    mode.dropdown()
        .update_property(&[gtk4::accessible::Property::Description(
            COLORZONES_MODE_TOOLTIP,
        )]);
    root.append(mode.widget());

    let strength = full_width_slider(FullWidthSliderSpec {
        widget_name: &format!("{widget_id}-strength"),
        label: COLORZONES_STRENGTH_LABEL,
        tooltip: COLORZONES_STRENGTH_TOOLTIP,
        minimum: -200.0,
        maximum: 200.0,
        gtk_step: 1.0,
        value: f64::from(state.editor().strength()),
        input: SliderInputSpec::IDENTITY
            .with_suffix("%")
            .with_soft_range(-200.0, 200.0)
            .with_default_value(0.0)
            .with_digits(2)
            .with_automatic_step(),
    });
    strength
        .scale()
        .update_property(&[gtk4::accessible::Property::Description(
            COLORZONES_STRENGTH_TOOLTIP,
        )]);
    root.append(strength.widget());

    let interpolator = BauhausComboBox::new(
        &format!("{widget_id}-interpolator"),
        BauhausComboBoxSpec::new(
            COLORZONES_INTERPOLATION_LABEL,
            COLORZONES_INTERPOLATION_TOOLTIP,
            &COLORZONES_INTERPOLATION_OPTIONS,
        ),
    );
    let curve_type = state
        .editor()
        .curve_type(state.editor().output_channel())
        .raw();
    interpolator.set_selected(u32::try_from(curve_type).expect("curve interpolation index"));
    interpolator
        .widget()
        .set_tooltip_text(Some(interpolator.tooltip()));
    interpolator
        .dropdown()
        .update_property(&[gtk4::accessible::Property::Description(
            COLORZONES_INTERPOLATION_TOOLTIP,
        )]);
    root.append(interpolator.widget());

    let interaction = ColorZonesInteraction::new(state.editor().clone());
    let shared = Rc::new(Shared {
        snapshot: RefCell::new(state),
        interaction: RefCell::new(interaction),
        handler,
        preferences_handler: RefCell::new(None),
        suppress_updates: Cell::new(false),
        widgets: RefCell::new(None),
        scroll_origin: RefCell::new(None),
    });
    *shared.widgets.borrow_mut() = Some(WidgetRefs {
        root: root.downgrade(),
        notebook: notebook.downgrade(),
        graph: graph.downgrade(),
        bottom: bottom.downgrade(),
        edit_by_area: edit_by_area.downgrade(),
        select_by: select_by.clone(),
        mode: mode.clone(),
        strength: strength.clone(),
        interpolator: interpolator.clone(),
    });
    shared.sync_widgets();

    connect_channel_tabs(&notebook, &shared);
    connect_non_graph_controls(
        &edit_by_area,
        &select_by,
        &mode,
        &strength,
        &interpolator,
        &shared,
    );
    connect_graph(&graph, &shared);
    connect_bottom(&bottom, &shared);

    ColorZonesGtkLeaf { root, shared }
}

struct Shared {
    snapshot: RefCell<ColorZonesGtkState>,
    interaction: RefCell<ColorZonesInteraction>,
    handler: Option<ColorZonesGtkActionHandler>,
    preferences_handler: RefCell<Option<ColorZonesGtkPreferencesHandler>>,
    suppress_updates: Cell<bool>,
    widgets: RefCell<Option<WidgetRefs>>,
    scroll_origin: RefCell<Option<ColorZonesInteraction>>,
}

struct WidgetRefs {
    root: glib::WeakRef<gtk4::Box>,
    notebook: glib::WeakRef<gtk4::Notebook>,
    graph: glib::WeakRef<gtk4::DrawingArea>,
    bottom: glib::WeakRef<gtk4::DrawingArea>,
    edit_by_area: glib::WeakRef<gtk4::CheckButton>,
    select_by: BauhausComboBox,
    mode: BauhausComboBox,
    strength: BauhausSlider,
    interpolator: BauhausComboBox,
}

impl Shared {
    fn reconcile(&self, state: ColorZonesGtkState) {
        self.scroll_origin.borrow_mut().take();
        *self.interaction.borrow_mut() = ColorZonesInteraction::new(state.editor().clone());
        *self.snapshot.borrow_mut() = state;
        self.sync_widgets();
    }

    fn persist_preferences(&self, preferences: ColorZonesGtkPreferences) -> bool {
        self.preferences_handler
            .borrow()
            .as_ref()
            .is_none_or(|handler| handler(preferences))
    }

    fn resize_graph_height(&self, delta_y: f64) {
        let before = self.snapshot.borrow().preferences();
        let next = resized_graph_height(before.graph_height(), delta_y);
        if next == before.graph_height() {
            self.queue_graph_draw();
            return;
        }
        self.snapshot.borrow_mut().graph_height = next;
        let after = ColorZonesGtkPreferences::new(before.output_channel(), next);
        if !self.persist_preferences(after) {
            self.snapshot.borrow_mut().graph_height = before.graph_height();
        }
        self.sync_widgets();
    }

    fn settle(&self, before: ColorZonesInteraction) {
        let parameters = self.interaction.borrow().editor().parameters_value();
        if parameters == before.editor().parameters_value() {
            self.queue_draw();
            return;
        }
        let snapshot = self.snapshot.borrow().clone();
        let action = ColorZonesSettledAction {
            target: snapshot.operation_id,
            expected_revision: snapshot.revision,
            output_channel: self.interaction.borrow().editor().output_channel(),
            parameters,
            enable_required: !snapshot.enabled,
            materialization_required: snapshot.materialization_required,
        };
        let outcome = self
            .handler
            .as_ref()
            .map_or(ColorZonesGtkHandlerOutcome::Rollback, |handler| {
                handler(action)
            });
        match outcome {
            ColorZonesGtkHandlerOutcome::Commit { revision } => {
                let editor = self.interaction.borrow().editor().clone();
                let mut snapshot = self.snapshot.borrow_mut();
                snapshot.revision = revision;
                *snapshot.editor = editor;
                if action.enable_required {
                    snapshot.enabled = true;
                }
                if action.materialization_required {
                    snapshot.materialization_required = false;
                }
            }
            ColorZonesGtkHandlerOutcome::Rollback => {
                *self.interaction.borrow_mut() = before;
            }
            ColorZonesGtkHandlerOutcome::Reconcile(state) => {
                *self.interaction.borrow_mut() = ColorZonesInteraction::new(state.editor().clone());
                *self.snapshot.borrow_mut() = state;
            }
        }
        self.sync_widgets();
    }

    fn begin_graph_scroll_sequence(&self) {
        let mut origin = self.scroll_origin.borrow_mut();
        if origin.is_none() {
            *origin = Some(self.interaction.borrow().clone());
        }
    }

    fn end_graph_scroll_sequence(&self) {
        if let Some(before) = self.scroll_origin.borrow_mut().take() {
            self.settle(before);
        }
    }

    fn route_graph_scroll(
        &self,
        delta_y: f64,
        modifiers: gdk::ModifierType,
    ) -> Result<ColorZonesScrollOutcome, ColorZonesInteractionError> {
        self.route_graph_scroll_with_settlement(delta_y, modifiers, true)
    }

    fn route_graph_scroll_sequence(
        &self,
        delta_y: f64,
        modifiers: gdk::ModifierType,
    ) -> Result<ColorZonesScrollOutcome, ColorZonesInteractionError> {
        self.begin_graph_scroll_sequence();
        self.route_graph_scroll_with_settlement(delta_y, modifiers, false)
    }

    fn route_raw_graph_scroll(
        &self,
        unit: gdk::ScrollUnit,
        delta_x: f64,
        delta_y: f64,
        modifiers: gdk::ModifierType,
    ) -> Result<Option<ColorZonesScrollOutcome>, ColorZonesInteractionError> {
        let Some(delta) = source_scroll_unit_delta(unit, delta_x, delta_y) else {
            return Ok(None);
        };
        if unit == gdk::ScrollUnit::Wheel {
            self.route_graph_scroll(f64::from(delta), modifiers)
                .map(Some)
        } else {
            self.route_graph_scroll_sequence(f64::from(delta), modifiers)
                .map(Some)
        }
    }

    fn route_graph_scroll_with_settlement(
        &self,
        delta_y: f64,
        modifiers: gdk::ModifierType,
        settle_immediately: bool,
    ) -> Result<ColorZonesScrollOutcome, ColorZonesInteractionError> {
        if delta_y.is_finite() && exact_height_resize_modifiers(modifiers) {
            self.resize_graph_height(delta_y);
            return Ok(ColorZonesScrollOutcome::Consumed);
        }
        let before = self.interaction.borrow().clone();
        let outcome =
            route_graph_scroll_interaction(&mut self.interaction.borrow_mut(), delta_y, modifiers);
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                self.queue_draw();
                return Err(error);
            }
        };
        match outcome {
            ColorZonesScrollOutcome::NodeMoved if settle_immediately => self.settle(before),
            ColorZonesScrollOutcome::ForwardToChannelTabs { delta_y } => {
                let notebook = self
                    .widgets
                    .borrow()
                    .as_ref()
                    .and_then(|widgets| widgets.notebook.upgrade());
                if let Some(notebook) = notebook {
                    switch_output_page(&notebook, delta_y);
                }
                self.queue_draw();
            }
            ColorZonesScrollOutcome::NodeMoved
            | ColorZonesScrollOutcome::Zoomed
            | ColorZonesScrollOutcome::RadiusChanged
            | ColorZonesScrollOutcome::Consumed => self.queue_draw(),
        }
        Ok(outcome)
    }

    fn sync_widgets(&self) {
        let widget_refs = self.widgets.borrow();
        let Some(widgets) = widget_refs.as_ref() else {
            return;
        };
        self.suppress_updates.set(true);
        let snapshot = self.snapshot.borrow();
        let interaction = self.interaction.borrow();
        let editor = interaction.editor();
        if let Some(root) = widgets.root.upgrade() {
            root.set_sensitive(snapshot.sensitive);
        }
        if let Some(notebook) = widgets.notebook.upgrade() {
            notebook.set_current_page(Some(channel_index(editor.output_channel())));
        }
        if let Some(graph) = widgets.graph.upgrade() {
            let height = i32::from(snapshot.graph_height.logical_pixels());
            graph.set_content_height(height);
            graph.set_height_request(height);
        }
        if let Some(edit_by_area) = widgets.edit_by_area.upgrade() {
            edit_by_area.set_active(interaction.edit_by_area());
        }
        widgets
            .select_by
            .set_selected(channel_index(editor.selection_channel()));
        widgets
            .mode
            .set_selected(u32::try_from(editor.mode().raw()).expect("mode index is nonnegative"));
        widgets.strength.set_value(f64::from(editor.strength()));
        let curve_type = editor.curve_type(editor.output_channel()).raw();
        widgets.interpolator.set_selected(
            u32::try_from(curve_type).expect("curve interpolation index is nonnegative"),
        );
        self.suppress_updates.set(false);
        drop(interaction);
        drop(snapshot);
        self.queue_draw();
    }

    fn queue_graph_draw(&self) {
        let widget_refs = self.widgets.borrow();
        let Some(widgets) = widget_refs.as_ref() else {
            return;
        };
        if let Some(graph) = widgets.graph.upgrade() {
            graph.queue_draw();
        }
    }

    fn queue_draw(&self) {
        self.queue_graph_draw();
        let widget_refs = self.widgets.borrow();
        let Some(widgets) = widget_refs.as_ref() else {
            return;
        };
        if let Some(bottom) = widgets.bottom.upgrade() {
            bottom.queue_draw();
        }
    }
}

fn connect_channel_tabs(notebook: &gtk4::Notebook, shared: &Rc<Shared>) {
    {
        let shared = Rc::clone(shared);
        notebook.connect_switch_page(move |_, _, page| {
            if shared.suppress_updates.get() {
                return;
            }
            let Some(channel) = ColorZonesChannel::from_raw(i32::try_from(page).unwrap_or(-1))
            else {
                return;
            };
            let before = shared.snapshot.borrow().preferences();
            shared
                .interaction
                .borrow_mut()
                .editor_mut()
                .set_output_channel(channel);
            shared
                .snapshot
                .borrow_mut()
                .editor
                .set_output_channel(channel);
            let after = ColorZonesGtkPreferences::new(channel, before.graph_height());
            if !shared.persist_preferences(after) {
                shared
                    .interaction
                    .borrow_mut()
                    .editor_mut()
                    .set_output_channel(before.output_channel());
                shared
                    .snapshot
                    .borrow_mut()
                    .editor
                    .set_output_channel(before.output_channel());
            }
            shared.sync_widgets();
        });
    }

    let scroll = gtk4::EventControllerScroll::new(
        gtk4::EventControllerScrollFlags::BOTH_AXES | gtk4::EventControllerScrollFlags::KINETIC,
    );
    scroll.set_name(Some("dt-colorzones-channel-scroll"));
    scroll.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let notebook = notebook.downgrade();
        scroll.connect_scroll(move |controller, delta_x, delta_y| {
            let Some(notebook) = notebook.upgrade() else {
                return glib::Propagation::Stop;
            };
            if let Some(delta) = source_scroll_unit_delta(controller.unit(), delta_x, delta_y) {
                switch_output_page(&notebook, if delta < 0 { -1.0 } else { 1.0 });
            }
            glib::Propagation::Stop
        });
    }
    scroll.connect_scroll_end(|_| reset_source_scroll_units());
    notebook.add_controller(scroll);
}

fn connect_non_graph_controls(
    edit_by_area: &gtk4::CheckButton,
    select_by: &BauhausComboBox,
    mode: &BauhausComboBox,
    strength: &BauhausSlider,
    interpolator: &BauhausComboBox,
    shared: &Rc<Shared>,
) {
    {
        let shared = Rc::clone(shared);
        edit_by_area.connect_toggled(move |toggle| {
            if shared.suppress_updates.get() {
                return;
            }
            shared
                .interaction
                .borrow_mut()
                .set_edit_by_area(toggle.is_active());
            shared.queue_draw();
        });
    }
    {
        let shared = Rc::clone(shared);
        select_by.connect_selection_changed(move |choice| {
            if shared.suppress_updates.get() {
                return;
            }
            let Some(channel) = selected_channel(choice) else {
                return;
            };
            let before = shared.interaction.borrow().clone();
            shared
                .interaction
                .borrow_mut()
                .editor_mut()
                .set_selection_channel(channel);
            shared.settle(before);
        });
    }
    {
        let shared = Rc::clone(shared);
        mode.connect_selection_changed(move |choice| {
            if shared.suppress_updates.get() {
                return;
            }
            let Some(mode) = selected_combo_index(choice).and_then(ColorZonesMode::from_raw) else {
                return;
            };
            let before = shared.interaction.borrow().clone();
            shared.interaction.borrow_mut().editor_mut().set_mode(mode);
            shared.settle(before);
        });
    }
    {
        let shared = Rc::clone(shared);
        strength.connect_value_settled(move |value| {
            if shared.suppress_updates.get() {
                return;
            }
            let before = shared.interaction.borrow().clone();
            #[allow(
                clippy::cast_possible_truncation,
                reason = "the Bauhaus range is bounded to native f32 strength"
            )]
            let value = value as f32;
            if shared
                .interaction
                .borrow_mut()
                .editor_mut()
                .set_strength(value)
                .is_ok()
            {
                shared.settle(before);
            }
        });
    }
    {
        let shared = Rc::clone(shared);
        interpolator.connect_selection_changed(move |choice| {
            if shared.suppress_updates.get() {
                return;
            }
            let Some(curve_type) =
                selected_combo_index(choice).and_then(ColorZonesCurveType::from_raw)
            else {
                return;
            };
            let before = shared.interaction.borrow().clone();
            let mut interaction = shared.interaction.borrow_mut();
            let channel = interaction.editor().output_channel();
            interaction.editor_mut().set_curve_type(channel, curve_type);
            drop(interaction);
            shared.settle(before);
        });
    }
}

#[allow(
    deprecated,
    reason = "GTK4 still resolves the source named graph_overlay color through StyleContext"
)]
fn resolved_graph_overlay(widget: &impl IsA<gtk4::Widget>) -> Option<gdk::RGBA> {
    widget.style_context().lookup_color("graph_overlay")
}

#[allow(clippy::too_many_lines)]
fn connect_graph(graph: &gtk4::DrawingArea, shared: &Rc<Shared>) {
    {
        let shared = Rc::clone(shared);
        graph.set_draw_func(move |area, cairo, width, height| {
            let graph_overlay = resolved_graph_overlay(area);
            paint_graph(
                cairo,
                width,
                height,
                &shared.interaction.borrow(),
                graph_overlay.as_ref(),
            );
        });
    }

    let primary_down = Rc::new(Cell::new(false));
    let drag_origin = Rc::new(RefCell::new(None::<ColorZonesInteraction>));

    let motion = gtk4::EventControllerMotion::new();
    motion.set_name(Some("dt-colorzones-motion"));
    {
        let graph = graph.downgrade();
        let shared = Rc::clone(shared);
        let primary_down = Rc::clone(&primary_down);
        motion.connect_motion(move |controller, x, y| {
            let Some(graph) = graph.upgrade() else {
                return;
            };
            let Some((pointer_x, pointer_y, below_graph)) = graph_pointer(&graph, x, y) else {
                return;
            };
            let mut interaction = shared.interaction.borrow_mut();
            if primary_down.get() {
                let _ = interaction.primary_drag_to_with_speed(
                    pointer_x,
                    pointer_y,
                    modifier_speed(controller.current_event_state()),
                );
            } else {
                let _ = interaction.set_pointer(pointer_x, pointer_y);
                if interaction.edit_by_area() {
                    interaction.update_area_x_marker(below_graph);
                } else {
                    interaction.update_hover_selection();
                }
            }
            if matches!(interaction.selection(), ColorZonesSelection::Node(_)) {
                graph.grab_focus();
            }
            drop(interaction);
            graph.queue_draw();
        });
    }
    {
        let graph = graph.downgrade();
        let shared = Rc::clone(shared);
        motion.connect_leave(move |_| {
            shared.interaction.borrow_mut().leave();
            if let Some(graph) = graph.upgrade() {
                graph.queue_draw();
            }
        });
    }
    graph.add_controller(motion);

    let click = gtk4::GestureClick::new();
    click.set_name(Some("dt-colorzones-click"));
    click.set_button(1);
    click.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let graph = graph.downgrade();
        let shared = Rc::clone(shared);
        let primary_down = Rc::clone(&primary_down);
        let drag_origin = Rc::clone(&drag_origin);
        click.connect_pressed(move |gesture, count, x, y| {
            let Some(graph) = graph.upgrade() else {
                return;
            };
            let Some((pointer_x, pointer_y, _)) = graph_pointer(&graph, x, y) else {
                return;
            };
            primary_down.set(true);
            let before = shared.interaction.borrow().clone();
            let modifiers = normalized_modifiers(gesture.current_event_state());
            let sampled = sampled_active_curve(&before, pointer_x);
            let mut interaction = shared.interaction.borrow_mut();
            let _ = interaction.set_pointer(pointer_x, pointer_y);
            let click = if count == 2 {
                ColorZonesClick::Double
            } else {
                ColorZonesClick::Single
            };
            let outcome = interaction.primary_press(click, modifiers, sampled);
            drop(interaction);
            match outcome {
                Ok(
                    ColorZonesPrimaryOutcome::NodeInserted(_)
                    | ColorZonesPrimaryOutcome::CurveReset,
                ) => {
                    drag_origin.borrow_mut().take();
                    primary_down.set(false);
                    shared.settle(before);
                }
                Ok(
                    ColorZonesPrimaryOutcome::AreaDragStarted | ColorZonesPrimaryOutcome::Ignored,
                ) => {
                    *drag_origin.borrow_mut() = Some(before);
                }
                Ok(ColorZonesPrimaryOutcome::SampleOutsideViewport) | Err(_) => {
                    drag_origin.borrow_mut().take();
                    primary_down.set(false);
                    shared.queue_draw();
                }
            }
            let _ = gesture.set_state(gtk4::EventSequenceState::Claimed);
        });
    }
    {
        let shared = Rc::clone(shared);
        let primary_down = Rc::clone(&primary_down);
        let drag_origin = Rc::clone(&drag_origin);
        click.connect_released(move |_, _, _, _| {
            primary_down.set(false);
            shared.interaction.borrow_mut().primary_release();
            if let Some(before) = drag_origin.borrow_mut().take() {
                shared.settle(before);
            } else {
                shared.queue_draw();
            }
        });
    }
    {
        let shared = Rc::clone(shared);
        let primary_down = Rc::clone(&primary_down);
        let drag_origin = Rc::clone(&drag_origin);
        click.connect_cancel(move |_, _| {
            primary_down.set(false);
            if let Some(before) = drag_origin.borrow_mut().take() {
                *shared.interaction.borrow_mut() = before;
            }
            shared.queue_draw();
        });
    }
    graph.add_controller(click);

    let secondary = gtk4::GestureClick::new();
    secondary.set_name(Some("dt-colorzones-secondary-click"));
    secondary.set_button(3);
    secondary.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let graph = graph.downgrade();
        let shared = Rc::clone(shared);
        secondary.connect_pressed(move |gesture, _, x, y| {
            let Some(graph) = graph.upgrade() else {
                return;
            };
            if let Some((pointer_x, pointer_y, _)) = graph_pointer(&graph, x, y) {
                let before = shared.interaction.borrow().clone();
                let outcome = route_graph_secondary_press(
                    &mut shared.interaction.borrow_mut(),
                    pointer_x,
                    pointer_y,
                    gesture.current_event_state(),
                );
                if matches!(
                    outcome,
                    Ok(ColorZonesSecondaryOutcome::Deleted(_)
                        | ColorZonesSecondaryOutcome::Neutralized)
                ) {
                    shared.settle(before);
                } else {
                    shared.queue_draw();
                }
            }
            let _ = gesture.set_state(gtk4::EventSequenceState::Claimed);
        });
    }
    graph.add_controller(secondary);

    let scroll = gtk4::EventControllerScroll::new(
        gtk4::EventControllerScrollFlags::BOTH_AXES | gtk4::EventControllerScrollFlags::KINETIC,
    );
    scroll.set_name(Some("dt-colorzones-scroll"));
    scroll.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let shared = Rc::clone(shared);
        scroll.connect_scroll_begin(move |controller| {
            if controller
                .current_event()
                .is_some_and(|event| event.is_pointer_emulated())
            {
                return;
            }
            if controller.unit() != gdk::ScrollUnit::Wheel {
                shared.begin_graph_scroll_sequence();
            }
        });
    }
    {
        let shared = Rc::clone(shared);
        scroll.connect_scroll(move |controller, delta_x, delta_y| {
            if controller
                .current_event()
                .is_some_and(|event| event.is_pointer_emulated())
            {
                return glib::Propagation::Stop;
            }
            let _ = shared.route_raw_graph_scroll(
                controller.unit(),
                delta_x,
                delta_y,
                controller.current_event_state(),
            );
            glib::Propagation::Stop
        });
    }
    {
        let shared = Rc::clone(shared);
        scroll.connect_scroll_end(move |controller| {
            if controller
                .current_event()
                .is_some_and(|event| event.is_pointer_emulated())
            {
                return;
            }
            reset_source_scroll_units();
            shared.end_graph_scroll_sequence();
        });
    }
    graph.add_controller(scroll);

    let key = gtk4::EventControllerKey::new();
    key.set_name(Some("dt-colorzones-key"));
    key.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let shared = Rc::clone(shared);
        key.connect_key_pressed(move |_, key, _, modifiers| {
            let key = match key {
                gdk::Key::Up | gdk::Key::KP_Up => ColorZonesArrowKey::Up,
                gdk::Key::Down | gdk::Key::KP_Down => ColorZonesArrowKey::Down,
                gdk::Key::Left | gdk::Key::KP_Left => ColorZonesArrowKey::Left,
                gdk::Key::Right | gdk::Key::KP_Right => ColorZonesArrowKey::Right,
                _ => return glib::Propagation::Proceed,
            };
            let before = shared.interaction.borrow().clone();
            let outcome = shared
                .interaction
                .borrow_mut()
                .key_press_with_speed(key, modifier_speed(modifiers));
            match outcome {
                Ok(true) => shared.settle(before),
                Ok(false) | Err(_) => shared.queue_draw(),
            }
            glib::Propagation::Stop
        });
    }
    graph.add_controller(key);
}

fn connect_bottom(bottom: &gtk4::DrawingArea, shared: &Rc<Shared>) {
    {
        let shared = Rc::clone(shared);
        bottom.set_draw_func(move |area, cairo, width, height| {
            let graph_overlay = resolved_graph_overlay(area);
            paint_bottom_strip(
                cairo,
                width,
                height,
                &shared.interaction.borrow(),
                graph_overlay.as_ref(),
            );
        });
    }

    let click = gtk4::GestureClick::new();
    click.set_name(Some(COLORZONES_BOTTOM_CLICK_CONTROLLER_NAME));
    click.set_button(COLORZONES_PRIMARY_BUTTON);
    click.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let shared = Rc::clone(shared);
        click.connect_pressed(move |gesture, count, _, _| {
            let handled =
                route_bottom_primary_press(&mut shared.interaction.borrow_mut(), count, || {
                    shared.queue_graph_draw();
                });
            if handled {
                let _ = gesture.set_state(gtk4::EventSequenceState::Claimed);
            }
        });
    }
    bottom.add_controller(click);
}

fn route_graph_secondary_press(
    interaction: &mut ColorZonesInteraction,
    pointer_x: f32,
    pointer_y: f32,
    modifiers: gdk::ModifierType,
) -> Result<ColorZonesSecondaryOutcome, ColorZonesInteractionError> {
    if !interaction.edit_by_area() {
        interaction.set_pointer(pointer_x, pointer_y)?;
        interaction.update_hover_selection();
    }
    interaction.secondary_press(normalized_modifiers(modifiers))
}

fn route_bottom_primary_press(
    interaction: &mut ColorZonesInteraction,
    click_count: i32,
    redraw_graph: impl FnOnce(),
) -> bool {
    if click_count != 2 {
        return false;
    }
    interaction.reset_zoom();
    redraw_graph();
    true
}

fn source_scroll_unit_delta(unit: gdk::ScrollUnit, delta_x: f64, delta_y: f64) -> Option<i32> {
    normalize_source_scroll_unit_delta(unit, delta_x, delta_y)
}

fn route_graph_scroll_interaction(
    interaction: &mut ColorZonesInteraction,
    delta_y: f64,
    modifiers: gdk::ModifierType,
) -> Result<ColorZonesScrollOutcome, ColorZonesInteractionError> {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "GTK scroll deltas are immediately bounded by editor operations"
    )]
    let delta_y = delta_y as f32;
    interaction.scroll_with_speed(
        delta_y,
        normalized_modifiers(modifiers),
        false,
        modifier_speed(modifiers),
    )
}

fn graph_pointer(graph: &gtk4::DrawingArea, x: f64, y: f64) -> Option<(f32, f32, bool)> {
    let inset = f64::from(COLORZONES_GRAPH_INSET);
    let width = f64::from(graph.allocated_width()) - 2.0 * inset;
    let height = f64::from(graph.allocated_height()) - 2.0 * inset;
    if !x.is_finite() || !y.is_finite() || width <= 0.0 || height <= 0.0 {
        return None;
    }
    #[allow(
        clippy::cast_possible_truncation,
        reason = "normalized finite pointer coordinates fit f32"
    )]
    let pointer_x = ((x - inset).clamp(0.0, width) / width) as f32;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "normalized finite pointer coordinates fit f32"
    )]
    let pointer_y = (1.0 - (y - inset).clamp(0.0, height) / height) as f32;
    Some((pointer_x, pointer_y, y > inset + height))
}

fn sampled_active_curve(interaction: &ColorZonesInteraction, pointer_x: f32) -> Option<f32> {
    let model = ColorZonesRenderModel::new(interaction.editor()).ok()?;
    let curve_x = interaction.view_to_curve(pointer_x, interaction.offsets().0);
    let curve_y = model.sample(interaction.editor().output_channel(), curve_x)?;
    Some(interaction.curve_to_view(curve_y, interaction.offsets().1))
}

fn switch_output_page(notebook: &gtk4::Notebook, delta_y: f32) {
    if delta_y < 0.0 {
        notebook.next_page();
    } else if delta_y > 0.0 {
        notebook.prev_page();
    }
}

fn exact_height_resize_modifiers(modifiers: gdk::ModifierType) -> bool {
    let relevant = gdk::ModifierType::SHIFT_MASK
        | gdk::ModifierType::CONTROL_MASK
        | gdk::ModifierType::ALT_MASK
        | gdk::ModifierType::META_MASK;
    modifiers & relevant == gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::ALT_MASK
}

fn resized_graph_height(current: ColorZonesGraphHeight, delta_y: f64) -> ColorZonesGraphHeight {
    if !delta_y.is_finite() || delta_y == 0.0 {
        return current;
    }
    #[allow(
        clippy::cast_possible_truncation,
        reason = "source-normalized scroll units are integral and bounded to i32"
    )]
    let delta = delta_y
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX))
        .trunc() as i32;
    let current = i32::from(current.logical_pixels());
    let bounded = current.saturating_add(delta).clamp(
        i32::from(COLORZONES_GRAPH_HEIGHT_MIN),
        i32::from(COLORZONES_GRAPH_HEIGHT_MAX),
    );
    ColorZonesGraphHeight::new(u16::try_from(bounded).expect("bounded graph height fits u16"))
        .expect("bounded graph height is valid")
}

fn selected_combo_index(choice: &BauhausComboBox) -> Option<i32> {
    let selected = usize::try_from(choice.selected()).ok()?;
    if selected >= choice.option_count() {
        return None;
    }
    choice.option_label(selected)?;
    i32::try_from(selected).ok()
}

fn selected_channel(choice: &BauhausComboBox) -> Option<ColorZonesChannel> {
    selected_combo_index(choice).and_then(ColorZonesChannel::from_raw)
}

fn channel_index(channel: ColorZonesChannel) -> u32 {
    u32::try_from(channel.raw()).expect("Color Zones channel index is nonnegative")
}

fn normalized_modifiers(modifiers: gdk::ModifierType) -> ColorZonesModifiers {
    // `ColorZonesModifiers` stores only the three source-relevant exact-match
    // bits. Keep Meta as an additional non-Control bit so Command cannot turn
    // into an exact Ctrl gesture or an exact Alt tab-switch gesture.
    let shift_or_meta =
        modifiers.intersects(gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::META_MASK);
    ColorZonesModifiers::new(
        shift_or_meta,
        modifiers.contains(gdk::ModifierType::CONTROL_MASK),
        modifiers.contains(gdk::ModifierType::ALT_MASK),
    )
}

fn modifier_speed(modifiers: gdk::ModifierType) -> f32 {
    #[cfg(target_os = "macos")]
    let primary = gdk::ModifierType::META_MASK;
    #[cfg(not(target_os = "macos"))]
    let primary = gdk::ModifierType::CONTROL_MASK;
    let relevant = primary
        | gdk::ModifierType::SHIFT_MASK
        | gdk::ModifierType::CONTROL_MASK
        | gdk::ModifierType::ALT_MASK;
    let cleaned = modifiers & relevant;
    if cleaned == primary {
        0.1
    } else if cleaned == gdk::ModifierType::SHIFT_MASK
        || cleaned == primary | gdk::ModifierType::SHIFT_MASK
    {
        10.0
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use rusttable_core::{OperationId, OperationOpacity, Revision};

    use super::super::COLORZONES_GRAPH_HEIGHT_DEFAULT;
    use super::*;
    use crate::iop::colorcorrection::{
        colorcorrection_scroll_end, colorcorrection_scroll_unit_delta,
    };

    #[test]
    fn source_labels_options_tooltips_and_ranges_are_exact() {
        assert_eq!(COLORZONES_OUTPUT_LABELS, ["lightness", "chroma", "hue"]);
        assert_eq!(COLORZONES_SELECTION_LABEL, "select by");
        assert_eq!(COLORZONES_MODE_OPTIONS, ["smooth", "strong"]);
        assert_eq!(COLORZONES_STRENGTH_LABEL, "mix");
        assert_eq!(COLORZONES_INTERPOLATION_LABEL, "interpolation method");
        assert_eq!(
            COLORZONES_INTERPOLATION_OPTIONS,
            ["cubic spline", "centripetal spline", "monotonic spline"]
        );
        assert!(COLORZONES_INTERPOLATION_TOOLTIP.contains("oscillations or cusps"));
        assert_eq!(COLORZONES_GRAPH_HEIGHT_DEFAULT, 200);
        assert_eq!(COLORZONES_GRAPH_INSET.to_bits(), 5.0_f32.to_bits());
    }

    #[test]
    fn bottom_primary_double_click_resets_zoom_and_requests_only_graph_redraw() {
        assert_eq!(
            COLORZONES_BOTTOM_CLICK_CONTROLLER_NAME,
            "dt-colorzones-bottom-click"
        );
        assert_eq!(COLORZONES_PRIMARY_BUTTON, 1);

        let mut interaction = ColorZonesInteraction::default();
        interaction.set_pointer(0.75, 0.25).unwrap();
        interaction.zoom_at_pointer(-1.0).unwrap();
        let redraws = Cell::new(0_u8);

        assert!(!route_bottom_primary_press(&mut interaction, 1, || {
            redraws.set(redraws.get() + 1);
        }));
        assert!(interaction.zoom_factor() > 1.0);
        assert_eq!(redraws.get(), 0);

        assert!(route_bottom_primary_press(&mut interaction, 2, || {
            redraws.set(redraws.get() + 1);
        }));
        assert_eq!(interaction.zoom_factor().to_bits(), 1.0_f32.to_bits());
        assert_eq!(interaction.offsets(), (0.0, 0.0));
        assert_eq!(redraws.get(), 1);
    }

    #[test]
    fn exact_source_control_gestures_do_not_accept_meta() {
        assert!(
            normalized_modifiers(gdk::ModifierType::CONTROL_MASK)
                .is_exact(ColorZonesModifiers::CONTROL)
        );
        assert!(
            !normalized_modifiers(gdk::ModifierType::META_MASK)
                .is_exact(ColorZonesModifiers::CONTROL)
        );
        assert!(
            !normalized_modifiers(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::META_MASK)
                .is_exact(ColorZonesModifiers::CONTROL)
        );
    }

    #[test]
    fn edit_by_area_secondary_press_does_not_hit_test_the_event_position() {
        let mut without_selection = ColorZonesInteraction::default();
        without_selection.set_edit_by_area(true);
        let before = without_selection.editor().parameters_value();
        assert_eq!(
            route_graph_secondary_press(
                &mut without_selection,
                0.25,
                0.5,
                gdk::ModifierType::empty(),
            )
            .unwrap(),
            ColorZonesSecondaryOutcome::Ignored
        );
        assert_eq!(without_selection.selection(), ColorZonesSelection::None);
        assert_eq!(without_selection.editor().parameters_value(), before);

        let mut retained_selection = ColorZonesInteraction::default();
        retained_selection.set_pointer(0.25, 0.5).unwrap();
        retained_selection.update_hover_selection();
        assert!(matches!(
            retained_selection.selection(),
            ColorZonesSelection::Node(_)
        ));
        retained_selection.set_edit_by_area(true);
        assert!(matches!(
            route_graph_secondary_press(
                &mut retained_selection,
                0.5,
                0.9,
                gdk::ModifierType::empty(),
            )
            .unwrap(),
            ColorZonesSecondaryOutcome::Deleted(_)
        ));
    }

    #[test]
    fn source_scroll_units_discretize_wheels_and_accumulate_smooth_fractions() {
        reset_source_scroll_units();
        assert_eq!(
            source_scroll_unit_delta(gdk::ScrollUnit::Wheel, 0.0, -8.0),
            Some(-1)
        );
        assert_eq!(
            source_scroll_unit_delta(gdk::ScrollUnit::Wheel, 0.0, 0.25),
            Some(1)
        );

        #[cfg(target_os = "macos")]
        let half_unit = 25.0;
        #[cfg(not(target_os = "macos"))]
        let half_unit = 0.5;
        assert_eq!(
            source_scroll_unit_delta(gdk::ScrollUnit::Surface, 0.0, half_unit,),
            None
        );
        assert_eq!(
            source_scroll_unit_delta(gdk::ScrollUnit::Surface, 0.0, half_unit,),
            Some(1)
        );
        reset_source_scroll_units();
        assert_eq!(
            source_scroll_unit_delta(gdk::ScrollUnit::Surface, 0.0, half_unit,),
            None
        );
    }

    #[test]
    fn graph_and_colorcorrection_share_fractions_and_either_stop_clears_them() {
        reset_source_scroll_units();
        let state = ColorZonesGtkState::new(
            OperationId::new(95).expect("operation ID"),
            Revision::from_u64(1),
            ColorZonesEditorState::default(),
            true,
            OperationOpacity::ONE,
            true,
            false,
        );
        let interaction = ColorZonesInteraction::new(state.editor().clone());
        let graph = Shared {
            snapshot: RefCell::new(state),
            interaction: RefCell::new(interaction),
            handler: None,
            preferences_handler: RefCell::new(None),
            suppress_updates: Cell::new(false),
            widgets: RefCell::new(None),
            scroll_origin: RefCell::new(None),
        };
        #[cfg(target_os = "macos")]
        let half_unit = 25.0;
        #[cfg(not(target_os = "macos"))]
        let half_unit = 0.5;

        assert_eq!(
            graph.route_raw_graph_scroll(
                gdk::ScrollUnit::Surface,
                0.0,
                half_unit,
                gdk::ModifierType::empty(),
            ),
            Ok(None),
            "the Color Zones graph retains the first half unit"
        );
        assert_eq!(
            colorcorrection_scroll_unit_delta(gdk::ScrollUnit::Surface, 0.0, half_unit),
            Some(1),
            "Color Correction emits the graph's retained fraction through the shared helper"
        );

        assert_eq!(
            graph.route_raw_graph_scroll(
                gdk::ScrollUnit::Surface,
                0.0,
                half_unit,
                gdk::ModifierType::empty(),
            ),
            Ok(None)
        );
        colorcorrection_scroll_end();
        assert_eq!(
            colorcorrection_scroll_unit_delta(gdk::ScrollUnit::Surface, 0.0, half_unit),
            None,
            "a stop from another production caller clears the graph's global remainder"
        );
        reset_source_scroll_units();
    }

    #[test]
    fn explicit_graph_scroll_routing_preserves_exact_alt_only_precedence() {
        let mut interaction = ColorZonesInteraction::default();
        assert_eq!(
            route_graph_scroll_interaction(&mut interaction, -1.0, gdk::ModifierType::ALT_MASK,)
                .unwrap(),
            ColorZonesScrollOutcome::ForwardToChannelTabs { delta_y: -1.0 }
        );
        assert_eq!(interaction.zoom_factor().to_bits(), 1.0_f32.to_bits());

        assert_eq!(
            route_graph_scroll_interaction(
                &mut interaction,
                -1.0,
                gdk::ModifierType::ALT_MASK | gdk::ModifierType::SHIFT_MASK,
            )
            .unwrap(),
            ColorZonesScrollOutcome::Consumed
        );
    }

    #[test]
    fn exact_shift_alt_resizes_one_logical_pixel_with_native_bounds() {
        let resize = gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::ALT_MASK;
        assert!(exact_height_resize_modifiers(resize));
        assert!(!exact_height_resize_modifiers(gdk::ModifierType::ALT_MASK));
        assert!(!exact_height_resize_modifiers(
            resize | gdk::ModifierType::CONTROL_MASK
        ));
        assert!(!exact_height_resize_modifiers(
            resize | gdk::ModifierType::META_MASK
        ));

        let default = ColorZonesGraphHeight::default();
        assert_eq!(
            resized_graph_height(default, -8.0).logical_pixels(),
            COLORZONES_GRAPH_HEIGHT_DEFAULT - 8
        );
        assert_eq!(
            resized_graph_height(default, 0.25).logical_pixels(),
            COLORZONES_GRAPH_HEIGHT_DEFAULT
        );
        let minimum = ColorZonesGraphHeight::new(COLORZONES_GRAPH_HEIGHT_MIN).expect("minimum");
        let maximum = ColorZonesGraphHeight::new(COLORZONES_GRAPH_HEIGHT_MAX).expect("maximum");
        assert_eq!(
            resized_graph_height(minimum, -1.0).logical_pixels(),
            COLORZONES_GRAPH_HEIGHT_MIN
        );
        assert_eq!(
            resized_graph_height(maximum, 1.0).logical_pixels(),
            COLORZONES_GRAPH_HEIGHT_MAX
        );
    }

    #[test]
    fn graph_resize_persists_durable_channel_and_height_or_rolls_back() {
        let state = ColorZonesGtkState::new(
            OperationId::new(97).expect("operation ID"),
            Revision::from_u64(3),
            ColorZonesEditorState::with_output_channel(ColorZonesChannel::Hue),
            true,
            OperationOpacity::ONE,
            true,
            false,
        );
        let observed = Rc::new(RefCell::new(Vec::new()));
        let observed_for_handler = Rc::clone(&observed);
        let handler: ColorZonesGtkPreferencesHandler = Rc::new(move |preferences| {
            observed_for_handler.borrow_mut().push(preferences);
            true
        });
        let shared = Shared {
            snapshot: RefCell::new(state.clone()),
            interaction: RefCell::new(ColorZonesInteraction::new(state.editor().clone())),
            handler: None,
            preferences_handler: RefCell::new(Some(handler)),
            suppress_updates: Cell::new(false),
            widgets: RefCell::new(None),
            scroll_origin: RefCell::new(None),
        };
        let resize = gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::ALT_MASK;

        assert_eq!(
            shared.route_graph_scroll(1.0, resize),
            Ok(ColorZonesScrollOutcome::Consumed)
        );
        assert_eq!(
            shared.snapshot.borrow().graph_height().logical_pixels(),
            COLORZONES_GRAPH_HEIGHT_DEFAULT + 1
        );
        assert_eq!(
            observed.borrow().as_slice(),
            &[ColorZonesGtkPreferences::new(
                ColorZonesChannel::Hue,
                ColorZonesGraphHeight::new(COLORZONES_GRAPH_HEIGHT_DEFAULT + 1)
                    .expect("resized height"),
            )]
        );

        shared.preferences_handler.replace(Some(Rc::new(|_| false)));
        assert_eq!(
            shared.route_graph_scroll(1.0, resize),
            Ok(ColorZonesScrollOutcome::Consumed)
        );
        assert_eq!(
            shared.snapshot.borrow().graph_height().logical_pixels(),
            COLORZONES_GRAPH_HEIGHT_DEFAULT + 1,
            "failed persistence restores the last durable height"
        );
    }

    #[test]
    fn settled_action_freezes_exact_target_revision_parameters_and_requirements() {
        let operation_id = OperationId::new(91).expect("operation ID");
        let revision = Revision::from_u64(7);
        let editor = ColorZonesEditorState::default();
        let state = ColorZonesGtkState::new(
            operation_id,
            revision,
            editor.clone(),
            false,
            OperationOpacity::ONE,
            true,
            true,
        );
        assert_eq!(state.operation_id(), operation_id);
        assert_eq!(state.revision(), revision);
        assert!(!state.enabled());
        assert_eq!(state.opacity(), OperationOpacity::ONE);
        assert!(state.sensitive());
        assert!(state.materialization_required());

        let action = ColorZonesSettledAction {
            target: state.operation_id(),
            expected_revision: state.revision(),
            output_channel: ColorZonesChannel::Lightness,
            parameters: editor.parameters_value(),
            enable_required: !state.enabled(),
            materialization_required: state.materialization_required(),
        };
        assert_eq!(action.target(), operation_id);
        assert_eq!(action.expected_revision(), revision);
        assert_eq!(action.output_channel(), ColorZonesChannel::Lightness);
        assert_eq!(action.parameters(), editor.parameters_value());
        assert!(action.enable_required());
        assert!(action.materialization_required());
    }
}
