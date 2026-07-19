use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{Read, Write};
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
const ARTIFACT_LIMIT: usize = 256 * 1024;
const ARTIFACT_TAIL_LIMIT: usize = 64 * 1024;
const ARTIFACT_MARKER: &[u8] =
    b"\n[output truncated; inspect the command receipt for the bounded artifact]\n";
const ARTIFACT_HEAD_LIMIT: usize = ARTIFACT_LIMIT - ARTIFACT_TAIL_LIMIT - ARTIFACT_MARKER.len();

#[derive(Debug, Clone)]
pub struct ProcessRequest {
    program: String,
    args: Vec<String>,
    current_dir: Option<PathBuf>,
    environment: BTreeMap<String, String>,
    limits: ProcessLimits,
    cancellation: Option<Arc<AtomicBool>>,
    stdout_artifact: Option<PathBuf>,
    stderr_artifact: Option<PathBuf>,
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
            stdout_artifact: None,
            stderr_artifact: None,
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

    #[must_use]
    pub fn artifacts(mut self, stdout: impl Into<PathBuf>, stderr: impl Into<PathBuf>) -> Self {
        self.stdout_artifact = Some(stdout.into());
        self.stderr_artifact = Some(stderr.into());
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
    #[serde(default)]
    pub stdout_truncated: bool,
    #[serde(default)]
    pub stderr_truncated: bool,
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
        let (mut child, receiver, stdout_thread, stderr_thread) = spawn_process(&request)?;
        let mut streams = Streams::default();
        let (mut reason, exit_code) = wait_for_exit(&mut child, &receiver, &mut streams, &request)?;
        join_readers(
            &receiver,
            &mut streams,
            &mut reason,
            stdout_thread,
            stderr_thread,
        )?;
        if let Some((path, message)) = streams.artifact_error {
            return Err(ProcessError::Artifact { path, message });
        }
        let status = reason.status();
        let receipt = CommandReceipt {
            schema_version: 1,
            program: request.program,
            args: request.args,
            current_dir: request.current_dir.map(|path| path.display().to_string()),
            environment_hash: hash_environment(&request.environment),
            status: status.to_owned(),
            exit_code,
            stdout_hash: hash_bytes(&streams.stdout.bytes),
            stderr_hash: hash_bytes(&streams.stderr.bytes),
            stdout_truncated: streams.stdout.truncated,
            stderr_truncated: streams.stderr.truncated,
        };
        Ok(ProcessResult {
            receipt,
            stdout: streams.stdout.bytes,
            stderr: streams.stderr.bytes,
        })
    }
}

type RunningProcess = (Child, Receiver<StreamEvent>, JoinHandle<()>, JoinHandle<()>);

fn spawn_process(request: &ProcessRequest) -> Result<RunningProcess, ProcessError> {
    let stdout_artifact = request
        .stdout_artifact
        .as_deref()
        .map(ArtifactWriter::create)
        .transpose()?;
    let stderr_artifact = request
        .stderr_artifact
        .as_deref()
        .map(ArtifactWriter::create)
        .transpose()?;
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
        stdout_artifact,
    );
    let stderr_thread = spawn_reader(
        stderr,
        StreamKind::Stderr,
        request.limits.max_stderr_bytes,
        sender,
        stderr_artifact,
    );
    Ok((child, receiver, stdout_thread, stderr_thread))
}

fn wait_for_exit(
    child: &mut Child,
    receiver: &Receiver<StreamEvent>,
    streams: &mut Streams,
    request: &ProcessRequest,
) -> Result<(StopReason, Option<i32>), ProcessError> {
    let deadline = Instant::now() + request.limits.timeout;
    let cancellation = request
        .cancellation
        .clone()
        .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
    let mut reason = StopReason::None;
    let mut exit_code = None;
    while matches!(reason, StopReason::None) && exit_code.is_none() {
        drain_events(receiver, streams, &mut reason);
        if cancellation.load(Ordering::Acquire) {
            reason = StopReason::Cancelled;
        } else if Instant::now() >= deadline {
            reason = StopReason::TimedOut;
        } else if let Some(status) = child.try_wait().map_err(ProcessError::Wait)? {
            exit_code = status.code();
        } else {
            match receiver.recv_timeout(Duration::from_millis(10)) {
                Ok(event) => accept_event(event, streams, &mut reason),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {}
            }
        }
    }
    if !matches!(reason, StopReason::None) {
        terminate_process_tree(child);
    }
    if exit_code.is_none() {
        exit_code = child.wait().map_err(ProcessError::Wait)?.code();
    }
    Ok((reason, exit_code))
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
    Artifact { path: PathBuf, message: String },
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
            Self::Artifact { path, message } => {
                write!(
                    formatter,
                    "cannot write process artifact {}: {message}",
                    path.display()
                )
            }
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
    ArtifactError(StreamKind, PathBuf, String),
}

