use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use rusttable_core::{Asset, AssetId, Edit, EditId, ImageMetadata, Photo, PhotoId, Revision};
use rusttable_image::ImageProbe;

use crate::{ImportRecord, ImportRepository, RepositoryError, SourcePath};

/// One owned catalog item joined from import provenance and current catalog state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    record: ImportRecord,
    edits: Vec<Edit>,
}

impl CatalogEntry {
    #[must_use]
    pub fn source(&self) -> &SourcePath {
        self.record.source()
    }

    #[must_use]
    pub fn photo(&self) -> &Photo {
        self.record.photo()
    }

    #[must_use]
    pub fn primary_asset(&self) -> &Asset {
        self.record.photo().primary_asset()
    }

    #[must_use]
    pub const fn probe(&self) -> ImageProbe {
        self.record.probe()
    }

    #[must_use]
    pub fn metadata(&self) -> &ImageMetadata {
        self.record.metadata()
    }

    pub fn edits(&self) -> impl Iterator<Item = &Edit> {
        self.edits.iter()
    }

    pub(crate) fn clone_record(&self) -> ImportRecord {
        self.record.clone()
    }
}

/// A deterministic, owned, read-only projection of catalog state and imports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogSnapshot {
    revision: Revision,
    entries: Vec<CatalogEntry>,
    by_source: BTreeMap<SourcePath, usize>,
    by_photo_id: BTreeMap<PhotoId, usize>,
    by_edit_id: BTreeMap<EditId, usize>,
}

impl CatalogSnapshot {
    /// Loads all import records once and joins them with one immutable state revision.
    ///
    /// # Errors
    ///
    /// Returns a typed repository or cross-boundary consistency error. No partial
    /// snapshot is returned when validation fails.
    pub fn load(
        state: &crate::CatalogState,
        repository: &dyn ImportRepository,
    ) -> Result<Self, CatalogSnapshotError> {
        let mut records = repository
            .list()
            .map_err(CatalogSnapshotError::Repository)?;
        records.sort_by(|left, right| left.source().cmp(right.source()));

        validate_unique_sources(&records)?;
        validate_unique_photos(&records)?;
        validate_unique_assets(&records)?;
        validate_persisted_photos(state, &records)?;
        validate_state_photos(state, &records)?;

        let mut entries = Vec::with_capacity(records.len());
        let mut by_source = BTreeMap::new();
        let mut by_photo_id = BTreeMap::new();
        let mut by_edit_id = BTreeMap::new();
        for record in records {
            let index = entries.len();
            let photo_id = record.photo().id();
            let edits: Vec<Edit> = state
                .edits()
                .filter(|edit| edit.photo_id() == photo_id)
                .cloned()
                .collect();
            for edit in &edits {
                by_edit_id.insert(edit.id(), index);
            }
            by_source.insert(record.source().clone(), index);
            by_photo_id.insert(photo_id, index);
            entries.push(CatalogEntry { record, edits });
        }

        Ok(Self {
            revision: state.revision(),
            entries,
            by_source,
            by_photo_id,
            by_edit_id,
        })
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    pub fn entries(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.entries.iter()
    }

    #[must_use]
    pub fn by_source(&self, source: &SourcePath) -> Option<&CatalogEntry> {
        self.by_source
            .get(source)
            .and_then(|index| self.entries.get(*index))
    }

    #[must_use]
    pub fn by_photo_id(&self, photo_id: PhotoId) -> Option<&CatalogEntry> {
        self.by_photo_id
            .get(&photo_id)
            .and_then(|index| self.entries.get(*index))
    }

    pub(crate) fn edit_by_id(&self, edit_id: EditId) -> Option<&Edit> {
        let entry_index = self.by_edit_id.get(&edit_id)?;
        self.entries
            .get(*entry_index)
            .and_then(|entry| entry.edits.iter().find(|edit| edit.id() == edit_id))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogSnapshotError {
    Repository(RepositoryError),
    DuplicateSource {
        source: SourcePath,
    },
    DuplicatePhotoId {
        photo_id: PhotoId,
    },
    DuplicateAssetId {
        asset_id: AssetId,
    },
    PersistedPhotoMissingFromState {
        source: SourcePath,
        photo_id: PhotoId,
    },
    PersistedPhotoMismatch {
        source: SourcePath,
        photo_id: PhotoId,
    },
    StatePhotoMissingFromRepository {
        photo_id: PhotoId,
    },
}

impl fmt::Display for CatalogSnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Repository(source) => {
                write!(formatter, "catalog snapshot repository failure: {source}")
            }
            Self::DuplicateSource { source } => {
                write!(formatter, "duplicate import source {source}")
            }
            Self::DuplicatePhotoId { photo_id } => {
                write!(formatter, "duplicate imported photo ID {photo_id}")
            }
            Self::DuplicateAssetId { asset_id } => {
                write!(formatter, "duplicate imported primary asset ID {asset_id}")
            }
            Self::PersistedPhotoMissingFromState { source, photo_id } => write!(
                formatter,
                "persisted source {source} references photo {photo_id}, which is absent from catalog state",
            ),
            Self::PersistedPhotoMismatch { source, photo_id } => write!(
                formatter,
                "persisted source {source} photo {photo_id} differs from catalog state",
            ),
            Self::StatePhotoMissingFromRepository { photo_id } => write!(
                formatter,
                "catalog state photo {photo_id} has no persisted import record",
            ),
        }
    }
}

