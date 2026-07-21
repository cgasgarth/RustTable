//! Generation-safe controller for mask-manager intent and service results.

#![allow(clippy::missing_errors_doc)]

use super::model::{
    MASK_MANAGER_MAX_FEATHER, MaskManagerAction, MaskManagerServiceError, MaskManagerServicePort,
    MaskManagerSnapshot,
};

#[derive(Debug, Clone, PartialEq)]
pub enum MaskManagerControllerError {
    Service(MaskManagerServiceError),
    InvalidControl,
    StaleGeneration { expected: u64, actual: u64 },
}

impl std::fmt::Display for MaskManagerControllerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Service(error) => error.fmt(formatter),
            Self::InvalidControl => formatter.write_str("mask-manager control is invalid"),
            Self::StaleGeneration { expected, actual } => {
                write!(
                    formatter,
                    "stale mask-manager generation: expected {expected}, got {actual}"
                )
            }
        }
    }
}

impl std::error::Error for MaskManagerControllerError {}

impl From<MaskManagerServiceError> for MaskManagerControllerError {
    fn from(error: MaskManagerServiceError) -> Self {
        Self::Service(error)
    }
}

#[derive(Debug)]
pub struct MaskManagerController<S> {
    service: S,
    state: MaskManagerSnapshot,
}

impl<S: MaskManagerServicePort> MaskManagerController<S> {
    #[must_use]
    pub fn new(service: S) -> Self {
        Self {
            service,
            state: MaskManagerSnapshot::unavailable(0, "mask-manager service is unavailable"),
        }
    }

    #[must_use]
    pub const fn state(&self) -> &MaskManagerSnapshot {
        &self.state
    }

    pub fn refresh(&mut self, generation: u64) -> Result<(), MaskManagerControllerError> {
        let snapshot = self.service.snapshot(generation)?;
        self.install(snapshot)
    }

    pub fn dispatch(
        &mut self,
        action: &MaskManagerAction,
    ) -> Result<(), MaskManagerControllerError> {
        if matches!(action, MaskManagerAction::SetFeather(value) if !value.is_finite()
            || !(0.0..=MASK_MANAGER_MAX_FEATHER).contains(value))
            || matches!(action, MaskManagerAction::SetOpacity(value) if !value.is_finite()
                || !(0.0..=1.0).contains(value))
        {
            return Err(MaskManagerControllerError::InvalidControl);
        }
        let generation = self.state.generation();
        let snapshot = self.service.dispatch(generation, action)?;
        self.install(snapshot)
    }

    fn install(&mut self, snapshot: MaskManagerSnapshot) -> Result<(), MaskManagerControllerError> {
        if snapshot.generation() < self.state.generation() {
            return Err(MaskManagerControllerError::StaleGeneration {
                expected: self.state.generation(),
                actual: snapshot.generation(),
            });
        }
        self.state = snapshot;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mask_manager::{MaskGroupOption, MaskManagerCapability};

    #[derive(Debug)]
    struct FakeService;

    impl MaskManagerServicePort for FakeService {
        fn snapshot(
            &mut self,
            generation: u64,
        ) -> Result<MaskManagerSnapshot, MaskManagerServiceError> {
            Ok(MaskManagerSnapshot::available(
                generation,
                vec![MaskGroupOption::new("subject", "Subject")],
                Some("subject".to_owned()),
            ))
        }

        fn dispatch(
            &mut self,
            generation: u64,
            _: &MaskManagerAction,
        ) -> Result<MaskManagerSnapshot, MaskManagerServiceError> {
            Ok(MaskManagerSnapshot::unavailable(generation, "test"))
        }
    }

    #[test]
    fn invalid_values_are_rejected_before_service_dispatch() {
        let mut controller = MaskManagerController::new(FakeService);
        let error = controller
            .dispatch(&MaskManagerAction::SetOpacity(2.0))
            .expect_err("opacity is bounded");
        assert_eq!(error, MaskManagerControllerError::InvalidControl);
    }

    #[test]
    fn refresh_installs_typed_group_state() {
        let mut controller = MaskManagerController::new(FakeService);
        controller.refresh(3).expect("snapshot");
        assert!(matches!(
            controller.state().capability(),
            MaskManagerCapability::Available
        ));
        assert_eq!(controller.state().selected_group(), Some("subject"));
    }
}
