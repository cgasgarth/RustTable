//! Display-independent roles for the darktable-inspired desktop composition.

/// Named regions in the top-level `RustTable` desktop shell.
///
/// These mirror the enduring responsibilities of darktable's GUI layout while
/// leaving each region's content to the appropriate Rust subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellRegion {
    /// Native window titlebar and global actions.
    Header,
    /// Persistent navigation and collection controls.
    Sidebar,
    /// The active library or photo-editing workspace.
    Workspace,
}

impl ShellRegion {
    /// Stable identifier for GTK widget names and structural tests.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Header => "header",
            Self::Sidebar => "sidebar",
            Self::Workspace => "workspace",
        }
    }
}

/// The primary central workspaces inherited from darktable's view model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkspaceRole {
    /// Catalog browsing, equivalent to darktable's lighttable responsibility.
    #[default]
    Library,
    /// Single-image editing, equivalent to darktable's darkroom responsibility.
    PhotoDetail,
}

impl WorkspaceRole {
    /// Stable GTK stack child name.
    #[must_use]
    pub const fn stack_name(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::PhotoDetail => "photo-detail",
        }
    }

    /// Human-readable title for a placeholder workspace.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Library => "Library",
            Self::PhotoDetail => "Photo detail",
        }
    }
}

/// Stable description of the reusable main-window composition.
///
/// This type contains no GTK values so callers can select and test a layout
/// before creating an application or connecting to a display server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShellLayout {
    initial_workspace: WorkspaceRole,
}

impl ShellLayout {
    const REGIONS: [ShellRegion; 3] = [
        ShellRegion::Header,
        ShellRegion::Sidebar,
        ShellRegion::Workspace,
    ];

    /// Creates a shell layout opening in the given central workspace.
    #[must_use]
    pub const fn new(initial_workspace: WorkspaceRole) -> Self {
        Self { initial_workspace }
    }

    /// Ordered regions of the top-level desktop shell.
    #[must_use]
    pub const fn regions(self) -> &'static [ShellRegion] {
        &Self::REGIONS
    }

    /// Workspace that is visible when the shell is first presented.
    #[must_use]
    pub const fn initial_workspace(self) -> WorkspaceRole {
        self.initial_workspace
    }

    /// Returns whether the named role belongs to this composition.
    #[must_use]
    pub fn contains(self, region: ShellRegion) -> bool {
        self.regions().contains(&region)
    }
}

#[cfg(test)]
mod tests {
    use super::{ShellLayout, ShellRegion, WorkspaceRole};

    #[test]
    fn default_layout_preserves_the_darktable_desktop_regions() {
        let layout = ShellLayout::default();

        assert_eq!(
            layout.regions(),
            &[
                ShellRegion::Header,
                ShellRegion::Sidebar,
                ShellRegion::Workspace
            ]
        );
        assert!(layout.contains(ShellRegion::Sidebar));
        assert_eq!(layout.initial_workspace(), WorkspaceRole::Library);
    }

    #[test]
    fn photo_detail_is_a_first_class_workspace_role() {
        let layout = ShellLayout::new(WorkspaceRole::PhotoDetail);

        assert_eq!(layout.initial_workspace().stack_name(), "photo-detail");
        assert_eq!(layout.initial_workspace().title(), "Photo detail");
        assert_eq!(WorkspaceRole::Library.stack_name(), "library");
    }

    #[test]
    fn region_identifiers_are_unique_and_stable() {
        let identifiers: Vec<_> = ShellLayout::default()
            .regions()
            .iter()
            .map(|region| region.identifier())
            .collect();

        assert_eq!(identifiers, ["header", "sidebar", "workspace"]);
    }
}
