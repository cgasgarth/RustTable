use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const READ_CHUNK: usize = 8192;
const DEFAULT_OUTPUT_LIMIT: usize = 64 * 1024;
const DEFAULT_TIMEOUT: Duration = Duration::from_mins(5);

#[derive(Debug, Clone)]
pub struct ProcessRequest {
    program: String,
    args: Vec<String>,
    current_dir: Option<PathBuf>,
    environment: BTreeMap<String, String>,
    limits: ProcessLimits,
    cancellation: Option<Arc<AtomicBool>>,
}

impl ProcessRequest {
    #[must_use]
    pub fn new<I, S>(program: impl Into<String>, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            current_dir: None,
            environment: std::env::vars().collect(),
            limits: ProcessLimits::default(),
            cancellation: None,
        }
    }

    #[must_use]
    pub fn current_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.current_dir = Some(path.into());
        self
    }

    #[must_use]
    pub fn environment(mut self, environment: BTreeMap<String, String>) -> Self {
        self.environment = environment;
        self
    }

    #[must_use]
    pub fn limits(mut self, limits: ProcessLimits) -> Self {
        self.limits = limits;
        self
    }

    #[must_use]
    pub fn cancellation(mut self, cancellation: Arc<AtomicBool>) -> Self {
        self.cancellation = Some(cancellation);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessLimits {
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
    pub timeout: Duration,
}

impl Default for ProcessLimits {
    fn default() -> Self {
        Self {
            max_stdout_bytes: DEFAULT_OUTPUT_LIMIT,
            max_stderr_bytes: DEFAULT_OUTPUT_LIMIT,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CommandReceipt {
    pub schema_version: u32,
    pub program: String,
    pub args: Vec<String>,
    pub current_dir: Option<String>,
    pub environment_hash: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub stdout_hash: String,
    pub stderr_hash: String,
}

impl CommandReceipt {
    #[must_use]
    pub fn success(&self) -> bool {
        self.status == "completed" && self.exit_code == Some(0)
    }
}

#[derive(Debug)]
pub struct ProcessResult {
    pub receipt: CommandReceipt,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct ProcessRunner {
    default_limits: ProcessLimits,
}

impl ProcessRunner {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            default_limits: ProcessLimits {
                max_stdout_bytes: DEFAULT_OUTPUT_LIMIT,
                max_stderr_bytes: DEFAULT_OUTPUT_LIMIT,
                timeout: DEFAULT_TIMEOUT,
            },
        }
    }

    /// Runs a command with explicit environment construction and bounded pipes.
    pub fn run(&self, mut request: ProcessRequest) -> Result<ProcessResult, ProcessError> {
        if request.limits == ProcessLimits::default() {
            request.limits = self.default_limits;
        }
        validate_limits(request.limits)?;
        let mut command = Command::new(&request.program);
        command.args(&request.args);
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        command.env_clear().envs(&request.environment);
        if let Some(directory) = &request.current_dir {
            command.current_dir(directory);
        }
        let mut child = command.spawn().map_err(|source| ProcessError::Spawn {
            program: request.program.clone(),
            message: source.to_string(),
        })?;
        let stdout = child
            .stdout
            .take()
            .ok_or(ProcessError::MissingPipe("stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or(ProcessError::MissingPipe("stderr"))?;
        let (sender, receiver) = mpsc::channel();
        let stdout_thread = spawn_reader(
            stdout,
            StreamKind::Stdout,
            request.limits.max_stdout_bytes,
            sender.clone(),
        );
        let stderr_thread = spawn_reader(
            stderr,
            StreamKind::Stderr,
            request.limits.max_stderr_bytes,
            sender,
        );
        let deadline = Instant::now() + request.limits.timeout;
        let cancellation = request
            .cancellation
            .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
        let mut streams = Streams::default();
        let mut reason = StopReason::None;
        let mut exit_code = None;

        while matches!(reason, StopReason::None) && exit_code.is_none() {
            drain_events(&receiver, &mut streams, &mut reason);
            if !matches!(reason, StopReason::None) {
                break;
            }
            if cancellation.load(Ordering::Acquire) {
                reason = StopReason::Cancelled;
                break;
            }
            if Instant::now() >= deadline {
                reason = StopReason::TimedOut;
                break;
            }
            if let Some(status) = child.try_wait().map_err(ProcessError::Wait)? {
                exit_code = status.code();
                break;
            }
            match receiver.recv_timeout(Duration::from_millis(10)) {
                Ok(event) => accept_event(event, &mut streams, &mut reason),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        if !matches!(reason, StopReason::None) {
            terminate_process_tree(&mut child);
        }
        if exit_code.is_none() {
            exit_code = child.wait().map_err(ProcessError::Wait)?.code();
        }
        join_readers(
            &receiver,
            &mut streams,
            &mut reason,
            stdout_thread,
            stderr_thread,
        )?;
        let status = reason.status();
        let receipt = CommandReceipt {
            schema_version: 1,
            program: request.program,
            args: request.args,
            current_dir: request.current_dir.map(|path| path.display().to_string()),
            environment_hash: hash_environment(&request.environment),
            status: status.to_owned(),
            exit_code,
            stdout_hash: hash_bytes(&streams.stdout),
            stderr_hash: hash_bytes(&streams.stderr),
        };
        Ok(ProcessResult {
            receipt,
            stdout: streams.stdout,
            stderr: streams.stderr,
        })
    }
}

fn validate_limits(limits: ProcessLimits) -> Result<(), ProcessError> {
    if limits.max_stdout_bytes == 0 || limits.max_stderr_bytes == 0 || limits.timeout.is_zero() {
        return Err(ProcessError::InvalidLimits);
    }
    Ok(())
}

#[derive(Debug)]
pub enum ProcessError {
    InvalidLimits,
    Spawn { program: String, message: String },
    MissingPipe(&'static str),
    Wait(std::io::Error),
    ReaderPanic,
}

impl fmt::Display for ProcessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimits => formatter.write_str("process limits must be non-zero"),
            Self::Spawn { program, message } => {
                write!(formatter, "cannot spawn {program}: {message}")
            }
            Self::MissingPipe(stream) => write!(formatter, "child did not provide {stream} pipe"),
            Self::Wait(error) => write!(formatter, "cannot wait for child: {error}"),
            Self::ReaderPanic => formatter.write_str("child output reader panicked"),
        }
    }
}

impl std::error::Error for ProcessError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamKind {
    Stdout,
    Stderr,
}

#[derive(Debug)]
enum StreamEvent {
    Data(StreamKind, Vec<u8>),
    Limit(StreamKind),
    Done(StreamKind),
}

fn spawn_reader<R: Read + Send + 'static>(
    mut reader: R,
    stream: StreamKind,
    limit: usize,
    sender: Sender<StreamEvent>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut buffer = [0; READ_CHUNK];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => {
                    let _ = sender.send(StreamEvent::Data(stream, bytes));
                    let _ = sender.send(StreamEvent::Done(stream));
                    return;
                }
                Ok(count) if bytes.len().saturating_add(count) > limit => {
                    let remaining = limit.saturating_sub(bytes.len());
                    bytes.extend_from_slice(&buffer[..remaining.min(count)]);
                    let _ = sender.send(StreamEvent::Data(stream, bytes));
                    let _ = sender.send(StreamEvent::Limit(stream));
                    return;
                }
                Ok(count) => bytes.extend_from_slice(&buffer[..count]),
            }
        }
    })
}

