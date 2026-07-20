use std::path::Path;

use rusttable_catalog::{EditRepository, RepositoryError};
use rusttable_catalog_store::RedbCatalogRepository;
use rusttable_core::{Edit, PhotoId};
use rusttable_image::{DecodeLimits, ImageDimensions};
use rusttable_render::{PreviewBounds, RenderTarget};

use crate::{CatalogPreviewError, CatalogPreviewRequest, CatalogPreviewService, PreviewService};

const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DECODE_DIMENSION: u32 = 16_384;
const MAX_DECODE_PIXELS: u64 = 64 * 1024 * 1024;
const MAX_DECODE_BYTES: u64 = 256 * 1024 * 1024;
const PREVIEW_EDGE: u32 = 1_536;

/// Immutable RGBA8 pixels ready for a presentation adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedPreview {
    photo_id: PhotoId,
    dimensions: ImageDimensions,
    pixels: Vec<u8>,
}

impl SelectedPreview {
    #[must_use]
    pub fn into_parts(self) -> (PhotoId, ImageDimensions, Vec<u8>) {
        (self.photo_id, self.dimensions, self.pixels)
    }
}

#[derive(Debug)]
pub enum WorkspacePreviewError {
    Catalog(RepositoryError),
    MissingEdit { photo_id: PhotoId },
    Preview(CatalogPreviewError),
}

impl std::fmt::Display for WorkspacePreviewError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Catalog(error) => write!(formatter, "catalog preview lookup failed: {error}"),
            Self::MissingEdit { photo_id } => {
                write!(formatter, "photo {photo_id} has no persisted edit")
            }
            Self::Preview(error) => write!(formatter, "selected preview failed: {error}"),
        }
    }
}

impl std::error::Error for WorkspacePreviewError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Catalog(error) => Some(error),
            Self::Preview(error) => Some(error),
            Self::MissingEdit { .. } => None,
        }
    }
}

/// Resolves and renders the current persisted edit for one selected catalog photo.
///
/// The catalog currently owns one immutable edit lineage per photo. A higher
/// revision wins; an equal-revision tie is resolved by edit ID so restart and
/// task scheduling cannot change the selected result.
///
/// # Errors
///
/// Returns a typed catalog, edit-selection, decode, or CPU-render failure.
pub fn load_selected_preview(
    catalog_path: &Path,
    source_root: &Path,
    photo_id: PhotoId,
) -> Result<SelectedPreview, WorkspacePreviewError> {
    let edit = load_selected_edit(catalog_path, photo_id)?;
    let repository =
        RedbCatalogRepository::open(catalog_path).map_err(WorkspacePreviewError::Catalog)?;
    let output = CatalogPreviewService::new(preview_service())
        .render(
            CatalogPreviewRequest::new(source_root, photo_id, edit.id()),
            &repository,
            &repository,
        )
        .map_err(WorkspacePreviewError::Preview)?;
    Ok(selected_preview(photo_id, &output))
}

/// Resolves the selected persisted edit and renders it at source resolution.
///
/// This is the application-side source of truth for a single-image export.
/// It deliberately returns the render output rather than presentation pixels,
/// so the export adapter receives canonical sRGB RGBA8 output from the same
/// edit pipeline without reusing a display-bounded preview.
///
/// # Errors
///
/// Returns the same typed catalog, edit-selection, source, decode, or render
/// errors as [`load_selected_preview`].
pub fn load_selected_export_render(
    catalog_path: &Path,
    source_root: &Path,
    photo_id: PhotoId,
    target: RenderTarget,
) -> Result<rusttable_render::RenderOutput, WorkspacePreviewError> {
    let edit = load_selected_edit(catalog_path, photo_id)?;
    let repository =
        RedbCatalogRepository::open(catalog_path).map_err(WorkspacePreviewError::Catalog)?;
    CatalogPreviewService::new(preview_service())
        .render_for_target(
            CatalogPreviewRequest::new(source_root, photo_id, edit.id()),
            &repository,
            &repository,
            target,
        )
        .map_err(WorkspacePreviewError::Preview)
}

/// Renders a supplied, non-persisted edit for one selected catalog photo.
///
/// The catalog is opened only for the import repository needed by the
/// composition boundary. The edit is never looked up or written, so a draft
/// can be previewed without changing the persisted edit lineage.
///
/// # Errors
///
/// Returns a typed catalog, source, ownership-validation, decode, or CPU-render failure.
pub fn load_preview_for_edit(
    catalog_path: &Path,
    source_root: &Path,
    photo_id: PhotoId,
    edit: &Edit,
) -> Result<SelectedPreview, WorkspacePreviewError> {
    let repository =
        RedbCatalogRepository::open(catalog_path).map_err(WorkspacePreviewError::Catalog)?;
    let output = CatalogPreviewService::new(preview_service())
        .render_edit(source_root, edit, &repository)
        .map_err(WorkspacePreviewError::Preview)?;
    Ok(selected_preview(photo_id, &output))
}

