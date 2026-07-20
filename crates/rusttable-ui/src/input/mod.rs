use rusttable_core::PhotoId;

use crate::library::LibraryState;
use crate::navigation::{NavigationIntent, WorkspaceRoute};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiMessage {
    ToggleSidebar,
    ImportFiles,
    CancelImport,
    RetryImport(u64),
    RemoveImportResult(u64),
    CloseImportPanel,
    Navigate(NavigationIntent),
    RetryLibrary,
    Input(InputIntent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    SidebarToggle,
    Library,
    ImportFiles,
    CancelImport,
    RetryImport(u64),
    RemoveImportResult(u64),
    CloseImportPanel,
    RetryLibrary,
    Preview(PhotoId),
    PhotoCard(PhotoId),
    BackToLibrary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputIntent {
    FocusNext,
    FocusPrevious,
    FocusNextPhoto,
    FocusPreviousPhoto,
    Activate,
    Escape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEffect {
    None,
    ToggleSidebar,
    Navigate(NavigationIntent),
    RetryLibrary,
    ImportFiles,
    CancelImport,
    RetryImport(u64),
    RemoveImportResult(u64),
    CloseImportPanel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputState {
    focused: FocusTarget,
    origin: Option<PhotoId>,
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            focused: FocusTarget::SidebarToggle,
            origin: None,
        }
    }
}

impl InputState {
    #[must_use]
    pub fn focused(&self) -> FocusTarget {
        self.focused
    }

    #[must_use]
    pub fn is_focused(&self, target: FocusTarget) -> bool {
        self.focused == target
    }

    pub fn apply(
        &mut self,
        intent: InputIntent,
        sidebar_visible: bool,
        route: WorkspaceRoute,
        library_state: &LibraryState,
    ) -> InputEffect {
        self.apply_with_import_panel(
            intent,
            sidebar_visible,
            route,
            library_state,
            &crate::ImportPanelViewModel::default(),
        )
    }

    pub fn apply_with_import_panel(
        &mut self,
        intent: InputIntent,
        sidebar_visible: bool,
        route: WorkspaceRoute,
        library_state: &LibraryState,
        import_panel: &crate::ImportPanelViewModel,
    ) -> InputEffect {
        match intent {
            InputIntent::FocusNext => {
                self.move_focus(1, sidebar_visible, route, library_state, import_panel);
                InputEffect::None
            }
            InputIntent::FocusPrevious => {
                self.move_focus(-1, sidebar_visible, route, library_state, import_panel);
                InputEffect::None
            }
            InputIntent::FocusNextPhoto => {
                self.move_photo_focus(1, route, library_state);
                InputEffect::None
            }
            InputIntent::FocusPreviousPhoto => {
                self.move_photo_focus(-1, route, library_state);
                InputEffect::None
            }
            InputIntent::Activate => self.activate(library_state),
            InputIntent::Escape => self.escape(route, library_state),
        }
    }

    pub fn reconcile(
        &mut self,
        sidebar_visible: bool,
        route: WorkspaceRoute,
        library_state: &LibraryState,
    ) {
        self.reconcile_with_import_panel(
            sidebar_visible,
            route,
            library_state,
            &crate::ImportPanelViewModel::default(),
        );
    }

    pub fn reconcile_with_import_panel(
        &mut self,
        sidebar_visible: bool,
        route: WorkspaceRoute,
        library_state: &LibraryState,
        import_panel: &crate::ImportPanelViewModel,
    ) {
        let chain =
            focus_chain_with_import_panel(sidebar_visible, route, library_state, import_panel);
        if !chain.contains(&self.focused) {
            self.focused = chain[0];
        }
    }

    pub fn note_navigation(&mut self, intent: NavigationIntent, library_state: &LibraryState) {
        match intent {
            NavigationIntent::ShowPhoto(photo_id) => {
                self.origin = Some(photo_id);
                self.focused = FocusTarget::Preview(photo_id);
            }
            NavigationIntent::ShowLibrary => {
                self.focused = self.library_return_target(library_state);
                self.origin = None;
            }
        }
    }

    fn move_focus(
        &mut self,
        direction: isize,
        sidebar_visible: bool,
        route: WorkspaceRoute,
        library_state: &LibraryState,
        import_panel: &crate::ImportPanelViewModel,
    ) {
        let chain =
            focus_chain_with_import_panel(sidebar_visible, route, library_state, import_panel);
        let index = chain
            .iter()
            .position(|target| *target == self.focused)
            .unwrap_or(0);
        let next = if direction.is_positive() {
            (index + 1) % chain.len()
        } else if index == 0 {
            chain.len() - 1
        } else {
            index - 1
        };
        self.focused = chain[next];
    }

    fn move_photo_focus(
        &mut self,
        direction: isize,
        route: WorkspaceRoute,
        library_state: &LibraryState,
    ) {
        if !matches!(route, WorkspaceRoute::Library) {
            return;
        }
        let Some(workspace) = library_state.ready_workspace() else {
            return;
        };
        let photos: Vec<_> = workspace
            .cards()
            .map(crate::PhotoCardViewModel::id)
            .collect();
        let Some(last_index) = photos.len().checked_sub(1) else {
            return;
        };
        let next_index = match self.focused {
            FocusTarget::PhotoCard(photo_id) => {
                let index = photos.iter().position(|candidate| *candidate == photo_id);
                match (index, direction.is_positive()) {
                    (Some(index), true) if index == last_index => 0,
                    (Some(index), true) => index + 1,
                    (Some(0) | None, false) => last_index,
                    (Some(index), false) => index - 1,
                    (None, true) => 0,
                }
            }
            _ if direction.is_positive() => 0,
            _ => last_index,
        };
        self.focused = FocusTarget::PhotoCard(photos[next_index]);
    }

    fn activate(&mut self, library_state: &LibraryState) -> InputEffect {
        match self.focused {
            FocusTarget::SidebarToggle => InputEffect::ToggleSidebar,
            FocusTarget::Library => InputEffect::Navigate(NavigationIntent::ShowLibrary),
            FocusTarget::ImportFiles => InputEffect::ImportFiles,
            FocusTarget::CancelImport => InputEffect::CancelImport,
            FocusTarget::RetryImport(item_id) => InputEffect::RetryImport(item_id),
            FocusTarget::RemoveImportResult(item_id) => InputEffect::RemoveImportResult(item_id),
            FocusTarget::CloseImportPanel => InputEffect::CloseImportPanel,
            FocusTarget::PhotoCard(photo_id) => {
                self.origin = Some(photo_id);
                self.focused = FocusTarget::Preview(photo_id);
                InputEffect::Navigate(NavigationIntent::ShowPhoto(photo_id))
            }
            FocusTarget::Preview(_) => InputEffect::None,
            FocusTarget::BackToLibrary => {
                self.focused = self.library_return_target(library_state);
                self.origin = None;
                InputEffect::Navigate(NavigationIntent::ShowLibrary)
            }
            FocusTarget::RetryLibrary => InputEffect::RetryLibrary,
        }
    }

    fn escape(&mut self, route: WorkspaceRoute, library_state: &LibraryState) -> InputEffect {
        if matches!(route, WorkspaceRoute::PhotoDetail(_)) {
            self.focused = self.library_return_target(library_state);
            self.origin = None;
            InputEffect::Navigate(NavigationIntent::ShowLibrary)
        } else {
            InputEffect::None
        }
    }

    fn library_return_target(&self, library_state: &LibraryState) -> FocusTarget {
        let Some(workspace) = library_state.ready_workspace() else {
            return FocusTarget::SidebarToggle;
        };
        self.origin
            .filter(|photo_id| workspace.cards().any(|card| card.id() == *photo_id))
            .map(FocusTarget::PhotoCard)
            .or_else(|| {
                workspace
                    .cards()
                    .next()
                    .map(|card| FocusTarget::PhotoCard(card.id()))
            })
            .unwrap_or(FocusTarget::SidebarToggle)
    }
}

#[must_use]
pub fn focus_chain(
    sidebar_visible: bool,
    route: WorkspaceRoute,
    library_state: &LibraryState,
) -> Vec<FocusTarget> {
    focus_chain_with_import_panel(
        sidebar_visible,
        route,
        library_state,
        &crate::ImportPanelViewModel::default(),
    )
}

#[must_use]
pub fn focus_chain_with_import_panel(
    sidebar_visible: bool,
    route: WorkspaceRoute,
    library_state: &LibraryState,
    import_panel: &crate::ImportPanelViewModel,
) -> Vec<FocusTarget> {
    let mut chain = vec![FocusTarget::SidebarToggle];
    if sidebar_visible {
        chain.push(FocusTarget::Library);
    }
    match route {
        WorkspaceRoute::Library => {
            chain.push(FocusTarget::ImportFiles);
            if let Some(workspace) = library_state.ready_workspace() {
                chain.extend(
                    workspace
                        .cards()
                        .map(|card| FocusTarget::PhotoCard(card.id())),
                );
            }
            if matches!(library_state, LibraryState::Failed(_)) {
                chain.push(FocusTarget::RetryLibrary);
            }
            if import_panel.active() {
                chain.push(FocusTarget::CancelImport);
            } else if import_panel.is_visible() {
                for row in import_panel.rows() {
                    if row.state().can_retry() {
                        chain.push(FocusTarget::RetryImport(row.item_id()));
                    }
                    chain.push(FocusTarget::RemoveImportResult(row.item_id()));
                }
                chain.push(FocusTarget::CloseImportPanel);
            }
        }
        WorkspaceRoute::PhotoDetail(photo_id) => {
            chain.push(FocusTarget::Preview(photo_id));
            chain.push(FocusTarget::BackToLibrary);
        }
    }
    chain
}

#[cfg(test)]
mod tests {
    use rusttable_core::PhotoId;

    use super::{
        FocusTarget, InputEffect, InputIntent, InputState, focus_chain,
        focus_chain_with_import_panel,
    };
    use crate::library::{LibraryFailureKind, LibraryState};
    use crate::presentation::{
        PhotoCardViewModel, PhotoDetailViewModel, PhotoWorkspaceViewModel, PresentationText,
    };
    use crate::{ImportPanelViewModel, ImportRowState, ImportRowViewModel};

    fn text(value: &str) -> PresentationText {
        PresentationText::new(value).expect("test text is valid")
    }

    fn workspace() -> LibraryState {
        let cards = vec![
            PhotoCardViewModel::new(PhotoId::new(1).unwrap(), text("One"), None),
            PhotoCardViewModel::new(PhotoId::new(2).unwrap(), text("Two"), None),
        ];
        let details = vec![
            PhotoDetailViewModel::new(PhotoId::new(1).unwrap(), text("One"), Vec::new()),
            PhotoDetailViewModel::new(PhotoId::new(2).unwrap(), text("Two"), Vec::new()),
        ];
        LibraryState::Ready(PhotoWorkspaceViewModel::new(cards, details).unwrap())
    }

    #[test]
    fn default_chain_starts_at_sidebar_toggle() {
        let model = workspace();

        assert_eq!(
            focus_chain(true, crate::navigation::WorkspaceRoute::Library, &model),
            vec![
                FocusTarget::SidebarToggle,
                FocusTarget::Library,
                FocusTarget::ImportFiles,
                FocusTarget::PhotoCard(PhotoId::new(1).unwrap()),
                FocusTarget::PhotoCard(PhotoId::new(2).unwrap()),
            ]
        );
        assert_eq!(InputState::default().focused(), FocusTarget::SidebarToggle);
    }

    #[test]
    fn focus_wraps_forward_and_backward() {
        let model = workspace();
        let route = crate::navigation::WorkspaceRoute::Library;
        let mut state = InputState::default();

        for _ in 0..5 {
            let _ = state.apply(InputIntent::FocusNext, true, route, &model);
        }
        assert_eq!(state.focused(), FocusTarget::SidebarToggle);
        let _ = state.apply(InputIntent::FocusPrevious, true, route, &model);
        assert_eq!(
            state.focused(),
            FocusTarget::PhotoCard(PhotoId::new(2).unwrap())
        );
    }

    #[test]
    fn photo_navigation_moves_only_between_catalog_cards() {
        let model = workspace();
        let route = crate::navigation::WorkspaceRoute::Library;
        let mut state = InputState::default();

        let _ = state.apply(InputIntent::FocusNextPhoto, true, route, &model);
        assert_eq!(
            state.focused(),
            FocusTarget::PhotoCard(PhotoId::new(1).unwrap())
        );
        let _ = state.apply(InputIntent::FocusNextPhoto, true, route, &model);
        assert_eq!(
            state.focused(),
            FocusTarget::PhotoCard(PhotoId::new(2).unwrap())
        );
        let _ = state.apply(InputIntent::FocusNextPhoto, true, route, &model);
        assert_eq!(
            state.focused(),
            FocusTarget::PhotoCard(PhotoId::new(1).unwrap())
        );
        let _ = state.apply(InputIntent::FocusPreviousPhoto, true, route, &model);
        assert_eq!(
            state.focused(),
            FocusTarget::PhotoCard(PhotoId::new(2).unwrap())
        );
    }

    #[test]
    fn hidden_sidebar_repairs_library_focus() {
        let model = workspace();
        let route = crate::navigation::WorkspaceRoute::Library;
        let mut state = InputState::default();
        let _ = state.apply(InputIntent::FocusNext, true, route, &model);

        state.reconcile(false, route, &model);

        assert_eq!(state.focused(), FocusTarget::SidebarToggle);
    }

    #[test]
    fn activation_and_escape_restore_origin() {
        let model = workspace();
        let library = crate::navigation::WorkspaceRoute::Library;
        let detail = crate::navigation::WorkspaceRoute::PhotoDetail(PhotoId::new(2).unwrap());
        let mut state = InputState::default();
        let _ = state.apply(InputIntent::FocusNext, true, library, &model);
        let _ = state.apply(InputIntent::FocusNext, true, library, &model);
        let _ = state.apply(InputIntent::FocusNext, true, library, &model);
        let _ = state.apply(InputIntent::FocusNext, true, library, &model);

        assert_eq!(
            state.apply(InputIntent::Activate, true, library, &model),
            InputEffect::Navigate(crate::navigation::NavigationIntent::ShowPhoto(
                PhotoId::new(2).unwrap()
            ))
        );
        assert_eq!(
            state.focused(),
            FocusTarget::Preview(PhotoId::new(2).unwrap())
        );
        assert_eq!(
            state.apply(InputIntent::Escape, true, detail, &model),
            InputEffect::Navigate(crate::navigation::NavigationIntent::ShowLibrary)
        );
        assert_eq!(
            state.focused(),
            FocusTarget::PhotoCard(PhotoId::new(2).unwrap())
        );
    }

    #[test]
    fn detail_focus_chain_places_preview_before_back_control() {
        let model = workspace();
        let photo_id = PhotoId::new(2).unwrap();

        assert_eq!(
            focus_chain(
                true,
                crate::navigation::WorkspaceRoute::PhotoDetail(photo_id),
                &model,
            ),
            vec![
                FocusTarget::SidebarToggle,
                FocusTarget::Library,
                FocusTarget::Preview(photo_id),
                FocusTarget::BackToLibrary,
            ]
        );
    }

    #[test]
    fn missing_origin_falls_back_to_first_photo() {
        let model = workspace();
        let mut state = InputState::default();
        state.note_navigation(
            crate::navigation::NavigationIntent::ShowPhoto(PhotoId::new(99).unwrap()),
            &model,
        );
        let _ = state.apply(
            InputIntent::Escape,
            true,
            crate::navigation::WorkspaceRoute::PhotoDetail(PhotoId::new(99).unwrap()),
            &model,
        );

        assert_eq!(
            state.focused(),
            FocusTarget::PhotoCard(PhotoId::new(1).unwrap())
        );
    }

    #[test]
    fn non_ready_library_states_remove_photo_card_focus() {
        let photo_id = PhotoId::new(1).expect("test photo ID is non-zero");
        for library_state in [
            LibraryState::Loading,
            LibraryState::Empty,
            LibraryState::Failed(LibraryFailureKind::RepositoryUnavailable),
        ] {
            let mut state = InputState {
                focused: FocusTarget::PhotoCard(photo_id),
                origin: Some(photo_id),
            };

            state.reconcile(
                true,
                crate::navigation::WorkspaceRoute::Library,
                &library_state,
            );

            assert_eq!(state.focused(), FocusTarget::SidebarToggle);
            assert!(
                !focus_chain(
                    true,
                    crate::navigation::WorkspaceRoute::Library,
                    &library_state,
                )
                .iter()
                .any(|target| matches!(target, FocusTarget::PhotoCard(_)))
            );
        }
    }

    #[test]
    fn failed_library_appends_retry_focus_and_activates_it() {
        let library = LibraryState::Failed(LibraryFailureKind::RepositoryUnavailable);
        let route = crate::navigation::WorkspaceRoute::Library;
        let chain = focus_chain(true, route, &library);
        assert_eq!(
            chain,
            vec![
                FocusTarget::SidebarToggle,
                FocusTarget::Library,
                FocusTarget::ImportFiles,
                FocusTarget::RetryLibrary,
            ]
        );

        let mut state = InputState::default();
        let _ = state.apply(InputIntent::FocusNext, true, route, &library);
        let _ = state.apply(InputIntent::FocusNext, true, route, &library);
        let _ = state.apply(InputIntent::FocusNext, true, route, &library);
        assert_eq!(state.focused(), FocusTarget::RetryLibrary);
        assert_eq!(
            state.apply(InputIntent::Activate, true, route, &library),
            InputEffect::RetryLibrary
        );
    }

    #[test]
    fn import_actions_join_focus_order_and_activate_with_typed_effects() {
        let library = LibraryState::Empty;
        let route = crate::navigation::WorkspaceRoute::Library;
        let failed = ImportPanelViewModel::new(
            vec![ImportRowViewModel::new(
                7,
                text("photo.png"),
                ImportRowState::Failed,
            )],
            false,
        );
        let chain = focus_chain_with_import_panel(true, route, &library, &failed);
        assert_eq!(
            &chain[3..],
            [
                FocusTarget::RetryImport(7),
                FocusTarget::RemoveImportResult(7),
                FocusTarget::CloseImportPanel,
            ]
        );
        for (target, expected) in [
            (FocusTarget::RetryImport(7), InputEffect::RetryImport(7)),
            (
                FocusTarget::RemoveImportResult(7),
                InputEffect::RemoveImportResult(7),
            ),
            (FocusTarget::CloseImportPanel, InputEffect::CloseImportPanel),
        ] {
            let mut input = InputState {
                focused: target,
                origin: None,
            };
            assert_eq!(
                input.apply_with_import_panel(
                    InputIntent::Activate,
                    true,
                    route,
                    &library,
                    &failed,
                ),
                expected
            );
        }

        let active = ImportPanelViewModel::new(
            vec![ImportRowViewModel::new(
                1,
                text("photo.png"),
                ImportRowState::Hashing,
            )],
            true,
        );
        assert_eq!(
            focus_chain_with_import_panel(true, route, &library, &active).last(),
            Some(&FocusTarget::CancelImport)
        );
        let mut input = InputState {
            focused: FocusTarget::CancelImport,
            origin: None,
        };
        assert_eq!(
            input.apply_with_import_panel(InputIntent::Activate, true, route, &library, &active,),
            InputEffect::CancelImport
        );
    }
}
