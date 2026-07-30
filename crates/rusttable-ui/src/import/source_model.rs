//! Typed import source, place, and per-row selection projections.

use std::path::{Path, PathBuf};

/// Upper bound for one review surface, matching the native open-event limit.
pub const MAX_IMPORT_SOURCE_ROWS: usize = 256;

/// File extensions treated as raw by Darktable's import affordance.
pub const RAW_EXTENSIONS: [&str; 18] = [
    "arw", "cr2", "cr3", "dng", "erf", "fff", "iiq", "kdc", "mef", "nef", "nrw", "orf", "pef",
    "raf", "raw", "rw2", "srw", "x3f",
];

/// A typed place shown in the import source rail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPlace {
    label: String,
    path: PathBuf,
    recent: bool,
}

impl ImportPlace {
    #[must_use]
    pub fn new(label: impl Into<String>, path: PathBuf, recent: bool) -> Self {
        Self {
            label: label.into(),
            path,
            recent,
        }
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn is_recent(&self) -> bool {
        self.recent
    }
}

/// Deterministic source-row state visible to the import dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSourceRow {
    path: PathBuf,
    label: String,
    raw: bool,
    new: bool,
    selected: bool,
}

impl ImportSourceRow {
    #[must_use]
    pub fn new(path: PathBuf, new: bool, selected: bool) -> Self {
        let label = path
            .file_name()
            .and_then(|name| name.to_str())
            .map_or_else(|| path.display().to_string(), str::to_owned);
        let raw = is_raw_path(&path);
        Self {
            path,
            label,
            raw,
            new,
            selected,
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub const fn is_raw(&self) -> bool {
        self.raw
    }

    #[must_use]
    pub const fn is_new(&self) -> bool {
        self.new
    }

    #[must_use]
    pub const fn selected(&self) -> bool {
        self.selected
    }
}

/// Source discovery state, including truthful empty/error/retry projections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportSourceState {
    Empty,
    Loading,
    Ready,
    Error { detail: String },
    Retrying,
}

impl ImportSourceState {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Empty => "Choose a source to continue.",
            Self::Loading => "Reading source…",
            Self::Ready => "Source ready.",
            Self::Error { .. } => "Source could not be read.",
            Self::Retrying => "Retrying source…",
        }
    }
}

/// Pure selection model used by the GTK rows and by the import request boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSourceModel {
    generation: u64,
    recursive: bool,
    select_new: bool,
    ignore_nonraws: bool,
    state: ImportSourceState,
    rows: Vec<ImportSourceRow>,
}

impl Default for ImportSourceModel {
    fn default() -> Self {
        Self {
            generation: 0,
            recursive: false,
            select_new: true,
            ignore_nonraws: false,
            state: ImportSourceState::Empty,
            rows: Vec::new(),
        }
    }
}

impl ImportSourceModel {
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn state(&self) -> &ImportSourceState {
        &self.state
    }

    #[must_use]
    pub fn rows(&self) -> impl ExactSizeIterator<Item = &ImportSourceRow> {
        self.rows.iter()
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

    pub fn begin(&mut self, generation: u64) {
        self.generation = generation;
        self.state = ImportSourceState::Loading;
        self.rows.clear();
    }

    pub fn fail(&mut self, detail: impl Into<String>) {
        self.state = ImportSourceState::Error {
            detail: detail.into(),
        };
        self.rows.clear();
    }

    pub fn retry(&mut self) {
        self.state = ImportSourceState::Retrying;
    }

    pub fn replace_rows(
        &mut self,
        generation: u64,
        paths: impl IntoIterator<Item = (PathBuf, bool)>,
    ) {
        self.generation = generation;
        self.rows = paths
            .into_iter()
            .take(MAX_IMPORT_SOURCE_ROWS)
            .map(|(path, new)| ImportSourceRow::new(path, new, self.select_new && new))
            .collect();
        self.state = if self.rows.is_empty() {
            ImportSourceState::Empty
        } else {
            ImportSourceState::Ready
        };
    }

    pub fn set_recursive(&mut self, recursive: bool) {
        self.recursive = recursive;
    }

    pub fn set_select_new(&mut self, select_new: bool) {
        self.select_new = select_new;
        if select_new {
            for row in &mut self.rows {
                row.selected = row.new;
            }
        }
    }

    pub fn set_ignore_nonraws(&mut self, ignore_nonraws: bool) {
        self.ignore_nonraws = ignore_nonraws;
    }

    pub fn select_all(&mut self) {
        for row in &mut self.rows {
            row.selected = true;
        }
    }

    pub fn select_none(&mut self) {
        for row in &mut self.rows {
            row.selected = false;
        }
    }

    pub fn set_selected(&mut self, path: &Path, selected: bool) {
        if let Some(row) = self.rows.iter_mut().find(|row| row.path == path) {
            row.selected = selected;
        }
    }

    #[must_use]
    pub fn row_is_effectively_selected(&self, row: &ImportSourceRow) -> bool {
        row.selected && (!self.ignore_nonraws || row.raw)
    }

    #[must_use]
    pub fn effective_paths(&self) -> Vec<PathBuf> {
        self.rows
            .iter()
            .filter(|row| self.row_is_effectively_selected(row))
            .map(|row| row.path.clone())
            .collect()
    }
}

#[must_use]
pub fn is_raw_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            RAW_EXTENSIONS
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(extension))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<(PathBuf, bool)> {
        vec![
            (PathBuf::from("new.nef"), true),
            (PathBuf::from("new.jpg"), true),
            (PathBuf::from("old.arw"), false),
        ]
    }

    #[test]
    fn selection_actions_project_effective_paths_and_visible_raw_filter() {
        let mut model = ImportSourceModel::default();
        model.replace_rows(4, rows());
        assert_eq!(names(&model), ["new.nef", "new.jpg"]);

        model.set_ignore_nonraws(true);
        assert_eq!(names(&model), ["new.nef"]);
        model.select_none();
        assert!(model.effective_paths().is_empty());
        model.select_all();
        assert_eq!(names(&model), ["new.nef", "old.arw"]);
    }

    #[test]
    fn select_new_preserves_existing_row_state_without_stale_paths() {
        let mut model = ImportSourceModel::default();
        model.replace_rows(9, rows());
        model.select_all();
        model.set_select_new(true);
        assert_eq!(names(&model), ["new.nef", "new.jpg"]);
        assert_eq!(model.generation(), 9);
    }

    fn names(model: &ImportSourceModel) -> Vec<String> {
        model
            .effective_paths()
            .iter()
            .map(|path| path.to_str().expect("test paths are UTF-8").to_owned())
            .collect()
    }

    #[test]
    fn source_states_cover_empty_error_and_retry() {
        let mut model = ImportSourceModel::default();
        assert_eq!(model.state().label(), "Choose a source to continue.");
        model.begin(2);
        assert_eq!(model.state().label(), "Reading source…");
        model.fail("permission denied");
        assert!(matches!(model.state(), ImportSourceState::Error { .. }));
        model.retry();
        assert_eq!(model.state().label(), "Retrying source…");
    }
}
