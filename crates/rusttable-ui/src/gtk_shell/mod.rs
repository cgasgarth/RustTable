//! GTK4 application-window composition for the `RustTable` desktop shell.
//!
//! The model is deliberately independent of GTK so it can be exercised without
//! initializing a display server. `GtkShell` is the runtime adapter that maps
//! those stable roles to GTK widgets.

mod collection_controls;
mod darktable_spec;
mod model;
mod photo_preview;
mod runtime;
mod theme;

pub use collection_controls::{
    CollectionControlAction, CollectionControlState, CollectionControls,
};
pub use darktable_spec::{
    ColorToken, DARKTABLE_COLORS, DARKTABLE_DESKTOP_SPEC, DESKTOP_REGIONS, DarktableColors,
    DarktableDesktopSpec, DesktopRegion, FilmstripHeights, LAYOUT_METRICS, LayoutMetrics,
    PANEL_SLOTS, PanelRole, PanelSlot as VisualPanelSlot, SidePanelWidths, TOP_BAR_SECTIONS,
    TopBarSection, ViewMode,
};
pub use model::{
    DarkroomWorkspaceViewModel, LibraryBrowserModel, LibraryPhoto, ModuleControlKind,
    ModuleControlViewModel, ModulePanelViewModel, PanelSlot, ShellLayout, ShellRegion,
    WorkspaceRole,
};
pub use photo_preview::{PhotoPreview, PhotoPreviewTextureError};
pub use runtime::GtkShell;
pub use theme::{
    DarktableTheme, ThemeRole, apply_theme_role, darktable_theme_css, install_darktable_theme,
};
