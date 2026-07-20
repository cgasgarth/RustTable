use std::path::Path;

use rusttable_catalog::{EditRepository, EditRepositoryError, ImportRepository, RepositoryError};
use rusttable_core::{EditId, PhotoId};
use rusttable_import::{
    FileSourceSnapshotReader, ImportSourceLimits, SourceSnapshotError, SourceSnapshotReader,
    decode_reference_source,
};
use rusttable_render::RenderOutput;

use crate::{PreviewError, PreviewService};

/// Resolves one persisted photo and edit into a bounded CPU preview.
///
/// The catalog records portable logical source keys; callers provide the
/// source root that owns those keys in the current application composition.
#[derive(Debug, Clone, Copy)]
pub struct CatalogPreviewService {
    preview: PreviewService,
}

impl CatalogPreviewService {
    #[must_use]
    pub const fn new(preview: PreviewService) -> Self {
        Self { preview }
    }

    /// Renders the exact persisted edit for one persisted photo.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when catalog lookup, edit lookup, ownership
    /// validation, source resolution, decoding, or CPU rendering fails.
    pub fn render(
        &self,
        request: CatalogPreviewRequest<'_>,
        imports: &dyn ImportRepository,
        edits: &dyn EditRepository,
    ) -> Result<RenderOutput, CatalogPreviewError> {
        let record = imports
            .find_by_photo_id(request.photo_id)
            .map_err(CatalogPreviewError::ImportRepository)?
            .ok_or(CatalogPreviewError::UnknownPhoto {
                photo_id: request.photo_id,
            })?;
        let edit = edits
            .find_by_edit_id(request.edit_id)
            .map_err(CatalogPreviewError::EditRepository)?
            .ok_or(CatalogPreviewError::UnknownEdit {
                edit_id: request.edit_id,
            })?;
        if edit.photo_id() != record.photo().id() {
            return Err(CatalogPreviewError::EditPhotoMismatch {
                edit_id: edit.id(),
                expected_photo_id: record.photo().id(),
                actual_photo_id: edit.photo_id(),
            });
        }
        let source = decode_reference_source(record.source())
            .unwrap_or_else(|_| request.source_root.join(record.source().as_str()));
        let limits = ImportSourceLimits::new(64 * 1024 * 1024)
            .map_err(|_| CatalogPreviewError::SourceLimits)?;
        let reader = FileSourceSnapshotReader;
        let snapshot = reader
            .read_snapshot(&source, limits)
            .map_err(CatalogPreviewError::Snapshot)?;
        let output = self
            .preview
            .render_bytes(snapshot.bytes(), &edit)
            .map_err(CatalogPreviewError::Preview)?;
        reader
            .revalidate(&snapshot, limits)
            .map_err(CatalogPreviewError::Snapshot)?;
        Ok(output)
    }
}

/// Identifies the persisted values and local source root for one preview.
#[derive(Debug, Clone, Copy)]
pub struct CatalogPreviewRequest<'a> {
    source_root: &'a Path,
    photo_id: PhotoId,
    edit_id: EditId,
}

impl<'a> CatalogPreviewRequest<'a> {
    #[must_use]
    pub const fn new(source_root: &'a Path, photo_id: PhotoId, edit_id: EditId) -> Self {
        Self {
            source_root,
            photo_id,
            edit_id,
        }
    }

    #[must_use]
    pub const fn source_root(self) -> &'a Path {
        self.source_root
    }

    #[must_use]
    pub const fn photo_id(self) -> PhotoId {
        self.photo_id
    }

    #[must_use]
    pub const fn edit_id(self) -> EditId {
        self.edit_id
    }
}

#[derive(Debug)]
pub enum CatalogPreviewError {
    ImportRepository(RepositoryError),
    EditRepository(EditRepositoryError),
    UnknownPhoto {
        photo_id: PhotoId,
    },
    UnknownEdit {
        edit_id: EditId,
    },
    EditPhotoMismatch {
        edit_id: EditId,
        expected_photo_id: PhotoId,
        actual_photo_id: PhotoId,
    },
    Preview(PreviewError),
    Snapshot(SourceSnapshotError),
    SourceLimits,
}

impl std::fmt::Display for CatalogPreviewError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ImportRepository(error) => {
                write!(formatter, "catalog import lookup failed: {error}")
            }
            Self::EditRepository(error) => write!(formatter, "catalog edit lookup failed: {error}"),
            Self::UnknownPhoto { photo_id } => {
                write!(formatter, "catalog photo {photo_id} is unknown")
            }
            Self::UnknownEdit { edit_id } => write!(formatter, "catalog edit {edit_id} is unknown"),
            Self::EditPhotoMismatch {
                edit_id,
                expected_photo_id,
                actual_photo_id,
            } => write!(
                formatter,
                "edit {edit_id} belongs to photo {actual_photo_id}, not {expected_photo_id}"
            ),
            Self::Preview(error) => write!(formatter, "catalog preview failed: {error}"),
            Self::Snapshot(error) => write!(formatter, "catalog preview source failed: {error}"),
            Self::SourceLimits => formatter.write_str("catalog preview source limits are invalid"),
        }
    }
}

