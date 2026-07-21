//! Display-independent Darktable-style GTK shell topology and library models.

use rusttable_core::PhotoId;

use crate::presentation::{
    DarkroomHistoryViewModel, DarkroomPanelProjection, DarkroomSnapshotsViewModel,
    PhotoDetailViewModel, PhotoWorkspaceViewModel, PresentationText, ThumbnailIndicators,
};

/// Persistent regions in the Darktable desktop composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellRegion {
    /// Application header with branding, global controls, and view selection.
    Header,
    /// Collection and navigation modules.
    LeftPanel,
    /// Lighttable or darkroom central content.
    Workspace,
    /// Processing and metadata modules.
    RightPanel,
    /// Persistent image strip for current-collection navigation.
    Filmstrip,
}

impl ShellRegion {
    /// Stable identifier for GTK widget names and structural tests.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Header => "header",
            Self::LeftPanel => "left-panel",
            Self::Workspace => "workspace",
            Self::RightPanel => "right-panel",
            Self::Filmstrip => "filmstrip",
        }
    }
}

/// Fine-grained panel slots mirrored from Darktable's GTK layout API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelSlot {
    HeaderLeft,
    HeaderCenter,
    HeaderRight,
    LeftTop,
    LeftCenter,
    LeftBottom,
    RightTop,
    RightCenter,
    RightBottom,
    CenterTop,
    CenterBottom,
    Bottom,
}

impl PanelSlot {
    /// Stable GTK widget name for this shell slot.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::HeaderLeft => "header-left",
            Self::HeaderCenter => "header-center",
            Self::HeaderRight => "header-right",
            Self::LeftTop => "left-top",
            Self::LeftCenter => "left-center",
            Self::LeftBottom => "left-bottom",
            Self::RightTop => "right-top",
            Self::RightCenter => "right-center",
            Self::RightBottom => "right-bottom",
            Self::CenterTop => "center-top",
            Self::CenterBottom => "center-bottom",
            Self::Bottom => "bottom",
        }
    }
}

/// The two top-level editing modes exposed directly by Darktable's view switcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkspaceRole {
    /// Catalog browser, collection tools, and thumbnail grid.
    #[default]
    Lighttable,
    /// Single-image processing workspace and image-operation controls.
    Darkroom,
}

impl WorkspaceRole {
    /// Stable GTK stack child name.
    #[must_use]
    pub const fn stack_name(self) -> &'static str {
        match self {
            Self::Lighttable => "lighttable",
            Self::Darkroom => "darkroom",
        }
    }

    /// Human-readable mode label.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Lighttable => "lighttable",
            Self::Darkroom => "darkroom",
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
    const REGIONS: [ShellRegion; 5] = [
        ShellRegion::Header,
        ShellRegion::LeftPanel,
        ShellRegion::Workspace,
        ShellRegion::RightPanel,
        ShellRegion::Filmstrip,
    ];
    const SLOTS: [PanelSlot; 12] = [
        PanelSlot::HeaderLeft,
        PanelSlot::HeaderCenter,
        PanelSlot::HeaderRight,
        PanelSlot::LeftTop,
        PanelSlot::LeftCenter,
        PanelSlot::LeftBottom,
        PanelSlot::RightTop,
        PanelSlot::RightCenter,
        PanelSlot::RightBottom,
        PanelSlot::CenterTop,
        PanelSlot::CenterBottom,
        PanelSlot::Bottom,
    ];

    /// Creates a shell layout opening in the given central workspace.
    #[must_use]
    pub const fn new(initial_workspace: WorkspaceRole) -> Self {
        Self { initial_workspace }
    }

    /// Ordered persistent regions of the top-level desktop shell.
    #[must_use]
    pub const fn regions(self) -> &'static [ShellRegion] {
        &Self::REGIONS
    }

    /// Ordered fine-grained slots that host Darktable-style modules.
    #[must_use]
    pub const fn panel_slots(self) -> &'static [PanelSlot] {
        &Self::SLOTS
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

/// One selectable entry in the GTK lighttable browser.
///
/// This is intentionally GTK-free: the same projection can be covered by
/// deterministic tests before a display server is initialized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryPhoto {
    id: PhotoId,
    title: String,
    secondary: Option<String>,
    indicators: ThumbnailIndicators,
}

impl LibraryPhoto {
    #[must_use]
    pub fn id(&self) -> PhotoId {
        self.id
    }

    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    #[must_use]
    pub fn secondary(&self) -> Option<&str> {
        self.secondary.as_deref()
    }

    #[must_use]
    pub const fn indicators(&self) -> ThumbnailIndicators {
        self.indicators
    }
}

