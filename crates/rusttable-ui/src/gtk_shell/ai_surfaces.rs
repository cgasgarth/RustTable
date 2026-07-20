//! `GtkShell` accessors for the AI settings and Neural Restore surfaces.

use super::runtime::GtkShell;
use crate::ai_models::{AiModelsAction, AiModelsPanel, AiModelsViewModel};
use crate::neural_restore::{NeuralRestoreAction, NeuralRestorePanel, NeuralRestoreViewModel};

impl GtkShell {
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

    /// Returns the single-photo Neural Restore module in the darkroom rail.
    #[must_use]
    pub const fn neural_restore_panel(&self) -> &NeuralRestorePanel {
        &self.neural_restore_panel
    }

    /// Projects preview state into the Neural Restore comparison surface.
    pub fn set_neural_restore_state(&self, state: &NeuralRestoreViewModel) {
        self.neural_restore_panel.set_state(state);
    }

    /// Connects typed preview actions to the application preview service bridge.
    pub fn connect_neural_restore_action<F>(&self, handler: F)
    where
        F: Fn(NeuralRestoreAction) + 'static,
    {
        self.neural_restore_panel.connect_action(handler);
    }
}
