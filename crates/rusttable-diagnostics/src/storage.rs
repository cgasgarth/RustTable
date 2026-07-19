use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::event::DiagnosticsError;

pub(crate) const ROTATION_BYTES: u64 = 10 * 1024 * 1024;
const RETAINED_FILES: usize = 5;

struct Sink {
    path: PathBuf,
    writer: BufWriter<File>,
    bytes: u64,
}

pub(crate) struct Storage {
    human: Mutex<Option<Sink>>,
    json: Mutex<Option<Sink>>,
}

impl Storage {
    pub(crate) fn open(directory: &Path) -> Result<Self, DiagnosticsError> {
        fs::create_dir_all(directory)?;
        let human = Sink::open(directory.join("rusttable.log"));
        let json = Sink::open(directory.join("rusttable.jsonl"));
        if human.is_err() && json.is_err() {
            return Err(human.err().or_else(|| json.err()).expect("sink error"));
        }
        Ok(Self {
            human: Mutex::new(human.ok()),
            json: Mutex::new(json.ok()),
        })
    }

    pub(crate) fn append(&self, human: &str, json: &str) -> Result<SinkStatus, DiagnosticsError> {
        let human_status = write_sink(&self.human, human);
        let json_status = write_sink(&self.json, json);
        if human_status.is_err() && json_status.is_err() {
            return Err(DiagnosticsError::StorageUnavailable);
        }
        Ok(SinkStatus {
            human_ok: human_status.is_ok(),
            json_ok: json_status.is_ok(),
        })
    }

    pub(crate) fn flush(&self) {
        for sink in [&self.human, &self.json] {
            if let Ok(mut sink) = sink.lock()
                && let Some(sink) = sink.as_mut()
            {
                let _ = sink.writer.flush();
            }
        }
    }
}

fn write_sink(sink: &Mutex<Option<Sink>>, line: &str) -> Result<(), DiagnosticsError> {
    let mut sink = sink.lock().map_err(|_| DiagnosticsError::Poisoned)?;
    let Some(sink) = sink.as_mut() else {
        return Err(DiagnosticsError::StorageUnavailable);
    };
    if sink.bytes.saturating_add(line.len() as u64) > ROTATION_BYTES {
        sink.rotate()?;
    }
    sink.writer.write_all(line.as_bytes())?;
    sink.writer.flush()?;
    sink.bytes = sink.bytes.saturating_add(line.len() as u64);
    Ok(())
}

impl Sink {
    fn open(path: PathBuf) -> Result<Self, DiagnosticsError> {
        refuse_symlink(&path, "diagnostics sink")?;
        let bytes = path.metadata().map_or(0, |metadata| metadata.len());
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            writer: BufWriter::new(file),
            bytes,
        })
    }

    fn rotate(&mut self) -> Result<(), DiagnosticsError> {
        self.writer.flush()?;
        for index in (1..RETAINED_FILES).rev() {
            let from = rotated_path(&self.path, index);
            let to = rotated_path(&self.path, index + 1);
            if from.exists() {
                let _ = fs::remove_file(&to);
                fs::rename(from, to)?;
            }
        }
        let first = rotated_path(&self.path, 1);
        let _ = fs::remove_file(&first);
        fs::rename(&self.path, &first)?;
        refuse_symlink(&self.path, "diagnostics sink")?;
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&self.path)?;
        self.writer = BufWriter::new(file);
        self.bytes = 0;
        Ok(())
    }
}

fn rotated_path(path: &Path, index: usize) -> PathBuf {
    PathBuf::from(format!("{}.{}", path.display(), index))
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SinkStatus {
    pub human_ok: bool,
    pub json_ok: bool,
}
