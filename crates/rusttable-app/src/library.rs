use crate::presentation::PhotoWorkspaceViewModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LibraryFailureKind {
    CatalogLocationUnavailable,
    RepositoryUnavailable,
    CorruptPersistedCatalog,
    PresentationConversionFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LibraryFailureProjection {
    title: &'static str,
    detail: &'static str,
}

impl LibraryFailureProjection {
    pub(crate) fn title(self) -> &'static str {
        self.title
    }

    pub(crate) fn detail(self) -> &'static str {
        self.detail
    }
}

impl LibraryFailureKind {
    pub(crate) fn projection(self) -> LibraryFailureProjection {
        let detail = match self {
            Self::CatalogLocationUnavailable => "The catalog location is unavailable.",
            Self::RepositoryUnavailable => "The catalog repository is unavailable.",
            Self::CorruptPersistedCatalog => "The persisted catalog is corrupt.",
            Self::PresentationConversionFailed => "A catalog record could not be shown.",
        };
        LibraryFailureProjection {
            title: "Library unavailable",
            detail,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum LibraryState {
    Loading,
    #[default]
    Empty,
    Ready(PhotoWorkspaceViewModel),
    Failed(LibraryFailureKind),
}

impl LibraryState {
    pub(crate) fn ready_workspace(&self) -> Option<&PhotoWorkspaceViewModel> {
        match self {
            Self::Ready(workspace) => Some(workspace),
            Self::Loading | Self::Empty | Self::Failed(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LibraryFailureKind, LibraryState};
    use crate::presentation::PhotoWorkspaceViewModel;

    #[test]
    fn library_states_are_closed_and_distinct() {
        let empty = LibraryState::Empty;
        let ready = LibraryState::Ready(PhotoWorkspaceViewModel::default());

        assert_ne!(LibraryState::Loading, empty);
        assert_ne!(empty, ready);
        assert_ne!(
            LibraryState::Failed(LibraryFailureKind::CatalogLocationUnavailable),
            LibraryState::Failed(LibraryFailureKind::RepositoryUnavailable),
        );
        assert_ne!(
            LibraryState::Failed(LibraryFailureKind::CorruptPersistedCatalog),
            LibraryState::Failed(LibraryFailureKind::PresentationConversionFailed),
        );
    }

    #[test]
    fn failure_projection_has_fixed_safe_copy() {
        for (kind, detail) in [
            (
                LibraryFailureKind::CatalogLocationUnavailable,
                "The catalog location is unavailable.",
            ),
            (
                LibraryFailureKind::RepositoryUnavailable,
                "The catalog repository is unavailable.",
            ),
            (
                LibraryFailureKind::CorruptPersistedCatalog,
                "The persisted catalog is corrupt.",
            ),
            (
                LibraryFailureKind::PresentationConversionFailed,
                "A catalog record could not be shown.",
            ),
        ] {
            let projection = kind.projection();
            assert_eq!(projection.title(), "Library unavailable");
            assert_eq!(projection.detail(), detail);
            assert!(!projection.detail().contains('/'));
            assert!(!projection.detail().contains("redb"));
        }
    }
}
