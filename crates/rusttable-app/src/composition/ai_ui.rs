//! Application-owned unavailable adapters for the GTK AI surfaces.

use std::cell::RefCell;
use std::rc::Rc;

use crate::ai_services::{
    UnavailableAiBatchService, UnavailableAiModelsService, UnavailableNeuralRestoreService,
};
use rusttable_ui::{
    AiBatchAction, AiBatchController, AiModelsAction, AiModelsController, NeuralRestoreAction,
    NeuralRestoreController,
};

pub(super) fn install_ai_batch_ui_bridge(
    shell: &rusttable_ui::GtkShell,
) -> Rc<RefCell<AiBatchController<UnavailableAiBatchService>>> {
    let controller = Rc::new(RefCell::new(AiBatchController::new(
        UnavailableAiBatchService,
    )));
    shell.set_ai_batch_state(controller.borrow().state());
    let controller_for_actions = Rc::clone(&controller);
    let action_shell = shell.clone();
    shell.connect_ai_batch_action(move |action: AiBatchAction| {
        let mut controller = controller_for_actions.borrow_mut();
        let _ = controller.dispatch(action);
        action_shell.set_ai_batch_state(controller.state());
    });
    controller
}

pub(super) fn install_ai_ui_bridges(
    shell: &rusttable_ui::GtkShell,
) -> Rc<RefCell<NeuralRestoreController<UnavailableNeuralRestoreService>>> {
    let ai_models_controller = Rc::new(RefCell::new(AiModelsController::new(
        UnavailableAiModelsService,
    )));
    shell.set_ai_models_state(ai_models_controller.borrow().state());
    let _ = ai_models_controller.borrow_mut().refresh();
    shell.set_ai_models_state(ai_models_controller.borrow().state());
    let ai_models_for_actions = Rc::clone(&ai_models_controller);
    let ai_models_shell = shell.clone();
    shell.connect_ai_models_action(move |action: AiModelsAction| {
        let mut controller = ai_models_for_actions.borrow_mut();
        let _ = controller.dispatch(action);
        ai_models_shell.set_ai_models_state(controller.state());
    });

    let neural_controller = Rc::new(RefCell::new(NeuralRestoreController::new(
        UnavailableNeuralRestoreService,
    )));
    shell.set_neural_restore_state(neural_controller.borrow().state());
    let neural_for_actions = Rc::clone(&neural_controller);
    let neural_shell = shell.clone();
    shell.connect_neural_restore_action(move |action: NeuralRestoreAction| {
        let mut controller = neural_for_actions.borrow_mut();
        let _ = controller.dispatch(action);
        neural_shell.set_neural_restore_state(controller.state());
    });
    neural_controller
}
