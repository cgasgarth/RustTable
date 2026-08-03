//! Source-shaped GTK4 editor for Darktable `src/iop/soften.c`.
//!
//! The native `gui_init` creates four Bauhaus sliders in parameter declaration
//! order. This leaf keeps that order and the native control metadata together,
//! validates edits with the existing Soften processing contract, and exposes an
//! optional callback boundary without pretending that persistence is wired here.

use std::{cell::RefCell, rc::Rc};

use gtk4::prelude::*;
use rusttable_core::{OperationId, Revision};
use rusttable_processing::operations::soften::{
    SoftenConfig, SoftenParameterError, SoftenParametersV1,
};

use crate::bauhaus::slider_input::{
    BauhausSlider, FullWidthSliderSpec, SliderInputSpec, full_width_slider,
};

/// Stable Darktable operation name used by history, styles, and module order.
pub const SOFTEN_MODULE_ID: &str = "soften";
/// Native module title from `src/iop/soften.c::name`.
pub const SOFTEN_TITLE: &str = "soften";
/// Native module description from `src/iop/soften.c::description`.
pub const SOFTEN_DESCRIPTION: &str = "create a softened image using the Orton effect";
/// Native module groups in declaration order from `default_group`.
pub const SOFTEN_GROUP_KEYS: [&str; 2] = ["group.effect", "group.effects"];

/// One parameter exposed by the native Soften editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SoftenParameter {
    Size,
    Saturation,
    Brightness,
    Amount,
}

impl SoftenParameter {
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Size => "size",
            Self::Saturation => "saturation",
            Self::Brightness => "brightness",
            Self::Amount => "amount",
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::Size => 0,
            Self::Saturation => 1,
            Self::Brightness => 2,
            Self::Amount => 3,
        }
    }
}

/// Exact source presentation for one native Soften Bauhaus slider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoftenSliderSpec {
    parameter: SoftenParameter,
    label: &'static str,
    minimum: f64,
    maximum: f64,
    default_value: f64,
    digits: i32,
    gtk_step: f64,
    suffix: &'static str,
    tooltip: &'static str,
}

impl SoftenSliderSpec {
    #[must_use]
    pub const fn parameter(self) -> SoftenParameter {
        self.parameter
    }

    #[must_use]
    pub const fn parameter_id(self) -> &'static str {
        self.parameter.id()
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        self.label
    }

    #[must_use]
    pub const fn range(self) -> (f64, f64) {
        (self.minimum, self.maximum)
    }

    #[must_use]
    pub const fn default_value(self) -> f64 {
        self.default_value
    }

    #[must_use]
    pub const fn digits(self) -> i32 {
        self.digits
    }

    #[must_use]
    pub const fn gtk_step(self) -> f64 {
        self.gtk_step
    }

    #[must_use]
    pub const fn suffix(self) -> &'static str {
        self.suffix
    }

    #[must_use]
    pub const fn tooltip(self) -> &'static str {
        self.tooltip
    }

    /// Returns the stable widget identity under a module-instance prefix.
    #[must_use]
    pub fn widget_name(self, widget_id: &str) -> String {
        format!("{widget_id}-{}", self.parameter_id())
    }
}

/// Source-required vertical control order for the Soften leaf.
pub const SOFTEN_SLIDERS: [SoftenSliderSpec; 4] = [
    SoftenSliderSpec {
        parameter: SoftenParameter::Size,
        label: "size",
        minimum: 0.0,
        maximum: 100.0,
        default_value: 50.0,
        digits: 2,
        gtk_step: 1.0,
        suffix: "%",
        tooltip: "the size of blur",
    },
    SoftenSliderSpec {
        parameter: SoftenParameter::Saturation,
        label: "saturation",
        minimum: 0.0,
        maximum: 100.0,
        default_value: 100.0,
        digits: 2,
        gtk_step: 1.0,
        suffix: "%",
        tooltip: "the saturation of blur",
    },
    SoftenSliderSpec {
        parameter: SoftenParameter::Brightness,
        label: "brightness",
        minimum: -2.0,
        maximum: 2.0,
        default_value: 0.33,
        digits: 2,
        gtk_step: 0.01,
        suffix: " EV",
        tooltip: "the brightness of blur",
    },
    SoftenSliderSpec {
        parameter: SoftenParameter::Amount,
        label: "amount",
        minimum: 0.0,
        maximum: 100.0,
        default_value: 50.0,
        digits: 2,
        gtk_step: 1.0,
        suffix: "%",
        tooltip: "the mix of effect",
    },
];

