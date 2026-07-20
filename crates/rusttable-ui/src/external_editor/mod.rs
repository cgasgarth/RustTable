//! GTK4 external-editor workflow, split into a service-safe model/controller and view.

mod controller;
mod model;
mod view;

pub use controller::{ExternalEditorController, ExternalEditorControllerError};
pub use model::{
    ArgumentRow, ArgumentRowError, CompletionAction, ExecutableApproval, ExecutableIdentity,
    ExternalEditorAction, ExternalEditorDraft, ExternalEditorJob, ExternalEditorPreset,
    ExternalEditorServiceError, ExternalEditorServicePort, ExternalEditorViewModel,
    InterchangeMode, InvocationReview, JobId, JobStage, Launchability, MetadataPolicy, Placeholder,
    PresetId, PresetValidationError, QualificationOutcome, QualificationReceipt,
    QualificationState, SendToEditorRequest, TiffBitDepth, WaitMode,
};
pub use view::{EXTERNAL_EDITOR_FOCUS_ORDER, ExternalEditorPanel};
