//! GTK4 application-window composition for the `RustTable` desktop shell.
//!
//! The model is deliberately independent of GTK so it can be exercised without
//! initializing a display server. `GtkShell` is the runtime adapter that maps
//! those stable roles to GTK widgets.

mod ai_surfaces;
mod collection_controls;
mod darkroom;
mod darkroom_controls;
mod darkroom_modules;
mod darkroom_status;
mod darktable_spec;
mod export_panel;
mod exposure_panel;
mod header;
mod left_panel;
mod lighttable;
mod lighttable_interaction;
mod lighttable_layout_controls;
mod lighttable_toolbar;
mod model;
mod photo_preview;
mod profile_controls;
mod runtime;
mod runtime_layout;
mod runtime_lighttable;
mod theme;
mod thumbnail;
mod viewport_canvas;

pub use crate::display_profile::DisplayProfileBanner;
pub use crate::import::ImportAction;
pub use collection_controls::{
    CollectionControlAction, CollectionControlState, CollectionControls, CollectionFilterState,
    LighttablePhotoState,
};
pub use darkroom::{
    DARKROOM_LAYOUT_FOCUS_ORDER, DARKROOM_WIDGET_IDS, DarkroomPanelVisibility,
    DarkroomPanelVisibilityAction, DarkroomView,
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
    DarkroomModuleAvailability, DarkroomModuleError, DarkroomModulePreset, DarkroomModuleSide,
    DarkroomModuleStatus, DarkroomModuleViewModel, DarkroomModulesViewModel, build_module_column,
    build_module_panel,
};
pub use darktable_spec::{
    ColorToken, DARKROOM_GEOMETRY, DARKROOM_OPERATION_FOCUS_ORDER, DARKROOM_RAIL_SCROLL_WIDGET_IDS,
    DARKTABLE_COLORS, DARKTABLE_DESKTOP_SPEC, DESKTOP_REGIONS, DarkroomGeometry,
    DarkroomGeometryReceipt, DarkroomWindowLayout, DarktableColors, DarktableDesktopSpec,
    DesktopRegion, FilmstripHeights, LAYOUT_METRICS, LIGHTTABLE_COMPOSITION,
    LIGHTTABLE_RIGHT_MODULES, LIGHTTABLE_TOOLBAR, LayoutMetrics, LighttableCompositionSpec,
    LighttableModuleSpec, LighttableToolbarSpec, PANEL_SLOTS, PanelRole,
    PanelSlot as VisualPanelSlot, SidePanelWidths, THUMBNAIL_METRICS, TOP_BAR_SECTIONS,
    ThumbnailMetrics, TopBarSection, ViewMode, darkroom_window_layout,
};
pub use export_panel::{ExportAction, ExportPanel, ExportSize};
pub use exposure_panel::ExposurePanel;
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