/// Finds only the four controls created by native `gui_init`.
#[must_use]
pub fn slider(parameter_id: &str) -> Option<SoftenSliderSpec> {
    SOFTEN_SLIDERS
        .iter()
        .copied()
        .find(|spec| spec.parameter_id() == parameter_id)
}

/// Validated, GTK-independent state for the native four-control editor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoftenEditorState {
    parameters: SoftenParametersV1,
}

impl SoftenEditorState {
    /// Creates editor state from persisted version-one parameters.
    ///
    /// # Errors
    ///
    /// Returns the processing contract's parameter error for non-finite or
    /// out-of-range values.
    pub fn new(parameters: SoftenParametersV1) -> Result<Self, SoftenParameterError> {
        SoftenConfig::try_from(parameters)?;
        Ok(Self { parameters })
    }

    #[must_use]
    pub const fn parameters(self) -> SoftenParametersV1 {
        self.parameters
    }

    #[must_use]
    pub const fn size(self) -> f32 {
        self.parameters.size
    }

    #[must_use]
    pub const fn saturation(self) -> f32 {
        self.parameters.saturation
    }

    #[must_use]
    pub const fn brightness(self) -> f32 {
        self.parameters.brightness
    }

    #[must_use]
    pub const fn amount(self) -> f32 {
        self.parameters.amount
    }

    #[must_use]
    pub const fn value(self, parameter: SoftenParameter) -> f32 {
        match parameter {
            SoftenParameter::Size => self.size(),
            SoftenParameter::Saturation => self.saturation(),
            SoftenParameter::Brightness => self.brightness(),
            SoftenParameter::Amount => self.amount(),
        }
    }

    /// Replaces one parameter while preserving the other three exactly.
    ///
    /// # Errors
    ///
    /// Returns the processing contract's parameter error for non-finite or
    /// out-of-range values, leaving this state unchanged.
    pub fn set(
        &mut self,
        parameter: SoftenParameter,
        value: f32,
    ) -> Result<bool, SoftenParameterError> {
        let mut candidate = self.parameters;
        match parameter {
            SoftenParameter::Size => candidate.size = value,
            SoftenParameter::Saturation => candidate.saturation = value,
            SoftenParameter::Brightness => candidate.brightness = value,
            SoftenParameter::Amount => candidate.amount = value,
        }
        SoftenConfig::try_from(candidate)?;
        let changed = candidate != self.parameters;
        self.parameters = candidate;
        Ok(changed)
    }

    /// Restores the native defaults and reports whether state changed.
    pub fn reset(&mut self) -> bool {
        let defaults = SoftenParametersV1::defaults();
        let changed = self.parameters != defaults;
        self.parameters = defaults;
        changed
    }
}

impl Default for SoftenEditorState {
    fn default() -> Self {
        Self {
            parameters: SoftenParametersV1::defaults(),
        }
    }
}

/// UI-owned snapshot for one Soften operation projection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoftenGtkState {
    operation_id: OperationId,
    revision: Revision,
    editor: SoftenEditorState,
    enabled: bool,
    sensitive: bool,
    materialization_required: bool,
}

