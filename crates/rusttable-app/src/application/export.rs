use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use iced::Task;
use rusttable_core::PhotoId;
use rusttable_export::{
    CollisionPolicy, PngCollisionResult, PngExportLimits, PngPublishError, PngPublisher,
};
use rusttable_render::{PreviewBounds, RenderTarget};
use rusttable_ui::ExportSize as UiExportSize;

use crate::workspace::load_selected_export_render;

use super::Message;

const MAX_OUTPUT_EDGE: u32 = 16_384;
const MAX_OUTPUT_BYTES: u64 = 512 * 1024 * 1024;

/// The requested output size for a single rendered-copy export.
///
/// Fit choices are immutable request values, so a later UI change cannot
/// change a running export request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExportSize {
    Original,
    FitMaximum(u32),
}

impl ExportSize {
    #[must_use]
    const fn max_edge(self) -> u32 {
        match self {
            Self::Original => MAX_OUTPUT_EDGE,
            Self::FitMaximum(maximum_edge) => maximum_edge,
        }
    }

    fn render_target(self) -> RenderTarget {
        match self {
            Self::Original => RenderTarget::FullResolution,
            Self::FitMaximum(maximum_edge) => RenderTarget::PreviewFit(
                PreviewBounds::new(maximum_edge, maximum_edge)
                    .expect("constant export fit bounds are valid"),
            ),
        }
    }
}

/// A typed size choice supplied by the Iced export surface.
///
/// Existing preset controls map directly through [`Self::from_ui`]. A future
/// custom-size input must construct [`Self::custom_maximum`] first, so app
/// orchestration never receives a zero or unbounded maximum edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExportSizeSelection {
    Original,
    Fit2048,
    Fit4096,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the future custom-size control enters through custom_maximum"
        )
    )]
    CustomMaximum(CustomMaximumEdge),
}

/// A validated custom fit maximum captured by an export selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CustomMaximumEdge(u32);

impl ExportSizeSelection {
    #[must_use]
    pub(crate) const fn from_ui(size: UiExportSize) -> Self {
        match size {
            UiExportSize::Original => Self::Original,
            UiExportSize::Fit2048 => Self::Fit2048,
            UiExportSize::Fit4096 => Self::Fit4096,
        }
    }

    /// Builds a bounded custom fit maximum for a future Iced custom control.
    ///
    /// # Errors
    ///
    /// Returns an error when the maximum is outside the export contract's
    /// inclusive `1..=16384` range.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the future custom-size control enters through this validator"
        )
    )]
    pub(crate) const fn custom_maximum(maximum_edge: u32) -> Result<Self, ExportSizeError> {
        if maximum_edge == 0 || maximum_edge > MAX_OUTPUT_EDGE {
            return Err(ExportSizeError::MaximumOutOfRange { maximum_edge });
        }
        Ok(Self::CustomMaximum(CustomMaximumEdge(maximum_edge)))
    }

    fn into_size(self) -> ExportSize {
        match self {
            Self::Original => ExportSize::Original,
            Self::Fit2048 => ExportSize::FitMaximum(2_048),
            Self::Fit4096 => ExportSize::FitMaximum(4_096),
            Self::CustomMaximum(CustomMaximumEdge(maximum_edge)) => {
                ExportSize::FitMaximum(maximum_edge)
            }
        }
    }
}

/// Rejects custom export limits outside the bounded PNG save contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the future custom-size control receives this validation error"
    )
)]
pub(crate) enum ExportSizeError {
    MaximumOutOfRange { maximum_edge: u32 },
}

impl std::fmt::Display for ExportSizeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MaximumOutOfRange { maximum_edge } => write!(
                formatter,
                "custom export maximum {maximum_edge} is outside 1..={MAX_OUTPUT_EDGE}"
            ),
        }
    }
}

impl std::error::Error for ExportSizeError {}

/// The collision action selected by the Iced export flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExportCollisionSelection {
    CreateNew,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the Iced collision dialog will select this after a target-exists status"
        )
    )]
    ReplaceExisting,
}

impl ExportCollisionSelection {
    #[must_use]
    const fn policy(self) -> CollisionPolicy {
        match self {
            Self::CreateNew => CollisionPolicy::CreateNew,
            Self::ReplaceExisting => CollisionPolicy::ReplaceExisting,
        }
    }
}

/// The user-visible collision state without exposing a filesystem path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExportCollisionStatus {
    AwaitingSelection,
    CreatedNew,
    ReplacedExisting,
}

impl ExportCollisionStatus {
    #[must_use]
    pub(crate) const fn text(self) -> &'static str {
        match self {
            Self::AwaitingSelection => {
                "A PNG already exists there. Choose another destination or replace it."
            }
            Self::CreatedNew => "Created a new PNG.",
            Self::ReplacedExisting => "Replaced the existing PNG.",
        }
    }
}

