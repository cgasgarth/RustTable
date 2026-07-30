use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::diagnostics::AppDiagnostics;
use rusttable_image::InputFormat;
use rusttable_import::{
    RasterImportBatch, RasterImportFailure, RasterImportReceipt, RasterImportStage,
    RasterImportStatus, RasterPreviewError, normalize_reference_path,
};
use rusttable_ui::{
    ImportItemOutcome, ImportRequest, ImportReviewRow, ImportSessionState, ImportSessionViewModel,
    is_raw_path,
};

use super::{
    CollectionController, GtkCatalogController, MacApplicationBridge, dispatch_open_request,
    thumbnails::ThumbnailLifecycle,
};

#[expect(
    clippy::too_many_arguments,
    reason = "the import bridge keeps the application ports explicit at this composition boundary"
)]
pub(super) fn dispatch_import_request(
    _shell: &rusttable_ui::GtkShell,
    native_bridge: &Rc<RefCell<MacApplicationBridge>>,
    active_shell: &Rc<RefCell<Option<rusttable_ui::GtkShell>>>,
    active_catalog: &Rc<RefCell<Option<Rc<RefCell<GtkCatalogController>>>>>,
    active_collection: &Rc<RefCell<Option<CollectionController>>>,
    thumbnail_lifecycle: &Rc<RefCell<ThumbnailLifecycle>>,
    request: &ImportRequest,
    diagnostics: &AppDiagnostics,
) {
    let existing = active_catalog
        .borrow()
        .as_ref()
        .map(|catalog| catalog.borrow().existing_source_paths())
        .unwrap_or_default();
    let paths = effective_import_paths(
        expand_import_paths(request.paths(), request.recursive()),
        request,
        &existing,
    );
    if paths.is_empty() {
        return;
    }
    let delivery = native_bridge.borrow_mut().receive_paths(paths);
    if let Some(open_request) = delivery.request().cloned() {
        dispatch_open_request(
            &open_request,
            active_shell,
            active_catalog,
            active_collection,
            thumbnail_lifecycle,
            diagnostics,
        );
    }
}

pub(super) fn running_import_session(paths: &[PathBuf]) -> ImportSessionViewModel {
    ImportSessionViewModel {
        state: ImportSessionState::Running,
        rows: paths
            .iter()
            .enumerate()
            .map(|(index, path)| ImportReviewRow {
                item_id: (index + 1).to_string(),
                alias: source_filename(path),
                outcome: ImportItemOutcome::Transferring,
                detail: None,
                receipt_id: None,
            })
            .collect(),
        total: u32::try_from(paths.len()).unwrap_or(u32::MAX),
        ..ImportSessionViewModel::default()
    }
}

pub(super) fn completed_import_session(
    batch: &RasterImportBatch,
    source_paths: &[PathBuf],
) -> ImportSessionViewModel {
    let rows = batch
        .receipts()
        .enumerate()
        .map(|(index, receipt)| review_row(receipt, source_paths.get(index).map(PathBuf::as_path)))
        .collect::<Vec<_>>();
    let needs_attention = batch.receipts().any(|receipt| {
        matches!(
            receipt.status,
            RasterImportStatus::Failed(_)
                | RasterImportStatus::ImportedPreviewFailed
                | RasterImportStatus::Cancelled
        )
    });
    let total = u32::try_from(rows.len()).unwrap_or(u32::MAX);
    ImportSessionViewModel {
        state: if needs_attention {
            ImportSessionState::Failed
        } else {
            ImportSessionState::Complete
        },
        rows,
        completed: total,
        total,
        diagnostic: needs_attention.then(|| "Import completed with problems".to_owned()),
        ..ImportSessionViewModel::default()
    }
}

pub(super) fn selected_photo_to_open(batch: &RasterImportBatch) -> Option<rusttable_core::PhotoId> {
    batch.first_selected_photo()
}

