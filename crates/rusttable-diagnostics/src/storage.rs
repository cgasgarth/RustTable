use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Mutex;

use crate::event::DiagnosticsError;

const ROTATION_BYTES: u64 = 5 * 1024 * 1024;

pub(crate) struct Storage {
    pub(crate) writer: Mutex<BufWriter<File>>,
}

impl Storage {
    pub(crate) fn open(directory: &Path) -> Result<Self, DiagnosticsError> {
        fs::create_dir_all(directory)?;
        let current = directory.join("rusttable.log");
        let backup = directory.join("rusttable.log.1");
        refuse_symlink(&current, "rusttable.log")?;
        refuse_symlink(&backup, "rusttable.log.1")?;
        if current.exists() && fs::metadata(&current)?.len() >= ROTATION_BYTES {
            if backup.exists() {
                fs::remove_file(&backup)?;
            }
            fs::rename(&current, &backup)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&current)?;
        Ok(Self {
            writer: Mutex::new(BufWriter::new(file)),
        })
    }

    pub(crate) fn append(&self, line: &str) -> Result<(), DiagnosticsError> {
        let mut writer = self.writer.lock().map_err(|_| DiagnosticsError::Poisoned)?;
        writer.write_all(line.as_bytes())?;
        writer.flush()?;
        Ok(())
    }
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
