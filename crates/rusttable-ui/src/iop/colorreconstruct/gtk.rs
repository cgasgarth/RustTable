//! GTK4 leaf for `src/iop/colorreconstruction.c::{gui_init,gui_changed,gui_update}`.
//!
//! Shared Bauhaus controls own pointer, scroll, keyboard, popup, formatting, and
//! per-control reset behavior. This adapter synchronizes source state in place,
//! applies the hue-precedence and monochrome transitions, and emits one action
//! only at the shared settled boundary.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk4::prelude::*;
use rusttable_core::{OperationId, Revision};
use rusttable_processing::operations::colorreconstruction::{
    ColorReconstructionPrecedence, ColorReconstructionV3,
};

use crate::bauhaus::{
    combobox::{BauhausComboBox, BauhausComboBoxSpec},
    slider_input::{BauhausSlider, FullWidthSliderSpec, SliderInputSpec, full_width_slider},
};

use super::{
    COLORRECONSTRUCTION_HUE_SLIDER, COLORRECONSTRUCTION_HUE_STOPS,
    COLORRECONSTRUCTION_MONOCHROME_LABEL, COLORRECONSTRUCTION_MONOCHROME_TOOLTIP,
    COLORRECONSTRUCTION_PRECEDENCE_LABEL, COLORRECONSTRUCTION_PRECEDENCE_OPTIONS,
    COLORRECONSTRUCTION_PRECEDENCE_TOOLTIP, COLORRECONSTRUCTION_RANGE_SLIDER,
    COLORRECONSTRUCTION_SLIDERS, COLORRECONSTRUCTION_SPATIAL_SLIDER,
    COLORRECONSTRUCTION_THRESHOLD_SLIDER, ColorReconstructionEditorState,
    ColorReconstructionParameter, ColorReconstructionSliderSpec,
};

const DEFAULT_STACK_CHILD: &str = "default";
const MONOCHROME_STACK_CHILD: &str = "monochrome";

/// Complete UI-owned snapshot for one Color Reconstruction leaf.
#[expect(
    clippy::struct_excessive_bools,
    reason = "The GTK snapshot carries independent enablement, sensitivity, materialization, and mode flags."
)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorReconstructionGtkState {
    operation_id: OperationId,
    revision: Revision,
    editor: ColorReconstructionEditorState,
    enabled: bool,
    sensitive: bool,
    materialization_required: bool,
    monochrome: bool,
}

impl ColorReconstructionGtkState {
    #[must_use]
    pub const fn new(
        operation_id: OperationId,
        revision: Revision,
        editor: ColorReconstructionEditorState,
        enabled: bool,
        sensitive: bool,
        materialization_required: bool,
    ) -> Self {
        Self {
            operation_id,
            revision,
            editor,
            enabled,
            sensitive,
            materialization_required,
            monochrome: false,
        }
    }

    /// Applies the image's current monochrome classification. Native
    /// `gui_update` swaps the complete control box for its explanation label.
    #[must_use]
    pub const fn with_monochrome(mut self, monochrome: bool) -> Self {
        self.monochrome = monochrome;
        self
    }

    #[must_use]
    pub const fn operation_id(self) -> OperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn editor(self) -> ColorReconstructionEditorState {
        self.editor
    }

    #[must_use]
    pub const fn enabled(self) -> bool {
        self.enabled
    }

    #[must_use]
    pub const fn sensitive(self) -> bool {
        self.sensitive
    }

    #[must_use]
    pub const fn materialization_required(self) -> bool {
        self.materialization_required
    }

    #[must_use]
    pub const fn monochrome(self) -> bool {
        self.monochrome
    }

    /// Native `gui_update` hides the module enable button for monochrome input.
    #[must_use]
    pub const fn hide_enable_button(self) -> bool {
        self.monochrome
    }
}

/// One settled canonical parameter replacement authored by this leaf.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorReconstructionSettledAction {
    target: OperationId,
    expected_revision: Revision,
    parameters: ColorReconstructionV3,
    enable_required: bool,
    materialization_required: bool,
}

