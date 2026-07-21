//! Generation-safe controller for RGB denoise intent and service results.

#![allow(clippy::missing_errors_doc)]

use super::model::{
    RGB_DENOISE_MAX_DETAIL_STRENGTH, RGB_DENOISE_SCALES, RGB_DENOISE_TILES, RgbDenoiseAction,
    RgbDenoiseCancellationState, RgbDenoiseFailure, RgbDenoiseJobKind, RgbDenoiseJobRequest,
    RgbDenoiseMemoryState, RgbDenoisePlan, RgbDenoisePlanError, RgbDenoiseProfileState,
    RgbDenoiseProviderState, RgbDenoiseServiceError, RgbDenoiseServiceEvent, RgbDenoiseServicePort,
    RgbDenoiseSnapshot, RgbDenoiseStatus, RgbDenoiseViewModel,
};
use crate::neural_restore::PhotoSelection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RgbDenoiseControllerError {
    Service(RgbDenoiseServiceError),
    InvalidControl,
    NoQualifiedModel,
    NoProvider,
    NoProfile,
    NoPlan,
    NoActiveJob,
}

impl From<RgbDenoiseServiceError> for RgbDenoiseControllerError {
    fn from(value: RgbDenoiseServiceError) -> Self {
        Self::Service(value)
    }
}

#[derive(Debug)]
pub struct RgbDenoiseController<S> {
    service: S,
    state: RgbDenoiseViewModel,
}

impl<S: RgbDenoiseServicePort> RgbDenoiseController<S> {
    #[must_use]
    pub fn new(service: S) -> Self {
        Self {
            service,
            state: RgbDenoiseViewModel::default(),
        }
    }

    #[must_use]
    pub const fn state(&self) -> &RgbDenoiseViewModel {
        &self.state
    }

    pub fn refresh(&mut self) -> Result<(), RgbDenoiseControllerError> {
        let selection = self.state.snapshot().selection().clone();
        let snapshot = self
            .service
            .snapshot(&selection)
            .inspect_err(|error| self.fail_from_service(error))?;
        if self.apply_snapshot(snapshot) {
            self.invalidate_and_plan()?;
        }
        Ok(())
    }

    pub fn dispatch(&mut self, action: RgbDenoiseAction) -> Result<(), RgbDenoiseControllerError> {
        match action {
            RgbDenoiseAction::Refresh => self.refresh(),
            RgbDenoiseAction::SetSelection(selection) => self.set_selection(&selection),
            RgbDenoiseAction::SelectModel(model) => {
                self.state.model = model;
                self.invalidate_and_plan()
            }
            RgbDenoiseAction::SelectProvider(provider) => {
                self.state.provider = provider;
                self.invalidate_and_plan()
            }
            RgbDenoiseAction::SelectWorkingProfile(profile) => {
                self.state.working_profile = profile;
                self.invalidate_and_plan()
            }
            RgbDenoiseAction::SelectModelProfile(profile) => {
                self.state.model_profile = profile;
                self.invalidate_and_plan()
            }
            RgbDenoiseAction::SetScale(scale) => {
                if !RGB_DENOISE_SCALES.contains(&scale) {
                    return Err(RgbDenoiseControllerError::InvalidControl);
                }
                self.state.scale = scale;
                self.invalidate_and_plan()
            }
            RgbDenoiseAction::SetTile(tile) => {
                if !RGB_DENOISE_TILES.contains(&tile) {
                    return Err(RgbDenoiseControllerError::InvalidControl);
                }
                self.state.tile_size = tile;
                self.invalidate_and_plan()
            }
            RgbDenoiseAction::SetStrength(strength) => {
                if strength > super::model::RGB_DENOISE_MAX_STRENGTH {
                    return Err(RgbDenoiseControllerError::InvalidControl);
                }
                self.state.strength = strength;
                self.invalidate_and_plan()
            }
            RgbDenoiseAction::SetGamut(gamut) => {
                self.state.gamut = gamut;
                self.invalidate_and_plan()
            }
            RgbDenoiseAction::SetShadows(shadows) => {
                self.state.shadows = shadows;
                self.invalidate_and_plan()
            }
            RgbDenoiseAction::SetDetail(detail) => {
                self.state.detail = detail;
                self.invalidate_and_plan()
            }
            RgbDenoiseAction::SetDetailStrength(strength) => {
                if strength > RGB_DENOISE_MAX_DETAIL_STRENGTH {
                    return Err(RgbDenoiseControllerError::InvalidControl);
                }
                self.state.detail_strength = strength;
                self.invalidate_and_plan()
            }
            RgbDenoiseAction::Preview => self.start(RgbDenoiseJobKind::Preview),
            RgbDenoiseAction::Full => self.start(RgbDenoiseJobKind::Full),
            RgbDenoiseAction::Export => self.start(RgbDenoiseJobKind::Export),
            RgbDenoiseAction::Cancel => self.cancel(),
        }
    }