#[derive(Debug, Default)]
struct Streams {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_done: bool,
    stderr_done: bool,
}

fn drain_events(receiver: &Receiver<StreamEvent>, streams: &mut Streams, reason: &mut StopReason) {
    loop {
        match receiver.try_recv() {
            Ok(event) => accept_event(event, streams, reason),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => return,
        }
    }
}

fn accept_event(event: StreamEvent, streams: &mut Streams, reason: &mut StopReason) {
    match event {
        StreamEvent::Data(StreamKind::Stdout, bytes) => streams.stdout = bytes,
        StreamEvent::Data(StreamKind::Stderr, bytes) => streams.stderr = bytes,
        StreamEvent::Limit(StreamKind::Stdout) => *reason = StopReason::StdoutLimit,
        StreamEvent::Limit(StreamKind::Stderr) => *reason = StopReason::StderrLimit,
        StreamEvent::Done(StreamKind::Stdout) => streams.stdout_done = true,
        StreamEvent::Done(StreamKind::Stderr) => streams.stderr_done = true,
    }
}

fn join_readers(
    receiver: &Receiver<StreamEvent>,
    streams: &mut Streams,
    reason: &mut StopReason,
    stdout_thread: JoinHandle<()>,
    stderr_thread: JoinHandle<()>,
) -> Result<(), ProcessError> {
    while !(streams.stdout_done && streams.stderr_done) {
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(event) => accept_event(event, streams, reason),
            Err(RecvTimeoutError::Timeout) if !matches!(reason, StopReason::None) => {
                break;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    stdout_thread
        .join()
        .map_err(|_| ProcessError::ReaderPanic)?;
    stderr_thread
        .join()
        .map_err(|_| ProcessError::ReaderPanic)?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopReason {
    None,
    TimedOut,
    Cancelled,
    StdoutLimit,
    StderrLimit,
}

impl StopReason {
    const fn status(self) -> &'static str {
        match self {
            Self::None => "completed",
            Self::TimedOut => "timed-out",
            Self::Cancelled => "cancelled",
            Self::StdoutLimit => "stdout-limit",
            Self::StderrLimit => "stderr-limit",
        }
    }
}

fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn hash_environment(environment: &BTreeMap<String, String>) -> String {
    let mut canonical = String::new();
    for (key, value) in environment {
        let _ = writeln!(canonical, "{key}={value}");
    }
    hash_bytes(canonical.as_bytes())
}

fn terminate_process_tree(child: &mut Child) {
    let pid = child.id();
    #[cfg(unix)]
    {
        let mut descendants = Vec::new();
        collect_descendants(pid, &mut descendants);
        descendants.reverse();
        for descendant in descendants {
            signal_process(descendant, "TERM");
        }
        signal_process(pid, "TERM");
    }
    let _ = child.kill();
}

#[cfg(unix)]
fn collect_descendants(pid: u32, descendants: &mut Vec<u32>) {
    let output = Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output();
    let Ok(output) = output else { return };
    let mut children = output
        .stdout
        .split(u8::is_ascii_whitespace)
        .filter_map(|bytes| std::str::from_utf8(bytes).ok())
        .filter_map(|value| value.parse::<u32>().ok())
        .collect::<Vec<_>>();
    children.sort_unstable();
    for child in children {
        collect_descendants(child, descendants);
        descendants.push(child);
    }
}

#[cfg(unix)]
fn signal_process(pid: u32, signal: &str) {
    let _ = Command::new("kill")
        .args([format!("-{signal}"), pid.to_string()])
        .status();
}

#[cfg(test)]
mod tests {
    use super::{ProcessLimits, ProcessRequest, ProcessRunner};
    use std::time::Duration;

    #[test]
    fn bounds_stdout_and_returns_a_stable_receipt() {
        let request =
            ProcessRequest::new("sh", ["-c", "printf 1234567890"]).limits(ProcessLimits {
                max_stdout_bytes: 4,
                max_stderr_bytes: 16,
                timeout: Duration::from_secs(2),
            });
        let result = ProcessRunner::new().run(request).expect("process result");
        assert_eq!(result.receipt.status, "stdout-limit");
        assert_eq!(result.stdout, b"1234");
        assert_eq!(result.receipt.schema_version, 1);
    }

    #[cfg(unix)]
    #[test]
    fn timeout_terminates_a_child_tree() {
        let request =
            ProcessRequest::new("sh", ["-c", "(sleep 30) & wait"]).limits(ProcessLimits {
                max_stdout_bytes: 64,
                max_stderr_bytes: 64,
                timeout: Duration::from_millis(100),
            });
        let result = ProcessRunner::new().run(request).expect("process result");
        assert_eq!(result.receipt.status, "timed-out");
    }
}
