use std::path::Path;

use rusttable_catalog::{
    EditRepository, EditRepositoryError, ImportRecord, ImportRepository, RepositoryError,
};
use rusttable_core::{Edit, EditId, PhotoId};
use rusttable_import::{
    FileSourceSnapshotReader, ImportSourceLimits, SourceSnapshotError, SourceSnapshotReadError,
    SourceSnapshotReader, decode_reference_source,
};
use rusttable_pixelpipe::CancellationScope;
use rusttable_render::{
    RenderOutput, RenderReceipt, RenderRequestContext, RenderSourceProvenance, RenderTarget,
    SourceColorPolicy,
};

pub(crate) mod smoke;

use crate::composition::services::preview::{PreviewError, PreviewService};

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
        self.render_with_receipt(request, imports, edits)
            .map(CatalogPreviewRender::into_output)
    }

    /// Renders the exact persisted edit and returns the immutable publication receipt.
    ///
    /// The receipt binds the edit revision, imported source evidence, source-color
    /// decision, explicit output transform, and publication generation. Consumers
    /// may downsample the returned output, but must not substitute an embedded preview.
    ///
    /// # Errors
    ///
    /// Returns a typed catalog, snapshot, decode, or render error.
    pub fn render_with_receipt(
        &self,
        request: CatalogPreviewRequest<'_>,
        imports: &dyn ImportRepository,
        edits: &dyn EditRepository,
    ) -> Result<CatalogPreviewRender, CatalogPreviewError> {
        let record = imports
            .find_by_photo_id(request.photo_id)
            .map_err(|error| {
                tracing::error!(target: "rusttable.preview", stage = "catalog_lookup", cause = "import_repository");
                CatalogPreviewError::ImportRepository(error)
            })?
            .ok_or_else(|| {
                tracing::error!(target: "rusttable.preview", stage = "catalog_lookup", cause = "unknown_photo");
                CatalogPreviewError::UnknownPhoto {
                photo_id: request.photo_id,
                }
            })?;
        let edit = edits
            .find_by_edit_id(request.edit_id)
            .map_err(|error| {
                tracing::error!(target: "rusttable.preview", stage = "edit_resolution", cause = "edit_repository");
                CatalogPreviewError::EditRepository(error)
            })?
            .ok_or_else(|| {
                tracing::error!(target: "rusttable.preview", stage = "edit_resolution", cause = "unknown_edit");
                CatalogPreviewError::UnknownEdit {
                edit_id: request.edit_id,
                }
            })?;
        self.render_record_with_receipt(request, &record, &edit)
    }

    /// Renders a persisted preview while propagating generation cancellation
    /// into the production pixelpipe execution.
    ///
    /// # Errors
    ///
    /// Returns a typed catalog, source, cancellation, decode, or render error.
    pub(crate) fn render_with_receipt_and_cancellation(
        &self,
        request: CatalogPreviewRequest<'_>,
        imports: &dyn ImportRepository,
        edits: &dyn EditRepository,
        cancellation: &CancellationScope,
    ) -> Result<CatalogPreviewRender, CatalogPreviewError> {
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
        self.render_snapshot_with_receipt(request, &record, &edit, |preview, bytes, edit| {
            preview.render_bytes_with_cancellation(bytes, edit, cancellation)
        })
    }

    /// Renders a caller-provided edit without reading or writing an edit record.
    ///
    /// The edit is still checked against the persisted photo record before its
    /// source is read. This keeps transient drafts on the same bounded,
    /// snapshot-based source path as persisted edits while preventing an edit
    /// for another photo from being rendered.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when photo lookup, ownership validation, source
    /// resolution, decoding, or CPU rendering fails.
    pub fn render_edit(
        &self,
        source_root: &Path,
        edit: &Edit,
        imports: &dyn ImportRepository,
    ) -> Result<RenderOutput, CatalogPreviewError> {
        let record = imports
            .find_by_photo_id(edit.photo_id())
            .map_err(|error| {
                tracing::error!(target: "rusttable.preview", stage = "catalog_lookup", cause = "import_repository");
                CatalogPreviewError::ImportRepository(error)
            })?
            .ok_or_else(|| {
                tracing::error!(target: "rusttable.preview", stage = "catalog_lookup", cause = "unknown_photo");
                CatalogPreviewError::UnknownPhoto {
                photo_id: edit.photo_id(),
                }
            })?;
        self.render_record(source_root, &record, edit)
    }

    /// Renders a transient edit with the same source and output receipt as a persisted edit.
    ///
    /// # Errors
    ///
    /// Returns a typed catalog, snapshot, decode, or render error.
    pub fn render_edit_with_receipt(
        &self,
        source_root: &Path,
        edit: &Edit,
        imports: &dyn ImportRepository,
        generation: u64,
    ) -> Result<CatalogPreviewRender, CatalogPreviewError> {
        let record = imports
            .find_by_photo_id(edit.photo_id())
            .map_err(CatalogPreviewError::ImportRepository)?
            .ok_or(CatalogPreviewError::UnknownPhoto {
                photo_id: edit.photo_id(),
            })?;
        self.render_record_with_receipt(
            CatalogPreviewRequest::new(source_root, edit.photo_id(), edit.id())
                .with_generation(generation),
            &record,
            edit,
        )
    }

    /// Renders the exact persisted edit at source resolution for publication.
    ///
    /// The source is read through the same immutable snapshot and revalidation
    /// boundary as preview rendering. Only the render target differs, so PNG
    /// publication cannot accidentally export the display-sized preview.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when catalog lookup, edit lookup, ownership
    /// validation, source resolution, decoding, or CPU rendering fails.
    pub fn render_full_resolution(
        &self,
        request: CatalogPreviewRequest<'_>,
        imports: &dyn ImportRepository,
        edits: &dyn EditRepository,
    ) -> Result<RenderOutput, CatalogPreviewError> {
        self.render_for_target(request, imports, edits, RenderTarget::FullResolution)
    }

    /// Renders the exact persisted edit through an explicit production target.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when catalog lookup, edit lookup, ownership
    /// validation, source resolution, decoding, or CPU rendering fails.
    pub fn render_for_target(
        &self,
        request: CatalogPreviewRequest<'_>,
        imports: &dyn ImportRepository,
        edits: &dyn EditRepository,
        target: RenderTarget,
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
        self.render_record_for_target(request.source_root, &record, &edit, target)
    }

    fn render_record(
        &self,
        source_root: &Path,
        record: &ImportRecord,
        edit: &Edit,
    ) -> Result<RenderOutput, CatalogPreviewError> {
        self.render_snapshot(source_root, record, edit, |preview, bytes, edit| {
            preview.render_bytes(bytes, edit)
        })
    }

    fn render_record_with_receipt(
        &self,
        request: CatalogPreviewRequest<'_>,
        record: &ImportRecord,
        edit: &Edit,
    ) -> Result<CatalogPreviewRender, CatalogPreviewError> {
        self.render_snapshot_with_receipt(request, record, edit, |preview, bytes, edit| {
            preview.render_bytes(bytes, edit)
        })
    }

    fn render_record_for_target(
        &self,
        source_root: &Path,
        record: &ImportRecord,
        edit: &Edit,
        target: RenderTarget,
    ) -> Result<RenderOutput, CatalogPreviewError> {
        self.render_snapshot(source_root, record, edit, |preview, bytes, edit| {
            preview.render_bytes_for_target(bytes, edit, target)
        })
    }

    fn render_snapshot(
        &self,
        source_root: &Path,
        record: &ImportRecord,
        edit: &Edit,
        render: impl FnOnce(&PreviewService, &[u8], &Edit) -> Result<RenderOutput, PreviewError>,
    ) -> Result<RenderOutput, CatalogPreviewError> {
        self.render_snapshot_with_receipt(
            CatalogPreviewRequest::new(source_root, edit.photo_id(), edit.id()),
            record,
            edit,
            render,
        )
        .map(CatalogPreviewRender::into_output)
    }

    fn render_snapshot_with_receipt(
        &self,
        request: CatalogPreviewRequest<'_>,
        record: &ImportRecord,
        edit: &Edit,
        render: impl FnOnce(&PreviewService, &[u8], &Edit) -> Result<RenderOutput, PreviewError>,
    ) -> Result<CatalogPreviewRender, CatalogPreviewError> {
        validate_edit_ownership(record, edit)?;
        let source = decode_reference_source(record.source())
            .unwrap_or_else(|_| request.source_root.join(record.source().as_str()));
        let limits = ImportSourceLimits::new(64 * 1024 * 1024)
            .map_err(|_| CatalogPreviewError::SourceLimits)?;
        let snapshot_reader = FileSourceSnapshotReader;
        let snapshot = snapshot_reader
            .read_snapshot(&source, limits)
            .map_err(CatalogPreviewError::Snapshot)?;
        let bytes = snapshot
            .materialize(limits)
            .map_err(CatalogPreviewError::SnapshotRead)?;
        let output = render(&self.preview, &bytes, edit).map_err(CatalogPreviewError::Preview)?;
        snapshot_reader
            .revalidate(&snapshot, limits)
            .map_err(CatalogPreviewError::Snapshot)?;
        let asset = record.photo().primary_asset();
        let source_provenance = RenderSourceProvenance::new(
            record.photo().id(),
            asset.id(),
            asset.content_hash(),
            asset.byte_length(),
            record.probe(),
        );
        let context = RenderRequestContext::new(
            source_provenance,
            edit,
            SourceColorPolicy::AssumeSrgbWhenUnspecified,
            output.plan(),
        );
        let render_receipt = RenderReceipt::new(context, &output);
        Ok(CatalogPreviewRender {
            output,
            receipt: CatalogPreviewReceipt {
                render: render_receipt,
                output_transform: PreviewOutputTransform::SrgbDisplayFallback,
                generation: request.generation,
            },
        })
    }
}

