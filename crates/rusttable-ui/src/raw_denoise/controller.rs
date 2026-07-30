//! Generation-safe controller for linear RAW denoise intent and service results.

#![allow(clippy::missing_errors_doc)]

use super::model::{
    RAW_DENOISE_MAX_STRENGTH, RAW_DENOISE_TILES, RawDenoiseAction, RawDenoiseCancellationState,
    RawDenoiseFailure, RawDenoiseJobKind, RawDenoiseJobRequest, RawDenoiseMemoryState,
    RawDenoisePlan, RawDenoisePlanError, RawDenoiseProgress, RawDenoiseProviderState,
    RawDenoiseServiceError, RawDenoiseServiceEvent, RawDenoiseServicePort, RawDenoiseSnapshot,
    RawDenoiseStatus, RawDenoiseViewModel,
};
use crate::neural_restore::PhotoSelection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawDenoiseControllerError {
    Service(RawDenoiseServiceError),
    InvalidControl,
    NoPlan,
}

impl From<RawDenoiseServiceError> for RawDenoiseControllerError {
    fn from(value: RawDenoiseServiceError) -> Self {
        Self::Service(value)
    }
}

#[derive(Debug)]
pub struct RawDenoiseController<S> {
    service: S,
    state: RawDenoiseViewModel,
}

impl<S: RawDenoiseServicePort> RawDenoiseController<S> {
    #[must_use]
    pub fn new(service: S) -> Self {
        Self {
            service,
            state: RawDenoiseViewModel::default(),
        }
    }

    #[must_use]
    pub const fn state(&self) -> &RawDenoiseViewModel {
        &self.state
    }

    pub fn refresh(&mut self) -> Result<(), RawDenoiseControllerError> {
        let selection = self.state.snapshot.selection().clone();
        let snapshot = self
            .service
            .snapshot(&selection)
            .inspect_err(|error| self.fail_from_service(error))?;
        if self.apply_snapshot(snapshot) {
            self.invalidate_and_plan()?;
        }
        Ok(())
    }

    pub fn dispatch(&mut self, action: RawDenoiseAction) -> Result<(), RawDenoiseControllerError> {
        match action {
            RawDenoiseAction::Refresh => self.refresh(),
            RawDenoiseAction::SetSelection(selection) => self.set_selection(&selection),
            RawDenoiseAction::SelectModel(model) => {
                self.state.model = model;
                self.invalidate_and_plan()
            }
            RawDenoiseAction::SelectProvider(provider) => {
                self.state.provider = provider;
                self.invalidate_and_plan()
            }
            RawDenoiseAction::SetStrength(strength) => {
                if strength > RAW_DENOISE_MAX_STRENGTH {
                    return Err(RawDenoiseControllerError::InvalidControl);
                }
                self.state.strength = strength;
                self.invalidate_and_plan()
            }
            RawDenoiseAction::SetTile(tile) => {
                if !RAW_DENOISE_TILES.contains(&tile) {
                    return Err(RawDenoiseControllerError::InvalidControl);
                }
                self.state.tile_size = tile;
                self.invalidate_and_plan()
            }
            RawDenoiseAction::SetPlanPolicy(policy) => {
                self.state.plan_policy = policy;
                self.invalidate_and_plan()
            }
            RawDenoiseAction::SetOutputPolicy(policy) => {
                self.state.output_policy = policy;
                self.invalidate_and_plan()
            }
            RawDenoiseAction::Preview => self.start(RawDenoiseJobKind::Preview),
            RawDenoiseAction::Full => self.start(RawDenoiseJobKind::Full),
            RawDenoiseAction::Export => self.start(RawDenoiseJobKind::Export),
            RawDenoiseAction::Cancel => self.cancel(),
        }
    }