impl SoftenGtkState {
    #[must_use]
    pub const fn new(
        operation_id: OperationId,
        revision: Revision,
        editor: SoftenEditorState,
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
        }
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
    pub const fn editor(self) -> SoftenEditorState {
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
}

/// One settled canonical parameter replacement authored by this leaf.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoftenSettledAction {
    target: OperationId,
    expected_revision: Revision,
    parameter: SoftenParameter,
    parameters: SoftenParametersV1,
    enable_required: bool,
    materialization_required: bool,
}

impl SoftenSettledAction {
    #[must_use]
    pub const fn target(self) -> OperationId {
        self.target
    }

    #[must_use]
    pub const fn expected_revision(self) -> Revision {
        self.expected_revision
    }

    #[must_use]
    pub const fn parameter(self) -> SoftenParameter {
        self.parameter
    }

    #[must_use]
    pub const fn parameters(self) -> SoftenParametersV1 {
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

/// Result returned by the optional leaf callback.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SoftenGtkHandlerOutcome {
    /// Keep the candidate values and advance to the committed edit revision.
    Commit { revision: Revision },
    /// Restore the values that were present before the edit.
    Rollback,
    /// Replace the complete leaf snapshot in place.
    Reconcile(SoftenGtkState),
}

pub type SoftenGtkActionHandler = Rc<dyn Fn(SoftenSettledAction) -> SoftenGtkHandlerOutcome>;

/// Mounted Soften leaf with stable widget identities and in-place synchronization.
#[derive(Clone)]
pub struct SoftenGtkLeaf {
    root: gtk4::Box,
    shared: Rc<Shared>,
}

impl SoftenGtkLeaf {
    #[must_use]
    pub const fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    #[must_use]
    pub fn state(&self) -> SoftenGtkState {
        *self.shared.snapshot.borrow()
    }

    /// Replaces persisted state and synchronizes all existing widgets in place.
    pub fn reconcile(&self, state: SoftenGtkState) {
        *self.shared.snapshot.borrow_mut() = state;
        self.shared.sync_widgets();
    }

    /// Returns the mounted scale for stable identity and boundary inspection.
    #[must_use]
    pub fn slider(&self, parameter: SoftenParameter) -> gtk4::Scale {
        self.shared.widgets.slider(parameter).scale().clone()
    }
}

/// Builds the four source-shaped ordinary Bauhaus sliders in native order.
///
/// The callback is intentionally optional so GTK boundary tests can exercise
/// the leaf without a controller; production mounting uses the shared custom
/// module projection.
#[must_use]
pub fn build_soften_gtk(
    widget_id: &str,
    state: SoftenGtkState,
    handler: Option<SoftenGtkActionHandler>,
) -> SoftenGtkLeaf {
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root.set_widget_name(&format!("{widget_id}-soften-editor"));
    root.set_hexpand(true);

    let sliders = SOFTEN_SLIDERS.map(|spec| build_slider(widget_id, spec, state.editor()));
    for slider in &sliders {
        root.append(slider.widget());
    }

    let shared = Rc::new(Shared {
        snapshot: RefCell::new(state),
        handler,
        widgets: Widgets {
            root: root.downgrade(),
            sliders,
        },
    });
    for parameter in [
        SoftenParameter::Size,
        SoftenParameter::Saturation,
        SoftenParameter::Brightness,
        SoftenParameter::Amount,
    ] {
        connect_slider(parameter, &shared);
    }
    shared.sync_widgets();

    SoftenGtkLeaf { root, shared }
}

fn build_slider(
    widget_id: &str,
    spec: SoftenSliderSpec,
    editor: SoftenEditorState,
) -> BauhausSlider {
    let (minimum, maximum) = spec.range();
    full_width_slider(FullWidthSliderSpec {
        widget_name: &spec.widget_name(widget_id),
        label: spec.label(),
        tooltip: spec.tooltip(),
        minimum,
        maximum,
        gtk_step: spec.gtk_step(),
        value: f64::from(editor.value(spec.parameter())),
        input: SliderInputSpec::IDENTITY
            .with_suffix(spec.suffix())
            .with_soft_range(minimum, maximum)
            .with_default_value(spec.default_value())
            .with_digits(spec.digits())
            .with_automatic_step(),
    })
}

fn connect_slider(parameter: SoftenParameter, shared: &Rc<Shared>) {
    let weak = Rc::downgrade(shared);
    shared
        .widgets
        .slider(parameter)
        .connect_value_settled(move |value| {
            let Some(shared) = weak.upgrade() else {
                return;
            };
            #[expect(
                clippy::cast_possible_truncation,
                reason = "The Bauhaus range is finite and bounded to native f32 values."
            )]
            shared.settle(parameter, value as f32);
        });
}

