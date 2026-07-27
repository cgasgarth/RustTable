#![forbid(unsafe_code)]

use std::{cell::RefCell, rc::Rc};

use gtk4::prelude::*;
use rusttable_core::{OperationId, Revision};
use rusttable_ui::iop::bloom::{
    BLOOM_DESCRIPTION, BLOOM_SLIDERS, BLOOM_TITLE, BloomEditorState, BloomGtkState,
};
use rusttable_ui::{
    DarkroomModuleAction, DarkroomModuleActionHandler, DarkroomModulesViewModel, GtkShell,
    WorkspaceRole, install_darktable_theme, reference_modules,
};

fn main() {
    gtk4::init().expect("GTK must initialize for the Bloom boundary");
    prohibit_macos_test_activation();
    let display = gtk4::gdk::Display::default().expect("GTK boundary needs the default display");
    install_darktable_theme(&display);
    production_rail_mounts_source_order_and_routes_one_settled_action();
    println!("Bloom GTK boundary passed");
}

#[cfg(target_os = "macos")]
fn prohibit_macos_test_activation() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    let marker = MainThreadMarker::new().expect("Bloom GTK boundary must start on the main thread");
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

fn production_rail_mounts_source_order_and_routes_one_settled_action() {
    let application = gtk4::Application::new(
        Some("com.cgasgarth.rusttable.test.bloom-boundary"),
        gtk4::gio::ApplicationFlags::default(),
    );
    application
        .register(None::<&gtk4::gio::Cancellable>)
        .expect("Bloom test application must register");
    let shell = GtkShell::new(&application);
    shell.window().set_focusable(false);
    shell.window().set_opacity(0.0);
    shell.window().set_default_size(1_200, 900);

    let operation_id = OperationId::new(7_401).expect("Bloom operation ID");
    let revision = Revision::from_u64(17);
    let state = BloomGtkState::new(
        operation_id,
        revision,
        BloomEditorState::default(),
        false,
        true,
        false,
    );
    let template = reference_modules()
        .expect("registry modules")
        .module("bloom")
        .expect("Bloom module")
        .clone();
    let module = template
        .with_operation_instance(operation_id, 0, 1)
        .with_bloom_editor_state(state);
    assert!(module.has_bloom_custom_editor());
    assert_eq!(module.controls().controls().count(), 0);
    let modules = DarkroomModulesViewModel::new(vec![module]).expect("Bloom module snapshot");
    let actions = Rc::new(RefCell::new(Vec::new()));
    let actions_for_handler = Rc::clone(&actions);
    let handler: DarkroomModuleActionHandler = Rc::new(move |action| {
        let revision = action
            .expected_revision()
            .checked_increment()
            .expect("test revision advances");
        actions_for_handler.borrow_mut().push(action);
        Ok(revision)
    });
    shell.set_darkroom_module_stack(&modules, Some(handler));
    shell.show_workspace(WorkspaceRole::Darkroom);
    settle_gtk();
    assert!(!shell.window().is_visible());
    assert!(!shell.window().is_active());

    let root: gtk4::Widget = shell.window().clone().upcast();
    let effects_group = find_widget(&root, "group-effects")
        .expect("Effects module group")
        .downcast::<gtk4::ToggleButton>()
        .expect("Effects module group type");
    effects_group.set_active(true);
    settle_gtk();

    let panel = find_widget(&root, "bloom")
        .expect("production rail mounts Bloom")
        .downcast::<gtk4::Expander>()
        .expect("Bloom panel type");
    let title_root = panel.label_widget().expect("Bloom title widget");
    let title = find_widget(&title_root, "bloom-label")
        .expect("Bloom title label")
        .downcast::<gtk4::Label>()
        .expect("Bloom title type");
    assert_eq!(title.text(), BLOOM_TITLE);
    assert_eq!(
        title_root.tooltip_text().as_deref(),
        Some(BLOOM_DESCRIPTION)
    );
    assert!(!widget::<gtk4::CheckButton>(&title_root, "bloom-enabled").is_active());

    let content = panel.child().expect("Bloom module content");
    let editor = find_widget(&content, "bloom-bloom-editor")
        .expect("source-specific Bloom editor")
        .downcast::<gtk4::Box>()
        .expect("Bloom editor root type");
    assert_eq!(editor.orientation(), gtk4::Orientation::Vertical);
    assert_eq!(editor.spacing(), 0);

    for spec in BLOOM_SLIDERS {
        let scale = widget::<gtk4::Scale>(&editor, &spec.widget_name("bloom"));
        assert_eq!(
            (scale.adjustment().lower(), scale.adjustment().upper()),
            spec.range(),
        );
        assert_eq!(scale.value().to_bits(), spec.default_value().to_bits());
        assert_eq!(scale.digits(), spec.digits());
        assert_eq!(scale.tooltip_text().as_deref(), Some(spec.tooltip()));
        let overlay = ancestor::<gtk4::Overlay>(&scale).expect("Bauhaus overlay");
        assert_eq!(
            descendant_label_text(&overlay),
            [
                spec.label().to_owned(),
                format!("{:.2}%", spec.default_value()),
            ],
        );
    }
    assert_direct_slider_order(&editor, ["bloom-size", "bloom-threshold", "bloom-strength"]);

    let strength = widget::<gtk4::Scale>(&editor, "bloom-strength");
    let key = controller_named::<gtk4::EventControllerKey>(&strength, "dt-bauhaus-open");
    assert!(key.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk4::gdk::Key::Up,
            &0_u32,
            &gtk4::gdk::ModifierType::empty(),
        ],
    ));
    settle_gtk();
    let actions = actions.borrow();
    assert_eq!(actions.len(), 1);
    let DarkroomModuleAction::BloomSettled {
        module_id,
        operation_id: Some(target),
        expected_revision,
        parameters,
        enable_required,
    } = &actions[0]
    else {
        panic!("Bloom leaf must emit one atomic settled action");
    };
    assert_eq!(module_id, "bloom");
    assert_eq!(*target, operation_id);
    assert_eq!(*expected_revision, revision);
    assert_eq!(parameters.size.to_bits(), 20.0_f32.to_bits());
    assert_eq!(parameters.threshold.to_bits(), 90.0_f32.to_bits());
    assert_eq!(parameters.strength.to_bits(), 26.0_f32.to_bits());
    assert!(*enable_required);
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
) -> T {
    let controllers = widget.as_ref().observe_controllers();
    for index in 0..controllers.n_items() {
        let controller = controllers
            .item(index)
            .expect("controller item")
            .downcast::<gtk4::EventController>()
            .expect("GTK controller type");
        if controller.name().as_deref() == Some(name)
            && let Ok(controller) = controller.downcast::<T>()
        {
            return controller;
        }
    }
    panic!("missing controller {name}");
}

