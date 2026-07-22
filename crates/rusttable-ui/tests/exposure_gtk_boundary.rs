use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use rusttable_core::Revision;
use rusttable_processing::ExposureAction;
use rusttable_ui::{
    DarkroomModuleAction, DarkroomModuleActionHandler, DarkroomModuleError, ExposurePanel,
};

fn main() {
    gtk4::init().expect("GTK must initialize for the Exposure callback regression");
    full_interaction_and_panicking_handlers();
    println!("Exposure GTK callback boundary regression passed");
}

#[allow(clippy::too_many_lines)] // Keep the native event sequence auditable in one regression.
fn full_interaction_and_panicking_handlers() {
    let panel = ExposurePanel::new();
    let root: gtk4::Widget = panel.widget().clone().upcast();
    let expander = panel.widget().clone();
    let enabled = find_widget(&root, "exposure-enabled")
        .expect("exposure enabled switch")
        .downcast::<gtk4::Switch>()
        .expect("exposure enabled switch type");
    let mode = find_widget(&root, "exposure-mode")
        .expect("exposure mode dropdown")
        .downcast::<gtk4::DropDown>()
        .expect("exposure mode dropdown type");
    let exposure = find_widget(&root, "exposure-ev")
        .expect("exposure scale")
        .downcast::<gtk4::Scale>()
        .expect("exposure scale type");
    let black = find_widget(&root, "exposure-black")
        .expect("black-level scale")
        .downcast::<gtk4::Scale>()
        .expect("black-level scale type");
    let reset = find_widget(&root, "exposure-reset")
        .expect("exposure reset button")
        .downcast::<gtk4::Button>()
        .expect("exposure reset button type");
    let status = find_widget(&root, "exposure-status")
        .expect("exposure status")
        .downcast::<gtk4::Label>()
        .expect("exposure status type");
    let actions = Rc::new(RefCell::new(Vec::<ExposureAction>::new()));
    let actions_for_handler = Rc::clone(&actions);
    panel.set_action_handler(move |action| actions_for_handler.borrow_mut().push(action));
    let module_actions = Rc::new(RefCell::new(Vec::<DarkroomModuleAction>::new()));
    let module_actions_for_handler = Rc::clone(&module_actions);
    let handler: DarkroomModuleActionHandler = Rc::new(move |action| {
        module_actions_for_handler.borrow_mut().push(action.clone());
        action
            .expected_revision()
            .checked_increment()
            .map_err(|_| DarkroomModuleError::RevisionOverflow)
    });
    panel.set_module_action_handler(Some(handler.clone()), Revision::ZERO);

    for revision in 1..=8 {
        let panel_for_projection = panel.clone();
        run_main_loop(move || {
            panel_for_projection
                .set_module_projection(
                    Revision::from_u64(revision),
                    revision % 2 == 0,
                    true,
                    0.0,
                    0.0,
                )
                .expect("valid exposure projection");
        });
    }
    for expanded in [false, true] {
        let expander = expander.clone();
        run_main_loop(move || expander.set_expanded(expanded));
    }
    for active in [false, true] {
        let enabled = enabled.clone();
        run_main_loop(move || enabled.set_active(active));
    }
    for selected in [1, 0] {
        let mode = mode.clone();
        run_main_loop(move || mode.set_selected(selected));
    }
    let exposure_for_test = exposure.clone();
    run_main_loop(move || exposure_for_test.set_value(1.25));
    let black_for_test = black.clone();
    run_main_loop(move || black_for_test.set_value(-0.25));
    let reset_for_test = reset.clone();
    run_main_loop(move || reset_for_test.emit_clicked());

    assert!(
        actions
            .borrow()
            .iter()
            .any(|action| matches!(action, ExposureAction::SetExpanded(_)))
    );
    assert!(
        actions
            .borrow()
            .iter()
            .any(|action| matches!(action, ExposureAction::SetEnabled(_)))
    );
    assert!(
        actions
            .borrow()
            .iter()
            .any(|action| matches!(action, ExposureAction::SetMode(_)))
    );
    assert!(actions.borrow().iter().any(|action| matches!(
        action,
        ExposureAction::SetExposureEv(value) if (*value - 1.25).abs() < 0.001
    )));
    assert!(actions.borrow().iter().any(|action| matches!(
        action,
        ExposureAction::SetBlackLevel(value) if (*value + 0.25).abs() < 0.001
    )));
    assert!(
        actions
            .borrow()
            .iter()
            .any(|action| matches!(action, ExposureAction::Reset))
    );
    assert!(
        module_actions
            .borrow()
            .iter()
            .any(|action| matches!(action, DarkroomModuleAction::Disclosure { .. }))
    );
    assert!(
        module_actions
            .borrow()
            .iter()
            .any(|action| matches!(action, DarkroomModuleAction::Enable { .. }))
    );
    assert!(module_actions.borrow().iter().any(|action| matches!(
        action,
        DarkroomModuleAction::Control { id, .. } if id == "exposure-stops"
    )));
    assert!(module_actions.borrow().iter().any(|action| matches!(
        action,
        DarkroomModuleAction::Control { id, .. } if id == "exposure-black"
    )));
    assert!(
        module_actions
            .borrow()
            .iter()
            .any(|action| matches!(action, DarkroomModuleAction::Reset { .. }))
    );
    assert!(status.text().starts_with("Ready · revision"));

    panel.set_action_handler(|_| panic!("deliberate Exposure action handler panic"));
    let expander_for_panic = expander.clone();
    run_main_loop(move || expander_for_panic.set_expanded(false));
    assert!(status.text().contains("Exposure callback failed"));
    assert!(
        status
            .text()
            .contains("deliberate Exposure action handler panic")
    );

    panel.set_action_handler(|_| {});
    panel.set_module_action_handler(
        Some(Rc::new(|_| {
            panic!("deliberate Exposure module handler panic")
        })),
        Revision::from_u64(99),
    );
    let enabled_for_panic = enabled.clone();
    run_main_loop(move || enabled_for_panic.set_active(false));
    assert!(status.text().contains("Exposure callback failed"));
    assert!(
        status
            .text()
            .contains("deliberate Exposure module handler panic")
    );

    panel.set_module_action_handler(Some(handler), Revision::from_u64(100));
    let panel_for_recovery = panel.clone();
    run_main_loop(move || {
        panel_for_recovery
            .set_module_projection(Revision::from_u64(101), true, true, 0.0, 0.0)
            .expect("recovery projection");
    });
    assert!(panel.state().enabled());
}

fn run_main_loop(callback: impl FnOnce() + 'static) {
    let main_loop = gtk4::glib::MainLoop::new(None, false);
    let main_loop_for_callback = main_loop.clone();
    gtk4::glib::idle_add_local_once(move || {
        callback();
        main_loop_for_callback.quit();
    });
    main_loop.run();
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
