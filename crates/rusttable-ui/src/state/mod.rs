use rusttable_core::PhotoId;

use crate::{
    FocusTarget, InputEffect, InputState, LibraryState, NavigationState, UiMessage, WorkspaceRoute,
};

#[derive(Debug, PartialEq, Eq)]
pub struct UiState {
    sidebar_visible: bool,
    navigation: NavigationState,
    library_state: LibraryState,
    input: InputState,
    import_panel: crate::ImportPanelViewModel,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            sidebar_visible: true,
            navigation: NavigationState::default(),
            library_state: LibraryState::default(),
            input: InputState::default(),
            import_panel: crate::ImportPanelViewModel::default(),
        }
    }
}

impl UiState {
    #[must_use]
    pub fn with_library_state(library_state: LibraryState) -> Self {
        Self {
            library_state,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn with_photo_workspace(workspace: crate::PhotoWorkspaceViewModel) -> Self {
        Self::with_library_state(LibraryState::Ready(workspace))
    }

    #[must_use]
    pub fn sidebar_visible(&self) -> bool {
        self.sidebar_visible
    }

    #[must_use]
    pub fn route(&self) -> WorkspaceRoute {
        self.navigation.route()
    }

    #[must_use]
    pub fn library_state(&self) -> &LibraryState {
        &self.library_state
    }

    #[must_use]
    pub fn is_focused(&self, target: FocusTarget) -> bool {
        self.input.is_focused(target)
    }

    pub fn set_library_state(&mut self, library_state: LibraryState) {
        self.library_state = library_state;
        self.reconcile_input();
    }

    pub fn begin_library_load(&mut self) {
        self.set_library_state(LibraryState::Loading);
    }

    #[must_use]
    pub const fn import_panel(&self) -> &crate::ImportPanelViewModel {
        &self.import_panel
    }

    pub fn set_import_panel(&mut self, panel: crate::ImportPanelViewModel) {
        self.import_panel = panel;
    }

    #[must_use]
    pub fn handle(&mut self, message: UiMessage) -> UiEffect {
        match message {
            UiMessage::ToggleSidebar => {
                self.sidebar_visible = !self.sidebar_visible;
            }
            UiMessage::ImportFiles => return UiEffect::ImportFiles,
            UiMessage::CancelImport => return UiEffect::CancelImport,
            UiMessage::RetryImport(item_id) => return UiEffect::RetryImport(item_id),
            UiMessage::RemoveImportResult(item_id) => {
                self.import_panel.remove(item_id);
            }
            UiMessage::CloseImportPanel => {
                if !self.import_panel.active() {
                    self.import_panel = crate::ImportPanelViewModel::default();
                }
            }
            UiMessage::Navigate(intent) => {
                let _ = self.navigation.apply(intent);
                self.input.note_navigation(intent, &self.library_state);
            }
            UiMessage::RetryLibrary => return UiEffect::RetryLibrary,
            UiMessage::Input(intent) => {
                let effect = self.input.apply(
                    intent,
                    self.sidebar_visible,
                    self.route(),
                    &self.library_state,
                );
                match effect {
                    InputEffect::None => {}
                    InputEffect::ToggleSidebar => self.sidebar_visible = !self.sidebar_visible,
                    InputEffect::Navigate(navigation) => {
                        let _ = self.navigation.apply(navigation);
                    }
                    InputEffect::RetryLibrary => return UiEffect::RetryLibrary,
                    InputEffect::ImportFiles => return UiEffect::ImportFiles,
                }
            }
        }
        self.reconcile_input();
        UiEffect::None
    }

    fn reconcile_input(&mut self) {
        self.input
            .reconcile(self.sidebar_visible, self.route(), &self.library_state);
    }

    #[must_use]
    pub fn focused_photo(&self) -> Option<PhotoId> {
        match self.input.focused() {
            FocusTarget::PhotoCard(photo_id) => Some(photo_id),
            FocusTarget::SidebarToggle
            | FocusTarget::Library
            | FocusTarget::ImportFiles
            | FocusTarget::RetryLibrary
            | FocusTarget::BackToLibrary => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiEffect {
    None,
    RetryLibrary,
    ImportFiles,
    CancelImport,
    RetryImport(u64),
}

#[cfg(test)]
mod tests {
    use super::{UiEffect, UiState};
    use crate::{InputIntent, LibraryState, UiMessage};

    #[test]
    fn default_state_has_visible_sidebar_and_library_route() {
        let state = UiState::default();

        assert!(state.sidebar_visible());
        assert_eq!(state.route(), crate::WorkspaceRoute::Library);
        assert_eq!(state.library_state(), &LibraryState::Empty);
    }

    #[test]
    fn input_effects_are_reduced_inside_ui_state() {
        let mut state = UiState::default();

        assert_eq!(state.handle(UiMessage::ToggleSidebar), UiEffect::None);
        assert!(!state.sidebar_visible());
        assert_eq!(
            state.handle(UiMessage::Input(InputIntent::Activate)),
            UiEffect::None
        );
        assert!(state.sidebar_visible());
    }
}
