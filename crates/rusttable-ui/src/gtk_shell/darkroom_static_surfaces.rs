//! `GtkShell` accessors for static, service-backed darkroom panels.

use crate::{
    MaskManagerAction, MaskManagerSnapshot, MultiscaleRetouchAction, MultiscaleRetouchSnapshot,
};

use super::runtime::GtkShell;

impl GtkShell {
    pub fn set_mask_manager_state(&self, state: &MaskManagerSnapshot) {
        self.darkroom.set_mask_manager_state(state);
    }

    pub fn connect_mask_manager_action<F>(&self, handler: F)
    where
        F: Fn(MaskManagerAction) + 'static,
    {
        self.darkroom.connect_mask_manager_action(handler);
    }

    pub fn set_multiscale_retouch_state(&self, state: &MultiscaleRetouchSnapshot) {
        self.darkroom.set_multiscale_retouch_state(state);
    }

    pub fn connect_multiscale_retouch_action<F>(&self, handler: F)
    where
        F: Fn(MultiscaleRetouchAction) + 'static,
    {
        self.darkroom.connect_multiscale_retouch_action(handler);
    }
}
