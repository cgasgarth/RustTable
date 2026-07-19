use std::backtrace::{Backtrace, BacktraceStatus};
use std::cmp::Reverse;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::DiagnosticsService;
use crate::json::{PanicFields, crash_line};
use crate::storage::refuse_symlink;

const CRASH_LIMIT: usize = 256 * 1024;
const RETAINED_REPORTS: usize = 5;

pub(crate) struct CrashState {
    pub(crate) directory: PathBuf,
    pub(crate) package_version: &'static str,
}

impl CrashState {
    pub(crate) fn write(&self, panic: &PanicHookInfo<'_>) {
        let timestamp = unix_millis();
        let pid = std::process::id();
        let name = format!("crash-{timestamp}-{pid}.json");
        let path = self.directory.join(name);
        if refuse_symlink(&path, "crash report").is_err() {
            return;
        }
        let payload_kind = payload_kind(panic);
        let backtrace = Backtrace::capture();
        let backtrace_status = match backtrace.status() {
            BacktraceStatus::Captured => "captured",
            BacktraceStatus::Disabled => "disabled",
            _ => "unsupported",
        };
        let backtrace_text = backtrace.to_string();
        let fields = PanicFields {
            file: panic.location().map(std::panic::Location::file),
            line: panic.location().map(std::panic::Location::line),
            column: panic.location().map(std::panic::Location::column),
            payload_kind,
        };
        let bounded_backtrace = truncate_utf8(&backtrace_text, CRASH_LIMIT / 2);
        let mut line = crash_line(
            self.package_version,
            timestamp,
            pid,
            &fields,
            backtrace_status,
            bounded_backtrace,
        );
        if line.len() > CRASH_LIMIT {
            line = crash_line(
                self.package_version,
                timestamp,
                pid,
                &fields,
                backtrace_status,
                truncate_utf8(bounded_backtrace, CRASH_LIMIT / 4),
            );
        }
        if line.len() > CRASH_LIMIT {
            line = truncate_utf8(&line, CRASH_LIMIT.saturating_sub(1)).to_owned();
            line.push('\n');
        }
        if let Ok(mut file) = OpenOptions::new().write(true).create_new(true).open(&path) {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
        self.retain();
    }

    fn retain(&self) {
        let mut reports = Vec::new();
        let Ok(entries) = fs::read_dir(&self.directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                continue;
            }
            let Some((timestamp, pid)) = parse_report_name(&path) else {
                continue;
            };
            reports.push((timestamp, pid, path));
        }
        reports.sort_by_key(|report| Reverse((report.0, report.1)));
        for (_, _, path) in reports.into_iter().skip(RETAINED_REPORTS) {
            let _ = fs::remove_file(path);
        }
    }
}

fn payload_kind(panic: &PanicHookInfo<'_>) -> &'static str {
    if panic.payload().downcast_ref::<&'static str>().is_some() {
        "static_str"
    } else if panic.payload().downcast_ref::<String>().is_some() {
        "dynamic_string"
    } else {
        "unknown"
    }
}

fn parse_report_name(path: &Path) -> Option<(u128, u32)> {
    let name = path.file_name()?.to_str()?;
    let value = name.strip_prefix("crash-")?.strip_suffix(".json")?;
    let (timestamp, pid) = value.split_once('-')?;
    Some((timestamp.parse().ok()?, pid.parse().ok()?))
}

fn truncate_utf8(value: &str, limit: usize) -> &str {
    let mut end = value.len().min(limit);
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

pub(crate) type PanicHook = Box<dyn Fn(&PanicHookInfo<'_>) + Send + Sync + 'static>;

pub(crate) struct HookState {
    pub(crate) crash: Arc<CrashState>,
    pub(crate) previous: std::sync::Mutex<Option<PanicHook>>,
    pub(crate) handling: AtomicBool,
    pub(crate) service: Arc<DiagnosticsService>,
}

pub(crate) fn hook(state: Arc<HookState>) -> PanicHook {
    Box::new(move |panic| {
        if state.handling.swap(true, Ordering::AcqRel) {
            return;
        }
        state.crash.write(panic);
        state.service.flush();
        if let Ok(previous) = state.previous.lock()
            && let Some(previous) = previous.as_ref()
        {
            previous(panic);
        }
        state.handling.store(false, Ordering::Release);
    })
}