/// Loads the exact current persisted edit for one selected catalog photo.
///
/// # Errors
///
/// Returns the same typed catalog and edit-selection failures used by selected-preview loading.
pub(crate) fn load_selected_edit(
    catalog_path: &Path,
    photo_id: PhotoId,
) -> Result<Edit, WorkspacePreviewError> {
    let repository =
        RedbCatalogRepository::open(catalog_path).map_err(WorkspacePreviewError::Catalog)?;
    current_edit(&repository, photo_id)
}

fn current_edit(
    repository: &RedbCatalogRepository,
    photo_id: PhotoId,
) -> Result<Edit, WorkspacePreviewError> {
    let edits = repository.list().map_err(|error| {
        WorkspacePreviewError::Preview(CatalogPreviewError::EditRepository(error))
    })?;
    select_current_edit(edits, photo_id).ok_or(WorkspacePreviewError::MissingEdit { photo_id })
}

fn select_current_edit(edits: Vec<Edit>, photo_id: PhotoId) -> Option<Edit> {
    edits
        .into_iter()
        .filter(|edit| edit.photo_id() == photo_id)
        .max_by_key(|edit| (edit.revision().get(), edit.id().get()))
}

fn selected_preview(photo_id: PhotoId, output: &rusttable_render::RenderOutput) -> SelectedPreview {
    SelectedPreview {
        photo_id,
        dimensions: output.image().dimensions(),
        pixels: output.image().pixels().to_vec(),
    }
}