    pub fn apply_event(&mut self, event: RawDenoiseServiceEvent) {
        let (generation, job) = match &event {
            RawDenoiseServiceEvent::Progress {
                generation, job, ..
            }
            | RawDenoiseServiceEvent::PendingPublication {
                generation, job, ..
            }
            | RawDenoiseServiceEvent::Completed {
                generation, job, ..
            }
            | RawDenoiseServiceEvent::Imported {
                generation, job, ..
            }
            | RawDenoiseServiceEvent::Failed {
                generation, job, ..
            }
            | RawDenoiseServiceEvent::Cancelled { generation, job } => (*generation, *job),
        };
        let job_matches =
            self.state.active_job == Some(job) || self.state.cancellation_job == Some(job);
        if generation != self.state.generation() || !job_matches {
            return;
        }
        match event {
            RawDenoiseServiceEvent::Progress { progress, .. } => {
                self.state.progress = Some(progress);
                let kind = self.running_kind().unwrap_or(RawDenoiseJobKind::Preview);
                self.state.status = RawDenoiseStatus::Running { kind, progress };
            }
            RawDenoiseServiceEvent::PendingPublication { artifact, .. } => {
                let kind = self.running_kind().unwrap_or(RawDenoiseJobKind::Export);
                self.state.status = RawDenoiseStatus::PendingPublication { kind, artifact };
            }
            RawDenoiseServiceEvent::Completed { artifact, .. } => {
                let kind = self.running_kind().unwrap_or(RawDenoiseJobKind::Full);
                self.finish(kind);
                self.state.status = RawDenoiseStatus::Completed { kind, artifact };
            }
            RawDenoiseServiceEvent::Imported { artifact, .. } => {
                let kind = self.running_kind().unwrap_or(RawDenoiseJobKind::Export);
                self.finish(kind);
                self.state.status = RawDenoiseStatus::Imported { kind, artifact };
            }
            RawDenoiseServiceEvent::Failed { error, .. } => self.fail(error),
            RawDenoiseServiceEvent::Cancelled { .. } => {
                self.state.active_job = None;
                self.state.cancellation_job = None;
                self.state.cancellation_state = RawDenoiseCancellationState::Cancelled;
                self.state.failure = Some(RawDenoiseFailure::Cancelled);
                self.state.status = RawDenoiseStatus::Cancelled;
            }
        }
    }

    fn set_selection(
        &mut self,
        selection: &PhotoSelection,
    ) -> Result<(), RawDenoiseControllerError> {
        self.cancel_active_if_needed()?;
        let snapshot = self.service.snapshot(selection).inspect_err(|error| {
            self.state.snapshot = RawDenoiseSnapshot::unavailable(selection.clone());
            self.fail_from_service(error);
        })?;
        let snapshot = if snapshot.generation() <= self.state.snapshot_generation {
            snapshot.with_generation(self.state.snapshot_generation.saturating_add(1))
        } else {
            snapshot
        };
        self.apply_snapshot(snapshot);
        self.invalidate_and_plan()
    }

    fn apply_snapshot(&mut self, snapshot: RawDenoiseSnapshot) -> bool {
        if !self.state.set_snapshot(snapshot) {
            return false;
        }
        self.state.model = self
            .state
            .model
            .as_ref()
            .and_then(|model| {
                self.state
                    .snapshot
                    .qualified_models()
                    .any(|option| option.hash() == model)
                    .then(|| model.clone())
            })
            .or_else(|| {
                self.state
                    .snapshot
                    .qualified_models()
                    .next()
                    .map(|option| option.hash().clone())
            });
        if self
            .state
            .provider
            .is_none_or(|provider| !self.state.snapshot.providers().contains(&provider))
        {
            self.state.provider = self.state.snapshot.providers().first().copied();
        }
        self.state.provider_state = if self.state.snapshot.providers().is_empty() {
            RawDenoiseProviderState::Unavailable
        } else {
            self.state.provider.map_or(
                RawDenoiseProviderState::Available,
                RawDenoiseProviderState::Selected,
            )
        };
        true
    }

    fn invalidate_and_plan(&mut self) -> Result<(), RawDenoiseControllerError> {
        self.state.generation = self.state.generation.saturating_add(1);
        self.state.plan = None;
        self.state.progress = None;
        self.state.failure = None;
        self.state.completed = None;
        self.state.status = RawDenoiseStatus::Planning;
        self.cancel_active_if_needed()?;
        match self.build_plan() {
            Ok(plan) => {
                self.state.memory_state = RawDenoiseMemoryState::Estimated {
                    bytes: plan.memory_bytes(),
                };
                self.state.plan = Some(plan);
                self.state.status = RawDenoiseStatus::Ready;
            }
            Err(error) => {
                self.state.memory_state = match error {
                    RawDenoisePlanError::MemoryLimit { bytes, limit } => {
                        RawDenoiseMemoryState::Exceeded { bytes, limit }
                    }
                    _ => RawDenoiseMemoryState::Unknown,
                };
                let failure = failure_from_plan(error);
                self.state.failure = Some(failure.clone());
                self.state.status = RawDenoiseStatus::Failed(failure);
            }
        }
        Ok(())
    }