/// The display output encoding used by the current application composition.
///
/// Monitor ICC conversion remains owned by the display-profile workstream. The
/// nested render receipt identifies whether this is colorimetric sRGB or the
/// versioned scene-referred RAW fallback shared by preview, filmstrip, and export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewOutputTransform {
    SrgbDisplayFallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogPreviewReceipt {
    render: RenderReceipt,
    output_transform: PreviewOutputTransform,
    generation: u64,
}

impl CatalogPreviewReceipt {
    #[must_use]
    pub const fn render(&self) -> &RenderReceipt {
        &self.render
    }

    #[must_use]
    pub const fn output_transform(&self) -> PreviewOutputTransform {
        self.output_transform
    }

    /// Returns the exact edit identity used by the completed render.
    #[must_use]
    pub const fn edit_id(&self) -> EditId {
        self.render.context().edit().source_edit_id()
    }

    /// Returns the revision used by the completed render.
    #[must_use]
    pub const fn edit_revision(&self) -> rusttable_core::Revision {
        self.render.context().edit().edit_revision()
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub fn identity_hash(&self) -> [u8; 32] {
        self.render.identity_hash()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogPreviewRender {
    output: RenderOutput,
    receipt: CatalogPreviewReceipt,
}

impl CatalogPreviewRender {
    #[must_use]
    pub const fn output(&self) -> &RenderOutput {
        &self.output
    }

    #[must_use]
    pub const fn receipt(&self) -> &CatalogPreviewReceipt {
        &self.receipt
    }

    #[must_use]
    pub fn into_parts(self) -> (RenderOutput, CatalogPreviewReceipt) {
        (self.output, self.receipt)
    }

    fn into_output(self) -> RenderOutput {
        self.output
    }
}

fn validate_edit_ownership(record: &ImportRecord, edit: &Edit) -> Result<(), CatalogPreviewError> {
    if edit.photo_id() != record.photo().id() {
        return Err(CatalogPreviewError::EditPhotoMismatch {
            edit_id: edit.id(),
            expected_photo_id: record.photo().id(),
            actual_photo_id: edit.photo_id(),
        });
    }
    Ok(())
}

/// Identifies the persisted values and local source root for one preview.
#[derive(Debug, Clone, Copy)]
pub struct CatalogPreviewRequest<'a> {
    source_root: &'a Path,
    photo_id: PhotoId,
    edit_id: EditId,
    generation: u64,
}

impl<'a> CatalogPreviewRequest<'a> {
    #[must_use]
    pub const fn new(source_root: &'a Path, photo_id: PhotoId, edit_id: EditId) -> Self {
        Self {
            source_root,
            photo_id,
            edit_id,
            generation: 0,
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

    #[must_use]
    pub const fn with_generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }

    #[must_use]
    pub const fn generation(self) -> u64 {
        self.generation
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
    SnapshotRead(SourceSnapshotReadError),
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
            Self::SnapshotRead(error) => {
                write!(formatter, "catalog preview source read failed: {error}")
            }
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
            Self::SnapshotRead(error) => Some(error),
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

    #[test]
    fn renders_a_caller_provided_edit_without_persisted_edit_lookup() {
        let photo_id = PhotoId::new(1).unwrap();
        let edit = edit(9, 1);
        let imports = Imports {
            records: BTreeMap::from([(
                photo_id,
                record(1, "fixtures/corpus/assets/raster-png-16-alpha.png"),
            )]),
        };
        let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let service = CatalogPreviewService::new(PreviewService::new(
            DecodeLimits::new(64 * 1024 * 1024, 4096, 4096, 16_777_216, 64 * 1024 * 1024).unwrap(),
            PreviewBounds::new(1, 1).unwrap(),
        ));

        let output = service
            .render_edit(&source_root, &edit, &imports)
            .expect("caller-provided edit preview renders");

        assert_eq!(output.provenance().source_photo_id(), photo_id);
        assert_eq!(output.provenance().source_edit_id(), edit.id());
    }

    #[test]
    fn full_resolution_render_preserves_the_source_dimensions() {
        let photo_id = PhotoId::new(1).unwrap();
        let persisted = edit(9, 1);
        let imports = Imports {
            records: BTreeMap::from([(
                photo_id,
                record(1, "fixtures/corpus/assets/raster-png-16-alpha.png"),
            )]),
        };
        let edits = Edits {
            edits: BTreeMap::from([(persisted.id(), persisted)]),
        };
        let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let service = CatalogPreviewService::new(PreviewService::new(
            DecodeLimits::new(64 * 1024 * 1024, 4096, 4096, 16_777_216, 64 * 1024 * 1024).unwrap(),
            PreviewBounds::new(1, 1).unwrap(),
        ));

        let output = service
            .render_full_resolution(
                CatalogPreviewRequest::new(&source_root, photo_id, EditId::new(9).unwrap()),
                &imports,
                &edits,
            )
            .expect("full-resolution catalog render succeeds");

        assert_eq!(
            output.image().dimensions(),
            ImageDimensions::new(4, 3).unwrap()
        );
    }

    #[test]
    fn rejects_a_caller_provided_edit_owned_by_another_photo_before_source_decode() {
        let photo_id = PhotoId::new(2).unwrap();
        let imports = Imports {
            records: BTreeMap::from([(photo_id, record(1, "missing-source.png"))]),
        };
        let edit = edit(9, photo_id.get());
        let service = CatalogPreviewService::new(PreviewService::new(
            DecodeLimits::new(1, 1, 1, 1, 1).unwrap(),
            PreviewBounds::new(1, 1).unwrap(),
        ));

        assert!(matches!(
            service.render_edit(Path::new("missing-source-root"), &edit, &imports,),
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