fn preview_service() -> PreviewService {
    PreviewService::new(
        DecodeLimits::new(
            MAX_SOURCE_BYTES,
            MAX_DECODE_DIMENSION,
            MAX_DECODE_DIMENSION,
            MAX_DECODE_PIXELS,
            MAX_DECODE_BYTES,
        )
        .expect("constant preview decode limits are valid"),
        PreviewBounds::new(PREVIEW_EDGE, PREVIEW_EDGE)
            .expect("constant workspace preview bounds are valid"),
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use rusttable_catalog::EditRepository;
    use rusttable_catalog_store::RedbCatalogRepository;
    use rusttable_core::{EditId, Revision};

    use super::{
        Edit, PhotoId, WorkspacePreviewError, load_preview_for_edit, load_selected_export_render,
        select_current_edit,
    };

    static TEST_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let number = TEST_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "rusttable-app-preview-loader-{}-{number}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("temporary preview-loader directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn photo_id(value: u128) -> PhotoId {
        PhotoId::new(value).expect("test photo ID is non-zero")
    }

    fn edit(id: u128, photo_id: PhotoId, revision: u64) -> Edit {
        Edit::from_parts(
            EditId::new(id).expect("test edit ID is non-zero"),
            photo_id,
            Revision::ZERO,
            Revision::from_u64(revision),
            [],
        )
        .expect("empty test edit is valid")
    }

    #[test]
    fn current_edit_prefers_the_highest_revision_for_the_selected_photo() {
        let selected = photo_id(1);
        let current = select_current_edit(
            vec![
                edit(10, selected, 2),
                edit(11, photo_id(2), 99),
                edit(12, selected, 3),
            ],
            selected,
        );

        assert_eq!(current.map(|edit| edit.id().get()), Some(12));
    }

    #[test]
    fn current_edit_breaks_equal_revision_ties_by_edit_id() {
        let selected = photo_id(1);
        let current =
            select_current_edit(vec![edit(12, selected, 3), edit(13, selected, 3)], selected);

        assert_eq!(current.map(|edit| edit.id().get()), Some(13));
    }

    #[test]
    fn current_edit_does_not_select_an_edit_for_another_photo() {
        assert_eq!(
            select_current_edit(vec![edit(1, photo_id(1), 1)], photo_id(2)),
            None
        );
    }

    #[test]
    fn supplied_edit_preview_preserves_selected_preview_contract_without_persisting_edit() {
        let directory = TestDirectory::new();
        let source = directory.0.join("selected.png");
        let catalog = directory.0.join("catalog.redb");
        let bytes = decode_base64(include_str!(
            "../../../rusttable-image-io/tests/fixtures/rgba-2x1.png.b64"
        ));
        fs::write(&source, bytes).expect("fixture source");

        let batch = crate::workspace::raster_import::run_raster_import(
            &catalog,
            vec![source],
            &rusttable_import::RasterImportCancellation::default(),
            &|_| {},
        );
        let selected = batch
            .receipts()
            .next()
            .and_then(|receipt| receipt.photo_id)
            .expect("fixture import photo");
        let persisted_before = RedbCatalogRepository::open(&catalog)
            .expect("open catalog before transient preview")
            .list()
            .expect("list persisted edits before transient preview");
        let transient = edit(100, selected, 1);

        let preview = load_preview_for_edit(
            &catalog,
            Path::new("unused-source-root"),
            selected,
            &transient,
        )
        .expect("transient edit preview");
        let (photo_id, dimensions, pixels) = preview.into_parts();

        assert_eq!(photo_id, selected);
        assert_eq!((dimensions.width(), dimensions.height()), (2, 1));
        assert_eq!(pixels.len(), 8);

        let persisted_after = RedbCatalogRepository::open(&catalog)
            .expect("reopen catalog after transient preview")
            .list()
            .expect("list persisted edits after transient preview");
        assert_eq!(persisted_after, persisted_before);
        assert!(
            !persisted_after
                .iter()
                .any(|persisted| persisted.id() == transient.id())
        );
    }

    #[test]
    fn selected_export_render_uses_full_resolution_not_the_display_preview_bound() {
        let directory = TestDirectory::new();
        let source = directory.0.join("selected.png");
        let catalog = directory.0.join("catalog.redb");
        let bytes = decode_base64(include_str!(
            "../../../rusttable-image-io/tests/fixtures/rgba-2x1.png.b64"
        ));
        fs::write(&source, bytes).expect("fixture source");
        let batch = crate::workspace::raster_import::run_raster_import(
            &catalog,
            vec![source],
            &rusttable_import::RasterImportCancellation::default(),
            &|_| {},
        );
        let selected = batch
            .receipts()
            .next()
            .and_then(|receipt| receipt.photo_id)
            .expect("fixture import photo");

        let output = load_selected_export_render(
            &catalog,
            Path::new("unused-source-root"),
            selected,
            rusttable_render::RenderTarget::FullResolution,
        )
        .expect("full export render");

        assert_eq!(output.image().dimensions().width(), 2);
        assert_eq!(output.image().dimensions().height(), 1);
        assert_eq!(output.provenance().source_photo_id(), selected);
    }

    #[test]
    fn selected_export_render_applies_fit_size_in_the_render_plan() {
        let directory = TestDirectory::new();
        let source = directory.0.join("selected.png");
        let catalog = directory.0.join("catalog.redb");
        let bytes = decode_base64(include_str!(
            "../../../rusttable-image-io/tests/fixtures/rgba-2x1.png.b64"
        ));
        fs::write(&source, bytes).expect("fixture source");
        let batch = crate::workspace::raster_import::run_raster_import(
            &catalog,
            vec![source],
            &rusttable_import::RasterImportCancellation::default(),
            &|_| {},
        );
        let selected = batch
            .receipts()
            .next()
            .and_then(|receipt| receipt.photo_id)
            .expect("fixture import photo");

        let target = rusttable_render::RenderTarget::PreviewFit(
            rusttable_render::PreviewBounds::new(1, 1).expect("nonzero fit bound"),
        );
        let output = load_selected_export_render(
            &catalog,
            Path::new("unused-source-root"),
            selected,
            target,
        )
        .expect("fit export render");

        assert_eq!(output.image().dimensions().width(), 1);
        assert_eq!(output.image().dimensions().height(), 1);
        assert_eq!(output.plan().target(), target);
    }

    #[test]
    fn edit_photo_validation_is_returned_through_the_composition_boundary() {
        let directory = TestDirectory::new();
        let source = directory.0.join("selected.png");
        let catalog = directory.0.join("catalog.redb");
        let bytes = decode_base64(include_str!(
            "../../../rusttable-image-io/tests/fixtures/rgba-2x1.png.b64"
        ));
        fs::write(&source, bytes).expect("fixture source");
        let batch = crate::workspace::raster_import::run_raster_import(
            &catalog,
            vec![source],
            &rusttable_import::RasterImportCancellation::default(),
            &|_| {},
        );
        let selected = batch
            .receipts()
            .next()
            .and_then(|receipt| receipt.photo_id)
            .expect("fixture import photo");

        let result = load_preview_for_edit(
            &catalog,
            Path::new("unused-source-root"),
            selected,
            &edit(101, photo_id(999), 1),
        );

        assert!(matches!(result, Err(WorkspacePreviewError::Preview(_))));
    }

    fn decode_base64(encoded: &str) -> Vec<u8> {
        let mut output = Vec::new();
        let mut bytes = encoded.bytes().filter(|byte| !byte.is_ascii_whitespace());
        while let Some(first) = bytes.next() {
            let second = bytes.next().expect("complete base64 fixture quartet");
            let third = bytes.next().expect("complete base64 fixture quartet");
            let fourth = bytes.next().expect("complete base64 fixture quartet");
            let values = [
                base64_value(first),
                base64_value(second),
                if third == b'=' {
                    0
                } else {
                    base64_value(third)
                },
                if fourth == b'=' {
                    0
                } else {
                    base64_value(fourth)
                },
            ];
            output.push((values[0] << 2) | (values[1] >> 4));
            if third != b'=' {
                output.push((values[1] << 4) | (values[2] >> 2));
            }
            if fourth != b'=' {
                output.push((values[2] << 6) | values[3]);
            }
        }
        output
    }

    fn base64_value(byte: u8) -> u8 {
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
            .bytes()
            .position(|candidate| candidate == byte)
            .expect("valid base64 fixture")
            .try_into()
            .expect("base64 alphabet index fits in u8")
    }
}