impl std::error::Error for CatalogSnapshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Repository(source) => Some(source),
            _ => None,
        }
    }
}

fn validate_unique_sources(records: &[ImportRecord]) -> Result<(), CatalogSnapshotError> {
    let mut sources = BTreeSet::new();
    for record in records {
        if !sources.insert(record.source().clone()) {
            return Err(CatalogSnapshotError::DuplicateSource {
                source: record.source().clone(),
            });
        }
    }
    Ok(())
}

fn validate_unique_photos(records: &[ImportRecord]) -> Result<(), CatalogSnapshotError> {
    let mut photo_ids = BTreeSet::new();
    for record in records {
        if !photo_ids.insert(record.photo().id()) {
            return Err(CatalogSnapshotError::DuplicatePhotoId {
                photo_id: record.photo().id(),
            });
        }
    }
    Ok(())
}

fn validate_unique_assets(records: &[ImportRecord]) -> Result<(), CatalogSnapshotError> {
    let mut asset_ids = BTreeSet::new();
    for record in records {
        if !asset_ids.insert(record.photo().primary_asset_id()) {
            return Err(CatalogSnapshotError::DuplicateAssetId {
                asset_id: record.photo().primary_asset_id(),
            });
        }
    }
    Ok(())
}

fn validate_persisted_photos(
    state: &crate::CatalogState,
    records: &[ImportRecord],
) -> Result<(), CatalogSnapshotError> {
    for record in records {
        let photo_id = record.photo().id();
        let Some(photo) = state.photo(photo_id) else {
            return Err(CatalogSnapshotError::PersistedPhotoMissingFromState {
                source: record.source().clone(),
                photo_id,
            });
        };
        if photo != record.photo() {
            return Err(CatalogSnapshotError::PersistedPhotoMismatch {
                source: record.source().clone(),
                photo_id,
            });
        }
    }
    Ok(())
}

fn validate_state_photos(
    state: &crate::CatalogState,
    records: &[ImportRecord],
) -> Result<(), CatalogSnapshotError> {
    let persisted_photo_ids = records
        .iter()
        .map(|record| record.photo().id())
        .collect::<BTreeSet<_>>();
    for photo in state.photos() {
        if !persisted_photo_ids.contains(&photo.id()) {
            return Err(CatalogSnapshotError::StatePhotoMissingFromRepository {
                photo_id: photo.id(),
            });
        }
    }
    Ok(())
}
