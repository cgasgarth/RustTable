//! GTK-facing catalog state without direct GTK runtime types.
//!
//! The GTK shell owns widgets and dispatches its signals to this controller. Keeping catalog
//! loading and selection transitions here makes those interactions deterministic and testable
//! without a display server.

mod collection;
mod darkroom_edit;
mod darkroom_panels;
pub mod export;
pub mod preview;
pub mod thumbnail;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rusttable_catalog::{
    CatalogChangeEvent, CatalogCommand, CollectionRepositoryError, EditRepository,
    ImportRepository, PhotoOrganizationState, RepositoryError,
};
use rusttable_catalog_store::{
    AtomicCatalogStoreError, RedbCatalogRepository, RedbImportRepository,
};
use rusttable_core::{Edit, PhotoId};
use rusttable_i18n::LocaleTag;
use rusttable_import::{decode_reference_source, normalize_reference_path};
use rusttable_ui::{LibraryFailureKind, PhotoWorkspaceViewModel};

use crate::library::{LibraryLoadResult, catalog_path, load_catalog, source_root};

pub use collection::{
    ActiveLighttableRestoreReport, CollectionController, CollectionSnapshot,
    LibraryCollectionService,
};
pub use darkroom_edit::{DarkroomEditOutcome, GtkDarkroomEditController};
pub use darkroom_panels::{
    DarkroomPanelControllerError, DarkroomPanelProjections, GtkDarkroomPanelController,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GtkCatalogMutationError {
    NotReady,
    Open(RepositoryError),
    Repository(AtomicCatalogStoreError),
    Collection(CollectionRepositoryError),
}

impl std::fmt::Display for GtkCatalogMutationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotReady => formatter.write_str("catalog is not ready for organization changes"),
            Self::Open(error) => {
                write!(formatter, "catalog organization store unavailable: {error}")
            }
            Self::Repository(error) => {
                write!(formatter, "catalog organization change failed: {error:?}")
            }
            Self::Collection(error) => {
                write!(formatter, "active collection persistence failed: {error:?}")
            }
        }
    }
}

impl std::error::Error for GtkCatalogMutationError {}

/// Persisted catalog state consumed by the GTK application shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GtkCatalogState {
    Empty(GtkCatalogLocation),
    Ready(GtkReadyCatalog),
    Failed(GtkCatalogFailure),
}

impl GtkCatalogState {
    #[must_use]
    pub fn workspace(&self) -> Option<&PhotoWorkspaceViewModel> {
        match self {
            Self::Ready(catalog) => Some(catalog.workspace()),
            Self::Empty(_) | Self::Failed(_) => None,
        }
    }
}

/// A catalog location whose source root has been resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GtkCatalogLocation {
    catalog_path: PathBuf,
    source_root: PathBuf,
}

impl GtkCatalogLocation {
    #[must_use]
    pub fn catalog_path(&self) -> &Path {
        &self.catalog_path
    }

    #[must_use]
    pub fn source_root(&self) -> &Path {
        &self.source_root
    }
}

/// A ready catalog and its display-safe workspace projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GtkReadyCatalog {
    location: GtkCatalogLocation,
    workspace: PhotoWorkspaceViewModel,
}

impl GtkReadyCatalog {
    #[must_use]
    pub fn location(&self) -> &GtkCatalogLocation {
        &self.location
    }

    #[must_use]
    pub fn workspace(&self) -> &PhotoWorkspaceViewModel {
        &self.workspace
    }
}

/// The existing library failure, retained verbatim for GTK presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GtkCatalogFailure {
    catalog_path: Option<PathBuf>,
    kind: LibraryFailureKind,
}

impl GtkCatalogFailure {
    #[must_use]
    pub fn catalog_path(&self) -> Option<&Path> {
        self.catalog_path.as_deref()
    }

    #[must_use]
    pub const fn kind(&self) -> LibraryFailureKind {
        self.kind
    }
}

/// Display-independent GTK catalog controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GtkCatalogController {
    state: GtkCatalogState,
    selected_photo: Option<PhotoId>,
}

