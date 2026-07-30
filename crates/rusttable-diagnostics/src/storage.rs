use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::event::{DiagnosticEvent, DiagnosticsError, MAX_EVENT_BYTES};
use crate::json::{event_line, human_line};
use crate::privacy::Redactor;
use crate::ring::{PresentationEvent, RecentEvents};

pub const ROTATION_BYTES: u64 = 10 * 1024 * 1024;
pub const RETAINED_FILES: usize = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SinkStatus {
    Active,
    Degraded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiagnosticsHealth {
    pub human: SinkStatus,
    pub json: SinkStatus,
}

#[derive(Debug)]
struct Sink {
    path: PathBuf,
    writer: BufWriter<File>,
    bytes: u64,
    status: SinkStatus,
}

impl Sink {
    fn open(directory: &Path, name: &'static str) -> Result<Self, DiagnosticsError> {
        let path = directory.join(name);
        refuse_symlink(&path, name)?;
        let bytes = if path.exists() {
            fs::metadata(&path)?.len()
        } else {
            0
        };
        let bytes = if bytes >= ROTATION_BYTES {
            rotate(&path)?;
            0
        } else {
            bytes
        };
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            writer: BufWriter::new(file),
            bytes,
            status: SinkStatus::Active,
        })
    }

    fn append(&mut self, line: &str) -> Result<(), std::io::Error> {
        let length = u64::try_from(line.len()).map_err(|_| std::io::ErrorKind::InvalidData)?;
        if length > u64::try_from(MAX_EVENT_BYTES).unwrap_or(u64::MAX) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "diagnostic record too large",
            ));
        }
        if self.bytes > 0 && self.bytes.saturating_add(length) > ROTATION_BYTES {
            self.writer.flush()?;
            rotate(&self.path).map_err(|error| match error {
                DiagnosticsError::Storage(error) => error,
                _ => std::io::Error::other("diagnostics rotation failed"),
            })?;
            let file = OpenOptions::new()
                .create_new(true)
                .append(true)
                .open(&self.path)?;
            self.writer = BufWriter::new(file);
            self.bytes = 0;
        }
        self.writer.write_all(line.as_bytes())?;
        self.writer.flush()?;
        self.bytes = self.bytes.saturating_add(length);
        Ok(())
    }
}

pub(crate) struct Storage {
    state: Mutex<State>,
    sequence: AtomicU64,
    redactor: Redactor,
    recent: RecentEvents,
}

struct State {
    human: Option<Sink>,
    json: Option<Sink>,
}

impl Storage {
    pub(crate) fn open(directory: &Path) -> Result<Self, DiagnosticsError> {
        fs::create_dir_all(directory)?;
        let human = Sink::open(directory, "rusttable.log").ok();
        let json = Sink::open(directory, "rusttable.jsonl").ok();
        if human.is_none() && json.is_none() {
            return Err(DiagnosticsError::NoAvailableSink);
        }
        Ok(Self {
            state: Mutex::new(State { human, json }),
            sequence: AtomicU64::new(0),
            redactor: Redactor::new(),
            recent: RecentEvents::new(),
        })
    }

    pub(crate) fn append(
        &self,
        event: &DiagnosticEvent,
        package_version: &str,
    ) -> Result<(), DiagnosticsError> {
        crate::emit(event);
        let sequence = self
            .sequence
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        let timestamp = unix_millis();
        let json = event_line(package_version, sequence, timestamp, event, &self.redactor);
        let human = human_line(sequence, timestamp, event, &self.redactor);
        if json.len() > MAX_EVENT_BYTES || human.len() > MAX_EVENT_BYTES {
            return Err(DiagnosticsError::EventTooLarge);
        }
        let presentation = PresentationEvent::new(sequence, json.clone())?;
        self.recent.push(presentation)?;

        let mut state = self.state.lock().map_err(|_| DiagnosticsError::Poisoned)?;
        let mut successes = 0_u8;
        if let Some(sink) = state.human.as_mut() {
            match sink.append(&human) {
                Ok(()) => successes = successes.saturating_add(1),
                Err(_) => sink.status = SinkStatus::Degraded,
            }
        }
        if let Some(sink) = state.json.as_mut() {
            match sink.append(&json) {
                Ok(()) => successes = successes.saturating_add(1),
                Err(_) => sink.status = SinkStatus::Degraded,
            }
        }
        if successes == 0 {
            if event.severity().is_warning_or_higher() {
                fallback(event);
            }
            return Err(DiagnosticsError::NoAvailableSink);
        }
        if event.severity().is_warning_or_higher()
            && matches!(
                state.human.as_ref().map(|sink| sink.status),
                Some(SinkStatus::Degraded)
            )
            && matches!(
                state.json.as_ref().map(|sink| sink.status),
                Some(SinkStatus::Degraded)
            )
        {
            fallback(event);
        }
        Ok(())
    }

    pub(crate) fn health(&self) -> Result<DiagnosticsHealth, DiagnosticsError> {
        self.state
            .lock()
            .map_err(|_| DiagnosticsError::Poisoned)
            .map(|state| DiagnosticsHealth {
                human: state
                    .human
                    .as_ref()
                    .map_or(SinkStatus::Degraded, |sink| sink.status),
                json: state
                    .json
                    .as_ref()
                    .map_or(SinkStatus::Degraded, |sink| sink.status),
            })
    }

    pub(crate) fn snapshot(&self) -> Result<Vec<PresentationEvent>, DiagnosticsError> {
        self.recent.snapshot()
    }

    pub(crate) fn redactor(&self) -> &Redactor {
        &self.redactor
    }
}

fn rotate(path: &Path) -> Result<(), DiagnosticsError> {
    refuse_symlink(path, "diagnostics log")?;
    for index in (1..=RETAINED_FILES).rev() {
        let source = if index == 1 {
            path.to_path_buf()
        } else {
            backup_path(path, index - 1)
        };
        let destination = backup_path(path, index);
        refuse_symlink(&destination, "diagnostics backup")?;
        if index == RETAINED_FILES {
            let _ = fs::remove_file(&destination);
        }
        if source.exists() {
            fs::rename(source, destination)?;
        }
    }
    Ok(())
}

fn backup_path(path: &Path, index: usize) -> PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(format!(".{index}"));
    PathBuf::from(value)
}

pub(crate) fn refuse_symlink(path: &Path, name: &'static str) -> Result<(), DiagnosticsError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(DiagnosticsError::SymlinkRefused(name))
        }
        Ok(metadata) if !metadata.is_file() => Err(DiagnosticsError::InvalidPath),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(DiagnosticsError::Storage(error)),
    }
}

fn fallback(event: &DiagnosticEvent) {
    tracing::error!(
        target: "rusttable.diagnostics",
        severity = event.severity().as_str(),
        code = %event.code().as_str(),
        "diagnostics sinks degraded"
    );
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}
