#![forbid(unsafe_code)]
#![doc = "GTK4 UI components and presentation models for `RustTable`."]

pub mod ai_batch;
pub mod ai_models;
pub mod camera;
pub mod collection;
pub mod darkroom_histogram;
pub mod display_profile;
pub mod external_editor;
pub mod gui;
pub mod import;
pub mod input;
pub mod input_mapping;
pub mod iop;
pub mod library;
pub mod libs;
pub mod mask_manager;
pub mod multiscale_retouch;
pub mod neural_restore;
pub mod presentation;
pub mod raw_denoise;
pub mod rgb_denoise;
pub mod viewport_presentation;
pub mod views;
pub mod widgets;

pub use gui as gtk_shell;

pub use ai_batch::{
    AI_BATCH_FOCUS_ORDER, AiBatchAction, AiBatchCollision, AiBatchController,
    AiBatchControllerError, AiBatchEligibility, AiBatchEnqueuePolicy, AiBatchItem, AiBatchPanel,
    AiBatchPreflight, AiBatchRecipe, AiBatchReview, AiBatchSelection, AiBatchServiceError,
    AiBatchServicePort, AiBatchStage, AiBatchState, AiBatchTask,
};
pub use ai_models::{
    AI_MODELS_FOCUS_ORDER, AiModelsAction, AiModelsController, AiModelsControllerError,
    AiModelsDisplayState, AiModelsFailure, AiModelsPanel, AiModelsServiceError,
    AiModelsServicePort, AiModelsSnapshot, AiModelsViewModel, AiProvider, AiProviderPolicy, AiTask,
    InstallSummary, InstalledModel, ModelHash, ModelServiceState, ProviderCapability,
    QualificationJob,
};
pub use camera::{
    CAMERA_FOCUS_ORDER, CameraAction, CameraController, CameraControllerError, CameraPanel,
    CameraViewModel,
};
pub use collection::{CollectionItem, CollectionProperty, CollectionRule};
pub use darkroom_histogram::{
    DARKROOM_HISTOGRAM_BINS, DARKROOM_HISTOGRAM_MAX_SAMPLES, HistogramBin, HistogramChannel,
    HistogramData, HistogramError, HistogramSample,
};
pub use display_profile::{DisplayProfileBanner, GtkMonitorInventory};
pub use external_editor::{
    ArgumentRow, ArgumentRowError, CompletionAction, EXTERNAL_EDITOR_FOCUS_ORDER,
    ExecutableApproval, ExecutableIdentity, ExternalEditorAction, ExternalEditorController,
    ExternalEditorControllerError, ExternalEditorDraft, ExternalEditorJob, ExternalEditorPanel,
    ExternalEditorPreset, ExternalEditorServiceError, ExternalEditorServicePort,
    ExternalEditorViewModel, InterchangeMode, InvocationReview, JobId, JobStage, Launchability,
    MetadataPolicy, Placeholder, PresetId, PresetValidationError, QualificationOutcome,
    QualificationReceipt, QualificationState, SendToEditorRequest, TiffBitDepth, WaitMode,
};
pub use gtk_shell::{
    CollectionControlAction, CollectionControlState, CollectionControls, CollectionFilterState,
    CullingRestriction, DarkroomModuleAction, DarkroomModuleActionHandler,
    DarkroomModuleAvailability, DarkroomModuleError, DarkroomModuleGroup, DarkroomModulePreset,
    DarkroomModuleSide, DarkroomModuleStatus, DarkroomModuleViewModel, DarkroomModulesViewModel,
    DarkroomWorkspaceViewModel, DarktableTheme, ExportAction, ExportPanel, ExportSize,
    ExposurePanel, GtkShell, LighttableColorLabel, LighttableContentState,
    LighttableInteractionState, LighttableLayout, LighttableLayoutAction, LighttableLayoutControls,
    LighttablePanel, LighttablePhotoState, LighttableRating, LighttableSelectionAction,
    LighttableSort, LighttableToolbar, LighttableToolbarAction, LighttableToolbarState,
    LighttableZoom, ModuleControlKind, ModuleControlViewModel, ModulePanelViewModel,
    NavigationDirection, SelectionModifiers, ShellLayout, ShellRegion, ThemeRole, WorkspaceRole,
    apply_theme_role, darktable_theme_css, install_darktable_theme, reference_modules,
};
pub use import::{
    IMPORT_DIALOG_FOCUS_ORDER, IMPORT_DIALOG_WIDGET_IDS, IMPORT_SESSION_FOCUS_ORDER, ImportAction,
    ImportDialog, ImportItemOutcome, ImportPanelViewModel, ImportPlace, ImportRequest,
    ImportReviewRow, ImportRowState, ImportRowViewModel, ImportSessionAction,
    ImportSessionController, ImportSessionControllerError, ImportSessionEvent, ImportSessionPanel,
    ImportSessionServiceError, ImportSessionServicePort, ImportSessionState,
    ImportSessionViewModel, ImportSourceModel, ImportSourceRow, ImportSourceState,
    MAX_IMPORT_SOURCE_ROWS, RAW_EXTENSIONS, is_raw_path,
};
pub use input::GtkInputAdapter;
pub use input_mapping::{
    ActionContext, ActionDefinition, ActionId, Binding, BindingSource, Curve, DeviceDescriptor,
    DeviceKind, EditorMessage, EditorState, EditorStatus, InputMappingEditor, MappingConflict,
    MappingProfile, MappingSnapshot, ResetScope, SoftTakeover,
};
pub use library::{LibraryFailureKind, LibraryFailureProjection, LibraryState};
pub use mask_manager::{
    MASK_MANAGER_FOCUS_ORDER, MASK_MANAGER_MAX_FEATHER, MaskCombination, MaskConsumptionState,
    MaskGroupOption, MaskManagerAction, MaskManagerCapability, MaskManagerController,
    MaskManagerControllerError, MaskManagerPanel, MaskManagerServiceError, MaskManagerServicePort,
    MaskManagerSnapshot,
};
pub use multiscale_retouch::{
    MULTISCALE_RETOUCH_BANDS, MULTISCALE_RETOUCH_FOCUS_ORDER, MULTISCALE_RETOUCH_MAX_STRENGTH,
    MultiscaleBand, MultiscaleCapability, MultiscaleProgress, MultiscaleRetouchAction,
    MultiscaleRetouchController, MultiscaleRetouchControllerError, MultiscaleRetouchPanel,
    MultiscaleRetouchRequest, MultiscaleRetouchServiceError, MultiscaleRetouchServiceEvent,
    MultiscaleRetouchServicePort, MultiscaleRetouchSnapshot, MultiscaleRetouchStatus,
    MultiscaleSourceTarget,
};
pub use neural_restore::{PhotoSelection, PhotoSourceKind};
pub use presentation::{
    ControlId, ControlIdError, ControlValidationError, DARKROOM_LEFT_PANEL_FOCUS_ORDER,
    DARKROOM_LEFT_PANEL_ORDER, DarkroomControlError, DarkroomControlKind, DarkroomControlValue,
    DarkroomControlViewModel, DarkroomControlsStatus, DarkroomControlsViewModel,
    DarkroomEditCommand, DarkroomEditRouteError, DarkroomEditRouter, DarkroomEditTarget,
    DarkroomImageInformationViewModel, DarkroomPanelAction, DarkroomPanelActionHandler,
    DarkroomPanelError, DarkroomPanelId, DarkroomPanelProjection, DarkroomPanelRouter,
    DarkroomPanelState, DarkroomPanelTarget, DarkroomSnapshotEntry, DarkroomSnapshotsViewModel,
    PhotoCardViewModel, PhotoDetailViewModel, PhotoFactViewModel, PhotoWorkspaceViewModel,
    PhotoWorkspaceViewModelError, PresentationText, PresentationTextError, PreviewDimensions,
    PreviewDimensionsError, Rgba8PreviewMetadata, Rgba8PreviewMetadataError,
    SelectedPreviewFailure, SelectedPreviewState, SliderSpec, ThumbnailIndicators,
};
pub use raw_denoise::{
    RAW_DENOISE_FOCUS_ORDER, RAW_DENOISE_MAX_STRENGTH, RAW_DENOISE_TILES, RawDenoiseAction,
    RawDenoiseCalibrationStatus, RawDenoiseCancellationState, RawDenoiseController,
    RawDenoiseControllerError, RawDenoiseFailure, RawDenoiseJobKind, RawDenoiseJobRequest,
    RawDenoiseMemoryState, RawDenoiseModelOption, RawDenoiseOutputPolicy, RawDenoisePanel,
    RawDenoisePlan, RawDenoisePlanError, RawDenoisePlanPolicy, RawDenoiseProfileStatus,
    RawDenoiseProgress, RawDenoiseProviderState, RawDenoiseServiceError, RawDenoiseServiceEvent,
    RawDenoiseServicePort, RawDenoiseSnapshot, RawDenoiseSourceInfo, RawDenoiseSourceLayout,
    RawDenoiseStatus, RawDenoiseViewModel,
};
pub use rgb_denoise::{
    RGB_DENOISE_FOCUS_ORDER, RGB_DENOISE_MAX_DETAIL_STRENGTH, RGB_DENOISE_MAX_STRENGTH,
    RGB_DENOISE_SCALES, RGB_DENOISE_TILES, RgbDenoiseAction, RgbDenoiseCancellationState,
    RgbDenoiseController, RgbDenoiseControllerError, RgbDenoiseDetailPolicy, RgbDenoiseFailure,
    RgbDenoiseGamutPolicy, RgbDenoiseJobKind, RgbDenoiseJobRequest, RgbDenoiseMemoryState,
    RgbDenoiseModelOption, RgbDenoisePanel, RgbDenoisePlan, RgbDenoisePlanError,
    RgbDenoiseProfileOption, RgbDenoiseProfileState, RgbDenoiseProgress, RgbDenoiseProviderState,
    RgbDenoiseServiceError, RgbDenoiseServiceEvent, RgbDenoiseServicePort, RgbDenoiseShadowPolicy,
    RgbDenoiseSnapshot, RgbDenoiseStatus, RgbDenoiseViewModel,
};
pub use viewport_presentation::{
    DisplayPresentationController, DisplayPresentationFrame, DisplayPresentationPort,
    DisplayPresentationRequest, DisplayPresentationState, PresentationFailure,
    PresentationGeneration, PresentationMode, PresentationStatus, PresentationTicket,
    SdrFallbackReason, ViewportColorMode, ViewportComparison, ViewportGeneration,
};
