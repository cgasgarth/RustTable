use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusttable_export::{
    PngExportLimits, PngPublishControl, PngPublishError, PngPublishProgress, PngPublishStage,
    PngPublisher,
};

use super::metadata::resolve;
use super::{ExportCancellation, ExportRequest};
use crate::workspace::{WorkspacePreviewError, load_selected_export_render_for_edit};

/// A user-visible phase of the GTK save workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportStage {
    Preparing,
    Rendering,
    ResolvingMetadata,
    Encoding,
    Verifying,
    Publishing,
    Completed,
    Cancelled,
    Failed,
}

impl ExportStage {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Preparing => "Preparing export…",
            Self::Rendering => "Rendering selected edit…",
            Self::ResolvingMetadata => "Resolving export metadata…",
            Self::Encoding => "Encoding PNG…",
            Self::Verifying => "Verifying PNG…",
            Self::Publishing => "Publishing PNG…",
            Self::Completed => "Export complete.",
            Self::Cancelled => "Export cancelled.",
            Self::Failed => "Export failed.",
        }
    }
}

/// Progress emitted at the bounded workflow's stage boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportStatus {
    stage: ExportStage,
    text: String,
}

impl ExportStatus {
    #[must_use]
    pub fn at(stage: ExportStage) -> Self {
        Self {
            stage,
            text: stage.label().to_owned(),
        }
    }

    #[must_use]
    pub const fn stage(&self) -> ExportStage {
        self.stage
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// The verified artifact summary returned after publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportCompletion {
    destination_alias: String,
    width: u32,
    height: u32,
    encoded_bytes: u64,
    completed_after_cancellation: bool,
    metadata: Arc<ExportMetadataEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExportMetadataEvidence {
    policy_identity: String,
    included_fields: Vec<String>,
    excluded_groups: Vec<String>,
}

impl ExportCompletion {
    #[must_use]
    pub fn summary(&self) -> String {
        let prefix = if self.completed_after_cancellation {
            "Saved after cancellation request"
        } else {
            "Saved"
        };
        format!(
            "{prefix} {} ({}×{}, {} bytes; metadata {})",
            self.destination_alias,
            self.width,
            self.height,
            self.encoded_bytes,
            self.metadata.policy_identity
        )
    }

    #[must_use]
    pub fn destination_alias(&self) -> &str {
        &self.destination_alias
    }

    #[must_use]
    pub const fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    #[must_use]
    pub const fn encoded_bytes(&self) -> u64 {
        self.encoded_bytes
    }

    #[must_use]
    pub fn metadata_policy_identity(&self) -> &str {
        &self.metadata.policy_identity
    }

    #[must_use]
    pub fn metadata_included_fields(&self) -> &[String] {
        &self.metadata.included_fields
    }

    #[must_use]
    pub fn metadata_excluded_groups(&self) -> &[String] {
        &self.metadata.excluded_groups
    }
}

/// Failures surfaced by the GTK workflow, including a collision retry point.
#[derive(Debug)]
pub enum ExportRunError {
    Cancelled,
    InvalidExtension(PathBuf),
    DestinationExists(PathBuf),
    Render(WorkspacePreviewError),
    Metadata(String),
    Publish(PngPublishError),
    InvalidDestination(PathBuf),
}

impl fmt::Display for ExportRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("export cancelled before publication"),
            Self::InvalidExtension(path) => write!(
                formatter,
                "choose a .png destination (received {})",
                path.display()
            ),
            Self::DestinationExists(path) => {
                write!(formatter, "a PNG already exists at {}", path.display())
            }
            Self::Render(error) => write!(formatter, "could not render the selected edit: {error}"),
            Self::Metadata(error) => {
                write!(formatter, "could not resolve export metadata: {error}")
            }
            Self::Publish(error) => write!(formatter, "could not save the rendered PNG: {error}"),
            Self::InvalidDestination(path) => {
                write!(formatter, "invalid PNG destination: {}", path.display())
            }
        }
    }
}

