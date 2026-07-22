//! GTK-facing preview state and the production selected-photo preview adapter.
//!
//! This module deliberately contains no widget or toolkit types. GTK can project the returned
//! state onto a texture while the application keeps catalog access, CPU rendering, and failure
//! redaction at this boundary.

use rusttable_catalog_store::RedbCatalogRepository;
use rusttable_core::PhotoId;
use rusttable_display_profile::DisplayProfileSnapshot;
use rusttable_image::ImageDimensions;
use rusttable_ui::PresentationStatus;

use crate::CatalogPreviewError;
use crate::CatalogPreviewReceipt;
use crate::composition::selected_preview::presentation::{
    PresentationReceipt, PresentedPreview, present,
};
use crate::diagnostics::AppDiagnostics;
use crate::gtk_controller::{GtkCatalogController, GtkCatalogState};
use crate::workspace::preview_loader::WorkspacePreviewError;
use crate::workspace::{
    SelectedPreview,
    preview_loader::{
        load_selected_preview_from_repository_for_edit_with_generation,
        load_selected_preview_with_generation,
    },
};

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
        Self::render_selected_with_diagnostics(catalog, &AppDiagnostics::default())
    }

    pub(crate) fn render_selected_with_diagnostics(
        catalog: &GtkCatalogController,
        diagnostics: &AppDiagnostics,
    ) -> GtkPreviewState {
        Self::render_selected_with_generation(catalog, diagnostics, 0, None)
    }

    pub(crate) fn render_selected_with_generation(
        catalog: &GtkCatalogController,
        diagnostics: &AppDiagnostics,
        generation: u64,
        display_profile: Option<&DisplayProfileSnapshot>,
    ) -> GtkPreviewState {
        let Some(photo_id) = catalog.selected_photo() else {
            return GtkPreviewState::failed(None, GtkPreviewFailureKind::NoSelection);
        };

        let GtkCatalogState::Ready(ready) = catalog.state() else {
            diagnostics.preview_failure(
                "render_selected",
                "catalog_lookup",
                "catalog_unavailable",
                Some(photo_id),
                None,
                None,
                None,
                None,
            );
            return GtkPreviewState::failed(
                Some(photo_id),
                GtkPreviewFailureKind::CatalogUnavailable,
            );
        };

        let result = load_selected_preview_with_generation(
            ready.location().catalog_path(),
            ready.location().source_root(),
            photo_id,
            generation,
        );
        match result {
            Ok(preview) => Self::from_loaded_preview(preview, display_profile, diagnostics),
            Err(error) => {
                let kind = GtkPreviewFailureKind::from_workspace_error(&error);
                diagnostics.preview_workspace_failure(
                    "render_selected",
                    kind.stage(),
                    kind.cause(),
                    Some(photo_id),
                    None,
                    None,
                    &error,
                );
                GtkPreviewState::failed(Some(photo_id), kind)
            }
        }
    }

    /// Renders the exact edit identity captured by the application publication coordinator.
    pub(crate) fn render_selected_with_generation_for_edit(
        catalog: &GtkCatalogController,
        diagnostics: &AppDiagnostics,
        edit_id: rusttable_core::EditId,
        edit_revision: rusttable_core::Revision,
        generation: u64,
        display_profile: Option<&DisplayProfileSnapshot>,
    ) -> GtkPreviewState {
        let Some(photo_id) = catalog.selected_photo() else {
            return GtkPreviewState::failed(None, GtkPreviewFailureKind::NoSelection);
        };
        let GtkCatalogState::Ready(ready) = catalog.state() else {
            diagnostics.preview_failure(
                "render_selected",
                "catalog_lookup",
                "catalog_unavailable",
                Some(photo_id),
                Some(edit_id),
                Some(generation),
                None,
                None,
            );
            return GtkPreviewState::failed(
                Some(photo_id),
                GtkPreviewFailureKind::CatalogUnavailable,
            );
        };
        let result = RedbCatalogRepository::open(ready.location().catalog_path())
            .map_err(WorkspacePreviewError::Catalog)
            .and_then(|repository| {
                load_selected_preview_from_repository_for_edit_with_generation(
                    &repository,
                    ready.location().source_root(),
                    photo_id,
                    edit_id,
                    edit_revision,
                    generation,
                )
            });
        match result {
            Ok(preview) => Self::from_loaded_preview(preview, display_profile, diagnostics),
            Err(error) => {
                let kind = GtkPreviewFailureKind::from_workspace_error(&error);
                diagnostics.preview_workspace_failure(
                    "render_selected",
                    kind.stage(),
                    kind.cause(),
                    Some(photo_id),
                    Some(edit_id),
                    Some(generation),
                    &error,
                );
                GtkPreviewState::failed(Some(photo_id), kind)
            }
        }
    }

    fn from_loaded_preview(
        preview: SelectedPreview,
        display_profile: Option<&DisplayProfileSnapshot>,
        diagnostics: &AppDiagnostics,
    ) -> GtkPreviewState {
        let presented = match present(
            preview,
            display_profile,
            rusttable_color::RenderingIntent::Relative,
        ) {
            Ok(presented) => presented,
            Err(error) => {
                diagnostics.preview_failure(
                    "render_selected",
                    "display_presentation",
                    "transform_unavailable",
                    None,
                    None,
                    None,
                    None,
                    None,
                );
                tracing::error!(
                    target: "rusttable.gtk.preview",
                    cause = ?error,
                    "selected preview presentation failed"
                );
                return GtkPreviewState::failed(None, GtkPreviewFailureKind::RenderUnavailable);
            }
        };
        let fallback = presented.presentation_receipt().fallback();
        if let Some(reason) = fallback {
            diagnostics.preview_fallback(
                "render_selected",
                reason.label(),
                Some(presented.presentation_receipt().generation()),
            );
        }
        GtkPreview::from_presented(&presented).map_or_else(
            |kind| {
                diagnostics.preview_failure(
                    "render_selected",
                    "texture",
                    kind.cause(),
                    None,
                    None,
                    None,
                    None,
                    None,
                );
                GtkPreviewState::failed(None, kind)
            },
            GtkPreviewState::Ready,
        )
    }
}

