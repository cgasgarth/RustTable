use super::session_model::{
    ImportReviewRow, ImportSessionEvent, ImportSessionState, ImportSessionViewModel,
};

/// Typed intent emitted by the import-session view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportSessionAction {
    Review,
    Start,
    Pause,
    Resume,
    Retry(String),
    Recover,
    Rollback,
}

/// Service-side failure with bounded, user-safe diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSessionServiceError {
    pub code: String,
    pub detail: String,
}

/// Commands accepted by the existing import orchestration adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportSessionCommand {
    Review,
    Start,
    Pause,
    Resume,
    Retry { item_id: String },
    Recover,
    Rollback,
}

/// Typed import service port; filesystem, decoding, and catalog work stay behind it.
pub trait ImportSessionServicePort {
    /// Sends one typed import-session command to the application service.
    ///
    /// # Errors
    ///
    /// Returns a bounded service diagnostic when orchestration cannot accept
    /// the command.
    fn dispatch(
        &mut self,
        command: ImportSessionCommand,
    ) -> Result<ImportSessionEvent, ImportSessionServiceError>;
}

/// Controller for deterministic review/progress/retry/resume/recovery state.
pub struct ImportSessionController<P> {
    port: P,
    model: ImportSessionViewModel,
}

impl<P: ImportSessionServicePort> ImportSessionController<P> {
    #[must_use]
    pub fn new(port: P) -> Self {
        Self {
            port,
            model: ImportSessionViewModel::default(),
        }
    }
    #[must_use]
    pub const fn model(&self) -> &ImportSessionViewModel {
        &self.model
    }
    /// Dispatches a typed import-session intent and returns the updated model.
    ///
    /// # Errors
    ///
    /// Returns the service diagnostic when import orchestration rejects the
    /// command.
    pub fn dispatch(
        &mut self,
        action: ImportSessionAction,
    ) -> Result<&ImportSessionViewModel, ImportSessionControllerError> {
        let event = self.port.dispatch(match action {
            ImportSessionAction::Review => ImportSessionCommand::Review,
            ImportSessionAction::Start => ImportSessionCommand::Start,
            ImportSessionAction::Pause => ImportSessionCommand::Pause,
            ImportSessionAction::Resume => ImportSessionCommand::Resume,
            ImportSessionAction::Retry(item_id) => ImportSessionCommand::Retry { item_id },
            ImportSessionAction::Recover => ImportSessionCommand::Recover,
            ImportSessionAction::Rollback => ImportSessionCommand::Rollback,
        })?;
        self.apply(event);
        Ok(&self.model)
    }

    fn apply(&mut self, event: ImportSessionEvent) {
        match event {
            ImportSessionEvent::Snapshot(snapshot) => self.model = snapshot,
            ImportSessionEvent::Progress {
                state,
                completed,
                total,
            } => {
                self.model.state = state;
                self.model.completed = completed;
                self.model.total = total;
            }
            ImportSessionEvent::Row(row) => upsert_row(&mut self.model.rows, row),
            ImportSessionEvent::Receipt { receipt_id } => self.model.receipt_id = Some(receipt_id),
            ImportSessionEvent::Error { code, detail } => {
                self.model.state = ImportSessionState::Failed;
                self.model.diagnostic = Some(format!("{code}: {detail}"));
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportSessionControllerError {
    Service(ImportSessionServiceError),
}

impl From<ImportSessionServiceError> for ImportSessionControllerError {
    fn from(error: ImportSessionServiceError) -> Self {
        Self::Service(error)
    }
}

fn upsert_row(rows: &mut Vec<ImportReviewRow>, row: ImportReviewRow) {
    if let Some(existing) = rows
        .iter_mut()
        .find(|existing| existing.item_id == row.item_id)
    {
        *existing = row;
    } else {
        rows.push(row);
    }
}

#[cfg(test)]
mod tests {
    use super::super::session_model::{ImportItemOutcome, ImportReviewRow};
    use super::*;

    struct Fake {
        event: Option<ImportSessionEvent>,
        command: Option<ImportSessionCommand>,
    }
    impl ImportSessionServicePort for Fake {
        fn dispatch(
            &mut self,
            command: ImportSessionCommand,
        ) -> Result<ImportSessionEvent, ImportSessionServiceError> {
            self.command = Some(command);
            Ok(self.event.take().expect("event"))
        }
    }

    #[test]
    fn controller_keeps_duplicate_and_retry_rows_visible() {
        let row = ImportReviewRow {
            item_id: "2".into(),
            alias: "camera/0002.cr3".into(),
            outcome: ImportItemOutcome::Failed { retryable: true },
            detail: Some("permission".into()),
            receipt_id: None,
        };
        let fake = Fake {
            event: Some(ImportSessionEvent::Row(row)),
            command: None,
        };
        let mut controller = ImportSessionController::new(fake);
        controller
            .dispatch(ImportSessionAction::Review)
            .expect("review");
        assert_eq!(controller.model().retryable_count(), 1);
        assert_eq!(
            controller.model().row("2").map(|row| row.alias.as_str()),
            Some("camera/0002.cr3")
        );
    }
}
