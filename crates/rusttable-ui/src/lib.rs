#![forbid(unsafe_code)]
#![doc = "Reusable Iced UI contracts and presentation components for `RustTable`."]

pub mod input;
pub mod library;
pub mod navigation;
pub mod presentation;
pub mod state;
pub mod theme;
pub mod view;
pub mod widgets;

pub use input::{FocusTarget, InputEffect, InputIntent, InputState, UiMessage};
pub use library::{LibraryFailureKind, LibraryFailureProjection, LibraryState};
pub use navigation::{NavigationIntent, NavigationState, WorkspaceRoute};
pub use presentation::{
    PhotoCardViewModel, PhotoDetailViewModel, PhotoFactViewModel, PhotoWorkspaceViewModel,
    PhotoWorkspaceViewModelError, PresentationText, PresentationTextError,
};
pub use state::{UiEffect, UiState};