/// Complete state that a GTK view can render without knowing about the catalog or renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::large_enum_variant,
    reason = "the ready state owns validated pixels and GTK publishes the enum by value"
)]
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
    status: PresentationStatus,
    receipt: Option<CatalogPreviewReceipt>,
    presentation: Option<PresentationReceipt>,
}

impl GtkPreview {
    #[cfg(test)]
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
            status: PresentationStatus::Ready {
                mode: rusttable_ui::PresentationMode::Sdr,
                profile: rusttable_display_profile::ProfileSelection::Unprofiled,
            },
            receipt: None,
            presentation: None,
        })
    }

    fn from_presented(preview: &PresentedPreview) -> Result<Self, GtkPreviewFailureKind> {
        let expected = preview
            .dimensions()
            .decoded_byte_count()
            .map_err(|_| GtkPreviewFailureKind::InvalidRgba8)?;
        let actual = u64::try_from(preview.pixels().len())
            .map_err(|_| GtkPreviewFailureKind::InvalidRgba8)?;
        if actual != expected {
            return Err(GtkPreviewFailureKind::InvalidRgba8);
        }
        Ok(Self {
            photo_id: preview.photo_id(),
            dimensions: preview.dimensions(),
            pixels: preview.pixels().to_owned(),
            status: preview.status(),
            receipt: Some(preview.receipt().clone()),
            presentation: Some(preview.presentation_receipt().clone()),
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
    pub const fn receipt(&self) -> Option<&CatalogPreviewReceipt> {
        self.receipt.as_ref()
    }

    #[must_use]
    pub(crate) const fn presentation_receipt(&self) -> Option<&PresentationReceipt> {
        self.presentation.as_ref()
    }

    #[must_use]
    pub(crate) const fn presentation_status(&self) -> PresentationStatus {
        self.status
    }
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
    DecodeUnavailable,
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
            Self::DecodeUnavailable => "The selected photo could not be decoded.",
            Self::RenderUnavailable => "The selected preview could not be rendered.",
            Self::InvalidRgba8 => "The selected preview returned invalid image data.",
        }
    }

    pub(crate) const fn stage(self) -> &'static str {
        match self {
            Self::NoSelection | Self::CatalogUnavailable => "catalog_lookup",
            Self::MissingPersistedEdit => "edit_resolution",
            Self::DecodeUnavailable => "decode",
            Self::RenderUnavailable => "processing",
            Self::InvalidRgba8 => "texture",
        }
    }

    pub(crate) const fn cause(self) -> &'static str {
        match self {
            Self::NoSelection => "no_selection",
            Self::CatalogUnavailable => "catalog_unavailable",
            Self::MissingPersistedEdit => "missing_persisted_edit",
            Self::DecodeUnavailable => "decode_unavailable",
            Self::RenderUnavailable => "render_unavailable",
            Self::InvalidRgba8 => "invalid_rgba8",
        }
    }

    const fn from_workspace_error(error: &WorkspacePreviewError) -> Self {
        match error {
            WorkspacePreviewError::Catalog(_) => Self::CatalogUnavailable,
            WorkspacePreviewError::MissingEdit { .. } => Self::MissingPersistedEdit,
            WorkspacePreviewError::Preview(error) => match error {
                CatalogPreviewError::Preview(preview) => match preview {
                    crate::PreviewError::Decode(_) | crate::PreviewError::RawDecode(_) => {
                        Self::DecodeUnavailable
                    }
                    crate::PreviewError::DecodedFrame
                    | crate::PreviewError::Render(_)
                    | crate::PreviewError::UnsupportedPixelpipeColor { .. }
                    | crate::PreviewError::PixelpipeInput(_)
                    | crate::PreviewError::PixelpipeSnapshot(_)
                    | crate::PreviewError::Graph(_)
                    | crate::PreviewError::RawPipeline(_)
                    | crate::PreviewError::Pixelpipe(_)
                    | crate::PreviewError::Prepared(_) => Self::RenderUnavailable,
                },
                CatalogPreviewError::ImportRepository(_)
                | CatalogPreviewError::EditRepository(_)
                | CatalogPreviewError::UnknownPhoto { .. }
                | CatalogPreviewError::UnknownEdit { .. }
                | CatalogPreviewError::EditPhotoMismatch { .. }
                | CatalogPreviewError::Snapshot(_)
                | CatalogPreviewError::SnapshotRead(_)
                | CatalogPreviewError::SourceLimits => Self::CatalogUnavailable,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use rusttable_core::PhotoId;
    use rusttable_display_profile::{
        DisplayProfileService, DisplayProvider, HdrDescriptor, MonitorDescriptor, MonitorGeometry,
        MonitorId, ProfileProbe, ProviderMonitor,
    };
    use rusttable_image::ImageInputError;
    use rusttable_import::RasterImportCancellation;
    use rusttable_ui::{PresentationStatus, SdrFallbackReason};

    use super::{GtkPreview, GtkPreviewController, GtkPreviewFailureKind, GtkPreviewState};
    use crate::workspace::preview_loader::WorkspacePreviewError;
    use crate::workspace::{load_selected_preview, run_raster_import};
    use crate::{CatalogPreviewError, PreviewError};

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

    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    fn matrix_profile(red: f32) -> Vec<u8> {
        let tag_count = 4_usize;
        let table_size = 4 + tag_count * 12;
        let profile_size = 128 + table_size + 4 * 20;
        let mut bytes = vec![0_u8; profile_size];
        bytes[0..4].copy_from_slice(&(profile_size as u32).to_be_bytes());
        bytes[8..12].copy_from_slice(b"mntr");
        bytes[12..16].copy_from_slice(b"RGB ");
        bytes[16..20].copy_from_slice(b"RGB ");
        bytes[20..24].copy_from_slice(b"XYZ ");
        bytes[36..40].copy_from_slice(b"acsp");
        bytes[128..132].copy_from_slice(&(tag_count as u32).to_be_bytes());
        for (index, (name, values)) in [
            (b"rXYZ", [red, 0.22897, 0.0]),
            (b"gXYZ", [0.26567, 0.69174, 0.04511]),
            (b"bXYZ", [0.19822, 0.07929, 1.04394]),
            (b"wtpt", [0.95047, 1.0, 1.08883]),
        ]
        .into_iter()
        .enumerate()
        {
            let table = 132 + index * 12;
            let offset = 128 + table_size + index * 20;
            bytes[table..table + 4].copy_from_slice(name);
            bytes[table + 4..table + 8].copy_from_slice(&(offset as u32).to_be_bytes());
            bytes[table + 8..table + 12].copy_from_slice(&20_u32.to_be_bytes());
            bytes[offset..offset + 4].copy_from_slice(b"XYZ ");
            for (channel, value) in values.into_iter().enumerate() {
                bytes[offset + 8 + channel * 4..offset + 12 + channel * 4]
                    .copy_from_slice(&((value * 65_536.0) as i32).to_be_bytes());
            }
        }
        bytes
    }

    fn monitor_descriptor(alias: &str) -> MonitorDescriptor {
        MonitorDescriptor::new(
            MonitorId::from_platform_parts(
                "test",
                Some("DP-1"),
                Some("maker"),
                Some("model"),
                None,
            ),
            alias,
            MonitorGeometry::new(0, 0, 1920, 1080, 1).expect("geometry"),
            HdrDescriptor {
                supported: false,
                active: false,
            },
        )
        .expect("descriptor")
    }

    #[test]
    fn workspace_preview_errors_keep_decode_and_processing_stages_distinct() {
        let decode = WorkspacePreviewError::Preview(CatalogPreviewError::Preview(
            PreviewError::Decode(ImageInputError::ArithmeticOverflow),
        ));
        assert_eq!(
            GtkPreviewFailureKind::from_workspace_error(&decode),
            GtkPreviewFailureKind::DecodeUnavailable
        );

        let processing = WorkspacePreviewError::Preview(CatalogPreviewError::Preview(
            PreviewError::UnsupportedPixelpipeColor {
                actual: rusttable_image::ColorEncoding::DisplayP3D65,
            },
        ));
        assert_eq!(
            GtkPreviewFailureKind::from_workspace_error(&processing),
            GtkPreviewFailureKind::RenderUnavailable
        );
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
            "../../../../rusttable-image-io/tests/fixtures/rgba-2x1.png.b64"
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
        let state = GtkPreviewController::from_loaded_preview(
            loaded,
            None,
            &crate::diagnostics::AppDiagnostics::default(),
        );

        let GtkPreviewState::Ready(preview) = state else {
            panic!("fixture preview should be ready");
        };
        assert_eq!(preview.photo_id(), selected);
        assert_eq!(preview.dimensions().width(), 2);
        assert_eq!(preview.dimensions().height(), 1);
        assert_eq!(preview.pixels().len(), 8);
        assert_eq!(
            preview.presentation_status(),
            PresentationStatus::SrgbFallback(SdrFallbackReason::MissingMonitorProfile)
        );
        let presentation = preview
            .presentation_receipt()
            .expect("presentation receipt");
        let _ = (
            presentation.scene_identity(),
            presentation.edit_identity(),
            presentation.presentation_identity(),
            presentation.monitor(),
            presentation.profile_id(),
            presentation.generation(),
            presentation.intent(),
        );
    }

    #[test]
    fn active_monitor_transform_changes_only_presentation_identity() {
        let directory = TestDirectory::new();
        let source = directory.0.join("selected.png");
        let catalog = directory.0.join("catalog.redb");
        fs::write(
            &source,
            decode_base64(include_str!(
                "../../../../rusttable-image-io/tests/fixtures/rgba-2x1.png.b64"
            )),
        )
        .expect("fixture source");
        let batch = run_raster_import(
            &catalog,
            vec![source],
            &RasterImportCancellation::default(),
            &|_| {},
        );
        let selected = batch.first_selected_photo().expect("fixture import photo");
        let mut catalog_controller =
            crate::gtk_controller::GtkCatalogController::load_catalog_at(catalog.clone());
        assert!(catalog_controller.select_photo(selected));
        let descriptor = monitor_descriptor("P3 active display");
        let monitor_id = descriptor.id();
        let mut profiles = DisplayProfileService::new();
        profiles
            .reconcile([ProviderMonitor::new(
                descriptor.clone(),
                DisplayProvider::Synthetic,
                ProfileProbe::Current(matrix_profile(0.48657)),
            )])
            .expect("first profile");
        let first_profile = profiles.snapshot(monitor_id).expect("first snapshot");
        profiles
            .reconcile([ProviderMonitor::new(
                descriptor,
                DisplayProvider::Synthetic,
                ProfileProbe::Current(matrix_profile(0.49)),
            )])
            .expect("second profile");
        let second_profile = profiles.snapshot(monitor_id).expect("second snapshot");
        let first = GtkPreviewController::render_selected_with_generation(
            &catalog_controller,
            &crate::diagnostics::AppDiagnostics::default(),
            1,
            Some(&first_profile),
        );
        let second = GtkPreviewController::render_selected_with_generation(
            &catalog_controller,
            &crate::diagnostics::AppDiagnostics::default(),
            2,
            Some(&second_profile),
        );
        let GtkPreviewState::Ready(first) = first else {
            panic!("first preview ready")
        };
        let GtkPreviewState::Ready(second) = second else {
            panic!("second preview ready")
        };
        assert_eq!(first.photo_id(), selected);
        assert_ne!(first.pixels(), second.pixels());
        let first_receipt = first.presentation_receipt().expect("first receipt");
        let second_receipt = second.presentation_receipt().expect("second receipt");
        assert_eq!(
            first_receipt.scene_identity(),
            second_receipt.scene_identity()
        );
        assert_eq!(
            first_receipt.edit_identity(),
            second_receipt.edit_identity()
        );
        assert_ne!(
            first_receipt.presentation_identity(),
            second_receipt.presentation_identity()
        );
        assert_ne!(first_receipt.generation(), second_receipt.generation());
        assert!(first_receipt.fallback().is_none());
        assert!(second_receipt.fallback().is_none());
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
