use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use iced::Task;
use rusttable_core::PhotoId;
use rusttable_export::{CollisionPolicy, PngExportLimits, PngPublisher};
use rusttable_render::{PreviewBounds, RenderTarget};

use crate::workspace::load_selected_export_render;

use super::Message;

const MAX_OUTPUT_EDGE: u32 = 16_384;
const MAX_OUTPUT_BYTES: u64 = 512 * 1024 * 1024;

/// The requested output size for a single rendered-copy export.
///
/// The current Iced action uses [`Self::Original`]. Fit choices are immutable
/// request values so the forthcoming size selector cannot change a running
/// export request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the Iced size picker selects all supported export sizes"
    )
)]
pub(crate) enum ExportSize {
    Original,
    Fit2048,
    Fit4096,
}

impl ExportSize {
    #[must_use]
    const fn max_edge(self) -> u32 {
        match self {
            Self::Original => MAX_OUTPUT_EDGE,
            Self::Fit2048 => 2_048,
            Self::Fit4096 => 4_096,
        }
    }

    fn render_target(self) -> RenderTarget {
        match self {
            Self::Original => RenderTarget::FullResolution,
            Self::Fit2048 | Self::Fit4096 => {
                let maximum_edge = self.max_edge();
                RenderTarget::PreviewFit(
                    PreviewBounds::new(maximum_edge, maximum_edge)
                        .expect("constant export fit bounds are valid"),
                )
            }
        }
    }
}

/// Immutable controls captured before an export task begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExportSettings {
    size: ExportSize,
    collision: CollisionPolicy,
}

impl ExportSettings {
    #[must_use]
    pub(crate) const fn original() -> Self {
        Self {
            size: ExportSize::Original,
            collision: CollisionPolicy::CreateNew,
        }
    }

    #[must_use]
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the Iced export settings dialog constructs selected settings"
        )
    )]
    pub(crate) const fn new(size: ExportSize, collision: CollisionPolicy) -> Self {
        Self { size, collision }
    }

    #[must_use]
    pub(crate) const fn size(self) -> ExportSize {
        self.size
    }

    #[must_use]
    pub(crate) const fn collision(self) -> CollisionPolicy {
        self.collision
    }
}

/// A complete selected-photo export request that cannot change after launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExportRequest {
    catalog_path: PathBuf,
    source_root: PathBuf,
    photo_id: PhotoId,
    destination: PathBuf,
    settings: ExportSettings,
}

impl ExportRequest {
    #[must_use]
    pub(crate) fn new(
        catalog_path: PathBuf,
        source_root: PathBuf,
        photo_id: PhotoId,
        destination: PathBuf,
        settings: ExportSettings,
    ) -> Self {
        Self {
            catalog_path,
            source_root,
            photo_id,
            destination,
            settings,
        }
    }

    #[must_use]
    pub(crate) const fn photo_id(&self) -> PhotoId {
        self.photo_id
    }

    #[must_use]
    pub(crate) fn destination(&self) -> &Path {
        &self.destination
    }

    #[must_use]
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the Iced export settings dialog reads its saved settings"
        )
    )]
    pub(crate) const fn settings(&self) -> ExportSettings {
        self.settings
    }
}

/// Cooperative cancellation shared by an Iced export task and its owner.
#[derive(Debug, Clone, Default)]
pub(crate) struct ExportCancellation(Arc<AtomicBool>);

impl ExportCancellation {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the Iced export progress control requests cancellation"
        )
    )]
    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    #[must_use]
    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// A user-visible phase of the bounded save workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExportStage {
    Preparing,
    Rendering,
    Publishing,
    Completed,
    Cancelled,
    Failed,
}

impl ExportStage {
    #[must_use]
    const fn label(self) -> &'static str {
        match self {
            Self::Preparing => "Preparing export…",
            Self::Rendering => "Rendering selected edit…",
            Self::Publishing => "Encoding, verifying, and publishing PNG…",
            Self::Completed => "Export complete.",
            Self::Cancelled => "Export cancelled.",
            Self::Failed => "Export failed.",
        }
    }
}

/// Status emitted at phase boundaries without exposing filesystem paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExportStatus {
    stage: ExportStage,
    text: String,
}

impl ExportStatus {
    #[must_use]
    fn at(stage: ExportStage) -> Self {
        Self {
            stage,
            text: stage.label().to_owned(),
        }
    }

    #[must_use]
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the Iced export progress view reads stage updates"
        )
    )]
    pub(crate) const fn stage(&self) -> ExportStage {
        self.stage
    }

    #[must_use]
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the Iced progress view reads status text")
    )]
    pub(crate) fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExportTaskResult {
    Completed(String),
    Failed(String),
}

