use rusttable_catalog::{
    CatalogState, ImportCandidate, ImportError, ImportOutcome, ImportRepository, ImportService,
};
use rusttable_core::{ByteLength, ContentHash, Revision};
use rusttable_image::{ImageInput, ImageInputError};
use rusttable_metadata::{MetadataInput, MetadataInputError};
use sha2::{Digest, Sha256};

use crate::{
    ImportSourceLimits, SourceImportRequest, SourceSnapshotError, SourceSnapshotReadError,
    SourceSnapshotReader,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceImportError {
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    Snapshot(SourceSnapshotError),
    SnapshotRead(SourceSnapshotReadError),
    Image(ImageInputError),
    Metadata(MetadataInputError),
    Candidate(rusttable_catalog::ImportCandidateError),
    Import(ImportError),
}

impl std::fmt::Display for SourceImportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "source import failed: {self:?}")
    }
}

impl std::error::Error for SourceImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Snapshot(error) => Some(error),
            Self::SnapshotRead(error) => Some(error),
            Self::Image(error) => Some(error),
            Self::Metadata(error) => Some(error),
            Self::Candidate(error) => Some(error),
            Self::Import(error) => Some(error),
            Self::StaleRevision { .. } => None,
        }
    }
}

pub struct SourceImportService;

impl SourceImportService {
    // The ports stay explicit so the service cannot hide or retain adapter state.
    #[expect(
        clippy::too_many_arguments,
        reason = "issue #86 requires explicit state, request, limits, repository, and three adapter ports"
    )]
    /// Inspects a source and registers its immutable catalog record.
    ///
    /// # Errors
    ///
    /// Returns a typed source, image, metadata, candidate, catalog, or
    /// repository failure without committing a partial import.
    pub fn inspect_and_register(
        state: &mut CatalogState,
        expected_revision: Revision,
        request: &SourceImportRequest,
        limits: ImportSourceLimits,
        repository: &mut dyn ImportRepository,
        snapshot_reader: &dyn SourceSnapshotReader,
        image_input: &dyn ImageInput,
        metadata_input: &dyn MetadataInput,
    ) -> Result<ImportOutcome, SourceImportError> {
        let _span = tracing::info_span!(
            target: "rusttable.import",
            "inspect_and_register",
            photo_id = request.photo_id().get(),
            operation = "source_import"
        )
        .entered();
        if expected_revision != state.revision() {
            tracing::warn!(target: "rusttable.import", stage = "revision", cause = "stale_revision");
            return Err(SourceImportError::StaleRevision {
                expected: expected_revision,
                actual: state.revision(),
            });
        }
        let snapshot = snapshot_reader
            .read_snapshot(request.physical_path(), limits)
            .map_err(|error| {
                tracing::warn!(target: "rusttable.import", stage = "snapshot", cause = "read_failed");
                SourceImportError::Snapshot(error)
            })?;
        let bytes = snapshot
            .materialize(limits)
            .map_err(|error| {
                tracing::warn!(target: "rusttable.import", stage = "snapshot_read", cause = "materialize_failed");
                SourceImportError::SnapshotRead(error)
            })?;
        let probe = image_input.probe_bytes(&bytes).map_err(|error| {
            tracing::warn!(target: "rusttable.import", stage = "decode", cause = "probe_failed");
            SourceImportError::Image(error)
        })?;
        let metadata = metadata_input
            .read_bytes(probe.format(), &bytes)
            .map_err(|error| {
                tracing::warn!(target: "rusttable.import", stage = "metadata", cause = "read_failed");
                SourceImportError::Metadata(error)
            })?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let hash: [u8; 32] = hasher.finalize().into();
        let candidate = ImportCandidate::new(
            request.photo_id(),
            request.asset_id(),
            request.source().clone(),
            ContentHash::Sha256(hash),
            ByteLength::from_bytes(snapshot.byte_length().get()),
            probe,
            metadata,
        )
        .map_err(|error| {
            tracing::warn!(target: "rusttable.import", stage = "candidate", cause = "validation_failed");
            SourceImportError::Candidate(error)
        })?;
        ImportService::register(state, expected_revision, &candidate, repository)
            .map_err(|error| {
                tracing::warn!(target: "rusttable.import", stage = "register", cause = "catalog_failed");
                SourceImportError::Import(error)
            })
    }
}
