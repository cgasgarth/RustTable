use std::sync::Arc;

use rusttable_core::{EditId, PhotoId};
use rusttable_diagnostics::{
    CorrelationContext, DiagnosticCode, DiagnosticEvent, DiagnosticField, DiagnosticsGuard,
    Severity, Subsystem,
};
use rusttable_image::{ImageDimensions, InputFormat};
use rusttable_image_io::{RawCapabilityKind, RawContainerKind, RawDecodeError};

use crate::lifecycle::Recorder;
use crate::workspace::preview_loader::WorkspacePreviewError;
use crate::{CatalogPreviewError, PreviewError};

/// The application-owned handle keeps the diagnostics guard alive for GTK callbacks and workers.
#[derive(Clone, Default)]
pub(crate) struct AppDiagnostics {
    guard: Option<Arc<DiagnosticsGuard>>,
}

impl AppDiagnostics {
    pub(crate) fn from_guard(guard: Option<Arc<DiagnosticsGuard>>) -> Self {
        Self { guard }
    }

    pub(crate) fn lifecycle_failure(&self, code: &'static str, operation: &'static str) {
        self.record(
            "app",
            code,
            Severity::Error,
            operation,
            "lifecycle",
            None,
            None,
            None,
            None,
            None,
            None,
            Vec::new(),
        );
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "preview diagnostics keep stage, cause, correlation, generation, and safe image metadata explicit"
    )]
    pub(crate) fn preview_failure(
        &self,
        operation: &'static str,
        stage: &'static str,
        cause: &'static str,
        photo_id: Option<PhotoId>,
        edit_id: Option<EditId>,
        generation: Option<u64>,
        format: Option<InputFormat>,
        dimensions: Option<ImageDimensions>,
    ) {
        self.record(
            "app",
            preview_code(stage),
            Severity::Error,
            operation,
            stage,
            Some(cause),
            photo_id,
            edit_id,
            generation,
            format,
            dimensions,
            Vec::new(),
        );
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "workspace preview diagnostics retain correlation plus typed decode detail"
    )]
    pub(crate) fn preview_workspace_failure(
        &self,
        operation: &'static str,
        stage: &'static str,
        cause: &'static str,
        photo_id: Option<PhotoId>,
        edit_id: Option<EditId>,
        generation: Option<u64>,
        error: &WorkspacePreviewError,
    ) {
        self.record(
            "app",
            preview_code(stage),
            Severity::Error,
            operation,
            stage,
            Some(cause),
            photo_id,
            edit_id,
            generation,
            None,
            None,
            workspace_preview_error_fields(error),
        );
    }

    pub(crate) fn preview_fallback(
        &self,
        operation: &'static str,
        cause: &'static str,
        generation: Option<u64>,
    ) {
        self.record(
            "app",
            "preview.display_presentation",
            Severity::Warning,
            operation,
            "display_presentation",
            Some(cause),
            None,
            None,
            generation,
            None,
            None,
            Vec::new(),
        );
    }

    pub(crate) fn import_preview_failure(
        &self,
        format: Option<InputFormat>,
        dimensions: Option<(u32, u32)>,
        cause: &'static str,
    ) {
        self.record(
            "import",
            "preview",
            Severity::Error,
            "import_preview",
            "import_preview",
            Some(cause),
            None,
            None,
            None,
            format,
            dimensions.and_then(|(width, height)| ImageDimensions::new(width, height).ok()),
            Vec::new(),
        );
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the bounded event builder receives the complete stable diagnostics contract"
    )]
    fn record(
        &self,
        subsystem: &'static str,
        code_identity: &str,
        severity: Severity,
        operation: &'static str,
        stage: &'static str,
        cause: Option<&'static str>,
        photo_id: Option<PhotoId>,
        edit_id: Option<EditId>,
        generation: Option<u64>,
        format: Option<InputFormat>,
        dimensions: Option<ImageDimensions>,
        detail_fields: Vec<Result<DiagnosticField, rusttable_diagnostics::DiagnosticsError>>,
    ) {
        let Ok(subsystem) = Subsystem::new(subsystem) else {
            tracing::error!(target: "rusttable.app", operation, stage, "invalid diagnostic subsystem");
            return;
        };
        let Ok(code) = DiagnosticCode::new(subsystem, code_identity) else {
            tracing::error!(target: "rusttable.app", operation, stage, code = code_identity, "invalid diagnostic code");
            return;
        };
        let code_text = code.as_str();
        let Ok(mut event) = DiagnosticEvent::new(code, severity, operation) else {
            tracing::error!(target: "rusttable.app", operation, stage, code = %code_text, "invalid diagnostic operation");
            return;
        };
        if let Some(guard) = self.guard.as_ref() {
            let mut context = CorrelationContext::default();
            let request = generation.map_or_else(
                || format!("{operation}:{stage}"),
                |generation| format!("{operation}:{generation}"),
            );
            context = context.request(guard.redactor(), &request);
            if let Some(photo_id) = photo_id {
                context = context.photo(guard.redactor(), &photo_id.to_string());
            }
            if let Some(edit_id) = edit_id {
                context = context.edit(guard.redactor(), &edit_id.to_string());
            }
            event = event.with_context(context);
        }
        for field in fields(stage, cause, generation, format, dimensions)
            .into_iter()
            .chain(detail_fields)
            .flatten()
        {
            event = match event.with_field(field) {
                Ok(event) => event,
                Err(_) => return,
            };
        }
        if let Some(guard) = self.guard.as_ref() {
            if guard.record(&event).is_err() {
                tracing::warn!(target: "rusttable.app", operation, stage, code = %code_text, "diagnostic record unavailable");
            }
        } else {
            rusttable_diagnostics::emit(&event);
        }
        tracing::error!(target: "rusttable.app", operation, stage, code = %code_text, "application boundary failure");
    }
}