struct Shared {
    snapshot: RefCell<SoftenGtkState>,
    handler: Option<SoftenGtkActionHandler>,
    widgets: Widgets,
}

struct Widgets {
    root: gtk4::glib::WeakRef<gtk4::Box>,
    sliders: [BauhausSlider; 4],
}

impl Widgets {
    const fn slider(&self, parameter: SoftenParameter) -> &BauhausSlider {
        &self.sliders[parameter.index()]
    }
}

impl Shared {
    fn settle(&self, parameter: SoftenParameter, value: f32) {
        let before = *self.snapshot.borrow();
        let mut candidate = before.editor();
        let Ok(changed) = candidate.set(parameter, value) else {
            self.sync_widgets();
            return;
        };
        if !changed {
            self.sync_widgets();
            return;
        }

        let action = SoftenSettledAction {
            target: before.operation_id(),
            expected_revision: before.revision(),
            parameter,
            parameters: candidate.parameters(),
            enable_required: !before.enabled(),
            materialization_required: before.materialization_required(),
        };
        let outcome = self
            .handler
            .as_ref()
            .map_or(SoftenGtkHandlerOutcome::Rollback, |handler| handler(action));
        match outcome {
            SoftenGtkHandlerOutcome::Commit { revision } => {
                *self.snapshot.borrow_mut() = SoftenGtkState::new(
                    before.operation_id(),
                    revision,
                    candidate,
                    before.enabled() || action.enable_required(),
                    before.sensitive(),
                    before.materialization_required(),
                );
            }
            SoftenGtkHandlerOutcome::Rollback => {}
            SoftenGtkHandlerOutcome::Reconcile(state) => {
                *self.snapshot.borrow_mut() = state;
            }
        }
        self.sync_widgets();
    }

    fn sync_widgets(&self) {
        let snapshot = *self.snapshot.borrow();
        if let Some(root) = self.widgets.root.upgrade() {
            root.set_sensitive(snapshot.sensitive());
        }
        for spec in SOFTEN_SLIDERS {
            self.widgets
                .slider(spec.parameter())
                .set_value(f64::from(snapshot.editor().value(spec.parameter())));
        }
    }
}

#[cfg(test)]
mod tests {
    use rusttable_processing::operations::soften::SoftenParametersV1;

    use super::*;

    #[test]
    fn source_metadata_and_slider_order_are_exact() {
        assert_eq!(SOFTEN_MODULE_ID, "soften");
        assert_eq!(SOFTEN_TITLE, "soften");
        assert_eq!(
            SOFTEN_DESCRIPTION,
            "create a softened image using the Orton effect"
        );
        assert_eq!(SOFTEN_GROUP_KEYS, ["group.effect", "group.effects"]);
        assert_eq!(
            SOFTEN_SLIDERS.map(SoftenSliderSpec::parameter),
            [
                SoftenParameter::Size,
                SoftenParameter::Saturation,
                SoftenParameter::Brightness,
                SoftenParameter::Amount,
            ]
        );
    }