impl std::error::Error for ExportRunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Render(error) => Some(error),
            Self::Publish(error) => Some(error),
            Self::Cancelled
            | Self::InvalidExtension(_)
            | Self::DestinationExists(_)
            | Self::Metadata(_)
            | Self::InvalidDestination(_) => None,
        }
    }
}

/// Runs one immutable selected-edit request synchronously for a worker thread.
///
/// # Errors
///
/// Returns a typed render, destination, collision, cancellation, or publish
/// failure.
pub fn run(
    request: &ExportRequest,
    cancellation: &ExportCancellation,
) -> Result<ExportCompletion, ExportRunError> {
    run_with_progress(request, cancellation, |_| {})
}

/// Runs one request and emits progress without exposing filesystem paths.
///
/// # Errors
///
/// Returns a typed render, destination, collision, cancellation, or publish
/// failure.
///
/// # Panics
///
/// Panics only if the compile-time PNG bounds become invalid.
pub fn run_with_progress(
    request: &ExportRequest,
    cancellation: &ExportCancellation,
    mut report: impl FnMut(ExportStatus),
) -> Result<ExportCompletion, ExportRunError> {
    report(ExportStatus::at(ExportStage::Preparing));
    check_cancelled(cancellation)?;
    let destination = png_destination(request.destination())?;
    report(ExportStatus::at(ExportStage::Rendering));
    let render = load_selected_export_render_for_edit(
        request.catalog_path(),
        request.source_root(),
        request.photo_id(),
        request.edit_id(),
        request.settings().size().render_target(),
    )
    .map_err(ExportRunError::Render)?;
    check_cancelled(cancellation)?;

    report(ExportStatus::at(ExportStage::ResolvingMetadata));
    let resolved_metadata = resolve(
        request.catalog_path(),
        request.photo_id(),
        request.edit_id(),
        &render,
        request.settings().metadata_policy(),
    )
    .map_err(|error| ExportRunError::Metadata(error.to_string()))?;
    check_cancelled(cancellation)?;

    let limits = PngExportLimits::new(
        request.settings().size().max_edge(),
        request.settings().size().max_edge(),
        super::MAX_OUTPUT_BYTES,
    )
    .expect("constant PNG export limits are valid");
    let publisher = PngPublisher::new(limits);
    let receipt = publisher
        .publish_with_metadata_and_observer(
            render.image(),
            &destination,
            request.settings().collision().policy(),
            Some(resolved_metadata.metadata),
            Some(resolved_metadata.receipt),
            |progress: PngPublishProgress| {
                if let Some(status) = publish_status(progress.stage()) {
                    report(status);
                }
                if cancellation.is_cancelled() {
                    PngPublishControl::Cancel
                } else {
                    PngPublishControl::Continue
                }
            },
        )
        .map_err(map_publish_error)?;

    report(ExportStatus::at(ExportStage::Completed));
    let alias = destination_alias(&destination);
    let (width, height) = (receipt.dimensions().width(), receipt.dimensions().height());
    let receipt_metadata = receipt
        .metadata()
        .expect("metadata-aware GTK export returns policy evidence");
    Ok(ExportCompletion {
        destination_alias: alias,
        width,
        height,
        encoded_bytes: receipt.encoded_byte_length(),
        completed_after_cancellation: matches!(
            receipt.completion(),
            rusttable_export::PngPublishCompletion::CompletedAfterCancellation
        ),
        metadata: Arc::new(ExportMetadataEvidence {
            policy_identity: receipt_metadata.policy_identity().to_owned(),
            included_fields: receipt_metadata.included_fields().to_vec(),
            excluded_groups: receipt_metadata.excluded_groups().to_vec(),
        }),
    })
}

fn check_cancelled(cancellation: &ExportCancellation) -> Result<(), ExportRunError> {
    if cancellation.is_cancelled() {
        return Err(ExportRunError::Cancelled);
    }
    Ok(())
}