/// Owns a cancellation handle and the associated Iced task.
///
/// The application retains the handle when it adds the export progress UI;
/// callers that only need the existing one-shot action can consume the task.
#[derive(Debug)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the Iced progress view retains this cancellation handle"
    )
)]
pub(crate) struct ExportTask {
    cancellation: ExportCancellation,
    task: Task<Message>,
}

impl ExportTask {
    #[must_use]
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the Iced progress view requests cancellation")
    )]
    pub(crate) fn cancellation(&self) -> ExportCancellation {
        self.cancellation.clone()
    }

    pub(crate) fn into_task(self) -> Task<Message> {
        self.task
    }
}

pub(super) fn pick_destination(photo_id: PhotoId) -> Task<Message> {
    Task::perform(
        async move {
            rfd::AsyncFileDialog::new()
                .add_filter("PNG image", &["png"])
                .set_file_name("RustTable export.png")
                .save_file()
                .await
                .map(|handle| handle.path().to_owned())
        },
        move |destination| Message::ExportDestinationSelected {
            photo_id,
            destination,
        },
    )
}

pub(super) fn start(
    catalog_path: PathBuf,
    source_root: PathBuf,
    photo_id: PhotoId,
    destination: PathBuf,
) -> Task<Message> {
    start_request(ExportRequest::new(
        catalog_path,
        source_root,
        photo_id,
        destination,
        ExportSettings::original(),
    ))
    .into_task()
}

/// Starts one immutable export request on Iced's task executor.
///
/// The caller can retain [`ExportTask::cancellation`] and request cooperative
/// cancellation before publication. A cancellation observed after publication
/// is reported as a completed export because the verified final artifact is
/// already visible.
#[must_use]
pub(crate) fn start_request(request: ExportRequest) -> ExportTask {
    let cancellation = ExportCancellation::default();
    let task_cancellation = cancellation.clone();
    let photo_id = request.photo_id();
    let task = Task::perform(
        async move {
            let mut ignored_status = |_| {};
            ExportTaskResult::from(run(&request, &task_cancellation, &mut ignored_status))
        },
        move |result| Message::ExportFinished { photo_id, result },
    );
    ExportTask { cancellation, task }
}

fn run(
    request: &ExportRequest,
    cancellation: &ExportCancellation,
    report_status: &mut dyn FnMut(ExportStatus),
) -> Result<String, ExportRunError> {
    let result = run_inner(request, cancellation, report_status);
    match result {
        Ok(_) => report_status(ExportStatus::at(ExportStage::Completed)),
        Err(ExportRunError::Cancelled) => report_status(ExportStatus::at(ExportStage::Cancelled)),
        Err(_) => report_status(ExportStatus::at(ExportStage::Failed)),
    }
    result
}

fn run_inner(
    request: &ExportRequest,
    cancellation: &ExportCancellation,
    report_status: &mut dyn FnMut(ExportStatus),
) -> Result<String, ExportRunError> {
    report_status(ExportStatus::at(ExportStage::Preparing));
    check_cancelled(cancellation)?;
    let destination = png_destination(request.destination())?;
    report_status(ExportStatus::at(ExportStage::Rendering));
    let render = load_selected_export_render(
        &request.catalog_path,
        &request.source_root,
        request.photo_id,
        request.settings.size().render_target(),
    )
    .map_err(ExportRunError::Render)?;
    check_cancelled(cancellation)?;
    report_status(ExportStatus::at(ExportStage::Publishing));
    let limits = PngExportLimits::new(
        request.settings.size().max_edge(),
        request.settings.size().max_edge(),
        MAX_OUTPUT_BYTES,
    )
    .expect("constant PNG export limits are valid");
    let receipt = PngPublisher::new(limits)
        .publish(render.image(), &destination, request.settings.collision())
        .map_err(ExportRunError::Publish)?;
    let completed_after_cancellation = cancellation.is_cancelled();
    let alias = destination.file_name().map_or_else(
        || destination.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let completion = if completed_after_cancellation {
        "Saved after cancellation request"
    } else {
        "Saved"
    };
    Ok(format!(
        "{completion} {alias} ({}×{}, {} bytes)",
        receipt.dimensions().width(),
        receipt.dimensions().height(),
        receipt.encoded_byte_length()
    ))
}

fn check_cancelled(cancellation: &ExportCancellation) -> Result<(), ExportRunError> {
    if cancellation.is_cancelled() {
        return Err(ExportRunError::Cancelled);
    }
    Ok(())
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

#[derive(Debug)]
enum ExportRunError {
    Cancelled,
    InvalidExtension(PathBuf),
    Render(crate::workspace::preview_loader::WorkspacePreviewError),
    Publish(rusttable_export::PngPublishError),
}

impl std::fmt::Display for ExportRunError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("export cancelled before publication"),
            Self::InvalidExtension(path) => write!(
                formatter,
                "choose a .png destination (received {})",
                path.display()
            ),
            Self::Render(error) => write!(formatter, "could not render the selected edit: {error}"),
            Self::Publish(error) => write!(formatter, "could not save the rendered PNG: {error}"),
        }
    }
}

