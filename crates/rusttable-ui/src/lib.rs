#![forbid(unsafe_code)]
#![doc = "GTK4 UI components and presentation models for `RustTable`."]

pub mod gtk_shell;
pub mod import;
pub mod library;
pub mod presentation;

pub use gtk_shell::{GtkShell, ShellLayout, ShellRegion, WorkspaceRole};
pub use import::{ImportPanelViewModel, ImportRowState, ImportRowViewModel};
pub use library::{LibraryFailureKind, LibraryFailureProjection, LibraryState};
pub use presentation::{
    PhotoCardViewModel, PhotoDetailViewModel, PhotoFactViewModel, PhotoWorkspaceViewModel,
    PhotoWorkspaceViewModelError, PresentationText, PresentationTextError, PreviewDimensions,
    PreviewDimensionsError, Rgba8PreviewMetadata, Rgba8PreviewMetadataError,
    SelectedPreviewFailure, SelectedPreviewState,
};