/// Display-ready lighttable content projected from the product presentation model.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LibraryBrowserModel {
    photos: Vec<LibraryPhoto>,
}

impl LibraryBrowserModel {
    /// Builds a lighttable projection without initializing GTK.
    #[must_use]
    pub fn from_workspace(workspace: &PhotoWorkspaceViewModel) -> Self {
        let photos = workspace
            .cards()
            .map(|card| LibraryPhoto {
                id: card.id(),
                title: card.title().as_str().to_owned(),
                secondary: card.secondary().map(|text| text.as_str().to_owned()),
                indicators: card.indicators(),
            })
            .collect();
        Self { photos }
    }

    #[must_use = "iterate over the lighttable entries"]
    pub fn photos(&self) -> impl ExactSizeIterator<Item = &LibraryPhoto> {
        self.photos.iter()
    }
}

/// The two render states of the lighttable canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LighttableContentState {
    /// The collection has no visible photos.
    Empty,
    /// The collection has at least one visible photo.
    Grid,
}

impl LighttableContentState {
    /// Converts a rendered-photo count into the stack child used by GTK.
    #[must_use]
    pub const fn from_rendered_count(count: usize) -> Self {
        if count == 0 { Self::Empty } else { Self::Grid }
    }

    /// Returns the stable GTK stack child name for this state.
    #[must_use]
    pub const fn stack_name(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Grid => "grid",
        }
    }
}

/// The GTK control shape requested by one darkroom module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleControlKind {
    Slider,
    Toggle,
    Choice,
}

/// One typed control in a darkroom module panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleControlViewModel {
    label: PresentationText,
    kind: ModuleControlKind,
}

impl ModuleControlViewModel {
    #[must_use]
    pub const fn new(label: PresentationText, kind: ModuleControlKind) -> Self {
        Self { label, kind }
    }

    #[must_use]
    pub const fn label(&self) -> &PresentationText {
        &self.label
    }

    #[must_use]
    pub const fn kind(&self) -> ModuleControlKind {
        self.kind
    }
}

/// A darkroom module assigned to one of the side-panel centers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModulePanelViewModel {
    title: PresentationText,
    controls: Vec<ModuleControlViewModel>,
}

impl ModulePanelViewModel {
    #[must_use]
    pub fn new(title: PresentationText, controls: Vec<ModuleControlViewModel>) -> Self {
        Self { title, controls }
    }

    #[must_use]
    pub const fn title(&self) -> &PresentationText {
        &self.title
    }

    #[must_use = "iterate over the module controls"]
    pub fn controls(&self) -> impl ExactSizeIterator<Item = &ModuleControlViewModel> {
        self.controls.iter()
    }
}

/// Typed content for the Darktable-style darkroom surface.
///
/// Controllers own this model and update the shell without introducing a
/// dependency from `rusttable-ui` back to `rusttable-app`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DarkroomWorkspaceViewModel {
    detail: PhotoDetailViewModel,
    left_modules: Vec<ModulePanelViewModel>,
    right_modules: Vec<ModulePanelViewModel>,
    history: Option<DarkroomPanelProjection<DarkroomHistoryViewModel>>,
    snapshots: Option<DarkroomPanelProjection<DarkroomSnapshotsViewModel>>,
}

impl DarkroomWorkspaceViewModel {
    #[must_use]
    pub fn new(
        detail: PhotoDetailViewModel,
        left_modules: Vec<ModulePanelViewModel>,
        right_modules: Vec<ModulePanelViewModel>,
    ) -> Self {
        Self {
            detail,
            left_modules,
            right_modules,
            history: None,
            snapshots: None,
        }
    }

    #[must_use]
    pub const fn detail(&self) -> &PhotoDetailViewModel {
        &self.detail
    }

    #[must_use = "iterate over the left-panel modules"]
    pub fn left_modules(&self) -> impl ExactSizeIterator<Item = &ModulePanelViewModel> {
        self.left_modules.iter()
    }

    #[must_use = "iterate over the right-panel modules"]
    pub fn right_modules(&self) -> impl ExactSizeIterator<Item = &ModulePanelViewModel> {
        self.right_modules.iter()
    }

    /// Adds a revision-checked history projection without changing module ownership.
    #[must_use]
    pub fn with_history_projection(
        mut self,
        projection: DarkroomPanelProjection<DarkroomHistoryViewModel>,
    ) -> Self {
        self.history = Some(projection);
        self
    }

    /// Adds a snapshot projection without introducing a second persistence path.
    #[must_use]
    pub fn with_snapshots_projection(
        mut self,
        projection: DarkroomPanelProjection<DarkroomSnapshotsViewModel>,
    ) -> Self {
        self.snapshots = Some(projection);
        self
    }

