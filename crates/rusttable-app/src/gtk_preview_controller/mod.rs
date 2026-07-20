//! GTK-facing preview state and the production selected-photo preview adapter.
//!
//! This module deliberately contains no widget or toolkit types. GTK can project the returned
//! state onto a texture while the application keeps catalog access, CPU rendering, and failure
//! redaction at this boundary.

use rusttable_core::PhotoId;
use rusttable_image::ImageDimensions;

use crate::gtk_controller::{GtkCatalogController, GtkCatalogState};
use crate::workspace::preview_loader::WorkspacePreviewError;
use crate::workspace::{SelectedPreview, load_selected_preview};

/// Stateless adapter for rendering the photo selected by the GTK catalog controller.
#[derive(Debug, Default)]
pub struct GtkPreviewController;

impl GtkPreviewController {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Loads the selected photo through the persisted-edit CPU preview path.
    #[must_use]
    pub fn render_selected(&self, catalog: &GtkCatalogController) -> GtkPreviewState {
        let Some(photo_id) = catalog.selected_photo() else {
            return GtkPreviewState::failed(None, GtkPreviewFailureKind::NoSelection);
        };

        let GtkCatalogState::Ready(ready) = catalog.state() else {
            return GtkPreviewState::failed(
                Some(photo_id),
                GtkPreviewFailureKind::CatalogUnavailable,
            );
        };

        let result = load_selected_preview(
            ready.location().catalog_path(),
            ready.location().source_root(),
            photo_id,
        );
        match result {
            Ok(preview) => Self::from_loaded_preview(preview),
            Err(error) => GtkPreviewState::failed(
                Some(photo_id),
                GtkPreviewFailureKind::from_workspace_error(&error),
            ),
        }
    }

    fn from_loaded_preview(preview: SelectedPreview) -> GtkPreviewState {
        let (photo_id, dimensions, pixels) = preview.into_parts();
        GtkPreview::new(photo_id, dimensions, pixels).map_or_else(
            |kind| GtkPreviewState::failed(Some(photo_id), kind),
            GtkPreviewState::Ready,
        )
    }
}

/// Complete state that a GTK view can render without knowing about the catalog or renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GtkPreviewState {
    Ready(GtkPreview),
    Failed(GtkPreviewFailure),
}

impl GtkPreviewState {
    fn failed(photo_id: Option<PhotoId>, kind: GtkPreviewFailureKind) -> Self {
        Self::Failed(GtkPreviewFailure { photo_id, kind })
    }

    #[must_use]
    pub const fn ready(&self) -> Option<&GtkPreview> {
        match self {
            Self::Ready(preview) => Some(preview),
            Self::Failed(_) => None,
        }
    }

    #[must_use]
    pub const fn failure(&self) -> Option<&GtkPreviewFailure> {
        match self {
            Self::Ready(_) => None,
            Self::Failed(failure) => Some(failure),
        }
    }
}

/// Validated RGBA8 preview pixels and their presentation status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GtkPreview {
    photo_id: PhotoId,
    dimensions: ImageDimensions,
    pixels: Vec<u8>,
    status: GtkPreviewStatus,
}

impl GtkPreview {
    fn new(
        photo_id: PhotoId,
        dimensions: ImageDimensions,
        pixels: Vec<u8>,
    ) -> Result<Self, GtkPreviewFailureKind> {
        let expected = dimensions
            .decoded_byte_count()
            .map_err(|_| GtkPreviewFailureKind::InvalidRgba8)?;
        let actual =
            u64::try_from(pixels.len()).map_err(|_| GtkPreviewFailureKind::InvalidRgba8)?;
        if actual != expected {
            return Err(GtkPreviewFailureKind::InvalidRgba8);
        }
        Ok(Self {
            photo_id,
            dimensions,
            pixels,
            status: GtkPreviewStatus::Rendered,
        })
    }

    #[must_use]
    pub const fn photo_id(&self) -> PhotoId {
        self.photo_id
    }

    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }

    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    #[must_use]
    pub const fn status(&self) -> GtkPreviewStatus {
        self.status
    }
}

/// Status of a validated preview payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GtkPreviewStatus {
    Rendered,
}

/// Bounded failure state safe to display in a GTK surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GtkPreviewFailure {
    photo_id: Option<PhotoId>,
    kind: GtkPreviewFailureKind,
}

impl GtkPreviewFailure {
    #[must_use]
    pub const fn photo_id(&self) -> Option<PhotoId> {
        self.photo_id
    }

    #[must_use]
    pub const fn kind(&self) -> GtkPreviewFailureKind {
        self.kind
    }

    /// Returns fixed copy suitable for direct display.
    #[must_use]
    pub const fn message(&self) -> &'static str {
        self.kind.message()
    }
}

