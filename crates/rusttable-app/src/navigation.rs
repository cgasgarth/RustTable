use rusttable_core::PhotoId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceRoute {
    Library,
    PhotoDetail(PhotoId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavigationIntent {
    ShowLibrary,
    ShowPhoto(PhotoId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NavigationState {
    route: WorkspaceRoute,
}

impl Default for NavigationState {
    fn default() -> Self {
        Self {
            route: WorkspaceRoute::Library,
        }
    }
}

impl NavigationState {
    pub(crate) fn route(&self) -> WorkspaceRoute {
        self.route
    }

    pub(crate) fn apply(&mut self, intent: NavigationIntent) -> bool {
        let next_route = match intent {
            NavigationIntent::ShowLibrary => WorkspaceRoute::Library,
            NavigationIntent::ShowPhoto(photo_id) => WorkspaceRoute::PhotoDetail(photo_id),
        };
        let changed = self.route != next_route;
        self.route = next_route;
        changed
    }
}

#[cfg(test)]
mod tests {
    use rusttable_core::PhotoId;

    use super::{NavigationIntent, NavigationState, WorkspaceRoute};

    fn photo_id() -> PhotoId {
        PhotoId::new(42).expect("test photo ID is non-zero")
    }

    #[test]
    fn default_route_is_library() {
        assert_eq!(NavigationState::default().route(), WorkspaceRoute::Library);
    }

    #[test]
    fn library_changes_to_photo_detail() {
        let mut state = NavigationState::default();

        assert!(state.apply(NavigationIntent::ShowPhoto(photo_id())));
        assert_eq!(state.route(), WorkspaceRoute::PhotoDetail(photo_id()));
    }

    #[test]
    fn photo_detail_changes_to_library() {
        let mut state = NavigationState::default();
        let _ = state.apply(NavigationIntent::ShowPhoto(photo_id()));

        assert!(state.apply(NavigationIntent::ShowLibrary));
        assert_eq!(state.route(), WorkspaceRoute::Library);
    }

    #[test]
    fn repeated_navigation_is_idempotent() {
        let mut state = NavigationState::default();

        assert!(!state.apply(NavigationIntent::ShowLibrary));
        assert!(state.apply(NavigationIntent::ShowPhoto(photo_id())));
        assert!(!state.apply(NavigationIntent::ShowPhoto(photo_id())));
    }
}
