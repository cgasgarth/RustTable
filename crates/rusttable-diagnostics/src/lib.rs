#![forbid(unsafe_code)]
#![doc = "Bounded, local, privacy-preserving diagnostics for `RustTable`."]

mod crash;
mod event;
mod json;
mod storage;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use directories::ProjectDirs;

pub use event::{ApplicationFailureCode, DiagnosticEvent, DiagnosticsError};

const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct DiagnosticsGuard {
    storage: Arc<storage::Storage>,
    hook_state: Arc<crash::HookState>,
}

/// Installs local diagnostics storage and the bounded panic hook.
///
/// # Errors
///
/// Returns an error when the platform data directory cannot be created or a
/// diagnostics file cannot be opened safely.
pub fn install() -> Result<DiagnosticsGuard, DiagnosticsError> {
    let directory = directory()?;
    let storage = Arc::new(storage::Storage::open(&directory)?);
    let previous_hook = std::panic::take_hook();
    let crash_state = Arc::new(crash::CrashState {
        directory,
        package_version: PACKAGE_VERSION,
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
    /// Records one closed-schema event and flushes it before returning.
    ///
    /// # Errors
    ///
    /// Returns an error when the diagnostics lock, write, or flush fails.
    pub fn record(&self, event: DiagnosticEvent) -> Result<(), DiagnosticsError> {
        self.storage
            .append(&json::event_line(PACKAGE_VERSION, unix_millis(), event))
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

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}
