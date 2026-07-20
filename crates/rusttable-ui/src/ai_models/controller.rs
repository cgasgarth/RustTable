//! Model-settings controller. All mutation is delegated to the registry service.

#![allow(clippy::missing_errors_doc)]

use std::path::PathBuf;

use super::model::{
    AiModelsAction, AiModelsServiceError, AiModelsServicePort, AiModelsSnapshot, AiModelsViewModel,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiModelsControllerError {
    Service(AiModelsServiceError),
    NoStagedPackage,
    NoQualification,
}

impl From<AiModelsServiceError> for AiModelsControllerError {
    fn from(value: AiModelsServiceError) -> Self {
        Self::Service(value)
    }
}

#[derive(Debug)]
pub struct AiModelsController<S> {
    service: S,
    state: AiModelsViewModel,
}

impl<S: AiModelsServicePort> AiModelsController<S> {
    #[must_use]
    pub fn new(service: S) -> Self {
        Self {
            service,
            state: AiModelsViewModel::default(),
        }
    }
    #[must_use]
    pub const fn state(&self) -> &AiModelsViewModel {
        &self.state
    }

    pub fn refresh(&mut self) -> Result<(), AiModelsControllerError> {
        match self.service.snapshot() {
            Ok(snapshot) => {
                self.state.replace_snapshot(snapshot);
                Ok(())
            }
            Err(error) => {
                self.state.fail(error.clone().failure());
                Err(error.into())
            }
        }
    }

    pub fn dispatch(&mut self, action: AiModelsAction) -> Result<(), AiModelsControllerError> {
        match action {
            AiModelsAction::Refresh => self.refresh(),
            AiModelsAction::SelectLocalPackage(path) => self.stage(path),
            AiModelsAction::ConfirmInstall => {
                if self.state.staging().is_none() {
                    return Err(AiModelsControllerError::NoStagedPackage);
                }
                let result = self.service.install_staged();
                let snapshot = match result {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        self.state.fail(error.clone().failure());
                        return Err(error.into());
                    }
                };
                self.state.replace_snapshot(snapshot);
                self.state.set_staging(None);
                self.state.announce(
                    "Model package installed atomically; no runtime details were exposed.",
                );
                Ok(())
            }
            AiModelsAction::CancelInstall => {
                self.state.set_staging(None);
                self.state.announce("Package installation cancelled.");
                Ok(())
            }
            AiModelsAction::SetProviderPolicy(policy) => {
                let result = self.service.set_provider_policy(policy);
                self.apply_snapshot(result)
            }
            AiModelsAction::SetTaskDefault { task, model } => {
                let result = self.service.set_task_default(task, model);
                self.apply_snapshot(result)
            }
            AiModelsAction::SetEnabled { model, enabled } => {
                let result = self.service.set_enabled(&model, enabled);
                self.apply_snapshot(result)
            }
            AiModelsAction::Qualify { model, provider } => {
                let job = self
                    .service
                    .start_qualification(&model, provider)
                    .inspect_err(|error| {
                        self.state.fail(error.clone().failure());
                    })?;
                self.state.set_qualification(Some(job));
                self.state
                    .announce("Provider qualification started; it uses bounded fixtures only.");
                Ok(())
            }
            AiModelsAction::CancelQualification(job) => {
                self.service
                    .cancel_qualification(job)
                    .inspect_err(|error| {
                        self.state.fail(error.clone().failure());
                    })?;
                self.state.set_qualification(None);
                self.state
                    .announce("Provider qualification cancellation requested.");
                Ok(())
            }
            AiModelsAction::Remove(model) => {
                let result = self.service.remove(&model);
                self.apply_snapshot(result)
            }
        }
    }

    fn stage(&mut self, path: PathBuf) -> Result<(), AiModelsControllerError> {
        let summary = self
            .service
            .stage_local_package(path)
            .inspect_err(|error| {
                self.state.fail(error.clone().failure());
            })?;
        self.state.set_staging(Some(summary));
        self.state
            .announce("Package validated by the model service; confirm to install.");
        Ok(())
    }

    fn apply_snapshot(
        &mut self,
        result: Result<AiModelsSnapshot, AiModelsServiceError>,
    ) -> Result<(), AiModelsControllerError> {
        match result {
            Ok(snapshot) => {
                self.state.replace_snapshot(snapshot);
                Ok(())
            }
            Err(error) => {
                self.state.fail(error.clone().failure());
                Err(error.into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AiModelsFailure, AiProvider, AiProviderPolicy, AiTask, InstallSummary, ModelHash,
        QualificationJob,
    };

    #[derive(Default)]
    struct Unavailable;
    impl AiModelsServicePort for Unavailable {
        fn snapshot(&mut self) -> Result<AiModelsSnapshot, AiModelsServiceError> {
            Err(AiModelsServiceError::Unavailable)
        }
        fn stage_local_package(
            &mut self,
            _: PathBuf,
        ) -> Result<InstallSummary, AiModelsServiceError> {
            Err(AiModelsServiceError::Unavailable)
        }
        fn install_staged(&mut self) -> Result<AiModelsSnapshot, AiModelsServiceError> {
            Err(AiModelsServiceError::Unavailable)
        }
        fn set_provider_policy(
            &mut self,
            _: AiProviderPolicy,
        ) -> Result<AiModelsSnapshot, AiModelsServiceError> {
            Err(AiModelsServiceError::Unavailable)
        }
        fn set_task_default(
            &mut self,
            _: AiTask,
            _: Option<ModelHash>,
        ) -> Result<AiModelsSnapshot, AiModelsServiceError> {
            Err(AiModelsServiceError::Unavailable)
        }
        fn set_enabled(
            &mut self,
            _: &ModelHash,
            _: bool,
        ) -> Result<AiModelsSnapshot, AiModelsServiceError> {
            Err(AiModelsServiceError::Unavailable)
        }
        fn start_qualification(
            &mut self,
            _: &ModelHash,
            _: AiProvider,
        ) -> Result<QualificationJob, AiModelsServiceError> {
            Err(AiModelsServiceError::Unavailable)
        }
        fn cancel_qualification(&mut self, _: u64) -> Result<(), AiModelsServiceError> {
            Err(AiModelsServiceError::Unavailable)
        }
        fn remove(&mut self, _: &ModelHash) -> Result<AiModelsSnapshot, AiModelsServiceError> {
            Err(AiModelsServiceError::Unavailable)
        }
    }

    #[test]
    fn unavailable_service_stays_truthful() {
        let mut controller = AiModelsController::new(Unavailable);
        assert!(controller.refresh().is_err());
        assert_eq!(
            controller.state().failure(),
            Some(AiModelsFailure::ServiceUnavailable)
        );
        assert!(
            controller
                .dispatch(AiModelsAction::SelectLocalPackage(PathBuf::from(
                    "model.rtmodel"
                )))
                .is_err()
        );
    }
}
