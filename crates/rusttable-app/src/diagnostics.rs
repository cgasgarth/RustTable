use std::sync::Arc;

use rusttable_core::{EditId, PhotoId};
use rusttable_diagnostics::{
    CorrelationContext, DiagnosticCode, DiagnosticEvent, DiagnosticField, DiagnosticsGuard,
    Severity, Subsystem,
};
use rusttable_image::{ImageDimensions, InputFormat};

use crate::lifecycle::Recorder;

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
        InputFormat::Png => "png",
        InputFormat::Tiff => "tiff",
        InputFormat::Raw => "raw",
        InputFormat::OpenExr => "openexr",
    }
}

#[cfg(test)]
mod tests {
    use rusttable_image::{ImageDimensions, InputFormat};

    use super::{fields, format_label, preview_code};

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
}