fn spawn_reader<R: Read + Send + 'static>(
    mut reader: R,
    stream: StreamKind,
    limit: usize,
    sender: Sender<StreamEvent>,
    mut artifact: Option<ArtifactWriter>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut truncated = false;
        let mut buffer = [0; READ_CHUNK];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => {
                    let _ = sender.send(StreamEvent::Data(stream, bytes));
                    if truncated {
                        let _ = sender.send(StreamEvent::Limit(stream));
                    }
                    if let Some(artifact) = artifact.take() {
                        let path = artifact.path.clone();
                        if let Err(message) = artifact.finish() {
                            let _ = sender.send(StreamEvent::ArtifactError(stream, path, message));
                        }
                    }
                    let _ = sender.send(StreamEvent::Done(stream));
                    return;
                }
                Ok(count) => {
                    let chunk = &buffer[..count];
                    if let Some(artifact) = artifact.as_mut() {
                        artifact.push(chunk);
                    }
                    let retained = bytes.len();
                    if retained < limit {
                        let remaining = limit - retained;
                        bytes.extend_from_slice(&chunk[..remaining.min(count)]);
                    }
                    truncated |= retained.saturating_add(count) > limit;
                }
            }
        }
    })
}

#[derive(Debug, Default)]
struct StreamCapture {
    bytes: Vec<u8>,
    done: bool,
    truncated: bool,
}

#[derive(Debug, Default)]
struct Streams {
    stdout: StreamCapture,
    stderr: StreamCapture,
    artifact_error: Option<(PathBuf, String)>,
}

fn drain_events(receiver: &Receiver<StreamEvent>, streams: &mut Streams, reason: &mut StopReason) {
    loop {
        match receiver.try_recv() {
            Ok(event) => accept_event(event, streams, reason),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => return,
        }
    }
}

fn accept_event(event: StreamEvent, streams: &mut Streams, _reason: &mut StopReason) {
    match event {
        StreamEvent::Data(StreamKind::Stdout, bytes) => streams.stdout.bytes = bytes,
        StreamEvent::Data(StreamKind::Stderr, bytes) => streams.stderr.bytes = bytes,
        StreamEvent::Limit(StreamKind::Stdout) => streams.stdout.truncated = true,
        StreamEvent::Limit(StreamKind::Stderr) => streams.stderr.truncated = true,
        StreamEvent::Done(StreamKind::Stdout) => streams.stdout.done = true,
        StreamEvent::Done(StreamKind::Stderr) => streams.stderr.done = true,
        StreamEvent::ArtifactError(stream, path, message) => {
            streams.artifact_error = Some((path, format!("{stream:?}: {message}")));
        }
    }
}