pub(super) fn disconnected_import_session(paths: &[PathBuf]) -> ImportSessionViewModel {
    let total = u32::try_from(paths.len()).unwrap_or(u32::MAX);
    ImportSessionViewModel {
        state: ImportSessionState::Failed,
        rows: paths
            .iter()
            .enumerate()
            .map(|(index, path)| ImportReviewRow {
                item_id: (index + 1).to_string(),
                alias: source_filename(path),
                outcome: ImportItemOutcome::Failed { retryable: false },
                detail: Some(
                    "Format: unknown; stage: import worker; reason: the import worker stopped unexpectedly"
                        .to_owned(),
                ),
                receipt_id: None,
            })
            .collect(),
        completed: 0,
        total,
        diagnostic: Some("Import worker stopped unexpectedly".to_owned()),
        ..ImportSessionViewModel::default()
    }
}

fn review_row(receipt: &RasterImportReceipt, source_path: Option<&Path>) -> ImportReviewRow {
    let (outcome, detail) = match receipt.status {
        RasterImportStatus::Imported => (ImportItemOutcome::Imported, None),
        RasterImportStatus::AlreadyImported => (ImportItemOutcome::Duplicate, None),
        RasterImportStatus::ImportedPreviewPending => (
            ImportItemOutcome::Imported,
            Some(format!(
                "Format: {}; stage: generating preview; reason: preview generation is pending",
                format_label(receipt.format)
            )),
        ),
        RasterImportStatus::ImportedPreviewFailed => (
            ImportItemOutcome::Imported,
            Some(format!(
                "Format: {}; stage: {}; reason: {}",
                format_label(receipt.format),
                stage_label(
                    receipt
                        .failure_stage
                        .unwrap_or(RasterImportStage::GeneratingPreview)
                ),
                preview_failure_label(receipt.preview_failure)
            )),
        ),
        RasterImportStatus::Failed(failure) => (
            ImportItemOutcome::Failed { retryable: false },
            Some(format!(
                "Format: {}; stage: {}; reason: {}",
                format_label(receipt.format),
                stage_label(receipt.failure_stage.unwrap_or(RasterImportStage::Failed)),
                failure_label(failure)
            )),
        ),
        RasterImportStatus::Cancelled => (
            ImportItemOutcome::Skipped,
            Some(format!(
                "Format: {}; stage: cancelled; reason: import was cancelled",
                format_label(receipt.format)
            )),
        ),
    };
    ImportReviewRow {
        item_id: receipt.item_id.get().to_string(),
        alias: source_path.map_or_else(|| receipt.source_alias.clone(), source_filename),
        outcome,
        detail,
        receipt_id: None,
    }
}

fn source_filename(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "unknown source".to_owned())
}

const fn format_label(format: Option<InputFormat>) -> &'static str {
    match format {
        Some(InputFormat::Jpeg) => "JPEG",
        Some(InputFormat::JpegXl) => "JPEG XL",
        Some(InputFormat::Png) => "PNG",
        Some(InputFormat::Tiff) => "TIFF",
        Some(InputFormat::OpenExr) => "OpenEXR",
        Some(InputFormat::Webp) => "WebP",
        Some(InputFormat::Raw) => "RAW",
        None => "unknown",
    }
}

const fn stage_label(stage: RasterImportStage) -> &'static str {
    match stage {
        RasterImportStage::Queued => "queued",
        RasterImportStage::Opening => "opening source",
        RasterImportStage::Hashing => "hashing source",
        RasterImportStage::Probing => "detecting format",
        RasterImportStage::DecodingHeader => "reading image header",
        RasterImportStage::Decoding => "decoding image",
        RasterImportStage::Registering => "saving to catalog",
        RasterImportStage::GeneratingPreview => "generating preview",
        RasterImportStage::Completed => "completed",
        RasterImportStage::AlreadyImported => "already imported",
        RasterImportStage::Failed => "failed",
        RasterImportStage::Cancelled => "cancelled",
    }
}