/// Immutable controls captured before an export task begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExportSettings {
    size: ExportSize,
    collision: ExportCollisionSelection,
}

impl ExportSettings {
    #[must_use]
    #[cfg(test)]
    pub(crate) const fn original() -> Self {
        Self {
            size: ExportSize::Original,
            collision: ExportCollisionSelection::CreateNew,
        }
    }

    #[must_use]
    pub(crate) fn from_selection(
        size: ExportSizeSelection,
        collision: ExportCollisionSelection,
    ) -> Self {
        Self {
            size: size.into_size(),
            collision,
        }
    }

    #[must_use]
    pub(crate) const fn size(self) -> ExportSize {
        self.size
    }

    #[must_use]
    pub(crate) const fn collision(self) -> ExportCollisionSelection {
        self.collision
    }

    #[must_use]
    pub(crate) const fn with_collision(self, collision: ExportCollisionSelection) -> Self {
        Self {
            size: self.size,
            collision,
        }
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
    pub(crate) const fn settings(&self) -> ExportSettings {
        self.settings
    }

    #[must_use]
    pub(crate) fn with_collision(&self, collision: ExportCollisionSelection) -> Self {
        Self {
            catalog_path: self.catalog_path.clone(),
            source_root: self.source_root.clone(),
            photo_id: self.photo_id,
            destination: self.destination.clone(),
            settings: self.settings.with_collision(collision),
        }
    }
}

/// Cooperative cancellation shared by an Iced export task and its owner.
#[derive(Debug, Clone, Default)]
pub(crate) struct ExportCancellation(Arc<AtomicBool>);

impl PartialEq for ExportCancellation {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for ExportCancellation {}

impl ExportCancellation {
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
    pub(crate) fn at(stage: ExportStage) -> Self {
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
    pub(crate) fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExportTaskResult {
    Completed {
        summary: String,
        collision: ExportCollisionStatus,
    },
    Collision(ExportCollisionStatus),
    Failed(String),
}

/// Owns a cancellation handle and the associated Iced task.
///
/// The application retains the handle when it adds the export progress UI;
/// callers that only need the existing one-shot action can consume the task.
#[derive(Debug)]
pub(crate) struct ExportTask {
    cancellation: ExportCancellation,
    task: Task<Message>,
}

impl ExportTask {
    #[must_use]
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
    let sipper = iced::task::sipper(move |sender| async move {
        let (finished, receiver) = iced::futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let sender = std::sync::Mutex::new(sender);
            let mut report_status = |status| {
                let mut sender = sender
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                iced::futures::executor::block_on(sender.send(status));
            };
            let result =
                ExportTaskResult::from(run(&request, &task_cancellation, &mut report_status));
            let _ = finished.send(result);
        });
        receiver.await.ok()
    });
    let task = Task::sip(
        sipper,
        move |status| Message::ExportStatus { photo_id, status },
        move |result| Message::ExportFinished {
            photo_id,
            result: result.unwrap_or_else(|| {
                ExportTaskResult::Failed("Export task ended before reporting a result.".to_owned())
            }),
        },
    );
    ExportTask { cancellation, task }
}

fn run(
    request: &ExportRequest,
    cancellation: &ExportCancellation,
    report_status: &mut dyn FnMut(ExportStatus),
) -> Result<ExportCompletion, ExportRunError> {
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
) -> Result<ExportCompletion, ExportRunError> {
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
        .publish(
            render.image(),
            &destination,
            request.settings.collision().policy(),
        )
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
    Ok(ExportCompletion {
        summary: format!(
            "{completion} {alias} ({}×{}, {} bytes)",
            receipt.dimensions().width(),
            receipt.dimensions().height(),
            receipt.encoded_byte_length()
        ),
        collision: match receipt.collision() {
            PngCollisionResult::CreatedNew => ExportCollisionStatus::CreatedNew,
            PngCollisionResult::ReplacedExisting => ExportCollisionStatus::ReplacedExisting,
        },
    })
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

#[derive(Debug)]
struct ExportCompletion {
    summary: String,
    collision: ExportCollisionStatus,
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

impl From<Result<ExportCompletion, ExportRunError>> for ExportTaskResult {
    fn from(result: Result<ExportCompletion, ExportRunError>) -> Self {
        match result {
            Ok(completion) => Self::Completed {
                summary: completion.summary,
                collision: completion.collision,
            },
            Err(ExportRunError::Publish(PngPublishError::DestinationExists { .. })) => {
                Self::Collision(ExportCollisionStatus::AwaitingSelection)
            }
            Err(error) => Self::Failed(error.to_string()),
        }
    }
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
