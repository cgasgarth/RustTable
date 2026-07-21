//! Generation-safe controller for multiscale-retouch intent and job events.

#![allow(clippy::missing_errors_doc)]

use super::model::{
    MULTISCALE_RETOUCH_BANDS, MULTISCALE_RETOUCH_MAX_STRENGTH, MultiscaleBand,
    MultiscaleRetouchAction, MultiscaleRetouchRequest, MultiscaleRetouchServiceError,
    MultiscaleRetouchServiceEvent, MultiscaleRetouchServicePort, MultiscaleRetouchSnapshot,
    MultiscaleRetouchStatus,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MultiscaleRetouchControllerError {
    Service(MultiscaleRetouchServiceError),
    InvalidControl,
    NoActiveJob,
    StaleGeneration { expected: u64, actual: u64 },
}

impl std::fmt::Display for MultiscaleRetouchControllerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Service(error) => error.fmt(formatter),
            Self::InvalidControl => formatter.write_str("multiscale-retouch control is invalid"),
            Self::NoActiveJob => formatter.write_str("no multiscale-retouch job is active"),
            Self::StaleGeneration { expected, actual } => write!(
                formatter,
                "stale multiscale-retouch generation: expected {expected}, got {actual}"
            ),
        }
    }
}

impl std::error::Error for MultiscaleRetouchControllerError {}

impl From<MultiscaleRetouchServiceError> for MultiscaleRetouchControllerError {
    fn from(error: MultiscaleRetouchServiceError) -> Self {
        Self::Service(error)
    }
}

#[derive(Debug)]
pub struct MultiscaleRetouchController<S> {
    service: S,
    state: MultiscaleRetouchSnapshot,
    active_job: Option<u64>,
}

impl<S: MultiscaleRetouchServicePort> MultiscaleRetouchController<S> {
    #[must_use]
    pub fn new(service: S) -> Self {
        Self {
            service,
            state: MultiscaleRetouchSnapshot::unavailable(
                0,
                "multiscale-retouch service is unavailable",
            ),
            active_job: None,
        }
    }

    #[must_use]
    pub const fn state(&self) -> &MultiscaleRetouchSnapshot {
        &self.state
    }

    pub fn refresh(&mut self, generation: u64) -> Result<(), MultiscaleRetouchControllerError> {
        let snapshot = self.service.snapshot(generation)?;
        self.install(snapshot)
    }

    pub fn dispatch(
        &mut self,
        action: MultiscaleRetouchAction,
    ) -> Result<(), MultiscaleRetouchControllerError> {
        match action {
            MultiscaleRetouchAction::SetBand(band) => {
                if matches!(band, MultiscaleBand::Band(value) if !MULTISCALE_RETOUCH_BANDS.contains(&value))
                {
                    return Err(MultiscaleRetouchControllerError::InvalidControl);
                }
                self.update_state(MultiscaleRetouchAction::SetBand(band))
            }
            MultiscaleRetouchAction::SetSource(source) => {
                self.update_state(MultiscaleRetouchAction::SetSource(source))
            }
            MultiscaleRetouchAction::SetTarget(target) => {
                self.update_state(MultiscaleRetouchAction::SetTarget(target))
            }
            MultiscaleRetouchAction::SetStrength(strength) => {
                if strength > MULTISCALE_RETOUCH_MAX_STRENGTH {
                    return Err(MultiscaleRetouchControllerError::InvalidControl);
                }
                self.update_state(MultiscaleRetouchAction::SetStrength(strength))
            }
            MultiscaleRetouchAction::Preview => {
                if !self.state.capability().is_available() {
                    return Err(MultiscaleRetouchControllerError::Service(
                        MultiscaleRetouchServiceError::BackendUnavailable,
                    ));
                }
                let request = MultiscaleRetouchRequest::new(
                    self.state.band(),
                    self.state.source(),
                    self.state.target(),
                    self.state.strength(),
                );
                let job = self.service.start(self.state.generation(), &request)?;
                self.active_job = Some(job);
                self.state = snapshot_running(&self.state, job);
                Ok(())
            }
            MultiscaleRetouchAction::Cancel => {
                let Some(job) = self.active_job else {
                    return Err(MultiscaleRetouchControllerError::NoActiveJob);
                };
                self.service.cancel(self.state.generation(), job)?;
                self.state = snapshot_cancelling(&self.state, job);
                Ok(())
            }
            MultiscaleRetouchAction::Refresh => self.refresh(self.state.generation()),
        }
    }