    fn build_plan(&self) -> Result<RawDenoisePlan, RawDenoisePlanError> {
        let model_hash = self
            .state
            .model
            .as_ref()
            .ok_or(RawDenoisePlanError::ModelUnavailable)?;
        let model = self
            .state
            .snapshot
            .models()
            .iter()
            .find(|model| model.hash() == model_hash)
            .ok_or(RawDenoisePlanError::ModelUnavailable)?;
        let provider = self
            .state
            .provider
            .ok_or(RawDenoisePlanError::ProviderUnavailable)?;
        RawDenoisePlan::build(
            self.state.generation,
            self.state.snapshot.source(),
            model,
            provider,
            self.state.strength,
            self.state.tile_size,
            self.state.plan_policy,
            self.state.output_policy,
        )
    }

    fn start(&mut self, kind: RawDenoiseJobKind) -> Result<(), RawDenoiseControllerError> {
        let plan = self
            .state
            .plan
            .clone()
            .ok_or(RawDenoiseControllerError::NoPlan)?;
        let request = RawDenoiseJobRequest::new(self.state.generation, kind, plan);
        let job = match kind {
            RawDenoiseJobKind::Preview => self.service.request_preview(&request),
            RawDenoiseJobKind::Full => self.service.request_full(&request),
            RawDenoiseJobKind::Export => self.service.request_export(&request),
        }
        .inspect_err(|error| self.fail_from_service(error))?;
        self.state.active_job = Some(job);
        self.state.cancellation_job = None;
        self.state.cancellation_state = RawDenoiseCancellationState::Idle;
        let progress = RawDenoiseProgress {
            completed: 0,
            total: 1,
        };
        self.state.progress = Some(progress);
        self.state.status = RawDenoiseStatus::Running { kind, progress };
        Ok(())
    }

    fn cancel(&mut self) -> Result<(), RawDenoiseControllerError> {
        self.cancel_active_if_needed()
    }

    fn cancel_active_if_needed(&mut self) -> Result<(), RawDenoiseControllerError> {
        let Some(job) = self.state.active_job else {
            return Ok(());
        };
        self.service
            .cancel(job)
            .inspect_err(|error| self.fail_from_service(error))?;
        self.state.cancellation_state = RawDenoiseCancellationState::Requested;
        self.state.status = RawDenoiseStatus::Cancelling;
        self.state.cancellation_job = Some(job);
        self.state.active_job = None;
        Ok(())
    }

    fn running_kind(&self) -> Option<RawDenoiseJobKind> {
        match self.state.status {
            RawDenoiseStatus::Running { kind, .. }
            | RawDenoiseStatus::PendingPublication { kind, .. } => Some(kind),
            _ => None,
        }
    }

    fn finish(&mut self, kind: RawDenoiseJobKind) {
        self.state.active_job = None;
        self.state.cancellation_job = None;
        self.state.completed = Some(kind);
        self.state.cancellation_state = RawDenoiseCancellationState::Idle;
    }

    fn fail_from_service(&mut self, error: &RawDenoiseServiceError) {
        let failure = match error {
            RawDenoiseServiceError::BackendUnavailable => RawDenoiseFailure::BackendUnavailable,
            RawDenoiseServiceError::UnsupportedLayout => {
                RawDenoiseFailure::UnsupportedLayout(self.state.snapshot.source().layout())
            }
            RawDenoiseServiceError::MissingCalibration => RawDenoiseFailure::MissingCalibration,
            RawDenoiseServiceError::MissingProfile => RawDenoiseFailure::MissingProfile,
            RawDenoiseServiceError::ModelUnavailable => RawDenoiseFailure::ModelUnavailable,
            RawDenoiseServiceError::ProviderUnavailable => RawDenoiseFailure::ProviderUnavailable,
            RawDenoiseServiceError::MemoryBudgetExceeded { bytes, limit } => {
                RawDenoiseFailure::MemoryBudgetExceeded {
                    bytes: *bytes,
                    limit: *limit,
                }
            }
            RawDenoiseServiceError::Cancelled => RawDenoiseFailure::Cancelled,
            RawDenoiseServiceError::Failed(message) => RawDenoiseFailure::Failed(message.clone()),
        };
        self.fail(failure);
    }

    fn fail(&mut self, failure: RawDenoiseFailure) {
        self.state.active_job = None;
        self.state.cancellation_job = None;
        self.state.failure = Some(failure.clone());
        self.state.status = RawDenoiseStatus::Failed(failure);
    }
}

