//! GTK4 leaf for `src/iop/bloom.c::gui_init` and automatic Bauhaus field sync.
//!
//! Shared Bauhaus controls own pointer, scroll, keyboard, popup, formatting, and
//! per-slider double-click reset behavior. This adapter emits one action only at
//! the shared settled boundary and never duplicates Bloom processing equations.

use std::{cell::RefCell, rc::Rc};

use gtk4::prelude::*;
use rusttable_core::{OperationId, Revision};
use rusttable_processing::operations::bloom::BloomParametersV1;

use crate::bauhaus::slider_input::{
    BauhausSlider, FullWidthSliderSpec, SliderInputSpec, full_width_slider,
};

use super::{BLOOM_SLIDERS, BloomEditorState, BloomParameter, BloomSliderSpec};

/// Complete UI-owned snapshot for one Bloom leaf.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BloomGtkState {
    operation_id: OperationId,
    revision: Revision,
    editor: BloomEditorState,
    enabled: bool,
    sensitive: bool,
    materialization_required: bool,
}

impl BloomGtkState {
    #[must_use]
    pub const fn new(
        operation_id: OperationId,
        revision: Revision,
        editor: BloomEditorState,
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
    pub const fn editor(self) -> BloomEditorState {
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
pub struct BloomSettledAction {
    target: OperationId,
    expected_revision: Revision,
    parameters: BloomParametersV1,
    enable_required: bool,
    materialization_required: bool,
}

impl BloomSettledAction {
    #[must_use]
    pub const fn target(self) -> OperationId {
        self.target
    }

    #[must_use]
    pub const fn expected_revision(self) -> Revision {
        self.expected_revision
    }

    #[must_use]
    pub const fn parameters(self) -> BloomParametersV1 {
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
pub enum BloomGtkHandlerOutcome {
    /// Keep the candidate editor state and advance to the committed edit revision.
    Commit { revision: Revision },
    /// Restore the exact state from before the attempted action.
    Rollback,
    /// Replace target, revision, parameters, and presentation state in place.
    Reconcile(BloomGtkState),
}

pub type BloomGtkActionHandler = Rc<dyn Fn(BloomSettledAction) -> BloomGtkHandlerOutcome>;

/// Mounted Bloom leaf with stable widget identities and in-place synchronization.
#[derive(Clone)]
pub struct BloomGtkLeaf {
    root: gtk4::Box,
    shared: Rc<Shared>,
}

impl BloomGtkLeaf {
    #[must_use]
    pub const fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    #[must_use]
    pub fn state(&self) -> BloomGtkState {
        *self.shared.snapshot.borrow()
    }

    /// Replaces persisted state and synchronizes all existing widgets in place.
    pub fn reconcile(&self, state: BloomGtkState) {
        *self.shared.snapshot.borrow_mut() = state;
        self.shared.sync_widgets();
    }

    /// Returns the mounted scale for stable identity and boundary inspection.
    #[must_use]
    pub fn slider(&self, parameter: BloomParameter) -> gtk4::Scale {
        self.shared.widgets.slider(parameter).scale().clone()
    }
}

/// Builds the three source-shaped ordinary Bauhaus sliders in native order.
#[must_use]
pub fn build_bloom_gtk(
    widget_id: &str,
    state: BloomGtkState,
    handler: Option<BloomGtkActionHandler>,
) -> BloomGtkLeaf {
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root.set_widget_name(&format!("{widget_id}-bloom-editor"));
    root.set_hexpand(true);

    let size = build_slider(widget_id, BLOOM_SLIDERS[0], state.editor());
    let threshold = build_slider(widget_id, BLOOM_SLIDERS[1], state.editor());
    let strength = build_slider(widget_id, BLOOM_SLIDERS[2], state.editor());

    root.append(size.widget());
    root.append(threshold.widget());
    root.append(strength.widget());

    let shared = Rc::new(Shared {
        snapshot: RefCell::new(state),
        handler,
        widgets: Widgets {
            root: root.downgrade(),
            threshold,
            size,
            strength,
        },
    });
    connect_slider(BloomParameter::Threshold, &shared);
    connect_slider(BloomParameter::Size, &shared);
    connect_slider(BloomParameter::Strength, &shared);
    shared.sync_widgets();

    BloomGtkLeaf { root, shared }
}

fn build_slider(widget_id: &str, spec: BloomSliderSpec, editor: BloomEditorState) -> BauhausSlider {
    let (minimum, maximum) = spec.range();
    let slider = full_width_slider(FullWidthSliderSpec {
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
    });
    slider
        .scale()
        .update_property(&[gtk4::accessible::Property::Description(spec.tooltip())]);
    slider
}

fn connect_slider(parameter: BloomParameter, shared: &Rc<Shared>) {
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
                reason = "The Bauhaus range is finite and bounded to native f32 percentages."
            )]
            shared.settle(parameter, value as f32);
        });
}

struct Shared {
    snapshot: RefCell<BloomGtkState>,
    handler: Option<BloomGtkActionHandler>,
    widgets: Widgets,
}

struct Widgets {
    root: gtk4::glib::WeakRef<gtk4::Box>,
    threshold: BauhausSlider,
    size: BauhausSlider,
    strength: BauhausSlider,
}

impl Widgets {
    const fn slider(&self, parameter: BloomParameter) -> &BauhausSlider {
        match parameter {
            BloomParameter::Threshold => &self.threshold,
            BloomParameter::Size => &self.size,
            BloomParameter::Strength => &self.strength,
        }
    }
}

impl Shared {
    fn settle(&self, parameter: BloomParameter, value: f32) {
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

        let action = BloomSettledAction {
            target: before.operation_id(),
            expected_revision: before.revision(),
            parameters: candidate.parameters(),
            enable_required: !before.enabled(),
            materialization_required: before.materialization_required(),
        };
        let outcome = self
            .handler
            .as_ref()
            .map_or(BloomGtkHandlerOutcome::Rollback, |handler| handler(action));
        match outcome {
            BloomGtkHandlerOutcome::Commit { revision } => {
                *self.snapshot.borrow_mut() = BloomGtkState {
                    operation_id: before.operation_id(),
                    revision,
                    editor: candidate,
                    enabled: before.enabled() || action.enable_required(),
                    sensitive: before.sensitive(),
                    materialization_required: false,
                };
            }
            BloomGtkHandlerOutcome::Rollback => {}
            BloomGtkHandlerOutcome::Reconcile(state) => {
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
        for spec in BLOOM_SLIDERS {
            self.widgets
                .slider(spec.parameter())
                .set_value(f64::from(snapshot.editor().value(spec.parameter())));
        }
    }
}
