use rusttable_core::PhotoId;

use crate::navigation::{NavigationIntent, WorkspaceRoute};
use crate::presentation::PhotoWorkspaceViewModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FocusTarget {
    SidebarToggle,
    Library,
    PhotoCard(PhotoId),
    BackToLibrary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputIntent {
    FocusNext,
    FocusPrevious,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "activation is exercised through the pure input reducer"
        )
    )]
    Activate,
    Escape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputEffect {
    None,
    ToggleSidebar,
    Navigate(NavigationIntent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InputState {
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
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the focused-target accessor is used by input tests"
        )
    )]
    pub(crate) fn focused(&self) -> FocusTarget {
        self.focused
    }

    pub(crate) fn is_focused(&self, target: FocusTarget) -> bool {
        self.focused == target
    }

    pub(crate) fn apply(
        &mut self,
        intent: InputIntent,
        sidebar_visible: bool,
        route: WorkspaceRoute,
        workspace: &PhotoWorkspaceViewModel,
    ) -> InputEffect {
        match intent {
            InputIntent::FocusNext => {
                self.move_focus(1, sidebar_visible, route, workspace);
                InputEffect::None
            }
            InputIntent::FocusPrevious => {
                self.move_focus(-1, sidebar_visible, route, workspace);
                InputEffect::None
            }
            InputIntent::Activate => self.activate(workspace),
            InputIntent::Escape => self.escape(route, workspace),
        }
    }

    pub(crate) fn reconcile(
        &mut self,
        sidebar_visible: bool,
        route: WorkspaceRoute,
        workspace: &PhotoWorkspaceViewModel,
    ) {
        let chain = focus_chain(sidebar_visible, route, workspace);
        if !chain.contains(&self.focused) {
            self.focused = chain[0];
        }
    }

    pub(crate) fn note_navigation(
        &mut self,
        intent: NavigationIntent,
        workspace: &PhotoWorkspaceViewModel,
    ) {
        match intent {
            NavigationIntent::ShowPhoto(photo_id) => {
                self.origin = Some(photo_id);
                self.focused = FocusTarget::BackToLibrary;
            }
            NavigationIntent::ShowLibrary => {
                self.focused = self.library_return_target(workspace);
                self.origin = None;
            }
        }
    }

    fn move_focus(
        &mut self,
        direction: isize,
        sidebar_visible: bool,
        route: WorkspaceRoute,
        workspace: &PhotoWorkspaceViewModel,
    ) {
        let chain = focus_chain(sidebar_visible, route, workspace);
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

    fn activate(&mut self, workspace: &PhotoWorkspaceViewModel) -> InputEffect {
        match self.focused {
            FocusTarget::SidebarToggle => InputEffect::ToggleSidebar,
            FocusTarget::Library => InputEffect::Navigate(NavigationIntent::ShowLibrary),
            FocusTarget::PhotoCard(photo_id) => {
                self.origin = Some(photo_id);
                self.focused = FocusTarget::BackToLibrary;
                InputEffect::Navigate(NavigationIntent::ShowPhoto(photo_id))
            }
            FocusTarget::BackToLibrary => {
                self.focused = self.library_return_target(workspace);
                self.origin = None;
                InputEffect::Navigate(NavigationIntent::ShowLibrary)
            }
        }
    }

    fn escape(
        &mut self,
        route: WorkspaceRoute,
        workspace: &PhotoWorkspaceViewModel,
    ) -> InputEffect {
        if matches!(route, WorkspaceRoute::PhotoDetail(_)) {
            self.focused = self.library_return_target(workspace);
            self.origin = None;
            InputEffect::Navigate(NavigationIntent::ShowLibrary)
        } else {
            InputEffect::None
        }
    }

    fn library_return_target(&self, workspace: &PhotoWorkspaceViewModel) -> FocusTarget {
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

pub(crate) fn focus_chain(
    sidebar_visible: bool,
    route: WorkspaceRoute,
    workspace: &PhotoWorkspaceViewModel,
) -> Vec<FocusTarget> {
    let mut chain = vec![FocusTarget::SidebarToggle];
    if sidebar_visible {
        chain.push(FocusTarget::Library);
    }
    match route {
        WorkspaceRoute::Library => {
            chain.extend(
                workspace
                    .cards()
                    .map(|card| FocusTarget::PhotoCard(card.id())),
            );
        }
        WorkspaceRoute::PhotoDetail(_) => chain.push(FocusTarget::BackToLibrary),
    }
    chain
}

#[cfg(test)]
mod tests {
    use rusttable_core::PhotoId;

    use super::{FocusTarget, InputEffect, InputIntent, InputState, focus_chain};
    use crate::presentation::{
        PhotoCardViewModel, PhotoDetailViewModel, PhotoWorkspaceViewModel, PresentationText,
    };

    fn text(value: &str) -> PresentationText {
        PresentationText::new(value).expect("test text is valid")
    }

    fn workspace() -> PhotoWorkspaceViewModel {
        let cards = vec![
            PhotoCardViewModel::new(PhotoId::new(1).unwrap(), text("One"), None),
            PhotoCardViewModel::new(PhotoId::new(2).unwrap(), text("Two"), None),
        ];
        let details = vec![
            PhotoDetailViewModel::new(PhotoId::new(1).unwrap(), text("One"), Vec::new()),
            PhotoDetailViewModel::new(PhotoId::new(2).unwrap(), text("Two"), Vec::new()),
        ];
        PhotoWorkspaceViewModel::new(cards, details).unwrap()
    }

    #[test]
    fn default_chain_starts_at_sidebar_toggle() {
        let model = workspace();

        assert_eq!(
            focus_chain(true, crate::navigation::WorkspaceRoute::Library, &model),
            vec![
                FocusTarget::SidebarToggle,
                FocusTarget::Library,
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

        for _ in 0..4 {
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

        assert_eq!(
            state.apply(InputIntent::Activate, true, library, &model),
            InputEffect::Navigate(crate::navigation::NavigationIntent::ShowPhoto(
                PhotoId::new(2).unwrap()
            ))
        );
        assert_eq!(state.focused(), FocusTarget::BackToLibrary);
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
}
