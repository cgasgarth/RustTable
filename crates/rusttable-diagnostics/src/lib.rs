#![forbid(unsafe_code)]
#![doc = "Bounded, local, privacy-preserving diagnostics for `RustTable`."]

mod crash;
mod event;
mod json;
mod redaction;
mod ring;
mod storage;
mod tracing_adapter;

use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};

use directories::ProjectDirs;

pub use event::{
    ApplicationFailureCode, CodeError, DiagnosticCode, DiagnosticContext, DiagnosticEvent,
    DiagnosticField, DiagnosticRecord, DiagnosticSeverity, DiagnosticSubsystem, DiagnosticsError,
    FieldError, FieldValue, PrivacyClass, SCHEMA_VERSION,
};
pub use ring::{MAX_RECENT_BYTES, MAX_RECENT_EVENTS, RecentEvent, RecentSubscription};
pub use tracing_adapter::TracingLayer;

const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct DiagnosticsGuard {
    service: Arc<DiagnosticsService>,
    hook_state: Arc<crash::HookState>,
}

pub struct DiagnosticsService {
    storage: Arc<storage::Storage>,
    redactor: redaction::Redactor,
    ring: Arc<ring::RecentRing>,
    sequence: AtomicU64,
    health: Mutex<Health>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Health {
    pub human_sink: bool,
    pub json_sink: bool,
    pub events_recorded: u64,
    pub degraded: bool,
}

/// Installs the local human/JSON sinks and a bounded panic hook.
///
/// # Errors
///
/// Returns [`DiagnosticsError`] when the diagnostics directory or both sinks cannot be opened.
pub fn install() -> Result<DiagnosticsGuard, DiagnosticsError> {
    let directory = directory()?;
    let storage = Arc::new(storage::Storage::open(&directory)?);
    let service = Arc::new(DiagnosticsService {
        storage,
        redactor: redaction::Redactor::new(),
        ring: Arc::new(ring::RecentRing::new()),
        sequence: AtomicU64::new(0),
        health: Mutex::new(Health {
            human_sink: true,
            json_sink: true,
            events_recorded: 0,
            degraded: false,
        }),
    });
    let previous_hook = std::panic::take_hook();
    let hook_state = Arc::new(crash::HookState {
        crash: Arc::new(crash::CrashState {
            directory,
            package_version: PACKAGE_VERSION,
        }),
        previous: Mutex::new(Some(previous_hook)),
        handling: AtomicBool::new(false),
        service: Arc::clone(&service),
    });
    std::panic::set_hook(crash::hook(Arc::clone(&hook_state)));
    Ok(DiagnosticsGuard {
        service,
        hook_state,
    })
}

impl DiagnosticsGuard {
    /// Records a compatibility event with a stable typed code.
    ///
    /// # Errors
    ///
    /// Returns [`DiagnosticsError`] when every local sink is unavailable.
    pub fn record(&self, event: DiagnosticEvent) -> Result<(), DiagnosticsError> {
        self.service.record_record(event.record())
    }
    /// Records a fully typed event.
    ///
    /// # Errors
    ///
    /// Returns [`DiagnosticsError`] when every local sink is unavailable.
    pub fn record_record(&self, record: DiagnosticRecord) -> Result<(), DiagnosticsError> {
        self.service.record_record(record)
    }
    #[must_use]
    pub fn service(&self) -> Arc<DiagnosticsService> {
        Arc::clone(&self.service)
    }
    #[must_use]
    pub fn snapshot(&self) -> Vec<RecentEvent> {
        self.service.snapshot()
    }
    #[must_use]
    pub fn subscribe(&self) -> RecentSubscription {
        self.service.subscribe()
    }
    #[must_use]
    pub fn health(&self) -> Health {
        self.service.health()
    }
}

impl DiagnosticsService {
    /// Writes one typed event to the active sinks and recent-event ring.
    ///
    /// # Errors
    ///
    /// Returns [`DiagnosticsError`] when every local sink is unavailable.
    pub fn record_record(&self, record: DiagnosticRecord) -> Result<(), DiagnosticsError> {
        let sequence = self
            .sequence
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        let timestamp = unix_millis();
        let human = json::human_line(&record, sequence, timestamp, &self.redactor);
        let encoded = json::record_line(
            &record,
            sequence,
            timestamp,
            PACKAGE_VERSION,
            &self.redactor,
        );
        let status = self.storage.append(&human, &encoded)?;
        if let Ok(mut health) = self.health.lock() {
            health.human_sink = status.human_ok;
            health.json_sink = status.json_ok;
            health.events_recorded = sequence;
            health.degraded = !status.human_ok || !status.json_ok;
        }
        self.ring.push(&RecentEvent {
            sequence,
            timestamp_unix_ms: timestamp,
            record,
        });
        Ok(())
    }

    #[must_use]
    pub fn snapshot(&self) -> Vec<RecentEvent> {
        self.ring.snapshot()
    }
    #[must_use]
    pub fn subscribe(&self) -> RecentSubscription {
        self.ring.subscribe()
    }
    #[must_use]
    pub fn health(&self) -> Health {
        self.health.lock().map_or(
            Health {
                human_sink: false,
                json_sink: false,
                events_recorded: 0,
                degraded: true,
            },
            |health| *health,
        )
    }
    pub(crate) fn flush(&self) {
        self.storage.flush();
    }
}

impl Drop for DiagnosticsGuard {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            self.service.flush();
            let _ = std::panic::take_hook();
            if let Ok(mut previous) = self.hook_state.previous.lock()
                && let Some(previous) = previous.take()
            {
                std::panic::set_hook(previous);
            }
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