    pub fn apply_event(&mut self, event: MultiscaleRetouchServiceEvent) {
        let (generation, job) = match &event {
            MultiscaleRetouchServiceEvent::Progress {
                generation, job, ..
            }
            | MultiscaleRetouchServiceEvent::Completed { generation, job }
            | MultiscaleRetouchServiceEvent::Cancelled { generation, job }
            | MultiscaleRetouchServiceEvent::Failed {
                generation, job, ..
            } => (*generation, *job),
        };
        if generation != self.state.generation() || self.active_job != Some(job) {
            return;
        }
        match event {
            MultiscaleRetouchServiceEvent::Progress { progress, .. } => {
                self.state = snapshot_with_progress(&self.state, progress);
            }
            MultiscaleRetouchServiceEvent::Completed { .. } => {
                self.active_job = None;
                self.state = snapshot_completed(&self.state, job);
            }
            MultiscaleRetouchServiceEvent::Cancelled { .. } => {
                self.active_job = None;
                self.state = snapshot_cancelled(&self.state, job);
            }
            MultiscaleRetouchServiceEvent::Failed { message, .. } => {
                self.active_job = None;
                self.state = snapshot_failed(&self.state, message);
            }
        }
    }

    fn install(
        &mut self,
        snapshot: MultiscaleRetouchSnapshot,
    ) -> Result<(), MultiscaleRetouchControllerError> {
        if snapshot.generation() < self.state.generation() {
            return Err(MultiscaleRetouchControllerError::StaleGeneration {
                expected: self.state.generation(),
                actual: snapshot.generation(),
            });
        }
        self.active_job = None;
        self.state = snapshot;
        Ok(())
    }

    fn update_state(
        &mut self,
        action: MultiscaleRetouchAction,
    ) -> Result<(), MultiscaleRetouchControllerError> {
        let snapshot = self.service.update(self.state.generation(), &action)?;
        self.install(snapshot)
    }
}

fn snapshot_running(state: &MultiscaleRetouchSnapshot, job: u64) -> MultiscaleRetouchSnapshot {
    let mut next = state.clone();
    next.status = MultiscaleRetouchStatus::Running { job };
    next
}

fn snapshot_cancelling(state: &MultiscaleRetouchSnapshot, job: u64) -> MultiscaleRetouchSnapshot {
    let mut next = state.clone();
    next.status = MultiscaleRetouchStatus::Cancelling { job };
    next
}

fn snapshot_with_progress(
    state: &MultiscaleRetouchSnapshot,
    progress: super::model::MultiscaleProgress,
) -> MultiscaleRetouchSnapshot {
    let mut next = state.clone();
    next.progress = Some(progress);
    next
}

fn snapshot_completed(state: &MultiscaleRetouchSnapshot, job: u64) -> MultiscaleRetouchSnapshot {
    let mut next = state.clone();
    next.status = MultiscaleRetouchStatus::Completed { job };
    next
}

fn snapshot_cancelled(state: &MultiscaleRetouchSnapshot, job: u64) -> MultiscaleRetouchSnapshot {
    let mut next = state.clone();
    next.status = MultiscaleRetouchStatus::Cancelled { job };
    next
}

fn snapshot_failed(
    state: &MultiscaleRetouchSnapshot,
    message: String,
) -> MultiscaleRetouchSnapshot {
    let mut next = state.clone();
    next.status = MultiscaleRetouchStatus::Failed { message };
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multiscale_retouch::{
        MultiscaleProgress, MultiscaleRetouchServicePort, MultiscaleSourceTarget,
    };

    #[derive(Debug, Default)]
    struct FakeService;

    impl MultiscaleRetouchServicePort for FakeService {
        fn snapshot(
            &mut self,
            generation: u64,
        ) -> Result<MultiscaleRetouchSnapshot, MultiscaleRetouchServiceError> {
            Ok(MultiscaleRetouchSnapshot::available(generation))
        }

        fn update(
            &mut self,
            generation: u64,
            _: &MultiscaleRetouchAction,
        ) -> Result<MultiscaleRetouchSnapshot, MultiscaleRetouchServiceError> {
            Ok(MultiscaleRetouchSnapshot::available(generation))
        }

        fn start(
            &mut self,
            _: u64,
            _: &MultiscaleRetouchRequest,
        ) -> Result<u64, MultiscaleRetouchServiceError> {
            Ok(9)
        }

        fn cancel(&mut self, _: u64, _: u64) -> Result<(), MultiscaleRetouchServiceError> {
            Ok(())
        }
    }

    #[test]
    fn stale_job_events_cannot_update_a_newer_generation() {
        let mut controller = MultiscaleRetouchController::new(FakeService);
        controller.refresh(3).expect("available snapshot");
        controller
            .dispatch(MultiscaleRetouchAction::SetSource(
                MultiscaleSourceTarget::Target,
            ))
            .expect("typed source update");
        controller
            .dispatch(MultiscaleRetouchAction::Preview)
            .expect("job request");
        controller.apply_event(MultiscaleRetouchServiceEvent::Progress {
            generation: 3,
            job: 9,
            progress: MultiscaleProgress::new(1, 2).expect("bounded progress"),
        });
        assert_eq!(
            controller.state().progress().expect("progress").completed(),
            1
        );
        controller.apply_event(MultiscaleRetouchServiceEvent::Progress {
            generation: 2,
            job: 9,
            progress: MultiscaleProgress::new(2, 2).expect("bounded progress"),
        });
        assert_eq!(
            controller
                .state()
                .progress()
                .expect("current progress")
                .completed(),
            1
        );
    }
}
