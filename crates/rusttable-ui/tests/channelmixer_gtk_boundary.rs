#![forbid(unsafe_code)]
#![allow(dead_code)]
#![allow(
    clippy::assertions_on_constants,
    clippy::float_cmp,
    clippy::type_complexity
)]

//! Production GTK boundary for the fail-closed Channel Mixer projection.
//!
//! The shared module registry intentionally does not mount Channel Mixer yet.
//! This test therefore exercises the leaf constructor directly and asserts the
//! fail-closed registration contract instead of pretending that the production
//! rail exposes the deprecated operation.

use std::{cell::RefCell, rc::Rc};

use gtk4::prelude::*;
use rusttable_core::{OperationId, Revision};

use rusttable_ui::iop::channelmixer::{
    CHANNEL_MIXER_DESTINATION_OPTIONS, CHANNEL_MIXER_PRODUCTION_ROUTING_INTEGRATED,
    CHANNEL_MIXER_SLIDERS, ChannelMixerDestination, ChannelMixerEditorState,
    ChannelMixerGtkActionHandler, ChannelMixerGtkHandlerOutcome, ChannelMixerGtkState,
    ChannelMixerInput, ChannelMixerSettledAction, build_channelmixer_gtk,
};

fn main() {
    gtk4::init().expect("GTK must initialize for the Channel Mixer boundary");
    prohibit_macos_test_activation();
    let _display = gtk4::gdk::Display::default().expect("GTK boundary needs the default display");
    leaf_preserves_source_hierarchy_and_fail_closed_registration();
    leaf_routes_one_full_matrix_action_and_keeps_transient_destination_out_of_payload();
}

#[cfg(target_os = "macos")]
fn prohibit_macos_test_activation() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    let marker = MainThreadMarker::new().expect("GTK boundary must start on the main thread");
    let application = NSApplication::sharedApplication(marker);
    application.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    assert_eq!(
        application.activationPolicy(),
        NSApplicationActivationPolicy::Prohibited,
        "automated GTK boundary must not activate or steal focus",
    );
}

#[cfg(not(target_os = "macos"))]
fn prohibit_macos_test_activation() {}

fn leaf_preserves_source_hierarchy_and_fail_closed_registration() {
    assert!(!CHANNEL_MIXER_PRODUCTION_ROUTING_INTEGRATED);

    let operation_id = OperationId::new(8_701).expect("Channel Mixer operation ID");
    let state = ChannelMixerGtkState::new(
        operation_id,
        Revision::from_u64(9),
        ChannelMixerEditorState::default(),
        false,
        true,
        false,
    );
    let leaf = build_channelmixer_gtk("channelmixer", state, None);
    let root = leaf.widget();

    assert_eq!(root.widget_name(), "channelmixer-channelmixer-editor");
    assert_eq!(root.orientation(), gtk4::Orientation::Vertical);
    assert_eq!(root.spacing(), 0);
    let child_names = direct_child_names(root);
    assert_eq!(child_names.len(), 4);
    assert_eq!(child_names[0], "channelmixer-destination");
    for spec in CHANNEL_MIXER_SLIDERS {
        assert!(find_widget(root, &spec.widget_name("channelmixer")).is_some());
    }
    assert!(find_widget(root, "channelmixer-algorithm").is_none());
    assert!(find_widget(root, "channelmixer-presets").is_none());

    let destination = leaf.destination();
    assert_eq!(destination.selected(), 3);
    assert_eq!(
        dropdown_options(&destination),
        CHANNEL_MIXER_DESTINATION_OPTIONS
    );
    assert_eq!(
        destination.widget_name(),
        "channelmixer-destination-selection"
    );

    for (input, spec) in ChannelMixerInput::ALL
        .into_iter()
        .zip(CHANNEL_MIXER_SLIDERS)
    {
        let scale = leaf.slider(input);
        assert_eq!(scale.widget_name(), spec.widget_name("channelmixer"));
        assert_eq!(
            (scale.adjustment().lower(), scale.adjustment().upper()),
            spec.range()
        );
        assert_eq!(scale.digits(), spec.digits());
        assert_eq!(scale.tooltip_text().as_deref(), Some(spec.tooltip()));
        assert_eq!(
            leaf.reset_default(input),
            spec.reset_default(ChannelMixerDestination::Red)
        );
    }

    destination.set_selected(4);
    settle_gtk();
    assert_eq!(
        leaf.state().editor().destination(),
        ChannelMixerDestination::Green
    );
    assert_eq!(
        [
            leaf.reset_default(ChannelMixerInput::Red),
            leaf.reset_default(ChannelMixerInput::Green),
            leaf.reset_default(ChannelMixerInput::Blue),
        ],
        [0.0, 1.0, 0.0]
    );
}