impl std::error::Error for CatalogPreviewError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ImportRepository(error) => Some(error),
            Self::EditRepository(error) => Some(error),
            Self::Preview(error) => Some(error),
            Self::Snapshot(error) => Some(error),
            Self::SourceLimits
            | Self::UnknownPhoto { .. }
            | Self::UnknownEdit { .. }
            | Self::EditPhotoMismatch { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use rusttable_catalog::{
        EditRepository, EditRepositoryError, ImportCandidate, ImportRecord, ImportRepository,
        RepositoryError, SourcePath,
    };
    use rusttable_core::{
        Asset, AssetId, AssetRole, ByteLength, ContentHash, Edit, EditId, FiniteF64, ImageMetadata,
        Operation, OperationId, OperationKey, ParameterName, ParameterValue, Photo, PhotoId,
        Revision,
    };
    use rusttable_image::{DecodeLimits, ImageDimensions, ImageProbe, InputFormat};
    use rusttable_render::PreviewBounds;

    use super::{CatalogPreviewError, CatalogPreviewRequest, CatalogPreviewService};
    use crate::PreviewService;

    #[test]
    fn rejects_an_edit_owned_by_another_photo_before_source_decode() {
        let imports = Imports {
            records: BTreeMap::from([(PhotoId::new(1).unwrap(), record(1, "fixture.png"))]),
        };
        let edits = Edits {
            edits: BTreeMap::from([(EditId::new(9).unwrap(), edit(9, 2))]),
        };
        let service = CatalogPreviewService::new(PreviewService::new(
            DecodeLimits::new(1, 1, 1, 1, 1).unwrap(),
            PreviewBounds::new(1, 1).unwrap(),
        ));

        assert!(matches!(
            service.render(
                CatalogPreviewRequest::new(
                    Path::new("missing-source-root"),
                    PhotoId::new(1).unwrap(),
                    EditId::new(9).unwrap(),
                ),
                &imports,
                &edits,
            ),
            Err(CatalogPreviewError::EditPhotoMismatch { .. })
        ));
    }

    struct Imports {
        records: BTreeMap<PhotoId, ImportRecord>,
    }

    impl ImportRepository for Imports {
        fn find_by_source(
            &self,
            source: &SourcePath,
        ) -> Result<Option<ImportRecord>, RepositoryError> {
            Ok(self
                .records
                .values()
                .find(|record| record.source() == source)
                .cloned())
        }

        fn find_by_photo_id(
            &self,
            photo_id: PhotoId,
        ) -> Result<Option<ImportRecord>, RepositoryError> {
            Ok(self.records.get(&photo_id).cloned())
        }

        fn find_by_asset_id(
            &self,
            asset_id: AssetId,
        ) -> Result<Option<ImportRecord>, RepositoryError> {
            Ok(self
                .records
                .values()
                .find(|record| record.photo().primary_asset_id() == asset_id)
                .cloned())
        }

        fn commit(&mut self, _record: &ImportRecord) -> Result<(), RepositoryError> {
            unreachable!("lookup test does not commit")
        }

        fn list(&self) -> Result<Vec<ImportRecord>, RepositoryError> {
            Ok(self.records.values().cloned().collect())
        }
    }

    struct Edits {
        edits: BTreeMap<EditId, Edit>,
    }

    impl EditRepository for Edits {
        fn find_by_edit_id(&self, edit_id: EditId) -> Result<Option<Edit>, EditRepositoryError> {
            Ok(self.edits.get(&edit_id).cloned())
        }

        fn list(&self) -> Result<Vec<Edit>, EditRepositoryError> {
            Ok(self.edits.values().cloned().collect())
        }

        fn commit_new(&mut self, _edit: &Edit) -> Result<(), EditRepositoryError> {
            unreachable!("lookup test does not commit")
        }

        fn commit_replacement(
            &mut self,
            _expected_edit_revision: Revision,
            _edit: &Edit,
        ) -> Result<(), EditRepositoryError> {
            unreachable!("lookup test does not commit")
        }
    }

    fn record(photo_id: u128, source: &str) -> ImportRecord {
        let photo_id = PhotoId::new(photo_id).unwrap();
        let asset_id = AssetId::new(photo_id.get() + 100).unwrap();
        let candidate = ImportCandidate::new(
            photo_id,
            asset_id,
            SourcePath::new(source).unwrap(),
            ContentHash::Sha256([1; 32]),
            ByteLength::from_bytes(1),
            ImageProbe::new(InputFormat::Png, ImageDimensions::new(1, 1).unwrap()),
            ImageMetadata::empty(),
        )
        .unwrap();
        let photo = Photo::new(
            photo_id,
            [Asset::new(
                asset_id,
                AssetRole::Primary,
                ContentHash::Sha256([1; 32]),
                ByteLength::from_bytes(1),
            )],
        )
        .unwrap();
        ImportRecord::new(&candidate, photo).unwrap()
    }

    fn edit(edit_id: u128, photo_id: u128) -> Edit {
        Edit::new(
            EditId::new(edit_id).unwrap(),
            PhotoId::new(photo_id).unwrap(),
            Revision::ZERO,
            [Operation::new(
                OperationId::new(1).unwrap(),
                OperationKey::new("rusttable.exposure").unwrap(),
                true,
                [(
                    ParameterName::new("stops").unwrap(),
                    ParameterValue::Scalar(FiniteF64::new(0.5).unwrap()),
                )],
            )
            .unwrap()],
        )
        .unwrap()
    }
}
