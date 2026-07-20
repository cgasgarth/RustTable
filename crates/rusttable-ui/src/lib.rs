#![forbid(unsafe_code)]
#![doc = "GTK4 UI components and presentation models for `RustTable`."]

pub mod ai_batch;
pub mod ai_models;
pub mod camera;
pub mod collection;
pub mod display_profile;
pub mod external_editor;
pub mod gtk_shell;
pub mod import;
pub mod input;
pub mod input_mapping;
pub mod library;
pub mod neural_restore;
pub mod presentation;
pub mod viewport_presentation;

pub use ai_batch::{
    AI_BATCH_FOCUS_ORDER, AiBatchAction, AiBatchCollision, AiBatchController,
    AiBatchControllerError, AiBatchEligibility, AiBatchEnqueuePolicy, AiBatchItem, AiBatchPanel,
    AiBatchPreflight, AiBatchRecipe, AiBatchReview, AiBatchSelection, AiBatchServiceError,
    AiBatchServicePort, AiBatchStage, AiBatchState, AiBatchTask,
};
pub use ai_models::{
    AI_MODELS_FOCUS_ORDER, AiModelsAction, AiModelsController, AiModelsControllerError,
    AiModelsFailure, AiModelsPanel, AiModelsServiceError, AiModelsServicePort, AiModelsSnapshot,
    AiModelsViewModel, AiProvider, AiProviderPolicy, AiTask, InstallSummary, InstalledModel,
    ModelHash, ModelServiceState, ProviderCapability, QualificationJob,
};
pub use camera::{
    CAMERA_FOCUS_ORDER, CameraAction, CameraController, CameraControllerError, CameraPanel,
    CameraViewModel,
};
pub use collection::{CollectionItem, CollectionProperty, CollectionRule};
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
    DarktableTheme, ExportAction, ExportPanel, ExportSize, ExposurePanel, GtkShell,
    LighttableColorLabel, LighttablePhotoState, LighttableRating, LighttableSort,
    LighttableToolbar, LighttableToolbarAction, LighttableToolbarState, ShellLayout, ShellRegion,
    ThemeRole, WorkspaceRole, apply_theme_role, darktable_theme_css, install_darktable_theme,
};
pub use import::{
    IMPORT_SESSION_FOCUS_ORDER, ImportAction, ImportItemOutcome, ImportPanelViewModel,
    ImportReviewRow, ImportRowState, ImportRowViewModel, ImportSessionAction,
    ImportSessionController, ImportSessionControllerError, ImportSessionEvent, ImportSessionPanel,
    ImportSessionServiceError, ImportSessionServicePort, ImportSessionState,
    ImportSessionViewModel,
};
pub use input::GtkInputAdapter;
pub use input_mapping::{
    ActionContext, ActionDefinition, ActionId, Binding, BindingSource, Curve, DeviceDescriptor,
    DeviceKind, EditorMessage, EditorState, EditorStatus, InputMappingEditor, MappingConflict,
    MappingProfile, MappingSnapshot, ResetScope, SoftTakeover,
};
pub use library::{LibraryFailureKind, LibraryFailureProjection, LibraryState};
pub use neural_restore::{
    ComparisonMode, NEURAL_RESTORE_FOCUS_ORDER, NeuralRestoreAction, NeuralRestoreController,
    NeuralRestoreControllerError, NeuralRestorePanel, NeuralRestorePreviewPort,
    NeuralRestoreSnapshot, NeuralRestoreViewModel, PhotoSelection, PhotoSourceKind,
    PreviewArtifact, PreviewCache, PreviewCacheKey, PreviewEligibility, PreviewFailure,
    PreviewFrame, PreviewFrameError, PreviewRequest, PreviewServiceError, PreviewStage,
    PreviewStatus, RestoreSettings, RestoreTask, Roi, ViewportState,
};
pub use presentation::{
    PhotoCardViewModel, PhotoDetailViewModel, PhotoFactViewModel, PhotoWorkspaceViewModel,
    PhotoWorkspaceViewModelError, PresentationText, PresentationTextError, PreviewDimensions,
    PreviewDimensionsError, Rgba8PreviewMetadata, Rgba8PreviewMetadataError,
    SelectedPreviewFailure, SelectedPreviewState, ThumbnailIndicators,
};
pub use viewport_presentation::{
    DisplayPresentationController, DisplayPresentationFrame, DisplayPresentationPort,
    DisplayPresentationRequest, DisplayPresentationState, PresentationFailure,
    PresentationGeneration, PresentationMode, PresentationStatus, PresentationTicket,
    SdrFallbackReason,
};
