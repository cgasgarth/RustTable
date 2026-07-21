//! Application-service bridges for the darkroom static panels.

use std::cell::RefCell;
use std::rc::Rc;

use crate::darkroom_services::{
    UnavailableMaskManagerService, UnavailableMultiscaleRetouchService,
};
use rusttable_ui::{
    MaskManagerAction, MaskManagerController, MultiscaleRetouchAction, MultiscaleRetouchController,
};

pub(super) fn install(shell: &rusttable_ui::GtkShell) {
    let mask_controller = Rc::new(RefCell::new(MaskManagerController::new(
        UnavailableMaskManagerService,
    )));
    shell.set_mask_manager_state(mask_controller.borrow().state());
    let mask_for_actions = Rc::clone(&mask_controller);
    let mask_shell = shell.clone();
    shell.connect_mask_manager_action(move |action: MaskManagerAction| {
        let mut controller = mask_for_actions.borrow_mut();
        if let Err(error) = controller.dispatch(&action) {
            mask_shell.set_darkroom_status(&error.to_string());
        }
        mask_shell.set_mask_manager_state(controller.state());
    });

    let retouch_controller = Rc::new(RefCell::new(MultiscaleRetouchController::new(
        UnavailableMultiscaleRetouchService,
    )));
    shell.set_multiscale_retouch_state(retouch_controller.borrow().state());
    let retouch_for_actions = Rc::clone(&retouch_controller);
    let retouch_shell = shell.clone();
    shell.connect_multiscale_retouch_action(move |action: MultiscaleRetouchAction| {
        let mut controller = retouch_for_actions.borrow_mut();
        if let Err(error) = controller.dispatch(action) {
            retouch_shell.set_darkroom_status(&error.to_string());
        }
        retouch_shell.set_multiscale_retouch_state(controller.state());
    });
}
