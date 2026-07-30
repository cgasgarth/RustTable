//! Controller for external-editor commands and service updates.

#![allow(clippy::missing_errors_doc)]

use super::model::{
    ExternalEditorAction, ExternalEditorServiceError, ExternalEditorServicePort,
    ExternalEditorViewModel, InvocationReview, Launchability,
};
use crate::presentation::PresentationText;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalEditorControllerError {
    Service(ExternalEditorServiceError),
    NoPreset,
    NoSelection,
    NotLaunchable(Launchability),
}

impl From<ExternalEditorServiceError> for ExternalEditorControllerError {
    fn from(value: ExternalEditorServiceError) -> Self {
        Self::Service(value)
    }
}

/// Keeps workflow state and service calls outside GTK widgets.
#[derive(Debug)]
pub struct ExternalEditorController<S> {
    service: S,
    state: ExternalEditorViewModel,
}

impl<S> ExternalEditorController<S>
where
    S: ExternalEditorServicePort,
{
    #[must_use]
    pub fn new(service: S) -> Self {
        Self {
            service,
            state: ExternalEditorViewModel::default(),
        }
    }

    #[must_use]
    pub const fn state(&self) -> &ExternalEditorViewModel {
        &self.state
    }

    pub fn refresh(&mut self) -> Result<(), ExternalEditorControllerError> {
        let presets = self.service.list_presets()?;
        self.state.replace_presets(presets);
        Ok(())
    }

    pub fn dispatch(
        &mut self,
        action: ExternalEditorAction,
    ) -> Result<(), ExternalEditorControllerError> {
        match action {
            ExternalEditorAction::SaveDraft(draft) => {
                let preset = self.service.save_preset(draft)?;
                self.state.upsert_preset(preset);
                self.announce("Preset saved")?;
            }
            ExternalEditorAction::SelectPreset(preset) => self.state.select_preset(preset),
            ExternalEditorAction::TestPreset(preset) => {
                let receipt = self.service.test_preset(preset)?;
                self.state.apply_receipt(&receipt);
            }
            ExternalEditorAction::ReviewSend => {
                let preset_id = self
                    .state
                    .selected_preset()
                    .ok_or(ExternalEditorControllerError::NoPreset)?;
                let preset = self
                    .state
                    .presets()
                    .iter()
                    .find(|preset| preset.id() == preset_id)
                    .ok_or(ExternalEditorControllerError::NoPreset)?;
                match preset.launchability(self.state.selected_photos().len()) {
                    Launchability::Ready => self.state.set_review(Some(InvocationReview::new(
                        preset,
                        self.state.selected_photos().to_vec(),
                        self.state.source_revision(),
                    ))),
                    other => return Err(ExternalEditorControllerError::NotLaunchable(other)),
                }
            }
            ExternalEditorAction::ConfirmSend(request) => {
                if request.photos.is_empty() {
                    return Err(ExternalEditorControllerError::NoSelection);
                }
                let jobs = self.service.send_to_editor(request)?;
                for job in jobs {
                    self.state.apply_job(job);
                }
                self.state.set_review(None);
                self.announce("External-editor jobs queued")?;
            }
            ExternalEditorAction::CancelJob(job) => {
                self.service.cancel_job(job)?;
                self.announce("Cancellation requested")?;
            }
            ExternalEditorAction::ReconcileJob(job) => {
                let job = self.service.reconcile_job(job)?;
                self.state.apply_job(job);
                self.announce("Reconciliation refreshed")?;
            }
            ExternalEditorAction::Complete(job, action) => {
                self.service.complete(job, action)?;
                self.announce("Completion action sent to service")?;
            }
            ExternalEditorAction::NewPreset
            | ExternalEditorAction::ChooseExecutable
            | ExternalEditorAction::AddLiteralArgument
            | ExternalEditorAction::AddPlaceholderArgument(_) => {
                self.announce("Preset editor changes are ready for the configuration service")?;
            }
        }
        Ok(())
    }

    fn announce(&mut self, value: &str) -> Result<(), ExternalEditorControllerError> {
        self.state
            .announce(PresentationText::new(value).map_err(|_| {
                ExternalEditorControllerError::Service(ExternalEditorServiceError::InvalidRequest)
            })?);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use rusttable_core::{PhotoId, Revision};

    use super::*;
    use crate::external_editor::model::{
        ArgumentRow, CompletionAction, ExecutableApproval, ExecutableIdentity, ExternalEditorJob,
        ExternalEditorPreset, InterchangeMode, JobId, JobStage, Placeholder, PresetId,
        QualificationOutcome, QualificationReceipt, SendToEditorRequest,
    };

    #[derive(Default)]
    struct FakeService {
        presets: Vec<ExternalEditorPreset>,
        receipts: VecDeque<QualificationReceipt>,
        jobs: Vec<ExternalEditorJob>,
        sent: usize,
        cancelled: usize,
        reconciled: usize,
    }

    impl ExternalEditorServicePort for FakeService {
        fn list_presets(
            &mut self,
        ) -> Result<Vec<ExternalEditorPreset>, ExternalEditorServiceError> {
            Ok(self.presets.clone())
        }
        fn save_preset(
            &mut self,
            _draft: super::super::model::ExternalEditorDraft,
        ) -> Result<ExternalEditorPreset, ExternalEditorServiceError> {
            self.presets
                .first()
                .cloned()
                .ok_or(ExternalEditorServiceError::NotFound)
        }
        fn test_preset(
            &mut self,
            _preset: PresetId,
        ) -> Result<QualificationReceipt, ExternalEditorServiceError> {
            self.receipts
                .pop_front()
                .ok_or(ExternalEditorServiceError::NotFound)
        }
        fn send_to_editor(
            &mut self,
            _request: SendToEditorRequest,
        ) -> Result<Vec<ExternalEditorJob>, ExternalEditorServiceError> {
            self.sent += 1;
            Ok(self.jobs.clone())
        }
        fn cancel_job(&mut self, _job: JobId) -> Result<(), ExternalEditorServiceError> {
            self.cancelled += 1;
            Ok(())
        }
        fn reconcile_job(
            &mut self,
            job: JobId,
        ) -> Result<ExternalEditorJob, ExternalEditorServiceError> {
            self.reconciled += 1;
            self.jobs
                .iter()
                .find(|value| value.id() == job)
                .cloned()
                .ok_or(ExternalEditorServiceError::NotFound)
        }
        fn complete(
            &mut self,
            _job: JobId,
            _action: CompletionAction,
        ) -> Result<(), ExternalEditorServiceError> {
            Ok(())
        }
    }

    fn text(value: &str) -> crate::presentation::PresentationText {
        crate::presentation::PresentationText::new(value).expect("valid text")
    }
    fn preset() -> ExternalEditorPreset {
        ExternalEditorPreset::new(
            PresetId::new(1).expect("id"),
            Revision::from_u64(2),
            text("Editor"),
            ExecutableIdentity::new(
                text("editor"),
                Some(text("editor")),
                ExecutableApproval::Current,
            ),
            InterchangeMode::InPlaceTiff,
            vec![ArgumentRow::placeholder(Placeholder::Input)],
            text("sRGB"),
            text("output.tiff"),
        )
        .expect("preset")
    }

    #[test]
    fn controller_qualifies_then_creates_an_immutable_send_review() {
        let preset_id = PresetId::new(1).expect("id");
        let mut service = FakeService {
            presets: vec![preset()],
            receipts: VecDeque::from([QualificationReceipt::new(
                preset_id,
                Revision::from_u64(2),
                QualificationOutcome::Qualified,
                text("qualified"),
            )]),
            ..FakeService::default()
        };
        let job = ExternalEditorJob::new(
            JobId::new(2).expect("job"),
            PhotoId::new(3).expect("photo"),
            JobStage::Staged,
            text("staged"),
        );
        service.jobs = vec![job];
        let mut controller = ExternalEditorController::new(service);
        controller.refresh().expect("refresh");
        controller
            .dispatch(ExternalEditorAction::TestPreset(preset_id))
            .expect("test");
        controller
            .state_mut_for_test()
            .set_selection(vec![PhotoId::new(3).expect("photo")], Revision::from_u64(8));
        controller
            .dispatch(ExternalEditorAction::ReviewSend)
            .expect("review");
        let review = controller.state().review().expect("review state");
        assert_eq!(review.photos(), &[PhotoId::new(3).expect("photo")]);
        assert_eq!(review.source_revision(), Revision::from_u64(8));
    }

    #[test]
    fn unqualified_send_is_rejected_before_service_call() {
        let mut controller = ExternalEditorController::new(FakeService {
            presets: vec![preset()],
            ..FakeService::default()
        });
        controller.refresh().expect("refresh");
        controller
            .state_mut_for_test()
            .set_selection(vec![PhotoId::new(3).expect("photo")], Revision::from_u64(1));
        assert_eq!(
            controller.dispatch(ExternalEditorAction::ReviewSend),
            Err(ExternalEditorControllerError::NotLaunchable(
                Launchability::PresetNotQualified
            ))
        );
    }

    impl<S> ExternalEditorController<S> {
        fn state_mut_for_test(&mut self) -> &mut ExternalEditorViewModel {
            &mut self.state
        }
    }
}