fn publish_status(stage: PngPublishStage) -> Option<ExportStatus> {
    let stage = match stage {
        PngPublishStage::Preparing => ExportStage::Preparing,
        PngPublishStage::Encoding => ExportStage::Encoding,
        PngPublishStage::Verifying => ExportStage::Verifying,
        PngPublishStage::Publishing => ExportStage::Publishing,
        PngPublishStage::Completed => return None,
    };
    Some(ExportStatus::at(stage))
}

fn map_publish_error(error: PngPublishError) -> ExportRunError {
    match error {
        PngPublishError::DestinationExists { path } => ExportRunError::DestinationExists(path),
        PngPublishError::Cancelled { .. } => ExportRunError::Cancelled,
        PngPublishError::InvalidDestination { path } => ExportRunError::InvalidDestination(path),
        other => ExportRunError::Publish(other),
    }
}

fn png_destination(destination: &Path) -> Result<PathBuf, ExportRunError> {
    let Some(file_name) = destination.file_name() else {
        return Err(ExportRunError::InvalidExtension(destination.to_owned()));
    };
    match destination
        .extension()
        .and_then(|extension| extension.to_str())
    {
        None => Ok(destination.with_file_name(format!("{}.png", file_name.to_string_lossy()))),
        Some(extension) if extension.eq_ignore_ascii_case("png") => Ok(destination.to_owned()),
        Some(_) => Err(ExportRunError::InvalidExtension(destination.to_owned())),
    }
}

fn destination_alias(destination: &Path) -> String {
    destination.file_name().map_or_else(
        || "PNG".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::super::{ExportSettings, ExportSizeSelection};
    use super::{
        ExportCancellation, ExportRequest, ExportStage, png_destination, run_with_progress,
    };
    use rusttable_import::RasterImportCancellation;

    static TEST_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let number = TEST_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "rusttable-gtk-export-worker-{}-{number}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("temporary export directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn png_destination_appends_only_a_missing_extension() {
        assert_eq!(
            png_destination(Path::new("rendered")).expect("extension appended"),
            Path::new("rendered.png")
        );
        assert_eq!(
            png_destination(Path::new("rendered.PNG")).expect("PNG accepted"),
            Path::new("rendered.PNG")
        );
        assert!(png_destination(Path::new("rendered.jpg")).is_err());
    }

    #[test]
    fn selected_persisted_edit_exports_full_resolution_and_reports_verified_stages() {
        let directory = TestDirectory::new();
        let catalog = directory.0.join("catalog.redb");
        let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("workspace root");
        let source = workspace_root.join("fixtures/corpus/assets/raster-png-16-alpha.png");
        let batch = crate::workspace::run_raster_import(
            &catalog,
            vec![source],
            &RasterImportCancellation::default(),
            &|_| {},
        );
        let photo_id = batch
            .receipts()
            .next()
            .and_then(|receipt| receipt.photo_id)
            .expect("fixture photo");
        let edit_id = crate::workspace::selected_edit_id(&catalog, photo_id)
            .expect("selected persisted edit");
        let destination = directory.0.join("rendered.png");
        let request = ExportRequest::new(
            catalog,
            workspace_root,
            photo_id,
            edit_id,
            destination.clone(),
            ExportSettings::from_selection(
                ExportSizeSelection::Original,
                super::super::ExportCollisionSelection::CreateNew,
            ),
        );
        let mut stages = Vec::new();
        let completion = run_with_progress(&request, &ExportCancellation::default(), |status| {
            stages.push(status.stage());
        })
        .expect("selected edit export");

        assert_eq!(completion.dimensions(), (4, 3));
        assert!(completion.summary().starts_with("Saved rendered.png (4×3,"));
        assert!(destination.is_file());
        assert_eq!(
            stages,
            vec![
                ExportStage::Preparing,
                ExportStage::Rendering,
                ExportStage::ResolvingMetadata,
                ExportStage::Preparing,
                ExportStage::Encoding,
                ExportStage::Verifying,
                ExportStage::Publishing,
                ExportStage::Completed,
            ]
        );
    }
}