fn failure_from_plan(error: RawDenoisePlanError) -> RawDenoiseFailure {
    match error {
        RawDenoisePlanError::UnsupportedLayout(layout) => {
            RawDenoiseFailure::UnsupportedLayout(layout)
        }
        RawDenoisePlanError::MissingCalibration => RawDenoiseFailure::MissingCalibration,
        RawDenoisePlanError::MissingProfile => RawDenoiseFailure::MissingProfile,
        RawDenoisePlanError::ModelUnavailable => RawDenoiseFailure::ModelUnavailable,
        RawDenoisePlanError::ProviderUnavailable => RawDenoiseFailure::ProviderUnavailable,
        RawDenoisePlanError::MemoryLimit { bytes, limit } => {
            RawDenoiseFailure::MemoryBudgetExceeded { bytes, limit }
        }
        other => RawDenoiseFailure::Failed(format!("cannot build plan: {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::{
        RawDenoiseCalibrationStatus, RawDenoiseModelOption, RawDenoiseOutputPolicy,
        RawDenoisePlanPolicy, RawDenoiseProfileStatus, RawDenoiseSourceInfo,
        RawDenoiseSourceLayout,
    };
    use super::*;
    use crate::ai_models::AiProvider;
    use crate::ai_models::ModelHash;
    use crate::neural_restore::PhotoSourceKind;
    use rusttable_core::PhotoId;

    #[derive(Default)]
    struct Service {
        snapshot: Option<RawDenoiseSnapshot>,
        next_job: u64,
        cancelled: Vec<u64>,
    }

    impl RawDenoiseServicePort for Service {
        fn snapshot(
            &mut self,
            _: &PhotoSelection,
        ) -> Result<RawDenoiseSnapshot, RawDenoiseServiceError> {
            self.snapshot
                .take()
                .ok_or(RawDenoiseServiceError::BackendUnavailable)
        }
        fn request_preview(
            &mut self,
            _: &RawDenoiseJobRequest,
        ) -> Result<u64, RawDenoiseServiceError> {
            self.next_job += 1;
            Ok(self.next_job)
        }
        fn request_full(
            &mut self,
            _: &RawDenoiseJobRequest,
        ) -> Result<u64, RawDenoiseServiceError> {
            self.next_job += 1;
            Ok(self.next_job)
        }
        fn request_export(
            &mut self,
            _: &RawDenoiseJobRequest,
        ) -> Result<u64, RawDenoiseServiceError> {
            self.next_job += 1;
            Ok(self.next_job)
        }
        fn cancel(&mut self, job: u64) -> Result<(), RawDenoiseServiceError> {
            self.cancelled.push(job);
            Ok(())
        }
    }

    fn hash() -> ModelHash {
        ModelHash::new("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            .expect("hash")
    }

    fn selection() -> PhotoSelection {
        PhotoSelection::single(
            PhotoId::new(7).expect("photo"),
            PhotoSourceKind::XTransRaw,
            true,
            3,
        )
    }

    fn snapshot(generation: u64) -> RawDenoiseSnapshot {
        RawDenoiseSnapshot::available(
            selection(),
            RawDenoiseSourceInfo::available(
                "source",
                "edit",
                "dng",
                (1024, 768),
                RawDenoiseSourceLayout::XTrans,
                RawDenoiseCalibrationStatus::Present,
                RawDenoiseProfileStatus::LinearRec2020,
            ),
            vec![RawDenoiseModelOption::new(
                hash(),
                "RawLinearDenoise",
                true,
                vec![
                    RawDenoiseSourceLayout::XTrans,
                    RawDenoiseSourceLayout::AlreadyLinear,
                ],
                256,
                vec![AiProvider::Cpu],
            )],
            vec![AiProvider::Cpu],
        )
        .with_generation(generation)
    }

    #[test]
    fn unavailable_backend_is_explicit_and_does_not_create_a_plan() {
        let state = RawDenoiseViewModel::unavailable();
        assert!(state.plan().is_none());
        assert!(
            state
                .failure()
                .expect("failure")
                .message()
                .contains("no inference")
        );
    }

    #[test]
    fn plan_requires_layout_calibration_and_linear_profile() {
        let mut source = RawDenoiseSourceInfo::available(
            "s",
            "e",
            "o",
            (10, 10),
            RawDenoiseSourceLayout::Unsupported,
            RawDenoiseCalibrationStatus::Present,
            RawDenoiseProfileStatus::LinearRec2020,
        );
        let model = RawDenoiseModelOption::new(
            hash(),
            "model",
            true,
            vec![RawDenoiseSourceLayout::XTrans],
            256,
            vec![AiProvider::Cpu],
        );
        assert_eq!(
            RawDenoisePlan::build(
                1,
                &source,
                &model,
                AiProvider::Cpu,
                50,
                256,
                RawDenoisePlanPolicy::MinimalRaw,
                RawDenoiseOutputPolicy::PreviewBuffer
            ),
            Err(RawDenoisePlanError::UnsupportedLayout(
                RawDenoiseSourceLayout::Unsupported
            ))
        );
        source = RawDenoiseSourceInfo::available(
            "s",
            "e",
            "o",
            (10, 10),
            RawDenoiseSourceLayout::XTrans,
            RawDenoiseCalibrationStatus::Missing,
            RawDenoiseProfileStatus::LinearRec2020,
        );
        assert_eq!(
            RawDenoisePlan::build(
                1,
                &source,
                &model,
                AiProvider::Cpu,
                50,
                256,
                RawDenoisePlanPolicy::MinimalRaw,
                RawDenoiseOutputPolicy::PreviewBuffer
            ),
            Err(RawDenoisePlanError::MissingCalibration)
        );
    }

    #[test]
    fn lifecycle_preserves_pending_publication_import_and_cancellation() {
        let service = Service {
            snapshot: Some(snapshot(1)),
            ..Service::default()
        };
        let mut controller = RawDenoiseController::new(service);
        controller.refresh().expect("refresh");
        controller
            .dispatch(RawDenoiseAction::Export)
            .expect("export");
        let generation = controller.state().generation();
        controller.apply_event(RawDenoiseServiceEvent::Progress {
            generation,
            job: 1,
            progress: RawDenoiseProgress {
                completed: 2,
                total: 4,
            },
        });
        assert!(matches!(
            controller.state().status(),
            RawDenoiseStatus::Running { .. }
        ));
        controller.apply_event(RawDenoiseServiceEvent::PendingPublication {
            generation,
            job: 1,
            artifact: "staged.dng".to_owned(),
        });
        assert!(matches!(
            controller.state().status(),
            RawDenoiseStatus::PendingPublication { .. }
        ));
        controller.apply_event(RawDenoiseServiceEvent::Imported {
            generation,
            job: 1,
            artifact: "imported.dng".to_owned(),
        });
        assert!(matches!(
            controller.state().status(),
            RawDenoiseStatus::Imported { .. }
        ));
        controller.dispatch(RawDenoiseAction::Full).expect("full");
        let generation = controller.state().generation();
        controller
            .dispatch(RawDenoiseAction::Cancel)
            .expect("cancel");
        controller.apply_event(RawDenoiseServiceEvent::Cancelled { generation, job: 2 });
        assert_eq!(
            controller.state().cancellation_state,
            RawDenoiseCancellationState::Cancelled
        );
    }

    #[test]
    fn stale_event_cannot_replace_new_plan() {
        let service = Service {
            snapshot: Some(snapshot(1)),
            ..Service::default()
        };
        let mut controller = RawDenoiseController::new(service);
        controller.refresh().expect("refresh");
        controller
            .dispatch(RawDenoiseAction::Preview)
            .expect("preview");
        let old_generation = controller.state().generation();
        controller
            .dispatch(RawDenoiseAction::SetTile(512))
            .expect("new plan");
        controller.apply_event(RawDenoiseServiceEvent::Completed {
            generation: old_generation,
            job: 1,
            artifact: None,
        });
        assert!(!matches!(
            controller.state().status(),
            RawDenoiseStatus::Completed { .. }
        ));
    }

    #[test]
    fn memory_budget_is_a_blocking_plan_state() {
        let source = RawDenoiseSourceInfo::available(
            "source",
            "edit",
            "dng",
            (40_000, 40_000),
            RawDenoiseSourceLayout::XTrans,
            RawDenoiseCalibrationStatus::Present,
            RawDenoiseProfileStatus::LinearRec2020,
        );
        let model = RawDenoiseModelOption::new(
            hash(),
            "RawLinearDenoise",
            true,
            vec![RawDenoiseSourceLayout::XTrans],
            256,
            vec![AiProvider::Cpu],
        );
        assert!(matches!(
            RawDenoisePlan::build(
                1,
                &source,
                &model,
                AiProvider::Cpu,
                50,
                256,
                RawDenoisePlanPolicy::MinimalRaw,
                RawDenoiseOutputPolicy::PreviewBuffer,
            ),
            Err(RawDenoisePlanError::MemoryLimit { .. })
        ));
    }
}
