//! GTK4 application-window composition for the `RustTable` desktop shell.
//!
//! The model is deliberately independent of GTK so it can be exercised without
//! initializing a display server. `GtkShell` is the runtime adapter that maps
//! those stable roles to GTK widgets.

mod model;
mod runtime;

pub use model::{
    DarkroomWorkspaceViewModel, LibraryBrowserModel, LibraryPhoto, ModuleControlKind,
    ModuleControlViewModel, ModulePanelViewModel, PanelSlot, ShellLayout, ShellRegion,
    WorkspaceRole,
};
pub use runtime::GtkShell;