fn ancestor<T: IsA<gtk4::Widget> + Clone + 'static>(widget: &impl IsA<gtk4::Widget>) -> Option<T> {
    let mut parent = widget.as_ref().parent();
    while let Some(current) = parent {
        if let Ok(found) = current.clone().downcast::<T>() {
            return Some(found);
        }
        parent = current.parent();
    }
    None
}

fn descendant_label_text(root: &impl IsA<gtk4::Widget>) -> Vec<String> {
    fn visit(widget: &gtk4::Widget, labels: &mut Vec<String>) {
        if let Ok(label) = widget.clone().downcast::<gtk4::Label>()
            && !label.text().is_empty()
        {
            labels.push(label.text().to_string());
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            visit(&current, labels);
            child = current.next_sibling();
        }
    }

    let mut labels = Vec::new();
    visit(root.as_ref(), &mut labels);
    labels
}

fn widget<T: IsA<gtk4::Widget> + Clone + 'static>(root: &impl IsA<gtk4::Widget>, name: &str) -> T {
    find_widget(root.as_ref(), name)
        .unwrap_or_else(|| panic!("missing widget {name}"))
        .downcast::<T>()
        .unwrap_or_else(|_| panic!("widget {name} has the wrong type"))
}

fn assert_direct_slider_order<const N: usize>(root: &gtk4::Box, expected: [&str; N]) {
    let mut names = Vec::new();
    let mut child = root.first_child();
    while let Some(current) = child {
        let scale = find_descendant_of_type::<gtk4::Scale>(&current)
            .expect("each direct Bloom child owns one Bauhaus scale");
        names.push(scale.widget_name().to_string());
        child = current.next_sibling();
    }
    assert_eq!(names, expected);
}

fn find_descendant_of_type<T: IsA<gtk4::Widget> + Clone + 'static>(
    root: &gtk4::Widget,
) -> Option<T> {
    if let Ok(widget) = root.clone().downcast::<T>() {
        return Some(widget);
    }
    let mut child = root.first_child();
    while let Some(current) = child {
        if let Some(found) = find_descendant_of_type::<T>(&current) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

fn find_widget(root: &gtk4::Widget, name: &str) -> Option<gtk4::Widget> {
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
