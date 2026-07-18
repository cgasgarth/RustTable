use std::fmt;

use rusttable_core::{
    Asset, AssetId, AssetRole, ByteLength, ContentHash, ImageMetadata, Photo, PhotoBuildError,
    PhotoId, Revision,
};
use rusttable_image::ImageProbe;

use crate::{
    CatalogCommand, CatalogError, CatalogState, ImportRepository, RepositoryError, SourcePath,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportCandidate {
    photo_id: PhotoId,
    asset_id: AssetId,
    source: SourcePath,
    content_hash: ContentHash,
    byte_length: ByteLength,
    probe: ImageProbe,
    metadata: ImageMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportCandidateError {
    ZeroByteLength,
}

impl ImportCandidate {
    /// Creates an already-inspected import candidate with explicit IDs.
    ///
    /// # Errors
    ///
    /// Returns an error when the source length is zero.
    pub fn new(
        photo_id: PhotoId,
        asset_id: AssetId,
        source: SourcePath,
        content_hash: ContentHash,
        byte_length: ByteLength,
        probe: ImageProbe,
        metadata: ImageMetadata,
    ) -> Result<Self, ImportCandidateError> {
        if byte_length == ByteLength::ZERO {
            return Err(ImportCandidateError::ZeroByteLength);
        }
        Ok(Self {
            photo_id,
            asset_id,
            source,
            content_hash,
            byte_length,
            probe,
            metadata,
        })
    }

    #[must_use]
    pub const fn photo_id(&self) -> PhotoId {
        self.photo_id
    }
    #[must_use]
    pub const fn asset_id(&self) -> AssetId {
        self.asset_id
    }
    #[must_use]
    pub fn source(&self) -> &SourcePath {
        &self.source
    }
    #[must_use]
    pub const fn content_hash(&self) -> ContentHash {
        self.content_hash
    }
    #[must_use]
    pub const fn byte_length(&self) -> ByteLength {
        self.byte_length
    }
    #[must_use]
    pub const fn probe(&self) -> ImageProbe {
        self.probe
    }
    #[must_use]
    pub fn metadata(&self) -> &ImageMetadata {
        &self.metadata
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRecord {
    photo: Photo,
    source: SourcePath,
    probe: ImageProbe,
    metadata: ImageMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportRecordError {
    ZeroByteLength,
    PhotoIdMismatch { expected: PhotoId, actual: PhotoId },
    AssetCount { actual: usize },
    PrimaryAssetIdMismatch { expected: AssetId, actual: AssetId },
    ContentHashMismatch,
    ByteLengthMismatch,
    PhotoBuild(PhotoBuildError),
}

impl ImportRecord {
    /// Constructs a record while rechecking the candidate's photo identity.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the photo does not contain exactly one
    /// matching primary asset or its content identity differs.
    pub fn new(candidate: &ImportCandidate, photo: Photo) -> Result<Self, ImportRecordError> {
        if candidate.byte_length == ByteLength::ZERO {
            return Err(ImportRecordError::ZeroByteLength);
        }
        if photo.id() != candidate.photo_id {
            return Err(ImportRecordError::PhotoIdMismatch {
                expected: candidate.photo_id,
                actual: photo.id(),
            });
        }
        let actual_count = photo.assets().count();
        if actual_count != 1 {
            return Err(ImportRecordError::AssetCount {
                actual: actual_count,
            });
        }
        if photo.primary_asset_id() != candidate.asset_id {
            return Err(ImportRecordError::PrimaryAssetIdMismatch {
                expected: candidate.asset_id,
                actual: photo.primary_asset_id(),
            });
        }
        let primary = photo.primary_asset();
        if primary.content_hash() != candidate.content_hash {
            return Err(ImportRecordError::ContentHashMismatch);
        }
        if primary.byte_length() != candidate.byte_length {
            return Err(ImportRecordError::ByteLengthMismatch);
        }
        Ok(Self {
            photo,
            source: candidate.source.clone(),
            probe: candidate.probe,
            metadata: candidate.metadata.clone(),
        })
    }

    #[must_use]
    pub const fn photo(&self) -> &Photo {
        &self.photo
    }
    #[must_use]
    pub fn source(&self) -> &SourcePath {
        &self.source
    }
    #[must_use]
    pub const fn probe(&self) -> ImageProbe {
        self.probe
    }
    #[must_use]
    pub fn metadata(&self) -> &ImageMetadata {
        &self.metadata
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportOutcome {
    Imported {
        record: ImportRecord,
        revision: Revision,
    },
    AlreadyPresent {
        record: ImportRecord,
        revision: Revision,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    Catalog(CatalogError),
    Candidate(ImportCandidateError),
    Record(ImportRecordError),
    Repository(RepositoryError),
    SourceContentChanged {
        source: SourcePath,
    },
    PersistedPhotoMissingFromCatalog {
        source: SourcePath,
        photo_id: PhotoId,
    },
    PersistedPhotoMismatch {
        source: SourcePath,
        photo_id: PhotoId,
    },
}

pub struct ImportService;

impl ImportService {
    /// Registers one checked candidate with optimistic, failure-atomic semantics.
    ///
    /// # Errors
    ///
    /// Returns a typed catalog, repository, consistency, or content-conflict
    /// error without mutating state before commit succeeds.
    pub fn register(
        state: &mut CatalogState,
        expected: Revision,
        candidate: &ImportCandidate,
        repository: &mut dyn ImportRepository,
    ) -> Result<ImportOutcome, ImportError> {
        if expected != state.revision() {
            return Err(ImportError::Catalog(
                CatalogError::CatalogRevisionConflict {
                    expected,
                    actual: state.revision(),
                },
            ));
        }
        if let Some(existing) = repository
            .find_by_source(candidate.source())
            .map_err(ImportError::Repository)?
        {
            let primary = existing.photo().primary_asset();
            if primary.content_hash() != candidate.content_hash()
                || primary.byte_length() != candidate.byte_length()
            {
                return Err(ImportError::SourceContentChanged {
                    source: candidate.source().clone(),
                });
            }
            match state.photo(existing.photo().id()) {
                None => {
                    return Err(ImportError::PersistedPhotoMissingFromCatalog {
                        source: candidate.source().clone(),
                        photo_id: existing.photo().id(),
                    });
                }
                Some(photo) if photo != existing.photo() => {
                    return Err(ImportError::PersistedPhotoMismatch {
                        source: candidate.source().clone(),
                        photo_id: existing.photo().id(),
                    });
                }
                Some(_) => {
                    return Ok(ImportOutcome::AlreadyPresent {
                        record: existing,
                        revision: state.revision(),
                    });
                }
            }
        }
        if repository
            .find_by_photo_id(candidate.photo_id())
            .map_err(ImportError::Repository)?
            .is_some()
        {
            return Err(ImportError::Repository(RepositoryError::PhotoIdConflict {
                photo_id: candidate.photo_id(),
            }));
        }
        if repository
            .find_by_asset_id(candidate.asset_id())
            .map_err(ImportError::Repository)?
            .is_some()
        {
            return Err(ImportError::Repository(RepositoryError::AssetIdConflict {
                asset_id: candidate.asset_id(),
            }));
        }

        let asset = Asset::new(
            candidate.asset_id(),
            AssetRole::Primary,
            candidate.content_hash(),
            candidate.byte_length(),
        );
        let photo = Photo::new(candidate.photo_id(), [asset])
            .map_err(|error| ImportError::Record(ImportRecordError::PhotoBuild(error)))?;
        let mut preflight = state.clone();
        let revision = preflight
            .apply(expected, CatalogCommand::RegisterPhoto(photo.clone()))
            .map_err(ImportError::Catalog)?;
        let record = ImportRecord::new(candidate, photo).map_err(ImportError::Record)?;
        repository
            .commit(&record)
            .map_err(ImportError::Repository)?;
        *state = preflight;
        Ok(ImportOutcome::Imported { record, revision })
    }
}

impl fmt::Display for ImportCandidateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("import candidate byte length must be nonzero")
    }
}
impl std::error::Error for ImportCandidateError {}
impl fmt::Display for ImportRecordError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid import record: {self:?}")
    }
}
impl std::error::Error for ImportRecordError {}
impl fmt::Display for ImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "import failed: {self:?}")
    }
}
impl std::error::Error for ImportError {}
