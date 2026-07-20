use std::path::Path;

use rusttable_catalog::{EditRepository, RepositoryError};
use rusttable_catalog_store::RedbCatalogRepository;
use rusttable_core::{Edit, PhotoId};
use rusttable_image::{DecodeLimits, ImageDimensions};
use rusttable_render::PreviewBounds;

use crate::{CatalogPreviewError, CatalogPreviewRequest, CatalogPreviewService, PreviewService};

const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DECODE_DIMENSION: u32 = 16_384;
const MAX_DECODE_PIXELS: u64 = 64 * 1024 * 1024;
const MAX_DECODE_BYTES: u64 = 256 * 1024 * 1024;
const PREVIEW_EDGE: u32 = 1_536;

/// Immutable RGBA8 pixels ready for a presentation adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelectedPreview {
    photo_id: PhotoId,
    dimensions: ImageDimensions,
    pixels: Vec<u8>,
}

impl SelectedPreview {
    #[must_use]
    pub(crate) fn into_parts(self) -> (PhotoId, ImageDimensions, Vec<u8>) {
        (self.photo_id, self.dimensions, self.pixels)
    }
}

#[derive(Debug)]
pub(crate) enum WorkspacePreviewError {
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
pub(crate) fn load_selected_preview(
    catalog_path: &Path,
    source_root: &Path,
    photo_id: PhotoId,
) -> Result<SelectedPreview, WorkspacePreviewError> {
    let repository =
        RedbCatalogRepository::open(catalog_path).map_err(WorkspacePreviewError::Catalog)?;
    let edit = current_edit(&repository, photo_id)?;
    let output = CatalogPreviewService::new(preview_service())
        .render(
            CatalogPreviewRequest::new(source_root, photo_id, edit.id()),
            &repository,
            &repository,
        )
        .map_err(WorkspacePreviewError::Preview)?;
    Ok(SelectedPreview {
        photo_id,
        dimensions: output.image().dimensions(),
        pixels: output.image().pixels().to_vec(),
    })
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
    use rusttable_core::{EditId, Revision};

    use super::{Edit, PhotoId, select_current_edit};

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
}
