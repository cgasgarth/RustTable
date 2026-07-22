use crate::event::DiagnosticsError;
use crate::privacy::DiagnosticField;

/// The bounded stages at which a selected preview can fail.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SelectedPreviewFailureStage {
    CatalogLookup,
    EditSelection,
    SourceDecode,
    Processing,
    HistogramGeneration,
    TextureAdaptation,
    ImportPreview,
    StaleResult,
}

impl SelectedPreviewFailureStage {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CatalogLookup => "catalog_lookup",
            Self::EditSelection => "edit_selection",
            Self::SourceDecode => "source_decode",
            Self::Processing => "processing",
            Self::HistogramGeneration => "histogram_generation",
            Self::TextureAdaptation => "texture_adaptation",
            Self::ImportPreview => "import_preview",
            Self::StaleResult => "stale_result",
        }
    }
}

impl std::fmt::Display for SelectedPreviewFailureStage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Stable, typed causes for selected-preview failures.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SelectedPreviewFailureCode {
    NoSelection,
    CatalogUnavailable,
    PhotoNotFound,
    EditUnavailable,
    EditNotFound,
    SourceUnavailable,
    UnsupportedFormat,
    MalformedSource,
    SourceDimensionsInvalid,
    DecodeFailed,
    ProcessingFailed,
    NonFiniteOutput,
    ResourceUnavailable,
    HistogramFailed,
    HistogramDimensionsInvalid,
    TextureAdaptationFailed,
    InvalidRgba8,
    ImportPreviewFailed,
    WorkerUnavailable,
    ChannelDisconnected,
    StaleGeneration,
    Cancelled,
}

impl SelectedPreviewFailureCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoSelection => "no_selection",
            Self::CatalogUnavailable => "catalog_unavailable",
            Self::PhotoNotFound => "photo_not_found",
            Self::EditUnavailable => "edit_unavailable",
            Self::EditNotFound => "edit_not_found",
            Self::SourceUnavailable => "source_unavailable",
            Self::UnsupportedFormat => "unsupported_format",
            Self::MalformedSource => "malformed_source",
            Self::SourceDimensionsInvalid => "source_dimensions_invalid",
            Self::DecodeFailed => "decode_failed",
            Self::ProcessingFailed => "processing_failed",
            Self::NonFiniteOutput => "non_finite_output",
            Self::ResourceUnavailable => "resource_unavailable",
            Self::HistogramFailed => "histogram_failed",
            Self::HistogramDimensionsInvalid => "histogram_dimensions_invalid",
            Self::TextureAdaptationFailed => "texture_adaptation_failed",
            Self::InvalidRgba8 => "invalid_rgba8",
            Self::ImportPreviewFailed => "import_preview_failed",
            Self::WorkerUnavailable => "worker_unavailable",
            Self::ChannelDisconnected => "channel_disconnected",
            Self::StaleGeneration => "stale_generation",
            Self::Cancelled => "cancelled",
        }
    }
}

impl std::fmt::Display for SelectedPreviewFailureCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Stable operation names for selected-preview instrumentation.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SelectedPreviewOperation {
    Select,
    LookupCatalog,
    SelectEdit,
    DecodeSource,
    Process,
    GenerateHistogram,
    AdaptTexture,
    LoadImportPreview,
    Publish,
    DiscardStaleResult,
}

impl SelectedPreviewOperation {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Select => "select",
            Self::LookupCatalog => "lookup_catalog",
            Self::SelectEdit => "select_edit",
            Self::DecodeSource => "decode_source",
            Self::Process => "process",
            Self::GenerateHistogram => "generate_histogram",
            Self::AdaptTexture => "adapt_texture",
            Self::LoadImportPreview => "load_import_preview",
            Self::Publish => "publish",
            Self::DiscardStaleResult => "discard_stale_result",
        }
    }
}

impl std::fmt::Display for SelectedPreviewOperation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Safe, bounded metadata allowed on a selected-preview failure.
///
/// This type deliberately has no free-form text or byte/payload constructor. Private
/// identifiers belong in [`CorrelationContext`](crate::CorrelationContext), where they are
/// converted to process-local aliases before serialization.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SelectedPreviewMetadata {
    fields: Vec<DiagnosticField>,
}

const MAX_SELECTED_PREVIEW_METADATA_FIELDS: usize = 8;