    #[test]
    fn source_sliders_preserve_ranges_defaults_formats_and_tooltips() {
        let expected = [
            (
                SOFTEN_SLIDERS[0],
                "size",
                (0.0, 100.0),
                50.0_f64,
                "%",
                "the size of blur",
            ),
            (
                SOFTEN_SLIDERS[1],
                "saturation",
                (0.0, 100.0),
                100.0,
                "%",
                "the saturation of blur",
            ),
            (
                SOFTEN_SLIDERS[2],
                "brightness",
                (-2.0, 2.0),
                0.33,
                " EV",
                "the brightness of blur",
            ),
            (
                SOFTEN_SLIDERS[3],
                "amount",
                (0.0, 100.0),
                50.0,
                "%",
                "the mix of effect",
            ),
        ];
        for (spec, id, range, default, suffix, tooltip) in expected {
            assert_eq!(spec.parameter_id(), id);
            assert_eq!(spec.label(), id);
            assert_eq!(spec.range(), range);
            assert_eq!(spec.default_value().to_bits(), default.to_bits());
            assert_eq!(spec.digits(), 2);
            assert_eq!(spec.suffix(), suffix);
            assert_eq!(spec.tooltip(), tooltip);
            assert_eq!(spec.widget_name("soften"), format!("soften-{id}"));
            assert_eq!(slider(id), Some(spec));
        }
        assert_eq!(SOFTEN_SLIDERS[0].gtk_step().to_bits(), 1.0_f64.to_bits());
        assert_eq!(SOFTEN_SLIDERS[2].gtk_step().to_bits(), 0.01_f64.to_bits());
    }

    #[test]
    fn unknown_parameters_have_no_source_control() {
        for hidden in ["blur", "mix", "radius", "enabled", ""] {
            assert_eq!(slider(hidden), None);
        }
    }

    #[test]
    fn editor_defaults_match_native_payload_order() {
        let editor = SoftenEditorState::default();
        assert_eq!(
            editor.parameters(),
            SoftenParametersV1::new(50.0, 100.0, 0.33, 50.0)
        );
        assert_eq!(
            editor.parameters().to_bytes(),
            SoftenParametersV1::new(50.0, 100.0, 0.33, 50.0).to_bytes()
        );
    }

    #[test]
    fn one_editor_change_preserves_the_other_parameters() {
        let mut editor = SoftenEditorState::default();
        assert!(editor.set(SoftenParameter::Size, 64.0).unwrap());
        assert_eq!(
            editor.parameters(),
            SoftenParametersV1::new(64.0, 100.0, 0.33, 50.0)
        );
        assert!(editor.set(SoftenParameter::Saturation, 42.0).unwrap());
        assert!(editor.set(SoftenParameter::Brightness, -1.25).unwrap());
        assert!(editor.set(SoftenParameter::Amount, 11.0).unwrap());
        assert_eq!(
            editor.parameters(),
            SoftenParametersV1::new(64.0, 42.0, -1.25, 11.0)
        );
        assert!(!editor.set(SoftenParameter::Amount, 11.0).unwrap());
    }

    #[test]
    fn invalid_edits_are_rejected_without_mutation() {
        let mut editor = SoftenEditorState::default();
        let before = editor;
        assert!(editor.set(SoftenParameter::Size, f32::NAN).is_err());
        assert_eq!(editor, before);
        assert!(editor.set(SoftenParameter::Saturation, 100.1).is_err());
        assert_eq!(editor, before);
        assert!(editor.set(SoftenParameter::Brightness, -2.1).is_err());
        assert_eq!(editor, before);
        assert!(editor.set(SoftenParameter::Amount, f32::INFINITY).is_err());
        assert_eq!(editor, before);
    }

    #[test]
    fn reset_restores_all_source_defaults_once() {
        let mut editor =
            SoftenEditorState::new(SoftenParametersV1::new(1.0, 2.0, 0.5, 3.0)).unwrap();
        assert!(editor.reset());
        assert_eq!(editor, SoftenEditorState::default());
        assert!(!editor.reset());
    }
}
