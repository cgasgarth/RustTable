#![forbid(unsafe_code)]
#![doc = "Bounded, local, privacy-preserving diagnostics for `RustTable`."]

mod context;
mod crash;
mod event;
mod json;
mod privacy;
mod ring;
mod selected_preview;
mod storage;

use std::path::PathBuf;
use std::sync::Arc;

use directories::ProjectDirs;

pub use context::CorrelationContext;
pub use event::{
    ApplicationFailureCode, DiagnosticCode, DiagnosticEvent, DiagnosticsError, SCHEMA_VERSION,
    Severity, Subsystem,
};
pub use privacy::{Alias, DiagnosticField, PrivacyClass, Redactor};
pub use ring::{PresentationEvent, RECENT_EVENT_BYTES, RECENT_EVENT_LIMIT};
pub use selected_preview::{
    SelectedPreviewFailureCode, SelectedPreviewFailureStage, SelectedPreviewMetadata,
    SelectedPreviewOperation,
};
pub use storage::{DiagnosticsHealth, RETAINED_FILES, ROTATION_BYTES, SinkStatus};

const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct DiagnosticsGuard {
    storage: Arc<storage::Storage>,
    hook_state: Arc<crash::HookState>,
}

/// Installs bounded local human/JSON diagnostics and the privacy-safe panic hook.
///
/// # Errors
///
/// Returns an error when the platform data directory cannot be created or no
/// local sink can be opened safely.
pub fn install() -> Result<DiagnosticsGuard, DiagnosticsError> {
    let directory = directory()?;
    let storage = Arc::new(storage::Storage::open(&directory)?);
    let previous_hook = std::panic::take_hook();
    let crash_state = Arc::new(crash::CrashState {
        directory,
        package_version: PACKAGE_VERSION,
        redactor: storage.redactor().clone(),
    });
    let hook_state = Arc::new(crash::HookState {
        crash: crash_state,
        previous: std::sync::Mutex::new(Some(previous_hook)),
    });
    std::panic::set_hook(crash::hook(Arc::clone(&hook_state)));
    Ok(DiagnosticsGuard {
        storage,
        hook_state,
    })
}

impl DiagnosticsGuard {
    /// Records one typed event. Secret and payload fields are omitted by both sinks.
    ///
    /// # Errors
    ///
    /// Returns an error only when the event is invalid/bounded out or all sinks
    /// are unavailable. A degraded individual sink never stops processing.
    pub fn record(&self, event: &DiagnosticEvent) -> Result<(), DiagnosticsError> {
        self.storage.append(event, PACKAGE_VERSION)
    }

    #[must_use]
    pub fn redactor(&self) -> &Redactor {
        self.storage.redactor()
    }

    /// # Errors
    ///
    /// Returns an error if the sink state lock is unavailable.
    pub fn health(&self) -> Result<DiagnosticsHealth, DiagnosticsError> {
        self.storage.health()
    }

    /// # Errors
    ///
    /// Returns an error if the bounded ring lock is unavailable.
    pub fn recent_snapshot(&self) -> Result<Vec<PresentationEvent>, DiagnosticsError> {
        self.storage.snapshot()
    }

    #[must_use]
    pub fn receipt(&self) -> DiagnosticsReceipt {
        DiagnosticsReceipt {
            schema_version: SCHEMA_VERSION,
            rotation_bytes: ROTATION_BYTES,
            retained_files: RETAINED_FILES,
            package_version: PACKAGE_VERSION,
        }
    }
}

/// Emits the safe identity portion of an event through the process tracing subscriber.
/// Classified field values stay owned by the diagnostics sinks and are never passed to
/// an unclassified subscriber.
pub fn emit(event: &DiagnosticEvent) {
    let code = event.code().as_str();
    let subsystem = event.code().subsystem().as_str();
    let operation = event.operation();
    match event.severity() {
        Severity::Trace => tracing::trace!(target: "rusttable", %code, %subsystem, %operation),
        Severity::Debug => tracing::debug!(target: "rusttable", %code, %subsystem, %operation),
        Severity::Info => tracing::info!(target: "rusttable", %code, %subsystem, %operation),
        Severity::Warning => tracing::warn!(target: "rusttable", %code, %subsystem, %operation),
        Severity::Error => tracing::error!(target: "rusttable", %code, %subsystem, %operation),
        Severity::Critical => {
            tracing::error!(target: "rusttable", %code, %subsystem, %operation, critical = true);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticsReceipt {
    schema_version: u16,
    rotation_bytes: u64,
    retained_files: usize,
    package_version: &'static str,
}

impl DiagnosticsReceipt {
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    #[must_use]
    pub const fn rotation_bytes(&self) -> u64 {
        self.rotation_bytes
    }

    #[must_use]
    pub const fn retained_files(&self) -> usize {
        self.retained_files
    }

    #[must_use]
    pub const fn package_version(&self) -> &'static str {
        self.package_version
    }
}

impl Drop for DiagnosticsGuard {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        let _ = std::panic::take_hook();
        if let Ok(mut previous) = self.hook_state.previous.lock()
            && let Some(previous) = previous.take()
        {
            std::panic::set_hook(previous);
        }
    }
}

fn directory() -> Result<PathBuf, DiagnosticsError> {
    if let Some(path) = std::env::var_os("RUSTTABLE_DIAGNOSTICS_DIR") {
        return Ok(PathBuf::from(path));
    }
    ProjectDirs::from("com", "cgasgarth", "RustTable")
        .map(|directories| directories.data_local_dir().to_path_buf())
        .ok_or(DiagnosticsError::DirectoryUnavailable)
}