impl GtkCatalogController {
    /// Loads the configured persisted catalog using the existing catalog location policy.
    #[must_use]
    pub fn load_persisted() -> Self {
        match catalog_path() {
            Ok(path) => Self::load_catalog_at(path),
            Err(kind) => Self::failed(None, kind),
        }
    }

    /// Returns whether the catalog-open operation produced a usable empty or ready state.
    #[must_use]
    pub fn opened_successfully(&self) -> bool {
        !matches!(self.state, GtkCatalogState::Failed(_))
    }

    /// Returns the current catalog state for GTK widget projection.
    #[must_use]
    pub fn state(&self) -> &GtkCatalogState {
        &self.state
    }

    /// Returns the selected photo, if the ready catalog still contains it.
    #[must_use]
    pub const fn selected_photo(&self) -> Option<PhotoId> {
        self.selected_photo
    }

    /// Captures the current persisted edit for one photo before an asynchronous render starts.
    ///
    /// The returned snapshot is the publication identity shared by the darkroom preview and the
    /// selected lighttable/filmstrip thumbnail. It is intentionally read-only here; edit writes
    /// remain owned by [`GtkDarkroomEditController`].
    ///
    /// # Errors
    ///
    /// Returns a repository error when the ready catalog or persisted edit list cannot be read.
    pub fn current_edit(&self, photo_id: PhotoId) -> Result<Option<Edit>, RepositoryError> {
        let GtkCatalogState::Ready(catalog) = &self.state else {
            return Ok(None);
        };
        let repository = RedbCatalogRepository::open(catalog.location().catalog_path())
            .map_err(|_| RepositoryError::Unavailable)?;
        Ok(EditRepository::list(&repository)
            .map_err(|_| RepositoryError::Unavailable)?
            .into_iter()
            .filter(|edit| edit.photo_id() == photo_id)
            .max_by_key(|edit| (edit.revision().get(), edit.id().get())))
    }

    /// Reopens the ready catalog's import records for collection filtering.
    ///
    /// The browser workspace remains the display projection; collection rules use the persisted
    /// source records so filmroll, folder, and filename values are not reconstructed from labels.
    #[must_use]
    pub fn collection_controller(&self) -> Option<CollectionController> {
        self.collection_controller_with_locale(LocaleTag::default_locale())
    }

    /// Reopens collection records with the locale selected by the application shell.
    #[must_use]
    pub fn collection_controller_with_locale(
        &self,
        locale: LocaleTag,
    ) -> Option<CollectionController> {
        self.try_collection_controller_with_locale(locale.clone()).map_or_else(
            |error| {
                tracing::warn!(target: "rusttable.catalog", error = %error, "active lighttable state unavailable; falling back to default collection state");
                self.default_collection_controller_with_locale(locale)
            },
            Some,
        )
    }

    fn default_collection_controller_with_locale(
        &self,
        locale: LocaleTag,
    ) -> Option<CollectionController> {
        let GtkCatalogState::Ready(catalog) = &self.state else {
            return None;
        };
        let repository = RedbCatalogRepository::open(catalog.location().catalog_path()).ok()?;
        let records = ImportRepository::list(&repository).ok()?;
        let organization = repository.organization_states().ok()?;
        Some(
            CollectionController::from_import_records_with_locale_and_organization(
                records.iter(),
                locale,
                &organization,
            ),
        )
    }

