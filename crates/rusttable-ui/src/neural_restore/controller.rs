//! Generation-safe preview controller with a deterministic debounce boundary.

#![allow(clippy::missing_errors_doc)]

use std::time::Duration;

use super::model::{
    NeuralRestoreAction, NeuralRestorePreviewPort, NeuralRestoreViewModel, PhotoSelection,
    PhotoSourceKind, PreviewArtifact, PreviewCache, PreviewCacheKey, PreviewEligibility,
    PreviewFailure, PreviewRequest, PreviewServiceError, PreviewStage, PreviewStatus, Roi,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NeuralRestoreControllerError {
    Service(PreviewServiceError),
    NoSelection,
    NoModel,
    NoProvider,
    NoPendingJob,
}
impl From<PreviewServiceError> for NeuralRestoreControllerError {
    fn from(value: PreviewServiceError) -> Self {
        Self::Service(value)
    }
}

#[derive(Debug)]
pub struct NeuralRestoreController<S> {
    service: S,
    state: NeuralRestoreViewModel,
    cache: PreviewCache,
    pending_job: Option<u64>,
    debounce_remaining: Option<Duration>,
    pending_request: Option<PreviewRequest>,
}

impl<S: NeuralRestorePreviewPort> NeuralRestoreController<S> {
    #[must_use]
    pub fn new(service: S) -> Self {
        Self {
            service,
            state: NeuralRestoreViewModel::default(),
            cache: PreviewCache::new(8),
            pending_job: None,
            debounce_remaining: None,
            pending_request: None,
        }
    }
    #[must_use]
    pub const fn state(&self) -> &NeuralRestoreViewModel {
        &self.state
    }

    pub fn set_selection(
        &mut self,
        selection: &PhotoSelection,
    ) -> Result<(), NeuralRestoreControllerError> {
        self.state.invalidate();
        self.cancel_active_job()?;
        self.state.snapshot = self.service.snapshot(selection).map_err(|error| {
            self.state.status = PreviewStatus::Ineligible(match error {
                PreviewServiceError::Ineligible(reason) => reason,
                _ => PreviewEligibility::ServiceUnavailable,
            });
            error
        })?;
        self.state.announcement = self.state.snapshot.announcement().to_owned();
        self.state.status =
            PreviewStatus::Ineligible(self.state.snapshot.eligibility(self.state.task));
        Ok(())
    }

    pub fn dispatch(
        &mut self,
        action: NeuralRestoreAction,
    ) -> Result<(), NeuralRestoreControllerError> {
        match action {
            NeuralRestoreAction::SetSelection(selection) => self.set_selection(&selection),
            NeuralRestoreAction::SelectTask(task) => {
                self.state.task = task;
                self.schedule();
                Ok(())
            }
            NeuralRestoreAction::SelectModel(model) => {
                self.state.model = model;
                self.schedule();
                Ok(())
            }
            NeuralRestoreAction::SelectProvider(provider) => {
                self.state.provider = Some(provider);
                self.schedule();
                Ok(())
            }
            NeuralRestoreAction::SetRawStrength(value) => {
                self.state.settings.set_raw_strength(value);
                self.schedule();
                Ok(())
            }
            NeuralRestoreAction::SetRgbStrength(value) => {
                self.state.settings.set_rgb_strength(value);
                self.schedule();
                Ok(())
            }
            NeuralRestoreAction::SetWideGamut(value) => {
                self.state.settings.set_preserve_wide_gamut(value);
                self.schedule();
                Ok(())
            }
            NeuralRestoreAction::SetScale(value) => {
                self.state.settings.set_scale(value);
                self.schedule();
                Ok(())
            }
            NeuralRestoreAction::SetComparison(mode) => {
                self.state.comparison = mode;
                Ok(())
            }
            NeuralRestoreAction::AdjustSplit(delta) => {
                self.state.viewport.adjust_split(f32::from(delta) / 100.0);
                Ok(())
            }
            NeuralRestoreAction::Cancel => self.cancel_active_job(),
        }
    }

    pub fn advance_debounce(
        &mut self,
        elapsed: Duration,
    ) -> Result<bool, NeuralRestoreControllerError> {
        let Some(remaining) = self.debounce_remaining else {
            return Ok(false);
        };
        if elapsed < remaining {
            self.debounce_remaining = remaining.checked_sub(elapsed);
            return Ok(false);
        }
        self.debounce_remaining = None;
        let Some(request) = self.pending_request.take() else {
            return Ok(false);
        };
        if let Some(artifact) = self.cache.get(request.key()) {
            self.state.artifact = Some(artifact);
            self.state.status = PreviewStatus::CacheHit;
            self.state.cache_size = self.cache.len();
            return Ok(true);
        }
        let job = self
            .service
            .request_preview(&request)
            .inspect_err(|error| {
                self.state.status = PreviewStatus::Failed(PreviewFailure::from(error.clone()));
            })?;
        self.pending_job = Some(job);
        self.state.status = PreviewStatus::Running(PreviewStage::Inference);
        Ok(true)
    }

    pub fn apply_result(
        &mut self,
        generation: u64,
        key: &PreviewCacheKey,
        result: Result<PreviewArtifact, PreviewFailure>,
    ) {
        if generation != self.state.generation {
            self.state.status = PreviewStatus::Failed(PreviewFailure::StaleGeneration);
            return;
        }
        match result {
            Ok(artifact) => {
                self.cache.insert(key.clone(), artifact.clone());
                self.state.artifact = Some(artifact);
                self.state.status = PreviewStatus::Ready;
                self.state.cache_size = self.cache.len();
                "Preview ready; source and catalog are unchanged."
                    .clone_into(&mut self.state.announcement);
            }
            Err(error) => self.state.status = PreviewStatus::Failed(error),
        }
    }

    fn schedule(&mut self) {
        self.state.invalidate();
        let eligibility = self.state.snapshot.eligibility(self.state.task);
        if !matches!(eligibility, PreviewEligibility::Eligible) {
            self.state.status = PreviewStatus::Ineligible(eligibility);
            self.pending_request = None;
            self.debounce_remaining = None;
            return;
        }
        let (Some(selection), Some(model), Some(provider)) = (
            self.state.snapshot.selection().photo(),
            self.state.model.clone(),
            self.state.provider,
        ) else {
            self.state.status = PreviewStatus::Ineligible(if self.state.model.is_none() {
                PreviewEligibility::MissingModel
            } else {
                PreviewEligibility::ProviderUnavailable
            });
            return;
        };
        let selection = PhotoSelection::single(
            selection,
            self.state
                .snapshot
                .selection()
                .source_kind()
                .unwrap_or(PhotoSourceKind::Raster),
            true,
            self.state.snapshot.selection().revision(),
        );
        let roi = Roi {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            scale: self.state.settings.scale(),
        };
        let Some(key) = PreviewCacheKey::new(
            &selection,
            self.state.task,
            model.clone(),
            provider,
            self.state.settings,
            roi,
            "viewport",
            "neural-restore-v1",
        ) else {
            self.state.status = PreviewStatus::Ineligible(PreviewEligibility::NoSelection);
            return;
        };
        self.pending_request = Some(PreviewRequest {
            generation: self.state.generation,
            key,
            task: self.state.task,
            model,
            provider,
            settings: self.state.settings,
            roi,
        });
        self.debounce_remaining = Some(Duration::from_millis(200));
    }

    fn cancel_active_job(&mut self) -> Result<(), NeuralRestoreControllerError> {
        if let Some(job) = self.pending_job.take() {
            self.service.cancel_preview(job)?;
            self.state.status = PreviewStatus::Failed(PreviewFailure::Cancelled);
        }
        Ok(())
    }
}

impl From<PreviewServiceError> for PreviewFailure {
    fn from(value: PreviewServiceError) -> Self {
        match value {
            PreviewServiceError::Unavailable => Self::ServiceUnavailable,
            PreviewServiceError::Ineligible(reason) => Self::Ineligible(reason),
            PreviewServiceError::Cancelled => Self::Cancelled,
            PreviewServiceError::Failed => Self::Inference,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NeuralRestoreSnapshot;
    use rusttable_core::PhotoId;

    #[derive(Default)]
    struct Service {
        jobs: u64,
        cancelled: u64,
    }
    impl NeuralRestorePreviewPort for Service {
        fn snapshot(
            &mut self,
            selection: &PhotoSelection,
        ) -> Result<NeuralRestoreSnapshot, PreviewServiceError> {
            Ok(NeuralRestoreSnapshot::unavailable(selection.clone()))
        }
        fn request_preview(&mut self, _: &PreviewRequest) -> Result<u64, PreviewServiceError> {
            self.jobs += 1;
            Ok(self.jobs)
        }
        fn cancel_preview(&mut self, _: u64) -> Result<(), PreviewServiceError> {
            self.cancelled += 1;
            Ok(())
        }
    }
    #[test]
    fn unavailable_service_never_dispatches_inference() {
        let mut controller = NeuralRestoreController::new(Service::default());
        controller
            .set_selection(&PhotoSelection::single(
                PhotoId::new(1).expect("photo"),
                PhotoSourceKind::Raster,
                true,
                1,
            ))
            .expect("snapshot");
        controller
            .dispatch(NeuralRestoreAction::SetRgbStrength(70))
            .expect("state");
        assert_eq!(
            *controller.state().status(),
            PreviewStatus::Ineligible(PreviewEligibility::ServiceUnavailable)
        );
        assert!(
            !controller
                .advance_debounce(Duration::from_millis(200))
                .expect("tick")
        );
    }
}
