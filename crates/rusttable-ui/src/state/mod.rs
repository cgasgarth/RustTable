use rusttable_core::PhotoId;

use crate::input::{BasicEditIntent, FocusTarget, InputEffect, InputIntent, InputState, UiMessage};
use crate::{LibraryState, NavigationState, WorkspaceRoute};

#[derive(Debug, PartialEq, Eq)]
pub struct UiState {
    sidebar_visible: bool,
    navigation: NavigationState,
    library_state: LibraryState,
    input: InputState,
    import_panel: crate::ImportPanelViewModel,
    basic_edit: Option<crate::presentation::BasicEditInspectorViewModel>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            sidebar_visible: true,
            navigation: NavigationState::default(),
            library_state: LibraryState::default(),
            input: InputState::default(),
            import_panel: crate::ImportPanelViewModel::default(),
            basic_edit: None,
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
    pub const fn basic_edit(&self) -> Option<&crate::presentation::BasicEditInspectorViewModel> {
        self.basic_edit.as_ref()
    }

    pub fn set_basic_edit_values(
        &mut self,
        photo_id: PhotoId,
        values: crate::presentation::BasicEditValues,
    ) {
        if matches!(self.route(), WorkspaceRoute::PhotoDetail(current) if current == photo_id) {
            self.basic_edit = Some(
                crate::presentation::BasicEditInspectorViewModel::with_values(photo_id, values),
            );
            self.reconcile_input();
        }
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
        self.reconcile_input();
    }

