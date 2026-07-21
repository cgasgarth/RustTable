//! Explicit unavailable adapters until the #478/#498/#499/#501/#502 services land.
//!
//! These adapters are intentionally boring: they make the GTK surfaces usable and truthful
//! during the backend transition without claiming model installation or inference succeeded.
//!
//! Integration seam for #778: once #478 lands, the application composition root should inject
//! its typed registry implementation here. The UI must not acquire package storage, runtime
//! handles, native provider diagnostics, or model persistence of its own.

use std::path::PathBuf;

use rusttable_ui::{
    AiBatchPreflight, AiBatchRecipe, AiBatchReview, AiBatchSelection, AiBatchServiceError,
    AiBatchServicePort,
};
use rusttable_ui::{
    AiModelsServiceError, AiModelsServicePort, AiModelsSnapshot, AiProvider, AiProviderPolicy,
    AiTask, InstallSummary, ModelHash, NeuralRestorePreviewPort, NeuralRestoreSnapshot,
    PhotoSelection, PreviewRequest, PreviewServiceError, QualificationJob,
};

#[derive(Debug, Default)]
pub(crate) struct UnavailableAiBatchService;

impl AiBatchServicePort for UnavailableAiBatchService {
    fn review(
        &mut self,
        _: &[AiBatchSelection],
        _: &AiBatchRecipe,
    ) -> Result<AiBatchReview, AiBatchServiceError> {
        Err(AiBatchServiceError::Unavailable)
    }
    fn preflight(&mut self, _: &AiBatchReview) -> Result<AiBatchPreflight, AiBatchServiceError> {
        Err(AiBatchServiceError::Unavailable)
    }
    fn enqueue(
        &mut self,
        _: &AiBatchReview,
        _: &AiBatchPreflight,
    ) -> Result<u64, AiBatchServiceError> {
        Err(AiBatchServiceError::Unavailable)
    }
    fn pause(&mut self, _: u64) -> Result<(), AiBatchServiceError> {
        Err(AiBatchServiceError::Unavailable)
    }
    fn resume(&mut self, _: u64) -> Result<(), AiBatchServiceError> {
        Err(AiBatchServiceError::Unavailable)
    }
    fn cancel(&mut self, _: u64) -> Result<(), AiBatchServiceError> {
        Err(AiBatchServiceError::Unavailable)
    }
    fn retry_failed(&mut self, _: u64) -> Result<(), AiBatchServiceError> {
        Err(AiBatchServiceError::Unavailable)
    }
    fn reconcile(&mut self, _: u64) -> Result<(), AiBatchServiceError> {
        Err(AiBatchServiceError::Unavailable)
    }
    fn remove_history(&mut self, _: u64) -> Result<(), AiBatchServiceError> {
        Err(AiBatchServiceError::Unavailable)
    }
}

#[derive(Debug, Default)]
pub(crate) struct UnavailableAiModelsService;

impl AiModelsServicePort for UnavailableAiModelsService {
    fn snapshot(&mut self) -> Result<AiModelsSnapshot, AiModelsServiceError> {
        Err(AiModelsServiceError::Unavailable)
    }
    fn stage_local_package(
        &mut self,
        _source: PathBuf,
    ) -> Result<InstallSummary, AiModelsServiceError> {
        Err(AiModelsServiceError::Unavailable)
    }
    fn install_staged(&mut self) -> Result<AiModelsSnapshot, AiModelsServiceError> {
        Err(AiModelsServiceError::Unavailable)
    }
    fn set_provider_policy(
        &mut self,
        _policy: AiProviderPolicy,
    ) -> Result<AiModelsSnapshot, AiModelsServiceError> {
        Err(AiModelsServiceError::Unavailable)
    }
    fn set_task_default(
        &mut self,
        _task: AiTask,
        _model: Option<ModelHash>,
    ) -> Result<AiModelsSnapshot, AiModelsServiceError> {
        Err(AiModelsServiceError::Unavailable)
    }
    fn set_enabled(
        &mut self,
        _model: &ModelHash,
        _enabled: bool,
    ) -> Result<AiModelsSnapshot, AiModelsServiceError> {
        Err(AiModelsServiceError::Unavailable)
    }
    fn start_qualification(
        &mut self,
        _model: &ModelHash,
        _provider: AiProvider,
    ) -> Result<QualificationJob, AiModelsServiceError> {
        Err(AiModelsServiceError::Unavailable)
    }
    fn cancel_qualification(&mut self, _job: u64) -> Result<(), AiModelsServiceError> {
        Err(AiModelsServiceError::Unavailable)
    }
    fn remove(&mut self, _model: &ModelHash) -> Result<AiModelsSnapshot, AiModelsServiceError> {
        Err(AiModelsServiceError::Unavailable)
    }
}

#[derive(Debug, Default)]
pub(crate) struct UnavailableNeuralRestoreService;

impl NeuralRestorePreviewPort for UnavailableNeuralRestoreService {
    fn snapshot(
        &mut self,
        selection: &PhotoSelection,
    ) -> Result<NeuralRestoreSnapshot, PreviewServiceError> {
        Ok(NeuralRestoreSnapshot::unavailable(selection.clone()))
    }
    fn request_preview(&mut self, _request: &PreviewRequest) -> Result<u64, PreviewServiceError> {
        Err(PreviewServiceError::Unavailable)
    }
    fn cancel_preview(&mut self, _job: u64) -> Result<(), PreviewServiceError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusttable_ui::AiModelsFailure;

    #[test]
    fn unavailable_model_adapter_is_explicit_and_privacy_safe() {
        let mut service = UnavailableAiModelsService;
        let path = PathBuf::from("/private/photos/secret/model.rtmodel");
        assert_eq!(
            service.stage_local_package(path),
            Err(AiModelsServiceError::Unavailable)
        );
        assert_eq!(service.snapshot(), Err(AiModelsServiceError::Unavailable));
        let message = AiModelsFailure::ServiceUnavailable.message();
        assert_eq!(
            message,
            "AI model service is unavailable; no package operation was performed."
        );
        assert!(!message.contains("secret"));
    }
}