    /// Rebuilds the collection controller from catalog records and durable lighttable state.
    ///
    /// # Errors
    ///
    /// Returns a typed catalog or active-state persistence error when the controller cannot be
    /// rebuilt safely.
    pub fn try_collection_controller_with_locale(
        &self,
        locale: LocaleTag,
    ) -> Result<CollectionController, GtkCatalogMutationError> {
        let GtkCatalogState::Ready(catalog) = &self.state else {
            return Err(GtkCatalogMutationError::NotReady);
        };
        let repository = RedbCatalogRepository::open(catalog.location().catalog_path())
            .map_err(GtkCatalogMutationError::Open)?;
        let records = ImportRepository::list(&repository)
            .map_err(|_| GtkCatalogMutationError::Collection(CollectionRepositoryError::Corrupt))?;
        let organization = repository
            .organization_states()
            .map_err(|_| GtkCatalogMutationError::Collection(CollectionRepositoryError::Corrupt))?;
        let collection_repository = rusttable_catalog_store::RedbCollectionRepository::open(
            catalog.location().catalog_path(),
        )
        .map_err(GtkCatalogMutationError::Collection)?;
        let active = collection_repository
            .load_active_lighttable_state()
            .map_err(GtkCatalogMutationError::Collection)?;
        let (controller, report) =
            CollectionController::from_import_records_with_locale_and_organization_and_active_state(
                records.iter(),
                locale,
                &organization,
                &active,
            );
        if report.discarded_missing > 0 || report.discarded_hidden > 0 {
            tracing::warn!(
                target: "rusttable.catalog",
                discarded_missing = report.discarded_missing,
                discarded_hidden = report.discarded_hidden,
                "reconciled invalid active lighttable selections"
            );
        }
        if controller.active_state() != active {
            collection_repository
                .persist_active_lighttable_state(&controller.active_state())
                .map_err(GtkCatalogMutationError::Collection)?;
        }
        Ok(controller)
    }

    /// Persists the complete active lighttable state without changing the caller's controller.
    ///
    /// # Errors
    ///
    /// Returns a typed readiness or catalog-store error. The previous durable state remains
    /// intact when the transaction fails.
    pub fn persist_active_collection_state(
        &self,
        state: &rusttable_catalog::ActiveLighttableState,
    ) -> Result<(), GtkCatalogMutationError> {
        let GtkCatalogState::Ready(catalog) = &self.state else {
            return Err(GtkCatalogMutationError::NotReady);
        };
        let repository = rusttable_catalog_store::RedbCollectionRepository::open(
            catalog.location().catalog_path(),
        )
        .map_err(GtkCatalogMutationError::Collection)?;
        repository
            .persist_active_lighttable_state(state)
            .map_err(GtkCatalogMutationError::Collection)
    }

    /// Applies and persists one collection-controller mutation transactionally.
    ///
    /// # Errors
    ///
    /// Returns a typed catalog-store error; the supplied controller is unchanged on failure.
    pub fn apply_collection_controller_change<F>(
        &self,
        controller: &mut CollectionController,
        update: F,
    ) -> Result<(), GtkCatalogMutationError>
    where
        F: FnOnce(&mut CollectionController),
    {
        let mut next = controller.clone();
        update(&mut next);
        self.persist_active_collection_state(&next.active_state())?;
        *controller = next;
        Ok(())
    }

    /// Atomically applies a lighttable organization command and returns its committed event.
    ///
    /// # Errors
    ///
    /// Returns a typed catalog readiness or persistence error. Failed commands do not update
    /// the GTK projection.
    pub fn apply_organization_command(
        &mut self,
        command: &CatalogCommand,
    ) -> Result<
        (
            CatalogChangeEvent,
            BTreeMap<PhotoId, PhotoOrganizationState>,
        ),
        GtkCatalogMutationError,
    > {
        let GtkCatalogState::Ready(catalog) = &self.state else {
            return Err(GtkCatalogMutationError::NotReady);
        };
        let mut repository = RedbCatalogRepository::open(catalog.location().catalog_path())
            .map_err(GtkCatalogMutationError::Open)?;
        let event = repository
            .apply_organization_command(command)
            .map_err(GtkCatalogMutationError::Repository)?;
        let organization = repository
            .organization_states()
            .map_err(GtkCatalogMutationError::Repository)?;
        Ok((event, organization))
    }

    /// Opens the saved/recent/active library-view service for this catalog.
    #[must_use]
    pub fn collection_service(&self) -> Option<LibraryCollectionService> {
        self.catalog_path()
            .and_then(|path| LibraryCollectionService::open(path).ok())
    }

