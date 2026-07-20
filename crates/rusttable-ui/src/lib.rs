#![forbid(unsafe_code)]
#![doc = "GTK4 UI components and presentation models for `RustTable`."]

pub mod collection;
pub mod gtk_shell;
pub mod import;
pub mod input;
pub mod library;
pub mod presentation;

pub use collection::{CollectionItem, CollectionProperty, CollectionRule};
pub use gtk_shell::{
    CollectionControlAction, CollectionControlState, CollectionControls, CollectionFilterState,
    DarktableTheme, ExportAction, ExportPanel, ExportSize, ExposurePanel, GtkShell, ShellLayout,
    ShellRegion, ThemeRole, WorkspaceRole, apply_theme_role, darktable_theme_css,
    install_darktable_theme,
};
pub use import::{ImportPanelViewModel, ImportRowState, ImportRowViewModel};
pub use input::GtkInputAdapter;
pub use library::{LibraryFailureKind, LibraryFailureProjection, LibraryState};
pub use presentation::{
    PhotoCardViewModel, PhotoDetailViewModel, PhotoFactViewModel, PhotoWorkspaceViewModel,
    PhotoWorkspaceViewModelError, PresentationText, PresentationTextError, PreviewDimensions,
    PreviewDimensionsError, Rgba8PreviewMetadata, Rgba8PreviewMetadataError,
    SelectedPreviewFailure, SelectedPreviewState,
};