fn join_readers(
    receiver: &Receiver<StreamEvent>,
    streams: &mut Streams,
    reason: &mut StopReason,
    stdout_thread: JoinHandle<()>,
    stderr_thread: JoinHandle<()>,
) -> Result<(), ProcessError> {
    while !(streams.stdout.done && streams.stderr.done) {
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
}

impl StopReason {
    const fn status(self) -> &'static str {
        match self {
            Self::None => "completed",
            Self::TimedOut => "timed-out",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug)]
struct ArtifactWriter {
    path: PathBuf,
    file: File,
    head: Vec<u8>,
    tail: VecDeque<u8>,
    truncated: bool,
}

pub(crate) fn write_bounded_artifact(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    let mut writer = ArtifactWriter::create(path).map_err(|error| error.to_string())?;
    writer.push(bytes);
    writer.finish()
}

impl ArtifactWriter {
    fn create(path: &std::path::Path) -> Result<Self, ProcessError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| ProcessError::Artifact {
                path: path.to_owned(),
                message: error.to_string(),
            })?;
        }
        let file = File::create(path).map_err(|error| ProcessError::Artifact {
            path: path.to_owned(),
            message: error.to_string(),
        })?;
        Ok(Self {
            path: path.to_owned(),
            file,
            head: Vec::with_capacity(ARTIFACT_HEAD_LIMIT),
            tail: VecDeque::with_capacity(ARTIFACT_TAIL_LIMIT),
            truncated: false,
        })
    }

    fn push(&mut self, bytes: &[u8]) {
        let remaining = ARTIFACT_HEAD_LIMIT.saturating_sub(self.head.len());
        let head_count = remaining.min(bytes.len());
        self.head.extend_from_slice(&bytes[..head_count]);
        if head_count < bytes.len() {
            self.truncated = true;
            self.push_tail(&bytes[head_count..]);
        }
    }

    fn push_tail(&mut self, bytes: &[u8]) {
        self.tail.extend(bytes.iter().copied());
        while self.tail.len() > ARTIFACT_TAIL_LIMIT {
            let _ = self.tail.pop_front();
        }
    }

    fn finish(mut self) -> Result<(), String> {
        self.file
            .write_all(&self.head)
            .and_then(|()| {
                if self.truncated {
                    self.file.write_all(ARTIFACT_MARKER)?;
                    self.file.write_all(self.tail.make_contiguous())?;
                }
                self.file.flush()
            })
            .map_err(|error| format!("{}: {error}", self.path.display()))
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
        assert_eq!(result.receipt.status, "completed");
        assert_eq!(result.stdout, b"1234");
        assert!(result.receipt.stdout_truncated);
        assert_eq!(result.receipt.schema_version, 1);
    }

    #[cfg(unix)]
    #[test]
    fn large_valid_output_is_bounded_without_changing_success() {
        let root =
            std::env::temp_dir().join(format!("rusttable-process-artifact-{}", std::process::id()));
        let stdout = root.join("stdout.log");
        let stderr = root.join("stderr.log");
        let request = ProcessRequest::new(
            "sh",
            [
                "-c",
                "i=0; while [ $i -lt 400000 ]; do printf x; i=$((i+1)); done",
            ],
        )
        .limits(ProcessLimits {
            max_stdout_bytes: 8,
            max_stderr_bytes: 8,
            timeout: Duration::from_secs(2),
        })
        .artifacts(&stdout, &stderr);
        let result = ProcessRunner::new().run(request).expect("process result");
        let artifact = std::fs::read_to_string(&stdout).expect("stdout artifact");
        assert!(result.receipt.success());
        assert!(result.receipt.stdout_truncated);
        assert!(artifact.contains("output truncated"));
        assert!(artifact.len() <= super::ARTIFACT_LIMIT);
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn large_failing_output_keeps_strict_failure_semantics_and_tail() {
        let root =
            std::env::temp_dir().join(format!("rusttable-process-failure-{}", std::process::id()));
        let stdout = root.join("stdout.log");
        let stderr = root.join("stderr.log");
        let request = ProcessRequest::new(
            "sh",
            [
                "-c",
                "i=0; while [ $i -lt 400000 ]; do printf x; i=$((i+1)); done; exit 7",
            ],
        )
        .limits(ProcessLimits {
            max_stdout_bytes: 8,
            max_stderr_bytes: 8,
            timeout: Duration::from_secs(2),
        })
        .artifacts(&stdout, &stderr);
        let result = ProcessRunner::new().run(request).expect("process result");
        let artifact = std::fs::read_to_string(&stdout).expect("stdout artifact");
        assert!(!result.receipt.success());
        assert_eq!(result.receipt.exit_code, Some(7));
        assert!(result.receipt.stdout_truncated);
        assert!(artifact.contains("output truncated"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn artifact_paths_are_platform_joined_and_stable() {
        let path = std::path::Path::new("target")
            .join("validation")
            .join("workspace-dag")
            .join("pull_request.stdout.log");
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("pull_request.stdout.log")
        );
        assert!(path.to_string_lossy().contains("workspace-dag"));
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