const fn failure_label(failure: RasterImportFailure) -> &'static str {
    match failure {
        RasterImportFailure::SourceUnavailable => "source could not be opened",
        RasterImportFailure::NonRegularSource => "source is not a regular file",
        RasterImportFailure::SymlinkRejected => "symbolic links are not accepted",
        RasterImportFailure::SourceChanged => "source changed during import",
        RasterImportFailure::SourceTooLarge => "source exceeds the import size limit",
        RasterImportFailure::UnsupportedOrMalformedRaster => "image is unsupported or malformed",
        RasterImportFailure::UnsupportedPathEncoding => "source filename encoding is unsupported",
        RasterImportFailure::CatalogUnavailable => "catalog is unavailable",
        RasterImportFailure::CatalogConflict => "catalog changed while importing",
        RasterImportFailure::CatalogCorrupt => "catalog data is corrupt",
        RasterImportFailure::CatalogCommitFailed => "import could not be saved",
        RasterImportFailure::PreviewFailed => "preview could not be generated",
        RasterImportFailure::InternalInvariant => "an internal import error occurred",
    }
}

const fn preview_failure_label(failure: Option<RasterPreviewError>) -> &'static str {
    match failure {
        Some(RasterPreviewError::Unavailable) => "preview service is unavailable",
        Some(RasterPreviewError::SourceChanged) => "source changed before preview generation",
        Some(RasterPreviewError::Decode) => "preview image decoding failed",
        Some(RasterPreviewError::RawDecode) => "preview RAW decoding failed",
        Some(RasterPreviewError::DecodedFrame) => "decoded preview frame could not be adapted",
        Some(RasterPreviewError::UnsupportedPixelpipeColor) => {
            "preview source color is unsupported"
        }
        Some(RasterPreviewError::PixelpipeInput) => "preview pixelpipe input could not be built",
        Some(RasterPreviewError::PixelpipeSnapshot) => {
            "preview pixelpipe snapshot could not be built"
        }
        Some(RasterPreviewError::Graph) => "preview processing graph could not be compiled",
        Some(RasterPreviewError::RawPipeline) => "preview RAW pipeline could not be compiled",
        Some(RasterPreviewError::Pixelpipe) => "preview pixelpipe execution failed",
        Some(RasterPreviewError::Prepared) => "preview render preparation failed",
        Some(RasterPreviewError::Render) => "final preview rendering failed",
        None => "preview could not be generated",
    }
}

fn effective_import_paths(
    paths: Vec<PathBuf>,
    request: &ImportRequest,
    existing: &BTreeSet<PathBuf>,
) -> Vec<PathBuf> {
    paths
        .into_iter()
        .map(|path| normalize_reference_path(&path).unwrap_or(path))
        .filter(|path| !request.ignore_nonraws() || is_raw_path(path))
        .filter(|path| !request.select_new() || !existing.contains(path))
        .collect()
}

fn expand_import_paths(paths: &[PathBuf], recursive: bool) -> Vec<PathBuf> {
    let mut expanded = Vec::new();
    for path in paths {
        if path.is_dir() {
            collect_import_files(path, recursive, &mut expanded);
        } else {
            expanded.push(path.clone());
        }
    }
    expanded
}

