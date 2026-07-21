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
    AiTask, InstallSummary, ModelHash, PhotoSelection, QualificationJob, RgbDenoiseJobRequest,
    RgbDenoiseServiceError, RgbDenoiseServicePort, RgbDenoiseSnapshot,
};
use rusttable_ui::{
    RawDenoiseJobRequest, RawDenoiseServiceError, RawDenoiseServicePort, RawDenoiseSnapshot,
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

/// The composition root keeps this seam explicit until the render snapshot and
/// qualified #478 provider executor are injected. It never claims inference ran.
#[derive(Debug, Default)]
pub(crate) struct UnavailableRgbDenoiseService;

impl RgbDenoiseServicePort for UnavailableRgbDenoiseService {
    fn snapshot(
        &mut self,
        selection: &PhotoSelection,
    ) -> Result<RgbDenoiseSnapshot, RgbDenoiseServiceError> {
        Ok(RgbDenoiseSnapshot::unavailable(selection.clone()))
    }
    fn request_preview(
        &mut self,
        _request: &RgbDenoiseJobRequest,
    ) -> Result<u64, RgbDenoiseServiceError> {
        Err(RgbDenoiseServiceError::Unavailable)
    }
    fn request_full(
        &mut self,
        _request: &RgbDenoiseJobRequest,
    ) -> Result<u64, RgbDenoiseServiceError> {
        Err(RgbDenoiseServiceError::Unavailable)
    }
    fn request_export(
        &mut self,
        _request: &RgbDenoiseJobRequest,
    ) -> Result<u64, RgbDenoiseServiceError> {
        Err(RgbDenoiseServiceError::Unavailable)
    }
    fn cancel(&mut self, _job: u64) -> Result<(), RgbDenoiseServiceError> {
        Err(RgbDenoiseServiceError::Unavailable)
    }
}

/// Typed RAW boundary for the current composition root.
///
/// The registry knows about `RawLinearDenoise`, but the decoder/calibration/profile
/// snapshot, qualified provider executor, DNG publisher, and catalog importer are not
/// wired into the app yet. Keeping this adapter explicit prevents GTK from pretending
/// those capabilities exist or from launching a substitute implementation.
#[derive(Debug, Default)]
pub(crate) struct UnavailableRawDenoiseService;

impl RawDenoiseServicePort for UnavailableRawDenoiseService {
    fn snapshot(
        &mut self,
        selection: &rusttable_ui::PhotoSelection,
    ) -> Result<RawDenoiseSnapshot, RawDenoiseServiceError> {
        Ok(RawDenoiseSnapshot::unavailable(selection.clone()))
    }

    fn request_preview(
        &mut self,
        _request: &RawDenoiseJobRequest,
    ) -> Result<u64, RawDenoiseServiceError> {
        Err(RawDenoiseServiceError::BackendUnavailable)
    }

    fn request_full(
        &mut self,
        _request: &RawDenoiseJobRequest,
    ) -> Result<u64, RawDenoiseServiceError> {
        Err(RawDenoiseServiceError::BackendUnavailable)
    }

    fn request_export(
        &mut self,
        _request: &RawDenoiseJobRequest,
    ) -> Result<u64, RawDenoiseServiceError> {
        Err(RawDenoiseServiceError::BackendUnavailable)
    }

    fn cancel(&mut self, _job: u64) -> Result<(), RawDenoiseServiceError> {
        Err(RawDenoiseServiceError::BackendUnavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusttable_ui::{
        AiModelsFailure, AiProvider, ModelHash, PhotoSelection, RawDenoiseCalibrationStatus,
        RawDenoiseJobKind, RawDenoiseModelOption, RawDenoiseOutputPolicy, RawDenoisePlan,
        RawDenoisePlanPolicy, RawDenoiseProfileStatus, RawDenoiseSourceInfo,
        RawDenoiseSourceLayout,
    };

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

    #[test]
    fn rgb_denoise_adapter_does_not_fake_a_plan_or_job() {
        let mut service = UnavailableRgbDenoiseService;
        let selection = PhotoSelection::none();
        assert_eq!(
            service
                .snapshot(&selection)
                .expect("truthful snapshot")
                .models(),
            &[]
        );
    }

    #[test]
    fn raw_denoise_adapter_exposes_missing_backend_without_side_effects() {
        let mut service = UnavailableRawDenoiseService;
        let snapshot = service
            .snapshot(&PhotoSelection::none())
            .expect("truthful snapshot");
        assert!(snapshot.models().is_empty());
        assert_eq!(
            service.request_export(&RawDenoiseJobRequest::new(
                0,
                RawDenoiseJobKind::Export,
                // This request is never dispatched by the unavailable adapter; the plan is
                // only constructed here to exercise the typed request boundary.
                RawDenoisePlan::build(
                    1,
                    &RawDenoiseSourceInfo::available(
                        "source",
                        "edit",
                        "dng",
                        (1, 1),
                        RawDenoiseSourceLayout::XTrans,
                        RawDenoiseCalibrationStatus::Present,
                        RawDenoiseProfileStatus::LinearRec2020,
                    ),
                    &RawDenoiseModelOption::new(
                        ModelHash::new(
                            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                        )
                        .expect("hash"),
                        "model",
                        true,
                        vec![RawDenoiseSourceLayout::XTrans],
                        256,
                        vec![AiProvider::Cpu],
                    ),
                    AiProvider::Cpu,
                    50,
                    256,
                    RawDenoisePlanPolicy::MinimalRaw,
                    RawDenoiseOutputPolicy::PublishAndImport,
                )
                .expect("typed plan"),
            )),
            Err(RawDenoiseServiceError::BackendUnavailable)
        );
    }
}
