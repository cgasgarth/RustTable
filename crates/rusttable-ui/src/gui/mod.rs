//! GTK4 application-window composition for the `RustTable` desktop shell.
//!
//! The model is deliberately independent of GTK so it can be exercised without
//! initializing a display server. `GtkShell` is the runtime adapter that maps
//! those stable roles to GTK widgets.

mod ai_surfaces;
pub(crate) mod darktable_components;
pub(crate) mod darktable_spec;
pub(crate) mod display_profile;
mod header;
pub(crate) mod model;
pub(crate) mod runtime;
mod theme;

pub(crate) use crate::iop::controls as darkroom_controls;
pub(crate) use crate::iop::modules as darkroom_modules;
pub(crate) use crate::libs::collect as collection_controls;
pub(crate) use crate::libs::export as export_panel;
pub(crate) use crate::libs::navigation as left_panel;
pub(crate) use crate::libs::profiles as profile_controls;
pub(crate) use crate::views::darkroom;
pub(crate) use crate::views::lighttable::interaction as lighttable_interaction;
pub(crate) use crate::views::lighttable::layout_controls as lighttable_layout_controls;
pub(crate) use crate::views::lighttable::toolbar as lighttable_toolbar;
pub(crate) use crate::widgets::canvas as viewport_canvas;
pub(crate) use crate::widgets::preview as photo_preview;

pub use crate::import::ImportAction;
pub use crate::iop::exposure::ExposurePanel;
pub use collection_controls::{
    CollectionControlAction, CollectionControlState, CollectionControls, CollectionFilterState,
    LighttablePhotoState,
};
pub use darkroom::{
    DARKROOM_LAYOUT_FOCUS_ORDER, DARKROOM_VIEWPORT_WIDGET_IDS, DARKROOM_WIDGET_IDS,
    DarkroomPanelVisibility, DarkroomPanelVisibilityAction, DarkroomView,
};
pub use darkroom_controls::{
    DarkroomControlFeedback, DarkroomControlMessage, DarkroomControlModel,
    DarkroomControlModelError, DarkroomModuleControl, DarkroomModuleControlError,
    DarkroomOperationStackFeedback, DarkroomOperationStackUpdate,
    DarkroomOperationStackUpdateMessage, DarkroomParameterAssignment, DarkroomParameterControl,
    DarkroomParameterValue, DarkroomParameterValueKind, DarkroomSelection,
    ParameterValidationError,
};
pub use darkroom_modules::{
    DarkroomModuleAction, DarkroomModuleActionHandler, DarkroomModuleAvailability,
    DarkroomModuleError, DarkroomModuleGroup, DarkroomModulePreset, DarkroomModuleSide,
    DarkroomModuleStatus, DarkroomModuleViewModel, DarkroomModulesViewModel, build_module_column,
    build_module_panel, reference_modules,
};
pub use darktable_spec::{
    ColorToken, DARKROOM_GEOMETRY, DARKROOM_OPERATION_FOCUS_ORDER, DARKROOM_RAIL_SCROLL_WIDGET_IDS,
    DARKTABLE_COLORS, DARKTABLE_DESKTOP_SPEC, DARKTABLE_UI_TOKENS, DESKTOP_REGIONS,
    DarkroomGeometry, DarkroomGeometryReceipt, DarkroomWindowLayout, DarktableColors,
    DarktableDesktopSpec, DesktopRegion, FilmstripHeights, LAYOUT_METRICS, LIGHTTABLE_COMPOSITION,
    LIGHTTABLE_RIGHT_MODULES, LIGHTTABLE_TOOLBAR, LayoutMetrics, LighttableCompositionSpec,
    LighttableModuleSpec, LighttableToolbarSpec, ModuleControlAllocationReceipt, PANEL_SLOTS,
    PanelRole, PanelSlot as VisualPanelSlot, ResponsiveGeometryReceipt, SidePanelWidths,
    THUMBNAIL_METRICS, TOP_BAR_SECTIONS, ThumbnailMetrics, TopBarSection, TypographyTokens,
    ViewMode, darkroom_window_layout,
};
pub use display_profile::{DisplayProfileBanner, GtkMonitorInventory};
pub use export_panel::{ExportAction, ExportPanel, ExportSize};
pub use lighttable_interaction::{
    CullingRestriction, LighttableInteractionState, LighttableLayout, LighttableSelectionAction,
    LighttableZoom, NavigationDirection, PanBounds, SelectionModifiers,
};
pub use lighttable_layout_controls::{
    LighttableLayoutAction, LighttableLayoutControls, LighttablePanel,
};
pub use lighttable_toolbar::{
    LighttableColorLabel, LighttableRating, LighttableSort, LighttableToolbar,
    LighttableToolbarAction, LighttableToolbarState,
};
pub use model::{
    DarkroomWorkspaceViewModel, LibraryBrowserModel, LibraryPhoto, LighttableContentState,
    ModuleControlKind, ModuleControlViewModel, ModulePanelViewModel, PanelSlot, ShellLayout,
    ShellRegion, WorkspaceRole,
};
pub use photo_preview::{DarkroomSelectionState, PhotoPreview, PhotoPreviewTextureError};
pub use profile_controls::{
    MAX_PROFILE_WARNINGS, PROFILE_CONTROL_WIDGET_IDS, PROFILE_CONTROLS_FOCUS_ORDER, ProfileChoice,
    ProfileControlAction, ProfileControlMessage, ProfileControls, ProfileControlsState,
    ProfileMismatchKind, ProfileRole, ProfileRoleState, ProfileRoleStatus,
    ProfileUnavailableReason, ProfileWarning, ProfileWarningKind,
};
pub use runtime::GtkShell;
pub use theme::{
    DarktableTheme, ThemeRole, apply_theme_role, darktable_theme_css, install_darktable_theme,
};
pub use viewport_canvas::{
    FrameProjectionResult, MAX_PAN, MAX_ZOOM_PERCENT, MIN_ZOOM_PERCENT, PanOffset, PreviewFrameKey,
    PreviewFrameProjection, ProjectedImage, RedrawToken, ViewportCanvasState, ViewportSize,
    ViewportSizeError, ViewportZoom, ZoomPercent, ZoomPercentError,
};