fn workspace_preview_error_fields(
    error: &WorkspacePreviewError,
) -> Vec<Result<DiagnosticField, rusttable_diagnostics::DiagnosticsError>> {
    let WorkspacePreviewError::Preview(CatalogPreviewError::Preview(PreviewError::RawDecode(
        error,
    ))) = error
    else {
        return Vec::new();
    };
    raw_decode_error_fields(error)
}

fn raw_decode_error_fields(
    error: &RawDecodeError,
) -> Vec<Result<DiagnosticField, rusttable_diagnostics::DiagnosticsError>> {
    let mut fields = vec![DiagnosticField::public_text(
        "raw_decode_kind",
        raw_decode_kind(error),
    )];
    let container = match error {
        RawDecodeError::Malformed { container, .. } => *container,
        RawDecodeError::Capability(error) => error.container,
        RawDecodeError::Backend { container, .. } => Some(*container),
        RawDecodeError::Cancelled
        | RawDecodeError::Source(_)
        | RawDecodeError::UnsupportedSignature { .. }
        | RawDecodeError::InvalidFrame(_)
        | RawDecodeError::Metadata(_) => None,
    };
    if let Some(container) = container {
        fields.push(DiagnosticField::public_text(
            "raw_container",
            raw_container_label(container),
        ));
    }
    if let RawDecodeError::Capability(capability) = error {
        fields.push(DiagnosticField::public_text(
            "raw_capability",
            raw_capability_label(capability.missing),
        ));
        if !capability.evidence.backend_format.is_empty() {
            fields.push(DiagnosticField::public_text(
                "raw_backend_format",
                &capability.evidence.backend_format,
            ));
        }
        if let Some(bit_depth) = capability.evidence.bit_depth {
            fields.push(DiagnosticField::unsigned(
                "raw_evidence_bit_depth",
                u64::from(bit_depth),
            ));
        }
    }
    fields
}

