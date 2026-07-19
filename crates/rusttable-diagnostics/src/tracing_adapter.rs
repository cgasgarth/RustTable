use std::fmt::Write;
use std::sync::Arc;

use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::layer::{Context, Layer};

use crate::{
    DiagnosticCode, DiagnosticField, DiagnosticRecord, DiagnosticSeverity, DiagnosticSubsystem,
    DiagnosticsService, PrivacyClass,
};

pub struct TracingLayer {
    service: Arc<DiagnosticsService>,
}

impl TracingLayer {
    #[must_use]
    pub fn new(service: Arc<DiagnosticsService>) -> Self {
        Self { service }
    }
}

impl<S> Layer<S> for TracingLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let metadata = event.metadata();
        let code = DiagnosticCode::new(format!("tracing.{}", metadata.name()))
            .unwrap_or_else(|_| DiagnosticCode::new("tracing.event").expect("static code"));
        let severity = match *metadata.level() {
            tracing::Level::ERROR => DiagnosticSeverity::Error,
            tracing::Level::WARN => DiagnosticSeverity::Warning,
            tracing::Level::INFO => DiagnosticSeverity::Info,
            tracing::Level::DEBUG => DiagnosticSeverity::Debug,
            tracing::Level::TRACE => DiagnosticSeverity::Trace,
        };
        let mut visitor = EventVisitor { fields: Vec::new() };
        event.record(&mut visitor);
        let Ok(mut record) =
            DiagnosticRecord::new(code, severity, DiagnosticSubsystem::System, "tracing")
        else {
            return;
        };
        for (name, value) in visitor.fields {
            if let Ok(field) = DiagnosticField::text(name, value, PrivacyClass::Operational) {
                record = record.with_field(field);
            }
        }
        let _ = self.service.record_record(record);
    }
}

struct EventVisitor {
    fields: Vec<(String, String)>,
}

impl Visit for EventVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let mut text = String::new();
        let _ = write!(text, "{value:?}");
        self.fields.push((field.name().to_owned(), text));
    }
}
