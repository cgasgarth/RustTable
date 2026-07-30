use std::backtrace::{Backtrace, BacktraceStatus};
use std::cmp::Reverse;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::json::escape;
use crate::privacy::Redactor;
use crate::storage::refuse_symlink;

const CRASH_LIMIT: usize = 256 * 1024;
const RETAINED_REPORTS: usize = 5;

pub(crate) struct CrashState {
    pub(crate) directory: PathBuf,
    pub(crate) package_version: &'static str,
    pub(crate) redactor: Redactor,
}

impl CrashState {
    pub(crate) fn write(&self, panic: &PanicHookInfo<'_>) {
        let timestamp = unix_millis();
        let pid = std::process::id();
        let path = self.directory.join(format!("crash-{timestamp}-{pid}.json"));
        if refuse_symlink(&path, "crash report").is_err() {
            return;
        }
        let backtrace = Backtrace::capture();
        let backtrace_status = match backtrace.status() {
            BacktraceStatus::Captured => "captured",
            BacktraceStatus::Disabled => "disabled",
            _ => "unsupported",
        };
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("unnamed");
        let thread_alias = self.redactor.alias(thread_name);
        let payload_kind = if panic.payload().is::<&'static str>() {
            "static_str"
        } else if panic.payload().is::<String>() {
            "dynamic_string"
        } else {
            "unknown"
        };
        let mut line = format!(
            "{{\"schema_version\":1,\"package_version\":\"{}\",\"timestamp_unix_ms\":{timestamp},\"pid\":{pid},\"target_os\":\"{}\",\"target_arch\":\"{}\",\"thread_alias\":\"{}\",\"payload_class\":\"payload\",\"payload_kind\":\"{}\",\"payload_text\":\"[redacted]\",\"backtrace_status\":\"{}\",\"backtrace_text\":\"{}\"}}\n",
            escape(self.package_version),
            std::env::consts::OS,
            std::env::consts::ARCH,
            escape(thread_alias.as_str()),
            payload_kind,
            backtrace_status,
            escape(truncate_utf8(&backtrace.to_string(), CRASH_LIMIT / 2)),
        );
        if line.len() > CRASH_LIMIT {
            line.truncate(truncate_utf8(&line, CRASH_LIMIT.saturating_sub(1)).len());
            line.push('\n');
        }
        if let Ok(mut file) = OpenOptions::new().write(true).create_new(true).open(&path) {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
        self.retain();
    }

    fn retain(&self) {
        let Ok(entries) = fs::read_dir(&self.directory) else {
            return;
        };
        let mut reports = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let metadata = fs::symlink_metadata(&path).ok()?;
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return None;
                }
                let (timestamp, pid) = parse_report_name(&path)?;
                Some((timestamp, pid, path))
            })
            .collect::<Vec<_>>();
        reports.sort_by_key(|report| Reverse((report.0, report.1)));
        for (_, _, path) in reports.into_iter().skip(RETAINED_REPORTS) {
            let _ = fs::remove_file(path);
        }
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
}

pub(crate) fn hook(state: Arc<HookState>) -> PanicHook {
    Box::new(move |panic| {
        state.crash.write(panic);
        if let Ok(previous) = state.previous.lock()
            && let Some(previous) = previous.as_ref()
        {
            previous(panic);
        }
    })
}
