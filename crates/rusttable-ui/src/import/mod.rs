use crate::PresentationText;

mod dialog;
mod source_model;

pub use dialog::{IMPORT_DIALOG_FOCUS_ORDER, IMPORT_DIALOG_WIDGET_IDS, ImportDialog};
pub use source_model::{
    ImportPlace, ImportSourceModel, ImportSourceRow, ImportSourceState, MAX_IMPORT_SOURCE_ROWS,
    RAW_EXTENSIONS, is_raw_path,
};

/// Typed request emitted by the GTK import dialog after source selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRequest {
    paths: Vec<std::path::PathBuf>,
    recursive: bool,
    select_new: bool,
    ignore_nonraws: bool,
    generation: u64,
}

impl ImportRequest {
    /// Creates a bounded request from the dialog's selected files or folder.
    #[must_use]
    pub fn new(
        paths: Vec<std::path::PathBuf>,
        recursive: bool,
        select_new: bool,
        ignore_nonraws: bool,
        generation: u64,
    ) -> Option<Self> {
        (!paths.is_empty()).then_some(Self {
            paths,
            recursive,
            select_new,
            ignore_nonraws,
            generation,
        })
    }

    #[must_use]
    pub fn paths(&self) -> &[std::path::PathBuf] {
        &self.paths
    }

    #[must_use]
    pub const fn recursive(&self) -> bool {
        self.recursive
    }

    #[must_use]
    pub const fn select_new(&self) -> bool {
        self.select_new
    }

    #[must_use]
    pub const fn ignore_nonraws(&self) -> bool {
        self.ignore_nonraws
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

/// Typed command emitted by GTK import controls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportAction {
    /// Open the GTK file chooser for one or more supported images.
    ChooseFiles,
    /// Import the selected files or source folder with its typed options.
    Import(ImportRequest),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportRowState {
    Queued,
    Opening,
    Hashing,
    Probing,
    DecodingHeader,
    Registering,
    GeneratingPreview,
    Completed,
    AlreadyImported,
    ImportedPreviewPending,
    ImportedPreviewFailed,
    Failed,
    Cancelled,
}

impl ImportRowState {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::Opening => "Opening",
            Self::Hashing => "Hashing",
            Self::Probing => "Probing",
            Self::DecodingHeader => "Reading image header",
            Self::Registering => "Registering",
            Self::GeneratingPreview => "Generating preview",
            Self::Completed => "Imported",
            Self::AlreadyImported => "Already imported",
            Self::ImportedPreviewPending => "Imported; preview pending",
            Self::ImportedPreviewFailed => "Imported; preview failed",
            Self::Failed => "Import failed",
            Self::Cancelled => "Cancelled",
        }
    }

    #[must_use]
    pub const fn can_retry(self) -> bool {
        matches!(self, Self::Failed | Self::ImportedPreviewFailed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRowViewModel {
    item_id: u64,
    alias: PresentationText,
    state: ImportRowState,
}

impl ImportRowViewModel {
    #[must_use]
    pub const fn new(item_id: u64, alias: PresentationText, state: ImportRowState) -> Self {
        Self {
            item_id,
            alias,
            state,
        }
    }

    #[must_use]
    pub const fn item_id(&self) -> u64 {
        self.item_id
    }

    #[must_use]
    pub const fn alias(&self) -> &PresentationText {
        &self.alias
    }

    #[must_use]
    pub const fn state(&self) -> ImportRowState {
        self.state
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportPanelViewModel {
    rows: Vec<ImportRowViewModel>,
    active: bool,
}

impl ImportPanelViewModel {
    #[must_use]
    pub fn new(rows: Vec<ImportRowViewModel>, active: bool) -> Self {
        Self { rows, active }
    }

    #[must_use]
    pub fn rows(&self) -> impl ExactSizeIterator<Item = &ImportRowViewModel> {
        self.rows.iter()
    }

    #[must_use]
    pub const fn active(&self) -> bool {
        self.active
    }

    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.active || !self.rows.is_empty()
    }

    pub fn remove(&mut self, item_id: u64) {
        self.rows.retain(|row| row.item_id != item_id);
    }

    pub fn update_state(&mut self, item_id: u64, state: ImportRowState) {
        if let Some(row) = self.rows.iter_mut().find(|row| row.item_id == item_id) {
            row.state = state;
        }
    }
}
mod session_controller;
mod session_model;
mod session_view;

pub use session_controller::{
    ImportSessionAction, ImportSessionController, ImportSessionControllerError,
    ImportSessionServiceError, ImportSessionServicePort,
};
pub use session_model::{
    ImportItemOutcome, ImportReviewRow, ImportSessionEvent, ImportSessionState,
    ImportSessionViewModel,
};
pub use session_view::{IMPORT_SESSION_FOCUS_ORDER, ImportSessionPanel};
