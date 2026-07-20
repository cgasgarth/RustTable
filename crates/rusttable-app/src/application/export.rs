use std::path::{Path, PathBuf};

use iced::Task;
use rusttable_core::PhotoId;
use rusttable_export::{CollisionPolicy, PngExportLimits, PngPublisher};

use crate::workspace::load_selected_export_render;

use super::Message;

const MAX_OUTPUT_EDGE: u32 = 16_384;
const MAX_OUTPUT_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExportTaskResult {
    Completed(String),
    Failed(String),
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
    Task::perform(
        async move { ExportTaskResult::from(run(&catalog_path, &source_root, photo_id, &destination)) },
        move |result| Message::ExportFinished { photo_id, result },
    )
}

fn run(
    catalog_path: &Path,
    source_root: &Path,
    photo_id: PhotoId,
    destination: &Path,
) -> Result<String, ExportRunError> {
    let destination = png_destination(destination)?;
    let render = load_selected_export_render(catalog_path, source_root, photo_id)
        .map_err(ExportRunError::Render)?;
    let limits = PngExportLimits::new(MAX_OUTPUT_EDGE, MAX_OUTPUT_EDGE, MAX_OUTPUT_BYTES)
        .expect("constant PNG export limits are valid");
    let receipt = PngPublisher::new(limits)
        .publish(render.image(), &destination, CollisionPolicy::CreateNew)
        .map_err(ExportRunError::Publish)?;
    let alias = destination.file_name().map_or_else(
        || destination.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    Ok(format!(
        "Saved {alias} ({}×{}, {} bytes)",
        receipt.dimensions().width(),
        receipt.dimensions().height(),
        receipt.encoded_byte_length()
    ))
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
    InvalidExtension(PathBuf),
    Render(crate::workspace::preview_loader::WorkspacePreviewError),
    Publish(rusttable_export::PngPublishError),
}

impl std::fmt::Display for ExportRunError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
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

    use super::{png_destination, run};

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

        let status = run(&catalog, &source_root, photo_id, &destination)
            .expect("selected edit is rendered and saved");

        assert!(status.starts_with("Saved rendered.png (4×3,"));
        assert!(destination.is_file());
    }
}