    pub fn apply_event(&mut self, event: RgbDenoiseServiceEvent) {
        let (generation, job) = match &event {
            RgbDenoiseServiceEvent::Progress {
                generation, job, ..
            }
            | RgbDenoiseServiceEvent::Completed {
                generation, job, ..
            }
            | RgbDenoiseServiceEvent::Failed {
                generation, job, ..
            }
            | RgbDenoiseServiceEvent::Cancelled { generation, job } => (*generation, *job),
        };
        if generation != self.state.generation() || self.state.active_job() != Some(job) {
            return;
        }
        match event {
            RgbDenoiseServiceEvent::Progress { progress, .. } => {
                self.state.progress = Some(progress);
                let kind = match self.state.status {
                    RgbDenoiseStatus::Running { kind, .. } => kind,
                    _ => RgbDenoiseJobKind::Preview,
                };
                self.state.status = RgbDenoiseStatus::Running { kind, progress };
            }
            RgbDenoiseServiceEvent::Completed { artifact, .. } => {
                let kind = match self.state.status {
                    RgbDenoiseStatus::Running { kind, .. } => kind,
                    _ => RgbDenoiseJobKind::Preview,
                };
                self.state.active_job = None;
                self.state.completed = Some(kind);
                self.state.cancellation_state = RgbDenoiseCancellationState::Idle;
                self.state.status = RgbDenoiseStatus::Completed { kind, artifact };
            }
            RgbDenoiseServiceEvent::Failed { error, .. } => self.fail(error),
            RgbDenoiseServiceEvent::Cancelled { .. } => {
                self.state.active_job = None;
                self.state.cancellation_state = RgbDenoiseCancellationState::Cancelled;
                self.state.failure = Some(RgbDenoiseFailure::Cancelled);
                self.state.status = RgbDenoiseStatus::Cancelled;
            }
        }
    }

    fn set_selection(
        &mut self,
        selection: &PhotoSelection,
    ) -> Result<(), RgbDenoiseControllerError> {
        self.cancel_active_if_needed()?;
        let snapshot = self.service.snapshot(selection).inspect_err(|error| {
            self.state.snapshot = RgbDenoiseSnapshot::unavailable(selection.clone());
            self.fail_from_service(error);
        })?;
        self.apply_snapshot(snapshot);
        self.invalidate_and_plan()
    }

