//! Explicit unavailable adapters until the #478/#498/#499/#501/#502 services land.
//!
//! These adapters are intentionally boring: they make the GTK surfaces usable and truthful
//! during the backend transition without claiming model installation or inference succeeded.

use std::path::PathBuf;

use rusttable_ui::{
    AiModelsServiceError, AiModelsServicePort, AiModelsSnapshot, AiProvider, AiProviderPolicy,
    AiTask, InstallSummary, ModelHash, NeuralRestorePreviewPort, NeuralRestoreSnapshot,
    PhotoSelection, PreviewRequest, PreviewServiceError, QualificationJob,
};

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