    /// Returns absolute source paths already represented by the ready catalog.
    #[must_use]
    pub fn existing_source_paths(&self) -> BTreeSet<PathBuf> {
        let GtkCatalogState::Ready(catalog) = &self.state else {
            return BTreeSet::new();
        };
        let Ok(repository) = RedbImportRepository::open(catalog.location().catalog_path()) else {
            return BTreeSet::new();
        };
        let Ok(records) = repository.list() else {
            return BTreeSet::new();
        };
        records
            .iter()
            .filter_map(|record| {
                chooser_source_path(record.source(), catalog.location().source_root())
            })
            .collect()
    }

    /// Selects a photo present in the ready workspace.
    ///
    /// Returns `true` only when the selection changed. Invalid selections and selections while
    /// loading an empty or failed catalog retain the current state.
    pub fn select_photo(&mut self, photo_id: PhotoId) -> bool {
        if self
            .state
            .workspace()
            .is_none_or(|workspace| workspace.detail(photo_id).is_none())
        {
            return false;
        }
        if self.selected_photo == Some(photo_id) {
            return false;
        }
        self.selected_photo = Some(photo_id);
        true
    }

    /// Clears the current photo selection and reports whether it changed.
    pub fn clear_selection(&mut self) -> bool {
        self.selected_photo.take().is_some()
    }

    /// Opens an explicitly selected `RustTable` catalog using catalog-open policy.
    #[must_use]
    pub fn load_catalog_at(catalog_path: PathBuf) -> Self {
        match source_root(&catalog_path) {
            Ok(source_root) => {
                let result = load_catalog(&catalog_path);
                Self::from_load_result(catalog_path, source_root, result)
            }
            Err(kind) => Self::failed(Some(catalog_path), kind),
        }
    }

    /// Returns the active catalog path, including empty and failed catalog states.
    #[must_use]
    pub fn catalog_path(&self) -> Option<&Path> {
        match &self.state {
            GtkCatalogState::Empty(location) => Some(location.catalog_path()),
            GtkCatalogState::Ready(catalog) => Some(catalog.location().catalog_path()),
            GtkCatalogState::Failed(failure) => failure.catalog_path(),
        }
    }

    fn from_load_result(
        catalog_path: PathBuf,
        source_root: PathBuf,
        result: LibraryLoadResult,
    ) -> Self {
        let location = GtkCatalogLocation {
            catalog_path,
            source_root,
        };
        let state = match result {
            LibraryLoadResult::Empty => GtkCatalogState::Empty(location),
            LibraryLoadResult::Ready(workspace) => GtkCatalogState::Ready(GtkReadyCatalog {
                location,
                workspace,
            }),
            LibraryLoadResult::Failed(kind) => GtkCatalogState::Failed(GtkCatalogFailure {
                catalog_path: Some(location.catalog_path),
                kind,
            }),
        };
        Self {
            state,
            selected_photo: None,
        }
    }

    fn failed(catalog_path: Option<PathBuf>, kind: LibraryFailureKind) -> Self {
        Self {
            state: GtkCatalogState::Failed(GtkCatalogFailure { catalog_path, kind }),
            selected_photo: None,
        }
    }
}