fn collect_import_files(path: &std::path::Path, recursive: bool, output: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if recursive {
                collect_import_files(&path, true, output);
            }
        } else {
            output.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_import_paths_apply_new_and_raw_filters_before_dispatch() {
        let paths = vec![
            PathBuf::from("new.nef"),
            PathBuf::from("new.jpg"),
            PathBuf::from("old.arw"),
        ];
        let existing = BTreeSet::from([PathBuf::from("old.arw")]);
        let request = ImportRequest::new(paths.clone(), false, true, true, 3).expect("request");
        assert_eq!(
            effective_import_paths(paths, &request, &existing)
                .into_iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            ["new.nef"]
        );
    }

    #[test]
    fn completed_batch_keeps_successes_and_exposes_typed_failure_evidence() {
        let request = rusttable_import::RasterImportRequest::new([
            PathBuf::from("good.png"),
            PathBuf::from("too-large.raw"),
        ])
        .expect("request");
        let item_ids = request
            .items()
            .map(|(item_id, _)| item_id)
            .collect::<Vec<_>>();
        let selected_photo = rusttable_core::PhotoId::new(7).expect("photo ID");
        let mut imported = receipt(
            item_ids[0],
            "good.png",
            Some(InputFormat::Png),
            RasterImportStatus::Imported,
            None,
            None,
        );
        imported.photo_id = Some(selected_photo);
        let batch = RasterImportBatch::new(vec![
            imported,
            receipt(
                item_ids[1],
                "Image",
                None,
                RasterImportStatus::Failed(RasterImportFailure::SourceTooLarge),
                Some(RasterImportStage::Opening),
                None,
            ),
        ]);

        let state = completed_import_session(
            &batch,
            &[PathBuf::from("good.png"), PathBuf::from("too-large.raw")],
        );

        assert_eq!(state.state, ImportSessionState::Failed);
        assert_eq!(selected_photo_to_open(&batch), Some(selected_photo));
        assert_eq!(state.completed, 2);
        assert_eq!(state.total, 2);
        assert_eq!(state.rows[0].alias, "good.png");
        assert_eq!(state.rows[0].outcome, ImportItemOutcome::Imported);
        assert_eq!(state.rows[0].detail, None);
        assert_eq!(state.rows[1].alias, "too-large.raw");
        assert_eq!(
            state.rows[1].outcome,
            ImportItemOutcome::Failed { retryable: false }
        );
        assert_eq!(
            state.rows[1].detail.as_deref(),
            Some(
                "Format: unknown; stage: opening source; reason: source exceeds the import size limit"
            )
        );
    }

    #[test]
    fn preview_failure_preserves_import_success_and_exact_reason() {
        let request = rusttable_import::RasterImportRequest::new([PathBuf::from("frame.exr")])
            .expect("request");
        let item_id = request.items().next().expect("item").0;
        let batch = RasterImportBatch::new(vec![receipt(
            item_id,
            "frame.exr",
            Some(InputFormat::OpenExr),
            RasterImportStatus::ImportedPreviewFailed,
            Some(RasterImportStage::GeneratingPreview),
            Some(RasterPreviewError::Prepared),
        )]);

        let state = completed_import_session(&batch, &[PathBuf::from("frame.exr")]);

        assert_eq!(state.state, ImportSessionState::Failed);
        assert_eq!(state.rows[0].outcome, ImportItemOutcome::Imported);
        assert_eq!(
            state.rows[0].detail.as_deref(),
            Some(
                "Format: OpenEXR; stage: generating preview; reason: preview render preparation failed"
            )
        );
    }

    #[test]
    fn disconnected_worker_exposes_readable_filenames_without_paths() {
        let paths = [
            PathBuf::from("/private/photos/one.nef"),
            PathBuf::from("/private/photos/two.cr3"),
        ];

        let state = disconnected_import_session(&paths);

        assert_eq!(state.state, ImportSessionState::Failed);
        assert_eq!(state.rows[0].alias, "one.nef");
        assert_eq!(state.rows[1].alias, "two.cr3");
        assert!(
            state
                .rows
                .iter()
                .all(|row| row.detail.as_deref().is_some_and(|detail| {
                    detail.contains("Format: unknown")
                        && detail.contains("stage: import worker")
                        && detail.contains("stopped unexpectedly")
                        && !detail.contains("/private/photos")
                }))
        );
    }

    fn receipt(
        item_id: rusttable_import::RasterImportItemId,
        alias: &str,
        format: Option<InputFormat>,
        status: RasterImportStatus,
        failure_stage: Option<RasterImportStage>,
        preview_failure: Option<RasterPreviewError>,
    ) -> RasterImportReceipt {
        RasterImportReceipt {
            schema_version: 3,
            item_id,
            source_alias: alias.to_owned(),
            content_sha256: None,
            format,
            photo_id: None,
            asset_id: None,
            edit_id: None,
            status,
            failure_stage,
            preview_failure,
            metadata_status: None,
            preview: None,
            duplicates: rusttable_catalog::DuplicateSearchResult::default(),
        }
    }
}