impl From<Result<String, ExportRunError>> for ExportTaskResult {
    fn from(result: Result<String, ExportRunError>) -> Self {
        match result {
            Ok(status) => Self::Completed(status),
            Err(error) => Self::Failed(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{
        ExportCancellation, ExportRequest, ExportSettings, ExportSize, ExportStage,
        png_destination, run, start_request,
    };
    use rusttable_core::PhotoId;
    use rusttable_export::CollisionPolicy;

    static TEST_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let sequence = TEST_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "rusttable-app-export-{}-{sequence}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("test export directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn adds_the_png_extension_when_the_picker_destination_omits_it() {
        assert_eq!(
            png_destination(Path::new("/tmp/rendered-copy")).expect("extension is appended"),
            Path::new("/tmp/rendered-copy.png")
        );
    }

    #[test]
    fn rejects_a_conflicting_destination_extension() {
        assert!(png_destination(Path::new("/tmp/rendered-copy.jpg")).is_err());
    }

    #[test]
    fn request_keeps_the_selected_size_and_collision_policy_immutable() {
        let request = ExportRequest::new(
            PathBuf::from("catalog.redb"),
            PathBuf::from("sources"),
            PhotoId::new(7).expect("non-zero photo ID"),
            PathBuf::from("copy.png"),
            ExportSettings::new(ExportSize::Fit4096, CollisionPolicy::ReplaceExisting),
        );

        assert_eq!(
            request.photo_id(),
            PhotoId::new(7).expect("non-zero photo ID")
        );
        assert_eq!(request.destination(), Path::new("copy.png"));
        assert_eq!(request.settings().size(), ExportSize::Fit4096);
        assert_eq!(
            request.settings().collision(),
            CollisionPolicy::ReplaceExisting
        );
        assert_eq!(ExportSize::Fit2048.max_edge(), 2_048);
        assert_eq!(ExportSize::Fit4096.max_edge(), 4_096);
    }

    #[test]
    fn export_task_keeps_a_cancellation_handle_for_its_immutable_request() {
        let task = start_request(ExportRequest::new(
            PathBuf::from("catalog.redb"),
            PathBuf::from("sources"),
            PhotoId::new(7).expect("non-zero photo ID"),
            PathBuf::from("copy.png"),
            ExportSettings::original(),
        ));
        let cancellation = task.cancellation();

        assert!(!cancellation.is_cancelled());
        cancellation.cancel();
        assert!(task.cancellation().is_cancelled());
    }

    #[test]
    fn cancellation_before_work_reports_preparing_then_cancelled() {
        let cancellation = ExportCancellation::default();
        cancellation.cancel();
        let request = ExportRequest::new(
            PathBuf::from("catalog.redb"),
            PathBuf::from("sources"),
            PhotoId::new(7).expect("non-zero photo ID"),
            PathBuf::from("copy.png"),
            ExportSettings::original(),
        );
        let mut stages = Vec::new();

        let result = run(&request, &cancellation, &mut |status| {
            assert_eq!(status.text(), status.stage().label());
            stages.push(status.stage());
        });

        assert!(result.is_err());
        assert_eq!(stages, vec![ExportStage::Preparing, ExportStage::Cancelled]);
    }

    #[test]
    fn selected_imported_edit_renders_and_saves_a_verified_png() {
        let directory = TestDirectory::new();
        let catalog = directory.0.join("catalog.redb");
        let source_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("workspace root");
        let source = source_root.join("fixtures/corpus/assets/raster-png-16-alpha.png");
        let batch = crate::workspace::run_raster_import(
            &catalog,
            vec![source],
            &rusttable_import::RasterImportCancellation::default(),
            &|_| {},
        );
        let receipt = batch.receipts().next().expect("one fixture receipt");
        let photo_id = receipt.photo_id.expect("fixture import photo");
        let destination = directory.0.join("rendered.png");

        let request = ExportRequest::new(
            catalog,
            source_root,
            photo_id,
            destination.clone(),
            ExportSettings::original(),
        );
        let cancellation = ExportCancellation::default();
        let mut stages = Vec::new();
        let status = run(&request, &cancellation, &mut |status| {
            stages.push(status.stage());
        })
        .expect("selected edit is rendered and saved");

        assert!(status.starts_with("Saved rendered.png (4×3,"));
        assert!(destination.is_file());
        assert_eq!(
            stages,
            vec![
                ExportStage::Preparing,
                ExportStage::Rendering,
                ExportStage::Publishing,
                ExportStage::Completed,
            ]
        );
    }
}