fn chooser_source_path(source: &rusttable_catalog::SourcePath, root: &Path) -> Option<PathBuf> {
    let path = decode_reference_source(source).unwrap_or_else(|_| root.join(source.as_str()));
    normalize_reference_path(&path).ok()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use rusttable_catalog::SourcePath;
    use rusttable_core::PhotoId;
    use rusttable_import::encode_reference_source;
    use rusttable_ui::{
        LibraryFailureKind, PhotoCardViewModel, PhotoDetailViewModel, PhotoWorkspaceViewModel,
        PresentationText,
    };

    use super::{GtkCatalogController, GtkCatalogState, chooser_source_path};
    use crate::library::LibraryLoadResult;

    fn photo_id(value: u128) -> PhotoId {
        PhotoId::new(value).expect("non-zero test photo ID")
    }

    fn workspace(ids: &[u128]) -> PhotoWorkspaceViewModel {
        let cards = ids
            .iter()
            .copied()
            .map(|id| {
                PhotoCardViewModel::new(
                    photo_id(id),
                    PresentationText::new(format!("photo-{id}")).expect("safe test title"),
                    None,
                )
            })
            .collect();
        let details = ids
            .iter()
            .copied()
            .map(|id| {
                PhotoDetailViewModel::new(
                    photo_id(id),
                    PresentationText::new(format!("photo-{id}")).expect("safe test title"),
                    Vec::new(),
                )
            })
            .collect();
        PhotoWorkspaceViewModel::new(cards, details).expect("matching test workspace")
    }

    fn ready_controller() -> GtkCatalogController {
        GtkCatalogController::from_load_result(
            PathBuf::from("/catalog/catalog.redb"),
            PathBuf::from("/catalog"),
            LibraryLoadResult::Ready(workspace(&[3, 7])),
        )
    }

    #[test]
    fn ready_catalog_retains_location_and_starts_without_a_selection() {
        let controller = ready_controller();

        let GtkCatalogState::Ready(catalog) = controller.state() else {
            panic!("expected ready GTK catalog");
        };
        assert_eq!(
            catalog.location().catalog_path(),
            PathBuf::from("/catalog/catalog.redb")
        );
        assert_eq!(catalog.location().source_root(), PathBuf::from("/catalog"));
        assert_eq!(catalog.workspace().cards().len(), 2);
        assert_eq!(controller.selected_photo(), None);
    }

    #[test]
    fn selection_transitions_only_accept_catalog_photo_ids() {
        let mut controller = ready_controller();
        let valid = photo_id(7);

        assert!(controller.select_photo(valid));
        assert_eq!(controller.selected_photo(), Some(valid));
        assert!(!controller.select_photo(valid));
        assert!(!controller.select_photo(photo_id(99)));
        assert_eq!(controller.selected_photo(), Some(valid));
        assert!(controller.clear_selection());
        assert!(!controller.clear_selection());
        assert_eq!(controller.selected_photo(), None);
    }

    #[test]
    fn empty_and_failed_results_clear_selection_and_preserve_failure_kind() {
        let empty = GtkCatalogController::from_load_result(
            PathBuf::from("/catalog/catalog.redb"),
            PathBuf::from("/catalog"),
            LibraryLoadResult::Empty,
        );
        assert!(!empty.clone().select_photo(photo_id(3)));
        assert_eq!(empty.selected_photo(), None);

        let failed = GtkCatalogController::from_load_result(
            PathBuf::from("/catalog/catalog.redb"),
            PathBuf::from("/catalog"),
            LibraryLoadResult::Failed(LibraryFailureKind::CorruptPersistedCatalog),
        );
        let GtkCatalogState::Failed(failure) = failed.state() else {
            panic!("expected failed GTK catalog");
        };
        let expected_catalog_path = PathBuf::from("/catalog/catalog.redb");
        assert_eq!(failure.kind(), LibraryFailureKind::CorruptPersistedCatalog);
        assert_eq!(
            failure.catalog_path(),
            Some(expected_catalog_path.as_path())
        );
        assert_eq!(failed.selected_photo(), None);
    }

    #[test]
    fn chooser_decodes_reference_v1_before_comparing_physical_paths() {
        let source = encode_reference_source(Path::new("/photos/./roll/../IMG_0001.JPG"), [7; 32])
            .expect("reference-v1 source");
        assert_eq!(
            chooser_source_path(&source, Path::new("/catalog")),
            Some(PathBuf::from("/photos/IMG_0001.JPG"))
        );
        let logical = SourcePath::new("folder/photo.jpg").expect("logical source");
        assert_eq!(
            chooser_source_path(&logical, Path::new("/catalog")),
            Some(PathBuf::from("/catalog/folder/photo.jpg"))
        );
    }
}