    #[must_use]
    pub const fn history_projection(
        &self,
    ) -> Option<&DarkroomPanelProjection<DarkroomHistoryViewModel>> {
        self.history.as_ref()
    }

    #[must_use]
    pub const fn snapshots_projection(
        &self,
    ) -> Option<&DarkroomPanelProjection<DarkroomSnapshotsViewModel>> {
        self.snapshots.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use rusttable_core::PhotoId;

    use super::{
        DarkroomWorkspaceViewModel, LibraryBrowserModel, LighttableContentState, ModuleControlKind,
        ModuleControlViewModel, ModulePanelViewModel, PanelSlot, ShellLayout, ShellRegion,
        WorkspaceRole,
    };
    use crate::presentation::{
        PhotoCardViewModel, PhotoDetailViewModel, PhotoWorkspaceViewModel, PresentationText,
    };

    fn id(value: u128) -> PhotoId {
        PhotoId::new(value).expect("non-zero test photo identifier")
    }

    fn text(value: &str) -> PresentationText {
        PresentationText::new(value).expect("valid test presentation text")
    }

    #[test]
    fn layout_preserves_darktable_regions_and_module_slots() {
        let layout = ShellLayout::default();

        assert_eq!(
            layout.regions(),
            &[
                ShellRegion::Header,
                ShellRegion::LeftPanel,
                ShellRegion::Workspace,
                ShellRegion::RightPanel,
                ShellRegion::Filmstrip,
            ]
        );
        assert!(layout.contains(ShellRegion::RightPanel));
        assert_eq!(layout.initial_workspace(), WorkspaceRole::Lighttable);
        assert_eq!(layout.panel_slots().len(), 12);
        assert_eq!(PanelSlot::RightCenter.identifier(), "right-center");
        assert_eq!(PanelSlot::Bottom.identifier(), "bottom");
    }

    #[test]
    fn darkroom_is_a_first_class_top_level_mode() {
        let layout = ShellLayout::new(WorkspaceRole::Darkroom);

        assert_eq!(layout.initial_workspace().stack_name(), "darkroom");
        assert_eq!(layout.initial_workspace().title(), "darkroom");
        assert_eq!(WorkspaceRole::Lighttable.stack_name(), "lighttable");
    }

    #[test]
    fn lighttable_projection_preserves_card_order_and_display_text() {
        let workspace = PhotoWorkspaceViewModel::new(
            vec![
                PhotoCardViewModel::new(id(2), text("Second"), None),
                PhotoCardViewModel::new(id(1), text("First"), Some(text("RAW · 24 MP"))),
            ],
            vec![
                PhotoDetailViewModel::new(id(1), text("First"), Vec::new()),
                PhotoDetailViewModel::new(id(2), text("Second"), Vec::new()),
            ],
        )
        .expect("matching cards and detail models");

        let browser = LibraryBrowserModel::from_workspace(&workspace);
        let entries = browser.photos().collect::<Vec<_>>();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id(), id(2));
        assert_eq!(entries[0].title(), "Second");
        assert_eq!(entries[0].secondary(), None);
        assert_eq!(entries[1].id(), id(1));
        assert_eq!(entries[1].title(), "First");
        assert_eq!(entries[1].secondary(), Some("RAW · 24 MP"));
    }

    #[test]
    fn lighttable_empty_state_uses_empty_stack_child_only_without_visible_photos() {
        assert_eq!(
            LighttableContentState::from_rendered_count(0).stack_name(),
            "empty"
        );
        assert_eq!(
            LighttableContentState::from_rendered_count(1).stack_name(),
            "grid"
        );
    }

    #[test]
    fn darkroom_model_keeps_controller_owned_module_assignments() {
        let detail = PhotoDetailViewModel::new(id(1), text("First"), Vec::new());
        let exposure = ModulePanelViewModel::new(
            text("exposure"),
            vec![ModuleControlViewModel::new(
                text("exposure"),
                ModuleControlKind::Slider,
            )],
        );
        let navigation = ModulePanelViewModel::new(
            text("navigation"),
            vec![ModuleControlViewModel::new(
                text("zoom"),
                ModuleControlKind::Choice,
            )],
        );

        let darkroom = DarkroomWorkspaceViewModel::new(detail, vec![navigation], vec![exposure]);

        assert_eq!(darkroom.detail().id(), id(1));
        assert_eq!(
            darkroom.left_modules().next().unwrap().title().as_str(),
            "navigation"
        );
        let control = darkroom
            .right_modules()
            .next()
            .unwrap()
            .controls()
            .next()
            .unwrap();
        assert_eq!(control.label().as_str(), "exposure");
        assert_eq!(control.kind(), ModuleControlKind::Slider);
    }
}