    fn apply_snapshot(&mut self, snapshot: RgbDenoiseSnapshot) -> bool {
        if !self.state.set_snapshot(snapshot) {
            return false;
        }
        let default_model = self
            .state
            .snapshot()
            .qualified_models()
            .next()
            .map(|option| option.hash().clone());
        let model_is_missing = self.state.model.as_ref().is_none_or(|model| {
            !self
                .state
                .snapshot()
                .qualified_models()
                .any(|option| option.hash() == model)
        });
        if model_is_missing {
            self.state.model = default_model;
        }
        if self
            .state
            .provider
            .is_none_or(|provider| !self.state.snapshot().providers().contains(&provider))
        {
            self.state.provider = self.state.snapshot().providers().first().copied();
        }
        if self.state.working_profile.is_none() {
            self.state.working_profile = self
                .state
                .snapshot()
                .working_profiles()
                .first()
                .map(|profile| profile.id().to_owned());
        }
        if self.state.model_profile.is_none() {
            self.state.model_profile = self
                .state
                .snapshot()
                .model_profiles()
                .first()
                .map(|profile| profile.id().to_owned());
        }
        self.state.provider_state = if self.state.snapshot().providers().is_empty() {
            RgbDenoiseProviderState::Unavailable
        } else {
            self.state.provider.map_or(
                RgbDenoiseProviderState::Available,
                RgbDenoiseProviderState::Selected,
            )
        };
        self.state.working_profile_state = if self.state.snapshot().working_profiles().is_empty() {
            RgbDenoiseProfileState::Unavailable
        } else if self.state.working_profile.is_some() {
            RgbDenoiseProfileState::Selected
        } else {
            RgbDenoiseProfileState::Available
        };
        self.state.model_profile_state = if self.state.snapshot().model_profiles().is_empty() {
            RgbDenoiseProfileState::Unavailable
        } else if self.state.model_profile.is_some() {
            RgbDenoiseProfileState::Selected
        } else {
            RgbDenoiseProfileState::Available
        };
        true
    }

    fn invalidate_and_plan(&mut self) -> Result<(), RgbDenoiseControllerError> {
        self.state.generation = self.state.generation.saturating_add(1);
        self.state.plan = None;
        self.state.progress = None;
        self.state.failure = None;
        self.state.completed = None;
        self.state.status = RgbDenoiseStatus::Planning;
        self.cancel_active_if_needed()?;
        match self.build_plan() {
            Ok(plan) => {
                self.state.memory_state = RgbDenoiseMemoryState::Estimated {
                    bytes: plan.memory_bytes(),
                };
                self.state.plan = Some(plan);
                self.state.status = RgbDenoiseStatus::Ready;
                Ok(())
            }
            Err(error) => {
                self.state.memory_state = match error {
                    RgbDenoisePlanError::MemoryLimit { bytes, limit } => {
                        RgbDenoiseMemoryState::Exceeded { bytes, limit }
                    }
                    _ => RgbDenoiseMemoryState::Unknown,
                };
                self.state.failure = Some(match error {
                    RgbDenoisePlanError::MemoryLimit { bytes, limit } => {
                        RgbDenoiseFailure::MemoryBudgetExceeded { bytes, limit }
                    }
                    RgbDenoisePlanError::MissingProfile => RgbDenoiseFailure::ProfileUnavailable,
                    _ => RgbDenoiseFailure::Failed(format!("cannot build plan: {error:?}")),
                });
                self.state.status = RgbDenoiseStatus::Failed(
                    self.state
                        .failure
                        .clone()
                        .expect("failure was just assigned"),
                );
                Ok(())
            }
        }
    }

    fn build_plan(&self) -> Result<RgbDenoisePlan, RgbDenoisePlanError> {
        let model_hash = self
            .state
            .model
            .clone()
            .ok_or(RgbDenoisePlanError::MissingProfile)?;
        let model = self
            .state
            .snapshot()
            .models()
            .iter()
            .find(|model| model.hash() == &model_hash)
            .ok_or(RgbDenoisePlanError::MissingProfile)?;
        if !model.qualified() {
            return Err(RgbDenoisePlanError::MissingProfile);
        }
        let dimensions = self
            .state
            .snapshot()
            .dimensions()
            .ok_or(RgbDenoisePlanError::EmptyImage)?;
        RgbDenoisePlan::build(
            self.state.generation,
            model_hash,
            self.state.working_profile.clone().unwrap_or_default(),
            self.state.model_profile.clone().unwrap_or_default(),
            self.state
                .provider
                .ok_or(RgbDenoisePlanError::MissingProfile)?,
            self.state.scale,
            self.state.tile_size,
            self.state.strength,
            self.state.gamut,
            self.state.shadows,
            self.state.detail,
            self.state.detail_strength,
            dimensions,
        )
    }