impl SelectedPreviewMetadata {
    /// Adds the preview generation that produced the failure.
    #[must_use]
    pub fn with_generation(mut self, generation: u64) -> Self {
        self.push_unsigned("generation", generation);
        self
    }

    /// Adds the generation currently expected by the presentation boundary.
    #[must_use]
    pub fn with_expected_generation(mut self, generation: u64) -> Self {
        self.push_unsigned("expected_generation", generation);
        self
    }

    /// Adds bounded, non-zero preview dimensions.
    ///
    /// # Errors
    ///
    /// Returns an error when either dimension is zero.
    pub fn with_dimensions(mut self, width: u32, height: u32) -> Result<Self, DiagnosticsError> {
        if width == 0 || height == 0 {
            return Err(DiagnosticsError::InvalidIdentifier("preview dimensions"));
        }
        self.push_unsigned("width", u64::from(width));
        self.push_unsigned("height", u64::from(height));
        Ok(self)
    }

    /// Adds a validated, non-sensitive format label such as `raw` or `rgba8`.
    ///
    /// # Errors
    ///
    /// Returns an error when the label is not a bounded lowercase identity.
    pub fn with_format(mut self, format: &str) -> Result<Self, DiagnosticsError> {
        self.push_identity("format", format)?;
        Ok(self)
    }

    /// Adds a validated source-kind label such as `raw` or `raster`.
    ///
    /// # Errors
    ///
    /// Returns an error when the label is not a bounded lowercase identity.
    pub fn with_source_kind(mut self, source_kind: &str) -> Result<Self, DiagnosticsError> {
        self.push_identity("source_kind", source_kind)?;
        Ok(self)
    }

    /// Adds a bounded output byte count without retaining the bytes.
    #[must_use]
    pub fn with_byte_length(mut self, byte_length: u64) -> Self {
        self.push_unsigned("byte_length", byte_length);
        self
    }

    pub(crate) fn fields(&self) -> &[DiagnosticField] {
        &self.fields
    }

    fn push_unsigned(&mut self, key: &str, value: u64) {
        self.push_field(
            DiagnosticField::unsigned(key, value).expect("selected-preview metadata key is static"),
        );
    }

    fn push_identity(&mut self, key: &str, value: &str) -> Result<(), DiagnosticsError> {
        if value.is_empty()
            || value.len() > 32
            || !value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
            })
        {
            return Err(DiagnosticsError::InvalidIdentifier("preview metadata"));
        }
        self.push_field(
            DiagnosticField::public_text(key, value)
                .expect("selected-preview metadata key is static"),
        );
        Ok(())
    }

    fn push_field(&mut self, field: DiagnosticField) {
        if let Some(existing) = self
            .fields
            .iter_mut()
            .find(|existing| existing.key() == field.key())
        {
            *existing = field;
        } else if self.fields.len() < MAX_SELECTED_PREVIEW_METADATA_FIELDS {
            self.fields.push(field);
        }
    }
}

pub(crate) const SELECTED_PREVIEW_SUBSYSTEM: &str = "preview";
pub(crate) const SELECTED_PREVIEW_FAILURE_IDENTITY: &str = "selected_failure";

#[cfg(test)]
mod tests {
    use super::{SelectedPreviewFailureCode, SelectedPreviewFailureStage, SelectedPreviewMetadata};

    #[test]
    fn taxonomy_strings_are_stable_and_bounded() {
        assert_eq!(
            SelectedPreviewFailureStage::SourceDecode.as_str(),
            "source_decode"
        );
        assert_eq!(
            SelectedPreviewFailureCode::DecodeFailed.as_str(),
            "decode_failed"
        );
        assert_eq!(
            SelectedPreviewFailureCode::StaleGeneration.as_str(),
            "stale_generation"
        );
    }

    #[test]
    fn metadata_rejects_untrusted_labels_and_never_accepts_payloads() {
        assert!(
            SelectedPreviewMetadata::default()
                .with_format("/private/photo.raw")
                .is_err()
        );
        assert!(
            SelectedPreviewMetadata::default()
                .with_source_kind("RAW image; secret")
                .is_err()
        );
        assert!(
            SelectedPreviewMetadata::default()
                .with_dimensions(0, 1)
                .is_err()
        );
    }
}
