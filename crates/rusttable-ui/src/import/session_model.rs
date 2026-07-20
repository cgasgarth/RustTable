/// State of the review/import/recovery workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImportSessionState {
    #[default]
    Idle,
    Reviewing,
    Running,
    Paused,
    Recovering,
    Complete,
    Failed,
}

impl ImportSessionState {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Idle => "No import session",
            Self::Reviewing => "Review before import",
            Self::Running => "Importing",
            Self::Paused => "Paused",
            Self::Recovering => "Recovering session",
            Self::Complete => "Import complete",
            Self::Failed => "Import needs attention",
        }
    }
}

/// Truthful row outcome used for duplicate/failure/retry visibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportItemOutcome {
    Queued,
    Transferring,
    Imported,
    Duplicate,
    Failed { retryable: bool },
    Skipped,
}

impl ImportItemOutcome {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::Transferring => "Transferring",
            Self::Imported => "Imported",
            Self::Duplicate => "Duplicate",
            Self::Failed { .. } => "Failed",
            Self::Skipped => "Skipped",
        }
    }
    #[must_use]
    pub const fn can_retry(&self) -> bool {
        matches!(self, Self::Failed { retryable: true })
    }
}

/// One privacy-safe review row; physical paths never enter the view model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReviewRow {
    pub item_id: String,
    pub alias: String,
    pub outcome: ImportItemOutcome,
    pub detail: Option<String>,
    pub receipt_id: Option<String>,
}

/// Projection returned by the import application service.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportSessionViewModel {
    pub session_id: Option<String>,
    pub state: ImportSessionState,
    pub rows: Vec<ImportReviewRow>,
    pub completed: u32,
    pub total: u32,
    pub receipt_id: Option<String>,
    pub diagnostic: Option<String>,
}

impl ImportSessionViewModel {
    #[must_use]
    pub fn row(&self, item_id: &str) -> Option<&ImportReviewRow> {
        self.rows.iter().find(|row| row.item_id == item_id)
    }
    #[must_use]
    pub fn retryable_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| row.outcome.can_retry())
            .count()
    }
}

/// Events emitted by the typed import-service port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportSessionEvent {
    Snapshot(ImportSessionViewModel),
    Progress {
        state: ImportSessionState,
        completed: u32,
        total: u32,
    },
    Row(ImportReviewRow),
    Receipt {
        receipt_id: String,
    },
    Error {
        code: String,
        detail: String,
    },
}