    fn start(&mut self, kind: RgbDenoiseJobKind) -> Result<(), RgbDenoiseControllerError> {
        let plan = self.state.plan.clone().ok_or_else(|| {
            self.state.failure = Some(RgbDenoiseFailure::ProviderUnavailable);
            RgbDenoiseControllerError::NoPlan
        })?;
        let request = RgbDenoiseJobRequest::new(self.state.generation, kind, plan);
        let job = match kind {
            RgbDenoiseJobKind::Preview => self.service.request_preview(&request),
            RgbDenoiseJobKind::Full => self.service.request_full(&request),
            RgbDenoiseJobKind::Export => self.service.request_export(&request),
        }
        .inspect_err(|error| self.fail_from_service(error))?;
        self.state.active_job = Some(job);
        self.state.cancellation_state = RgbDenoiseCancellationState::Idle;
        self.state.progress = Some(super::model::RgbDenoiseProgress {
            completed: 0,
            total: 1,
        });
        self.state.status = RgbDenoiseStatus::Running {
            kind,
            progress: self.state.progress.expect("progress was just assigned"),
        };
        Ok(())
    }

    fn cancel(&mut self) -> Result<(), RgbDenoiseControllerError> {
        self.cancel_active_if_needed()
    }

    fn cancel_active_if_needed(&mut self) -> Result<(), RgbDenoiseControllerError> {
        let Some(job) = self.state.active_job else {
            return Ok(());
        };
        self.service
            .cancel(job)
            .inspect_err(|error| self.fail_from_service(error))?;
        self.state.cancellation_state = RgbDenoiseCancellationState::Requested;
        self.state.status = RgbDenoiseStatus::Cancelling;
        self.state.active_job = None;
        Ok(())
    }

    fn fail_from_service(&mut self, error: &RgbDenoiseServiceError) {
        let failure = match error {
            RgbDenoiseServiceError::Unavailable => RgbDenoiseFailure::ServiceUnavailable,
            RgbDenoiseServiceError::ProviderUnavailable => RgbDenoiseFailure::ProviderUnavailable,
            RgbDenoiseServiceError::ProfileUnavailable => RgbDenoiseFailure::ProfileUnavailable,
            RgbDenoiseServiceError::MemoryBudgetExceeded { bytes, limit } => {
                RgbDenoiseFailure::MemoryBudgetExceeded {
                    bytes: *bytes,
                    limit: *limit,
                }
            }
            RgbDenoiseServiceError::Failed(message) => RgbDenoiseFailure::Failed(message.clone()),
        };
        self.fail(failure);
    }