fn leaf_routes_one_full_matrix_action_and_keeps_transient_destination_out_of_payload() {
    let operation_id = OperationId::new(8_702).expect("Channel Mixer action operation ID");
    let revision = Revision::from_u64(19);
    let actions = Rc::new(RefCell::new(Vec::<ChannelMixerSettledAction>::new()));
    let actions_for_handler = Rc::clone(&actions);
    let handler: ChannelMixerGtkActionHandler = Rc::new(move |action| {
        actions_for_handler.borrow_mut().push(action);
        ChannelMixerGtkHandlerOutcome::Commit {
            revision: action
                .expected_revision()
                .checked_increment()
                .expect("test revision advances"),
        }
    });
    let state = ChannelMixerGtkState::new(
        operation_id,
        revision,
        ChannelMixerEditorState::default(),
        false,
        true,
        false,
    );
    let leaf = build_channelmixer_gtk("channelmixer-action", state, Some(handler));
    leaf.destination().set_selected(4);
    settle_gtk();

    let green = leaf.slider(ChannelMixerInput::Green);
    let click = controller_named::<gtk4::GestureClick>(&green, "dt-bauhaus-main-click")
        .expect("Bauhaus green slider settled controller");
    click.emit_by_name::<()>("pressed", &[&1_i32, &0.0_f64, &0.0_f64]);
    green.set_value(0.375);
    click.emit_by_name::<()>("released", &[&1_i32, &0.0_f64, &0.0_f64]);
    settle_gtk();

    let actions = actions.borrow();
    assert_eq!(actions.len(), 1);
    let action = actions[0];
    assert_eq!(action.target(), operation_id);
    assert_eq!(action.expected_revision(), revision);
    assert!(action.enable_required());
    assert!(!action.materialization_required());
    assert_eq!(
        action.parameters().algorithm(),
        state.editor().parameters().algorithm()
    );
    assert_eq!(action.parameters().red(), state.editor().parameters().red());
    assert_eq!(
        action.parameters().blue(),
        state.editor().parameters().blue()
    );
    assert_eq!(
        action.parameters().green()[4].to_bits(),
        0.375_f32.to_bits()
    );
    for index in [0, 1, 2, 3, 5, 6] {
        assert_eq!(
            action.parameters().green()[index].to_bits(),
            state.editor().parameters().green()[index].to_bits()
        );
    }
    assert_eq!(leaf.state().revision(), Revision::from_u64(20));
    assert_eq!(
        leaf.state().editor().destination(),
        ChannelMixerDestination::Green
    );
}

fn settle_gtk() {
    let context = gtk4::glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
}

fn controller_named<T: IsA<gtk4::EventController> + Clone + 'static>(
    widget: &impl IsA<gtk4::Widget>,
    name: &str,
) -> Option<T> {
    let controllers = widget.as_ref().observe_controllers();
    for index in 0..controllers.n_items() {
        let controller = controllers
            .item(index)?
            .downcast::<gtk4::EventController>()
            .ok()?;
        if controller.name().as_deref() == Some(name) {
            return controller.downcast::<T>().ok();
        }
    }
    None
}

fn direct_child_names(root: &gtk4::Box) -> Vec<String> {
    let mut names = Vec::new();
    let mut child = root.first_child();
    while let Some(current) = child {
        names.push(current.widget_name().to_string());
        child = current.next_sibling();
    }
    names
}

fn dropdown_options(dropdown: &gtk4::DropDown) -> Vec<String> {
    let model = dropdown.model().expect("Channel Mixer destination model");
    let model = model
        .downcast::<gtk4::StringList>()
        .expect("Channel Mixer destination string model");
    (0..model.n_items())
        .map(|index| model.string(index).expect("destination label").to_string())
        .collect()
}

fn find_widget(root: &impl IsA<gtk4::Widget>, name: &str) -> Option<gtk4::Widget> {
    let root = root.as_ref();
    if root.widget_name() == name {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(current) = child {
        if let Some(found) = find_widget(&current, name) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}