    pub fn update_import_row(&mut self, item_id: u64, state: crate::ImportRowState) {
        self.import_panel.update_state(item_id, state);
        self.reconcile_input();
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
                if let InputIntent::BasicEdit(edit_intent) = intent {
                    self.apply_basic_edit(edit_intent);
                    self.reconcile_input();
                    return UiEffect::None;
                }
                let effect = self.input.apply_with_import_panel(
                    intent,
                    self.sidebar_visible,
                    self.route(),
                    &self.library_state,
                    &self.import_panel,
                );
                match effect {
                    InputEffect::None => {}
                    InputEffect::ToggleSidebar => self.sidebar_visible = !self.sidebar_visible,
                    InputEffect::Navigate(navigation) => {
                        let _ = self.navigation.apply(navigation);
                        self.input.note_navigation(navigation, &self.library_state);
                    }
                    InputEffect::RetryLibrary => return UiEffect::RetryLibrary,
                    InputEffect::ImportFiles => return UiEffect::ImportFiles,
                    InputEffect::CancelImport => return UiEffect::CancelImport,
                    InputEffect::RetryImport(item_id) => return UiEffect::RetryImport(item_id),
                    InputEffect::RemoveImportResult(item_id) => {
                        self.import_panel.remove(item_id);
                    }
                    InputEffect::CloseImportPanel => {
                        if !self.import_panel.active() {
                            self.import_panel = crate::ImportPanelViewModel::default();
                        }
                    }
                }
            }
        }
        self.reconcile_input();
        UiEffect::None
    }

    fn reconcile_input(&mut self) {
        self.input.reconcile_with_import_panel(
            self.sidebar_visible,
            self.route(),
            &self.library_state,
            &self.import_panel,
        );
        self.reconcile_basic_edit();
    }

    fn apply_basic_edit(&mut self, intent: BasicEditIntent) {
        let Some(inspector) = self.basic_edit.as_mut() else {
            return;
        };
        match intent {
            BasicEditIntent::Increment(field) => inspector.increment(field),
            BasicEditIntent::Decrement(field) => inspector.decrement(field),
            BasicEditIntent::Reset => inspector.reset(),
            BasicEditIntent::Commit => inspector.request_save(),
        }
    }

    fn reconcile_basic_edit(&mut self) {
        let selected_photo = match self.route() {
            WorkspaceRoute::PhotoDetail(photo_id) => self
                .library_state
                .ready_workspace()
                .and_then(|workspace| workspace.detail(photo_id).map(|_| photo_id)),
            WorkspaceRoute::Library => None,
        };
        match selected_photo {
            Some(photo_id)
                if self
                    .basic_edit
                    .is_none_or(|inspector| inspector.photo_id() != photo_id) =>
            {
                self.basic_edit = Some(crate::presentation::BasicEditInspectorViewModel::new(
                    photo_id,
                ));
            }
            Some(_) => {}
            None => self.basic_edit = None,
        }
    }

    #[must_use]
    pub fn focused_photo(&self) -> Option<PhotoId> {
        match self.input.focused() {
            FocusTarget::PhotoCard(photo_id) => Some(photo_id),
            FocusTarget::SidebarToggle
            | FocusTarget::Library
            | FocusTarget::ImportFiles
            | FocusTarget::CancelImport
            | FocusTarget::RetryImport(_)
            | FocusTarget::RemoveImportResult(_)
            | FocusTarget::CloseImportPanel
            | FocusTarget::RetryLibrary
            | FocusTarget::Preview(_)
            | FocusTarget::BasicEdit(_)
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
    use rusttable_core::FiniteF64;

    use super::{UiEffect, UiState};
    use crate::input::BasicEditIntent;
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

    #[test]
    fn preview_focus_returns_to_the_selected_catalog_card_on_escape() {
        let photo_id = rusttable_core::PhotoId::new(1).expect("test photo ID is non-zero");
        let workspace = crate::PhotoWorkspaceViewModel::new(
            vec![crate::PhotoCardViewModel::new(
                photo_id,
                crate::PresentationText::new("Photo 1").expect("test text is valid"),
                None,
            )],
            vec![crate::PhotoDetailViewModel::new(
                photo_id,
                crate::PresentationText::new("Photo 1").expect("test text is valid"),
                Vec::new(),
            )],
        )
        .expect("test workspace is valid");
        let mut state = UiState::with_photo_workspace(workspace);

        assert_eq!(
            state.handle(UiMessage::Navigate(crate::NavigationIntent::ShowPhoto(
                photo_id
            ),)),
            UiEffect::None
        );
        assert!(state.is_focused(crate::FocusTarget::Preview(photo_id)));

        assert_eq!(
            state.handle(UiMessage::Input(crate::InputIntent::Escape)),
            UiEffect::None
        );
        assert_eq!(state.route(), crate::WorkspaceRoute::Library);
        assert!(state.is_focused(crate::FocusTarget::PhotoCard(photo_id)));
    }

    #[test]
    fn basic_edit_intents_update_only_the_selected_photo_inspector() {
        let photo_id = rusttable_core::PhotoId::new(1).expect("test photo ID is non-zero");
        let workspace = crate::PhotoWorkspaceViewModel::new(
            vec![crate::PhotoCardViewModel::new(
                photo_id,
                crate::PresentationText::new("Photo 1").expect("test text is valid"),
                None,
            )],
            vec![crate::PhotoDetailViewModel::new(
                photo_id,
                crate::PresentationText::new("Photo 1").expect("test text is valid"),
                Vec::new(),
            )],
        )
        .expect("test workspace is valid");
        let mut state = UiState::with_photo_workspace(workspace);
        let _ = state.handle(UiMessage::Navigate(crate::NavigationIntent::ShowPhoto(
            photo_id,
        )));

        let _ = state.handle(UiMessage::Input(InputIntent::BasicEdit(
            BasicEditIntent::Increment(crate::presentation::BasicEditField::Exposure),
        )));
        let inspector = state.basic_edit().expect("selected photo has inspector");
        assert_eq!(
            inspector
                .values()
                .value(crate::presentation::BasicEditField::Exposure),
            FiniteF64::new(0.01).expect("edit step is finite")
        );
        assert_eq!(
            inspector.save_state(),
            crate::presentation::BasicEditSaveState::Unsaved
        );

        let _ = state.handle(UiMessage::Input(InputIntent::BasicEdit(
            BasicEditIntent::Reset,
        )));
        assert_eq!(
            state
                .basic_edit()
                .expect("selected photo has inspector")
                .save_state(),
            crate::presentation::BasicEditSaveState::Clean
        );
        let _ = state.handle(UiMessage::Input(InputIntent::BasicEdit(
            BasicEditIntent::Commit,
        )));
        assert_eq!(
            state
                .basic_edit()
                .expect("selected photo has inspector")
                .save_state(),
            crate::presentation::BasicEditSaveState::SaveRequested
        );
    }
}