impl ColorReconstructionSettledAction {
    #[must_use]
    pub const fn target(self) -> OperationId {
        self.target
    }

    #[must_use]
    pub const fn expected_revision(self) -> Revision {
        self.expected_revision
    }

    #[must_use]
    pub const fn parameters(self) -> ColorReconstructionV3 {
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
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorReconstructionGtkHandlerOutcome {
    /// Keep the candidate editor state and advance to the committed revision.
    Commit { revision: Revision },
    /// Restore the exact state from before the attempted action.
    Rollback,
    /// Replace target, revision, parameters, and presentation state in place.
    Reconcile(ColorReconstructionGtkState),
}

pub type ColorReconstructionGtkActionHandler =
    Rc<dyn Fn(ColorReconstructionSettledAction) -> ColorReconstructionGtkHandlerOutcome>;

/// Mounted Color Reconstruction leaf with stable identities and in-place sync.
#[derive(Clone)]
pub struct ColorReconstructionGtkLeaf {
    root: gtk4::Stack,
    shared: Rc<Shared>,
}

impl ColorReconstructionGtkLeaf {
    #[must_use]
    pub const fn widget(&self) -> &gtk4::Stack {
        &self.root
    }

    #[must_use]
    pub fn state(&self) -> ColorReconstructionGtkState {
        *self.shared.snapshot.borrow()
    }

    /// Replaces persisted and image presentation state without rebuilding any
    /// control or changing its stable identity.
    pub fn reconcile(&self, state: ColorReconstructionGtkState) {
        *self.shared.snapshot.borrow_mut() = state;
        self.shared.sync_widgets();
    }

    /// Routes the generic module reset through the same settled action boundary.
    pub fn reset(&self) {
        let before = *self.shared.snapshot.borrow();
        let mut candidate = before.editor();
        if candidate.reset() {
            self.shared.settle_candidate(before, candidate);
        } else {
            self.shared.sync_widgets();
        }
    }

    /// Returns a mounted slider for stable identity and boundary inspection.
    #[must_use]
    pub fn slider(&self, parameter: ColorReconstructionParameter) -> gtk4::Scale {
        self.shared.widgets.slider(parameter).scale().clone()
    }

    /// Returns the mounted precedence selection widget.
    #[must_use]
    pub fn precedence_dropdown(&self) -> gtk4::DropDown {
        self.shared.widgets.precedence.dropdown().clone()
    }

    #[must_use]
    pub fn hue_visible(&self) -> bool {
        self.shared.widgets.hue.widget().is_visible()
    }

    #[must_use]
    pub fn visible_stack_child_name(&self) -> Option<gtk4::glib::GString> {
        self.root.visible_child_name()
    }
}

/// Builds source-ordered Bauhaus controls and the monochrome replacement stack.
#[must_use]
pub fn build_colorreconstruction_gtk(
    widget_id: &str,
    state: ColorReconstructionGtkState,
    handler: Option<ColorReconstructionGtkActionHandler>,
) -> ColorReconstructionGtkLeaf {
    let controls = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    controls.set_widget_name(&format!("{widget_id}-colorreconstruction-controls"));
    controls.set_hexpand(true);

    let threshold = build_slider(
        widget_id,
        COLORRECONSTRUCTION_THRESHOLD_SLIDER,
        state.editor(),
    );
    let spatial = build_slider(
        widget_id,
        COLORRECONSTRUCTION_SPATIAL_SLIDER,
        state.editor(),
    );
    let range = build_slider(widget_id, COLORRECONSTRUCTION_RANGE_SLIDER, state.editor());
    let precedence = BauhausComboBox::new(
        &format!("{widget_id}-precedence"),
        BauhausComboBoxSpec::new(
            COLORRECONSTRUCTION_PRECEDENCE_LABEL,
            COLORRECONSTRUCTION_PRECEDENCE_TOOLTIP,
            &COLORRECONSTRUCTION_PRECEDENCE_OPTIONS,
        ),
    );
    precedence.set_selected(precedence_index(state.editor().precedence()));
    let hue = build_slider(widget_id, COLORRECONSTRUCTION_HUE_SLIDER, state.editor());
    install_hue_gradient(hue.scale());
    hue.scale().set_show_fill_level(false);

    controls.append(threshold.widget());
    controls.append(spatial.widget());
    controls.append(range.widget());
    controls.append(precedence.widget());
    controls.append(hue.widget());

    let monochrome = gtk4::Label::new(Some(COLORRECONSTRUCTION_MONOCHROME_LABEL));
    monochrome.set_widget_name(&format!("{widget_id}-colorreconstruction-monochrome"));
    monochrome.set_halign(gtk4::Align::Start);
    monochrome.set_xalign(0.0);
    monochrome.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    monochrome.set_tooltip_text(Some(COLORRECONSTRUCTION_MONOCHROME_TOOLTIP));
    monochrome.update_property(&[
        gtk4::accessible::Property::Label(COLORRECONSTRUCTION_MONOCHROME_LABEL),
        gtk4::accessible::Property::Description(COLORRECONSTRUCTION_MONOCHROME_TOOLTIP),
    ]);

    let root = gtk4::Stack::new();
    root.set_widget_name(&format!("{widget_id}-colorreconstruction-editor"));
    root.set_hexpand(true);
    root.set_hhomogeneous(false);
    root.set_vhomogeneous(false);
    root.add_named(&monochrome, Some(MONOCHROME_STACK_CHILD));
    root.add_named(&controls, Some(DEFAULT_STACK_CHILD));

    let shared = Rc::new(Shared {
        snapshot: RefCell::new(state),
        handler,
        suppress_updates: Cell::new(false),
        widgets: Widgets {
            root: root.downgrade(),
            threshold,
            spatial,
            range,
            precedence,
            hue,
        },
    });
    connect_slider(ColorReconstructionParameter::Threshold, &shared);
    connect_slider(ColorReconstructionParameter::Spatial, &shared);
    connect_slider(ColorReconstructionParameter::Range, &shared);
    connect_precedence(&shared);
    connect_slider(ColorReconstructionParameter::Hue, &shared);
    shared.sync_widgets();

    ColorReconstructionGtkLeaf { root, shared }
}

fn build_slider(
    widget_id: &str,
    spec: ColorReconstructionSliderSpec,
    editor: ColorReconstructionEditorState,
) -> BauhausSlider {
    let (minimum, maximum) = spec.range();
    let mut input = SliderInputSpec::IDENTITY
        .with_suffix(spec.suffix())
        .with_soft_range(minimum, maximum)
        .with_default_value(spec.default_value())
        .with_digits(spec.digits())
        .with_automatic_step();
    input.factor = spec.factor();
    let slider = full_width_slider(FullWidthSliderSpec {
        widget_name: &spec.widget_name(widget_id),
        label: spec.label(),
        tooltip: spec.tooltip(),
        minimum,
        maximum,
        gtk_step: spec.gtk_step(),
        value: f64::from(editor.value(spec.parameter())),
        input,
    });
    slider
        .scale()
        .update_property(&[gtk4::accessible::Property::Description(spec.tooltip())]);
    slider
}

fn install_hue_gradient(scale: &gtk4::Scale) {
    const HUE_GRADIENT_CLASS: &str = "dt_colorreconstruction_hue_gradient";

    let stops = COLORRECONSTRUCTION_HUE_STOPS
        .iter()
        .map(|stop| {
            let [red, green, blue] = stop.rgb();
            format!(
                "rgb({:.0}%, {:.0}%, {:.0}%) {:.1}%",
                red * 100.0,
                green * 100.0,
                blue * 100.0,
                stop.position() * 100.0
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let provider = gtk4::CssProvider::new();
    provider.load_from_data(&format!(
        ".{HUE_GRADIENT_CLASS} trough {{ background-image: linear-gradient(to right, {stops}); }}"
    ));
    gtk4::style_context_add_provider_for_display(
        &scale.display(),
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
    );
    scale.add_css_class(HUE_GRADIENT_CLASS);
}

fn connect_slider(parameter: ColorReconstructionParameter, shared: &Rc<Shared>) {
    let weak = Rc::downgrade(shared);
    shared
        .widgets
        .slider(parameter)
        .connect_value_settled(move |value| {
            let Some(shared) = weak.upgrade() else {
                return;
            };
            if shared.suppress_updates.get() {
                return;
            }
            #[expect(
                clippy::cast_possible_truncation,
                reason = "The Bauhaus range is finite and bounded to native f32 parameters."
            )]
            shared.settle_scalar(parameter, value as f32);
        });
}

fn connect_precedence(shared: &Rc<Shared>) {
    let weak = Rc::downgrade(shared);
    shared
        .widgets
        .precedence
        .connect_selection_changed(move |combobox| {
            let Some(shared) = weak.upgrade() else {
                return;
            };
            if shared.suppress_updates.get() {
                return;
            }
            let Some(precedence) = precedence_from_index(combobox.selected()) else {
                shared.sync_widgets();
                return;
            };
            shared.settle_precedence(precedence);
        });
}

struct Shared {
    snapshot: RefCell<ColorReconstructionGtkState>,
    handler: Option<ColorReconstructionGtkActionHandler>,
    suppress_updates: Cell<bool>,
    widgets: Widgets,
}

struct Widgets {
    root: gtk4::glib::WeakRef<gtk4::Stack>,
    threshold: BauhausSlider,
    spatial: BauhausSlider,
    range: BauhausSlider,
    precedence: BauhausComboBox,
    hue: BauhausSlider,
}

impl Widgets {
    const fn slider(&self, parameter: ColorReconstructionParameter) -> &BauhausSlider {
        match parameter {
            ColorReconstructionParameter::Threshold => &self.threshold,
            ColorReconstructionParameter::Spatial => &self.spatial,
            ColorReconstructionParameter::Range => &self.range,
            ColorReconstructionParameter::Hue => &self.hue,
        }
    }
}

const fn settled_action(
    before: ColorReconstructionGtkState,
    candidate: ColorReconstructionEditorState,
) -> ColorReconstructionSettledAction {
    ColorReconstructionSettledAction {
        target: before.operation_id(),
        expected_revision: before.revision(),
        parameters: candidate.parameters(),
        enable_required: !before.enabled(),
        materialization_required: before.materialization_required(),
    }
}

const fn committed_state(
    before: ColorReconstructionGtkState,
    candidate: ColorReconstructionEditorState,
    revision: Revision,
    action: ColorReconstructionSettledAction,
) -> ColorReconstructionGtkState {
    ColorReconstructionGtkState {
        operation_id: before.operation_id(),
        revision,
        editor: candidate,
        enabled: before.enabled() || action.enable_required(),
        sensitive: before.sensitive(),
        // A successful settled action has been accepted by persistence. If the
        // operation was only a materialization placeholder, it is now real.
        materialization_required: false,
        monochrome: before.monochrome(),
    }
}

impl Shared {
    fn settle_scalar(&self, parameter: ColorReconstructionParameter, value: f32) {
        let before = *self.snapshot.borrow();
        let mut candidate = before.editor();
        let Ok(changed) = candidate.set(parameter, value) else {
            self.sync_widgets();
            return;
        };
        if changed {
            self.settle_candidate(before, candidate);
        } else {
            self.sync_widgets();
        }
    }

    fn settle_precedence(&self, precedence: ColorReconstructionPrecedence) {
        let before = *self.snapshot.borrow();
        let mut candidate = before.editor();
        let Ok(changed) = candidate.set_precedence(precedence) else {
            self.sync_widgets();
            return;
        };
        if changed {
            self.settle_candidate(before, candidate);
        } else {
            self.sync_widgets();
        }
    }

    fn settle_candidate(
        &self,
        before: ColorReconstructionGtkState,
        candidate: ColorReconstructionEditorState,
    ) {
        let action = settled_action(before, candidate);
        let outcome = self
            .handler
            .as_ref()
            .map_or(ColorReconstructionGtkHandlerOutcome::Rollback, |handler| {
                handler(action)
            });
        let next = match outcome {
            ColorReconstructionGtkHandlerOutcome::Commit { revision } => {
                committed_state(before, candidate, revision, action)
            }
            ColorReconstructionGtkHandlerOutcome::Rollback => before,
            ColorReconstructionGtkHandlerOutcome::Reconcile(state) => state,
        };
        *self.snapshot.borrow_mut() = next;
        self.sync_widgets();
    }

    fn sync_widgets(&self) {
        let snapshot = *self.snapshot.borrow();
        self.suppress_updates.set(true);
        if let Some(root) = self.widgets.root.upgrade() {
            root.set_sensitive(snapshot.sensitive());
            root.set_visible_child_name(if snapshot.monochrome() {
                MONOCHROME_STACK_CHILD
            } else {
                DEFAULT_STACK_CHILD
            });
        }
        for spec in COLORRECONSTRUCTION_SLIDERS {
            self.widgets
                .slider(spec.parameter())
                .set_value(f64::from(snapshot.editor().value(spec.parameter())));
        }
        self.widgets
            .precedence
            .set_selected(precedence_index(snapshot.editor().precedence()));
        self.widgets
            .hue
            .widget()
            .set_visible(snapshot.editor().hue_control_visible());
        self.suppress_updates.set(false);
    }
}

fn precedence_index(precedence: ColorReconstructionPrecedence) -> u32 {
    u32::try_from(precedence.id()).expect("native precedence IDs are nonnegative")
}

fn precedence_from_index(index: u32) -> Option<ColorReconstructionPrecedence> {
    i32::try_from(index)
        .ok()
        .and_then(|id| ColorReconstructionPrecedence::from_id(id).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_indices_preserve_native_enum_order() {
        assert_eq!(precedence_index(ColorReconstructionPrecedence::None), 0);
        assert_eq!(precedence_index(ColorReconstructionPrecedence::Chroma), 1);
        assert_eq!(precedence_index(ColorReconstructionPrecedence::Hue), 2);
        assert_eq!(
            precedence_from_index(0),
            Some(ColorReconstructionPrecedence::None)
        );
        assert_eq!(
            precedence_from_index(1),
            Some(ColorReconstructionPrecedence::Chroma)
        );
        assert_eq!(
            precedence_from_index(2),
            Some(ColorReconstructionPrecedence::Hue)
        );
        assert_eq!(precedence_from_index(3), None);
        assert_eq!(precedence_from_index(u32::MAX), None);
    }

    #[test]
    fn settled_action_preserves_revision_target_and_materialization_contract() {
        let before = ColorReconstructionGtkState::new(
            OperationId::new(7).expect("nonzero operation id"),
            Revision::from_u64(11),
            ColorReconstructionEditorState::default(),
            false,
            true,
            true,
        )
        .with_monochrome(true);
        let mut candidate = before.editor();
        candidate
            .set(ColorReconstructionParameter::Threshold, 125.0)
            .expect("threshold remains in the native range");

        let action = settled_action(before, candidate);
        assert_eq!(action.target(), before.operation_id());
        assert_eq!(action.expected_revision(), before.revision());
        assert_eq!(action.parameters(), candidate.parameters());
        assert!(action.enable_required());
        assert!(action.materialization_required());
    }

    #[test]
    fn committed_state_enables_and_materializes_without_losing_presentation_state() {
        let before = ColorReconstructionGtkState::new(
            OperationId::new(7).expect("nonzero operation id"),
            Revision::from_u64(11),
            ColorReconstructionEditorState::default(),
            false,
            false,
            true,
        )
        .with_monochrome(true);
        let candidate = before.editor();
        let action = settled_action(before, candidate);
        let committed = committed_state(before, candidate, Revision::from_u64(12), action);

        assert_eq!(committed.operation_id(), before.operation_id());
        assert_eq!(committed.revision(), Revision::from_u64(12));
        assert_eq!(committed.editor(), candidate);
        assert!(committed.enabled());
        assert!(!committed.sensitive());
        assert!(!committed.materialization_required());
        assert!(committed.monochrome());
        assert!(committed.hide_enable_button());
    }
}