/// Redacted preview failure categories exposed to the GTK layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GtkPreviewFailureKind {
    NoSelection,
    CatalogUnavailable,
    MissingPersistedEdit,
    RenderUnavailable,
    InvalidRgba8,
}

impl GtkPreviewFailureKind {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::NoSelection => "Select a photo to preview it.",
            Self::CatalogUnavailable => "The library is unavailable.",
            Self::MissingPersistedEdit => "The selected photo has no persisted edit.",
            Self::RenderUnavailable => "The selected preview could not be rendered.",
            Self::InvalidRgba8 => "The selected preview returned invalid image data.",
        }
    }

    const fn from_workspace_error(error: &WorkspacePreviewError) -> Self {
        match error {
            WorkspacePreviewError::Catalog(_) => Self::CatalogUnavailable,
            WorkspacePreviewError::MissingEdit { .. } => Self::MissingPersistedEdit,
            WorkspacePreviewError::Preview(_) => Self::RenderUnavailable,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use rusttable_core::PhotoId;
    use rusttable_import::RasterImportCancellation;

    use super::{
        GtkPreview, GtkPreviewController, GtkPreviewFailureKind, GtkPreviewState, GtkPreviewStatus,
    };
    use crate::workspace::{load_selected_preview, run_raster_import};

    static TEST_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let number = TEST_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "rusttable-app-gtk-preview-{}-{number}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("temporary GTK preview directory");
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

    fn decode_base64(value: &str) -> Vec<u8> {
        let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut output = Vec::new();
        let mut quartet = [0_u8; 4];
        let mut length = 0;
        for byte in value.bytes().filter(|byte| !byte.is_ascii_whitespace()) {
            if byte == b'=' {
                break;
            }
            quartet[length] = u8::try_from(
                alphabet
                    .iter()
                    .position(|candidate| *candidate == byte)
                    .expect("fixture base64 character"),
            )
            .expect("base64 alphabet index fits in a byte");
            length += 1;
            if length == quartet.len() {
                output.push((quartet[0] << 2) | (quartet[1] >> 4));
                output.push((quartet[1] << 4) | (quartet[2] >> 2));
                output.push((quartet[2] << 6) | quartet[3]);
                length = 0;
            }
        }
        if length >= 2 {
            output.push((quartet[0] << 2) | (quartet[1] >> 4));
        }
        if length >= 3 {
            output.push((quartet[1] << 4) | (quartet[2] >> 2));
        }
        output
    }

    #[test]
    fn production_cpu_preview_becomes_validated_gtk_state() {
        let directory = TestDirectory::new();
        let source = directory.0.join("selected.png");
        let catalog = directory.0.join("catalog.redb");
        let bytes = decode_base64(include_str!(
            "../../../rusttable-image-io/tests/fixtures/rgba-2x1.png.b64"
        ));
        fs::write(&source, bytes).expect("fixture source");

        let batch = run_raster_import(
            &catalog,
            vec![source],
            &RasterImportCancellation::default(),
            &|_| {},
        );
        let selected = batch.first_selected_photo().expect("fixture import photo");
        let loaded = load_selected_preview(&catalog, &directory.0, selected)
            .expect("production CPU preview");
        let state = GtkPreviewController::from_loaded_preview(loaded);

        let GtkPreviewState::Ready(preview) = state else {
            panic!("fixture preview should be ready");
        };
        assert_eq!(preview.photo_id(), selected);
        assert_eq!(preview.dimensions().width(), 2);
        assert_eq!(preview.dimensions().height(), 1);
        assert_eq!(preview.pixels().len(), 8);
        assert_eq!(preview.status(), GtkPreviewStatus::Rendered);
    }

    #[test]
    fn invalid_rgba8_payload_becomes_a_safe_failure() {
        let dimensions = rusttable_image::ImageDimensions::new(2, 1).expect("dimensions");
        let state = GtkPreview::new(photo_id(7), dimensions, vec![0; 7]).map_or_else(
            |kind| GtkPreviewState::failed(Some(photo_id(7)), kind),
            GtkPreviewState::Ready,
        );

        let failure = state.failure().expect("invalid payload failure");
        assert_eq!(failure.photo_id(), Some(photo_id(7)));
        assert_eq!(failure.kind(), GtkPreviewFailureKind::InvalidRgba8);
        assert_eq!(
            failure.message(),
            "The selected preview returned invalid image data."
        );
    }

    #[test]
    fn failure_messages_do_not_include_internal_error_details() {
        for kind in [
            GtkPreviewFailureKind::CatalogUnavailable,
            GtkPreviewFailureKind::MissingPersistedEdit,
            GtkPreviewFailureKind::RenderUnavailable,
        ] {
            let state = GtkPreviewState::failed(Some(photo_id(9)), kind);
            let message = state.failure().expect("failure").message();
            assert!(!message.contains('/'));
            assert!(!message.contains("redb"));
            assert!(!message.contains("source"));
        }
    }
}
