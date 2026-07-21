//! Application-owned unavailable adapters for the GTK AI surfaces.

use std::cell::RefCell;
use std::rc::Rc;

use crate::ai_services::{
    UnavailableAiBatchService, UnavailableAiModelsService, UnavailableRawDenoiseService,
    UnavailableRgbDenoiseService,
};
use rusttable_ui::{
    AiBatchAction, AiBatchController, AiModelsAction, AiModelsController, RawDenoiseAction,
    RawDenoiseController, RgbDenoiseAction, RgbDenoiseController,
};

type RgbAiController = Rc<RefCell<RgbDenoiseController<UnavailableRgbDenoiseService>>>;
type RawAiController = Rc<RefCell<RawDenoiseController<UnavailableRawDenoiseService>>>;

pub(super) struct AiUiBridges {
    pub(super) rgb: RgbAiController,
    pub(super) raw: RawAiController,
}

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

pub(super) fn install_ai_ui_bridges(shell: &rusttable_ui::GtkShell) -> AiUiBridges {
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

    let rgb_controller = Rc::new(RefCell::new(RgbDenoiseController::new(
        UnavailableRgbDenoiseService,
    )));
    shell.set_rgb_denoise_state(rgb_controller.borrow().state());
    let rgb_for_actions = Rc::clone(&rgb_controller);
    let rgb_shell = shell.clone();
    shell.connect_rgb_denoise_action(move |action: RgbDenoiseAction| {
        let mut controller = rgb_for_actions.borrow_mut();
        let _ = controller.dispatch(action);
        rgb_shell.set_rgb_denoise_state(controller.state());
    });

    let raw_controller = Rc::new(RefCell::new(RawDenoiseController::new(
        UnavailableRawDenoiseService,
    )));
    shell.set_raw_denoise_state(raw_controller.borrow().state());
    let raw_for_actions = Rc::clone(&raw_controller);
    let raw_shell = shell.clone();
    shell.connect_raw_denoise_action(move |action: RawDenoiseAction| {
        let mut controller = raw_for_actions.borrow_mut();
        let _ = controller.dispatch(action);
        raw_shell.set_raw_denoise_state(controller.state());
    });
    AiUiBridges {
        rgb: rgb_controller,
        raw: raw_controller,
    }
}