    fn fail(&mut self, failure: RgbDenoiseFailure) {
        self.state.active_job = None;
        self.state.failure = Some(failure.clone());
        self.state.status = RgbDenoiseStatus::Failed(failure);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_models::{AiProvider, ModelHash};
    use crate::neural_restore::PhotoSourceKind;
    use rusttable_core::PhotoId;

    #[derive(Default)]
    struct Service {
        snapshots: Vec<RgbDenoiseSnapshot>,
        jobs: u64,
        requests: Vec<RgbDenoiseJobKind>,
        cancellations: usize,
    }

    impl RgbDenoiseServicePort for Service {
        fn snapshot(
            &mut self,
            _: &PhotoSelection,
        ) -> Result<RgbDenoiseSnapshot, RgbDenoiseServiceError> {
            self.snapshots
                .pop()
                .ok_or(RgbDenoiseServiceError::Unavailable)
        }
        fn request_preview(
            &mut self,
            _: &RgbDenoiseJobRequest,
        ) -> Result<u64, RgbDenoiseServiceError> {
            self.jobs += 1;
            self.requests.push(RgbDenoiseJobKind::Preview);
            Ok(self.jobs)
        }
        fn request_full(
            &mut self,
            _: &RgbDenoiseJobRequest,
        ) -> Result<u64, RgbDenoiseServiceError> {
            self.jobs += 1;
            self.requests.push(RgbDenoiseJobKind::Full);
            Ok(self.jobs)
        }
        fn request_export(
            &mut self,
            _: &RgbDenoiseJobRequest,
        ) -> Result<u64, RgbDenoiseServiceError> {
            self.jobs += 1;
            self.requests.push(RgbDenoiseJobKind::Export);
            Ok(self.jobs)
        }
        fn cancel(&mut self, _: u64) -> Result<(), RgbDenoiseServiceError> {
            self.cancellations += 1;
            Ok(())
        }
    }

    fn hash() -> ModelHash {
        ModelHash::new("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            .expect("fixture model hash")
    }

    fn snapshot(generation: u64) -> RgbDenoiseSnapshot {
        let selection = PhotoSelection::single(
            PhotoId::new(7).expect("photo"),
            PhotoSourceKind::Raster,
            true,
            3,
        );
        RgbDenoiseSnapshot::available(
            selection,
            (1024, 768),
            vec![super::super::model::RgbDenoiseModelOption::new(
                hash(),
                "RGB model",
                true,
                1,
                256,
                vec![AiProvider::Cpu],
                true,
            )],
            vec![AiProvider::Cpu],
            vec![super::super::model::RgbDenoiseProfileOption::new(
                "working", "Working",
            )],
            vec![super::super::model::RgbDenoiseProfileOption::new(
                "model", "Model",
            )],
        )
        .with_generation(generation)
    }

    #[test]
    fn unavailable_service_never_dispatches_inference() {
        let mut controller = RgbDenoiseController::new(Service::default());
        assert_eq!(
            controller.dispatch(RgbDenoiseAction::Preview),
            Err(RgbDenoiseControllerError::NoPlan)
        );
        assert!(matches!(
            controller.state().status(),
            RgbDenoiseStatus::Failed(_)
        ));
    }

    #[test]
    fn all_three_jobs_cross_one_typed_service_boundary_and_cancel() {
        let service = Service {
            snapshots: vec![snapshot(1)],
            ..Service::default()
        };
        let mut controller = RgbDenoiseController::new(service);
        controller.refresh().expect("snapshot");
        controller
            .dispatch(RgbDenoiseAction::Preview)
            .expect("preview");
        let generation = controller.state().generation();
        controller.apply_event(RgbDenoiseServiceEvent::Progress {
            generation,
            job: 1,
            progress: super::super::model::RgbDenoiseProgress {
                completed: 1,
                total: 4,
            },
        });
        assert!(matches!(
            controller.state().status(),
            RgbDenoiseStatus::Running { .. }
        ));
        controller
            .dispatch(RgbDenoiseAction::Cancel)
            .expect("cancel");
        controller.dispatch(RgbDenoiseAction::Full).expect("full");
        controller.apply_event(RgbDenoiseServiceEvent::Completed {
            generation,
            job: 2,
            artifact: None,
        });
        controller
            .dispatch(RgbDenoiseAction::Export)
            .expect("export");
        assert_eq!(
            controller.state().completed(),
            Some(RgbDenoiseJobKind::Full)
        );
    }

    #[test]
    fn stale_completion_cannot_replace_new_generation() {
        let service = Service {
            snapshots: vec![snapshot(1)],
            ..Service::default()
        };
        let mut controller = RgbDenoiseController::new(service);
        controller.refresh().expect("snapshot");
        controller
            .dispatch(RgbDenoiseAction::Preview)
            .expect("preview");
        let old_generation = controller.state().generation();
        controller
            .dispatch(RgbDenoiseAction::SetTile(512))
            .expect("new plan");
        controller.apply_event(RgbDenoiseServiceEvent::Completed {
            generation: old_generation,
            job: 1,
            artifact: None,
        });
        assert!(!matches!(
            controller.state().status(),
            RgbDenoiseStatus::Completed { .. }
        ));
    }
}
