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

impl AiModelsControllerError {
    #[must_use]
    pub const fn message(&self) -> &'static str {
        match self {
            Self::Service(error) => error.failure().message(),
            Self::NoStagedPackage => "No validated local model package is staged.",
            Self::NoQualification => "No active provider qualification can be cancelled.",
        }
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
            state: AiModelsViewModel::loading(),
        }
    }
    #[must_use]
    pub const fn state(&self) -> &AiModelsViewModel {
        &self.state
    }

    pub fn refresh(&mut self) -> Result<(), AiModelsControllerError> {
        self.state.begin_refresh();
        match self.service.snapshot() {
            Ok(snapshot) => {
                if !self.state.replace_snapshot_if_newer(snapshot) {
                    self.state.announce(
                        "A stale model registry refresh was ignored; current state kept.",
                    );
                }
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
                let selected_model = model.clone();
                let result = self.service.set_task_default(task, model);
                self.apply_snapshot(result)?;
                self.state.select_task(task);
                self.state.select_model(selected_model);
                Ok(())
            }
            AiModelsAction::SelectQualificationProvider(provider) => {
                self.state.select_provider(provider);
                self.state.announce(format!(
                    "{} selected for provider qualification.",
                    provider.label()
                ));
                Ok(())
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
    use super::super::model::QualificationState;
    use super::*;
    use crate::{
        AiModelsFailure, AiProvider, AiProviderPolicy, AiTask, InstallSummary, InstalledModel,
        ModelHash, ProviderCapability, QualificationJob,
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

    #[derive(Debug)]
    struct RefreshService {
        snapshots: Vec<AiModelsSnapshot>,
    }

    impl AiModelsServicePort for RefreshService {
        fn snapshot(&mut self) -> Result<AiModelsSnapshot, AiModelsServiceError> {
            Ok(self.snapshots.remove(0))
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
    fn refresh_rejects_older_generation_without_losing_selection() {
        let model = InstalledModel::new(
            "rgb-denoise",
            "1",
            ModelHash::new("c".repeat(64)).expect("hash"),
            AiTask::RawBayerDenoise,
            10,
            "NCHW f32",
            "256²",
            "linear RGB",
            vec![ProviderCapability::new(
                AiProvider::Cpu,
                QualificationState::Qualified,
            )],
        );
        let hash = model.hash().clone();
        let snapshots = vec![
            AiModelsSnapshot::available(vec![model.clone()]).with_generation(8),
            AiModelsSnapshot::available(Vec::new()).with_generation(7),
        ];
        let mut controller = AiModelsController::new(RefreshService { snapshots });
        controller.refresh().expect("first refresh");
        controller.refresh().expect("stale refresh is harmless");
        assert_eq!(controller.state().snapshot().generation(), 8);
        assert_eq!(controller.state().selected_model(), Some(&hash));
    }

    #[derive(Debug)]
    struct RecordingService {
        snapshot: AiModelsSnapshot,
        calls: Vec<&'static str>,
    }

    impl AiModelsServicePort for RecordingService {
        fn snapshot(&mut self) -> Result<AiModelsSnapshot, AiModelsServiceError> {
            self.calls.push("snapshot");
            Ok(self.snapshot.clone())
        }
        fn stage_local_package(
            &mut self,
            _: PathBuf,
        ) -> Result<InstallSummary, AiModelsServiceError> {
            self.calls.push("stage");
            Ok(InstallSummary::new(
                "model.rtmodel",
                "rgb-denoise",
                "1",
                ModelHash::new("d".repeat(64)).expect("hash"),
                100,
                AiTask::RawBayerDenoise,
                "bounded package validation passed",
            ))
        }
        fn install_staged(&mut self) -> Result<AiModelsSnapshot, AiModelsServiceError> {
            self.calls.push("install");
            Ok(self.snapshot.clone())
        }
        fn set_provider_policy(
            &mut self,
            _: AiProviderPolicy,
        ) -> Result<AiModelsSnapshot, AiModelsServiceError> {
            self.calls.push("provider-policy");
            Ok(self.snapshot.clone())
        }
        fn set_task_default(
            &mut self,
            _: AiTask,
            _: Option<ModelHash>,
        ) -> Result<AiModelsSnapshot, AiModelsServiceError> {
            self.calls.push("task-default");
            Ok(self.snapshot.clone())
        }
        fn set_enabled(
            &mut self,
            _: &ModelHash,
            _: bool,
        ) -> Result<AiModelsSnapshot, AiModelsServiceError> {
            self.calls.push("enabled");
            Ok(self.snapshot.clone())
        }
        fn start_qualification(
            &mut self,
            model: &ModelHash,
            provider: AiProvider,
        ) -> Result<QualificationJob, AiModelsServiceError> {
            self.calls.push("qualify");
            Ok(QualificationJob::new(7, model.clone(), provider, 2))
        }
        fn cancel_qualification(&mut self, _: u64) -> Result<(), AiModelsServiceError> {
            self.calls.push("cancel-qualification");
            Ok(())
        }
        fn remove(&mut self, _: &ModelHash) -> Result<AiModelsSnapshot, AiModelsServiceError> {
            self.calls.push("remove");
            Ok(self.snapshot.clone())
        }
    }

    #[test]
    fn every_management_action_crosses_the_typed_service_port() {
        let model = ModelHash::new("e".repeat(64)).expect("hash");
        let service = RecordingService {
            snapshot: AiModelsSnapshot::available(Vec::new()),
            calls: Vec::new(),
        };
        let mut controller = AiModelsController::new(service);
        controller
            .dispatch(AiModelsAction::Refresh)
            .expect("refresh");
        controller
            .dispatch(AiModelsAction::SelectLocalPackage(PathBuf::from(
                "model.rtmodel",
            )))
            .expect("stage");
        controller
            .dispatch(AiModelsAction::ConfirmInstall)
            .expect("install");
        controller
            .dispatch(AiModelsAction::SetProviderPolicy(AiProviderPolicy::Cpu))
            .expect("provider policy");
        controller
            .dispatch(AiModelsAction::SetTaskDefault {
                task: AiTask::RawBayerDenoise,
                model: Some(model.clone()),
            })
            .expect("task default");
        controller
            .dispatch(AiModelsAction::SetEnabled {
                model: model.clone(),
                enabled: false,
            })
            .expect("disable");
        controller
            .dispatch(AiModelsAction::SetEnabled {
                model: model.clone(),
                enabled: true,
            })
            .expect("enable");
        controller
            .dispatch(AiModelsAction::Qualify {
                model: model.clone(),
                provider: AiProvider::Cpu,
            })
            .expect("qualify");
        controller
            .dispatch(AiModelsAction::CancelQualification(7))
            .expect("cancel qualification");
        controller
            .dispatch(AiModelsAction::Remove(model))
            .expect("remove");
        assert_eq!(
            controller.service.calls,
            vec![
                "snapshot",
                "stage",
                "install",
                "provider-policy",
                "task-default",
                "enabled",
                "enabled",
                "qualify",
                "cancel-qualification",
                "remove",
            ]
        );
    }
}