const fn raw_decode_kind(error: &RawDecodeError) -> &'static str {
    match error {
        RawDecodeError::Cancelled => "cancelled",
        RawDecodeError::Source(_) => "source",
        RawDecodeError::UnsupportedSignature { .. } => "unsupported_signature",
        RawDecodeError::Malformed { .. } => "malformed",
        RawDecodeError::Capability(_) => "capability",
        RawDecodeError::InvalidFrame(_) => "invalid_frame",
        RawDecodeError::Metadata(_) => "metadata",
        RawDecodeError::Backend { .. } => "backend",
    }
}

const fn raw_capability_label(kind: RawCapabilityKind) -> &'static str {
    match kind {
        RawCapabilityKind::Container => "container",
        RawCapabilityKind::Camera => "camera",
        RawCapabilityKind::Compression => "compression",
        RawCapabilityKind::Layout => "layout",
        RawCapabilityKind::SampleType => "sample_type",
        RawCapabilityKind::ImageIndex => "image_index",
        RawCapabilityKind::ManifestDrift => "manifest_drift",
    }
}

const fn raw_container_label(container: RawContainerKind) -> &'static str {
    match container {
        RawContainerKind::Dng => "dng",
        RawContainerKind::Raf => "raf",
        RawContainerKind::Cr2 => "cr2",
        RawContainerKind::Cr3 => "cr3",
        RawContainerKind::Crw => "crw",
        RawContainerKind::Nef => "nef",
        RawContainerKind::Nrw => "nrw",
        RawContainerKind::Arw => "arw",
        RawContainerKind::Sr2 => "sr2",
        RawContainerKind::Srf => "srf",
        RawContainerKind::Orf => "orf",
        RawContainerKind::Rw2 => "rw2",
        RawContainerKind::Rwl => "rwl",
        RawContainerKind::Pef => "pef",
        RawContainerKind::Srw => "srw",
        RawContainerKind::Erf => "erf",
        RawContainerKind::Iiq => "iiq",
        RawContainerKind::ThreeFr => "3fr",
        RawContainerKind::Fff => "fff",
        RawContainerKind::Mrw => "mrw",
        RawContainerKind::X3f => "x3f",
        RawContainerKind::TiffRaw => "tiff_raw",
    }
}

impl Recorder for AppDiagnostics {
    fn record(
        &self,
        event: &DiagnosticEvent,
    ) -> Result<(), rusttable_diagnostics::DiagnosticsError> {
        self.guard
            .as_ref()
            .map_or(Ok(()), |guard| guard.record(event))
    }
}

fn preview_code(stage: &str) -> &str {
    match stage {
        "catalog_lookup" => "preview.catalog_lookup",
        "edit_resolution" => "preview.edit_resolution",
        "decode" => "preview.decode",
        "processing" => "preview.processing",
        "histogram" => "preview.histogram",
        "texture" => "preview.texture",
        "stale_generation" => "preview.stale_generation",
        "display_presentation" => "preview.display_presentation",
        _ => "preview.failure",
    }
}

fn fields(
    stage: &'static str,
    cause: Option<&'static str>,
    generation: Option<u64>,
    format: Option<InputFormat>,
    dimensions: Option<ImageDimensions>,
) -> Vec<Result<DiagnosticField, rusttable_diagnostics::DiagnosticsError>> {
    let mut fields = vec![DiagnosticField::public_text("stage", stage)];
    if let Some(cause) = cause {
        fields.push(DiagnosticField::public_text("cause", cause));
    }
    if let Some(generation) = generation {
        fields.push(DiagnosticField::unsigned("generation", generation));
    }
    if let Some(format) = format {
        fields.push(DiagnosticField::public_text("format", format_label(format)));
    }
    if let Some(dimensions) = dimensions {
        fields.push(DiagnosticField::unsigned(
            "width",
            u64::from(dimensions.width()),
        ));
        fields.push(DiagnosticField::unsigned(
            "height",
            u64::from(dimensions.height()),
        ));
    }
    fields
}

