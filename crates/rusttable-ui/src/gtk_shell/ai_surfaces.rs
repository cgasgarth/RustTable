//! `GtkShell` accessors for the AI settings and Neural Restore surfaces.

use super::runtime::GtkShell;
use crate::ai_batch::{AiBatchAction, AiBatchPanel, AiBatchState};
use crate::ai_models::{AiModelsAction, AiModelsPanel, AiModelsViewModel};
use crate::rgb_denoise::{RgbDenoiseAction, RgbDenoiseViewModel};

impl GtkShell {
    /// Projects RGB denoise service state into the darkroom processing rail.
    pub fn set_rgb_denoise_state(&self, state: &RgbDenoiseViewModel) {
        self.darkroom.set_rgb_denoise_state(state);
    }

    /// Connects RGB denoise controls to the application-owned service controller.
    pub fn connect_rgb_denoise_action<F>(&self, handler: F)
    where
        F: Fn(RgbDenoiseAction) + 'static,
    {
        self.darkroom.connect_rgb_denoise_action(handler);
    }

    /// Returns the AI Models preferences surface. The model registry remains an application port.
    #[must_use]
    pub const fn ai_models_panel(&self) -> &AiModelsPanel {
        &self.ai_models_panel
    }

    /// Projects registry/service state into the AI Models settings window.
    pub fn set_ai_models_state(&self, state: &AiModelsViewModel) {
        self.ai_models_panel.set_state(state);
    }

    /// Connects typed AI model-management actions to the application service bridge.
    pub fn connect_ai_models_action<F>(&self, handler: F)
    where
        F: Fn(AiModelsAction) + 'static,
    {
        self.ai_models_panel.connect_action(handler);
    }

    #[must_use]
    pub const fn ai_batch_panel(&self) -> &AiBatchPanel {
        &self.ai_batch_panel
    }

    pub fn set_ai_batch_state(&self, state: &AiBatchState) {
        self.ai_batch_panel.set_state(state);
    }

    pub fn connect_ai_batch_action<F>(&self, handler: F)
    where
        F: Fn(AiBatchAction) + 'static,
    {
        self.ai_batch_panel.connect_action(handler);
    }
}