fn format_label(format: InputFormat) -> &'static str {
    match format {
        InputFormat::Jpeg => "jpeg",
        InputFormat::JpegXl => "jpeg-xl",
        InputFormat::Png => "png",
        InputFormat::Tiff => "tiff",
        InputFormat::Raw => "raw",
        InputFormat::OpenExr => "openexr",
        InputFormat::Webp => "webp",
    }
}

#[cfg(test)]
mod tests {
    use rusttable_image::{ImageDimensions, InputFormat};
    use rusttable_image_io::{
        RawCapabilityError, RawCapabilityEvidence, RawCapabilityKind, RawCompression,
        RawCompressionEvidence, RawContainerKind, RawDecodeError, RawMetadataError,
    };

    use super::{
        fields, format_label, preview_code, raw_capability_label, raw_container_label,
        raw_decode_error_fields,
    };

    #[test]
    fn preview_contract_maps_implemented_stages_to_stable_codes() {
        for (stage, code) in [
            ("catalog_lookup", "preview.catalog_lookup"),
            ("edit_resolution", "preview.edit_resolution"),
            ("decode", "preview.decode"),
            ("processing", "preview.processing"),
            ("histogram", "preview.histogram"),
            ("texture", "preview.texture"),
            ("stale_generation", "preview.stale_generation"),
        ] {
            assert_eq!(preview_code(stage), code);
        }
        assert_eq!(preview_code("unknown"), "preview.failure");
    }

    #[test]
    fn preview_fields_are_safe_format_and_dimension_metadata() {
        let dimensions = ImageDimensions::new(2, 3).expect("valid test dimensions");
        let fields = fields(
            "texture",
            Some("gtk_texture_adaptation"),
            Some(7),
            Some(InputFormat::Png),
            Some(dimensions),
        )
        .into_iter()
        .map(|field| field.expect("diagnostic field is valid").key().to_owned())
        .collect::<Vec<_>>();

        assert_eq!(
            fields,
            ["stage", "cause", "generation", "format", "width", "height"]
        );
        assert_eq!(format_label(InputFormat::Png), "png");
        assert!(!fields.iter().any(|key| key == "path"));
    }

    #[test]
    fn raw_capability_diagnostics_keep_only_bounded_structured_evidence() {
        let error = RawDecodeError::Capability(RawCapabilityError {
            missing: RawCapabilityKind::ManifestDrift,
            container: Some(RawContainerKind::Raf),
            maker: "private maker".to_owned(),
            model: "private model".to_owned(),
            mode: String::new(),
            detail: "free-form backend detail".to_owned(),
            evidence: Box::new(RawCapabilityEvidence {
                signature: vec![1, 2, 3],
                raw_tags: vec![0xf003],
                backend_format: "RAF".to_owned(),
                compression: RawCompressionEvidence {
                    compression: RawCompression::Unknown,
                    container_code: None,
                },
                bit_depth: Some(16),
            }),
        });
        let keys = raw_decode_error_fields(&error)
            .into_iter()
            .map(|field| field.expect("bounded diagnostic field").key().to_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            keys,
            [
                "raw_decode_kind",
                "raw_container",
                "raw_capability",
                "raw_backend_format",
                "raw_evidence_bit_depth",
            ]
        );
        assert_eq!(
            raw_capability_label(RawCapabilityKind::ManifestDrift),
            "manifest_drift"
        );
        assert_eq!(raw_container_label(RawContainerKind::Raf), "raf");
        for forbidden in ["path", "maker", "model", "mode", "detail", "signature"] {
            assert!(!keys.iter().any(|key| key.contains(forbidden)));
        }
    }

    #[test]
    fn raw_metadata_errors_publish_only_the_stable_category() {
        let fields =
            raw_decode_error_fields(&RawDecodeError::Metadata(RawMetadataError::UnsafeSourceId))
                .into_iter()
                .map(|field| field.expect("bounded diagnostic field").key().to_owned())
                .collect::<Vec<_>>();

        assert_eq!(fields, ["raw_decode_kind"]);
    }
}
