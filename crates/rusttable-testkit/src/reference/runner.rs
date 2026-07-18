use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use super::identity::{ReferenceIdentity, isolation_arguments};
use super::schema::{
    CancellationToken, ExecutionMode, ExitStatus, ReferenceLimits, ReferenceReceipt,
    ReferenceRequest, ReferenceStatus,
};

const READ_CHUNK: usize = 8192;
static NEXT_ENVIRONMENT: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone)]
pub struct ReferenceRunner {
    identity: ReferenceIdentity,
    limits: ReferenceLimits,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceArtifacts {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub output: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceRun {
    pub receipt: ReferenceReceipt,
    pub artifacts: ReferenceArtifacts,
}

impl ReferenceRunner {
    #[must_use]
    pub const fn new(identity: ReferenceIdentity, limits: ReferenceLimits) -> Self {
        Self { identity, limits }
    }

    #[must_use]
    pub const fn identity(&self) -> &ReferenceIdentity {
        &self.identity
    }

    /// Runs one bounded reference invocation and returns its typed receipt.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid inputs, process failures to start, or resource-limit violations.
    pub fn run(&self, request: &ReferenceRequest) -> Result<ReferenceReceipt, ReferenceError> {
        Ok(self
            .run_with_artifacts(request, &CancellationToken::new())?
            .receipt)
    }

    /// Runs one invocation while observing a cooperative cancellation token.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid inputs, process failures to start, or resource-limit violations.
    pub fn run_with_cancellation(
        &self,
        request: &ReferenceRequest,
        cancellation: &CancellationToken,
    ) -> Result<ReferenceReceipt, ReferenceError> {
        Ok(self.run_with_artifacts(request, cancellation)?.receipt)
    }

    /// Runs one invocation and retains raw stdout, stderr, and output bytes for merge artifacts.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid inputs, process failures to start, or resource-limit violations.
    #[allow(clippy::too_many_lines)] // The lifecycle is kept together so every exit path shares cleanup.
    pub fn run_with_artifacts(
        &self,
        request: &ReferenceRequest,
        cancellation: &CancellationToken,
    ) -> Result<ReferenceRun, ReferenceError> {
        validate_request(request, &self.limits)?;
        let environment = TempEnvironment::new()?;
        let output_path = environment
            .output
            .join(format!("output.{}", request.output_format.extension()));
        let arguments = command_arguments(&self.identity, request, &environment, &output_path)?;
        let mut child = spawn(
            &self.identity,
            request,
            &environment,
            &arguments,
            &output_path,
        )?;
        let (tx, rx) = mpsc::channel();
        let stdout = child
            .stdout
            .take()
            .ok_or(ReferenceError::MissingPipe("stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or(ReferenceError::MissingPipe("stderr"))?;
        let stdout_thread = spawn_reader(
            stdout,
            StreamKind::Stdout,
            self.limits.max_stdout_bytes,
            tx.clone(),
        );
        let stderr_thread =
            spawn_reader(stderr, StreamKind::Stderr, self.limits.max_stderr_bytes, tx);

        let deadline = Instant::now() + request.timeout();
        let mut reason = None;
        let mut status = None;
        let mut streams = Streams::default();
        while status.is_none() {
            collect_events(&rx, &mut streams, &mut reason);
            if reason.is_some() {
                break;
            }
            if cancellation.is_cancelled() {
                reason = Some(KillReason::Cancelled);
                break;
            }
            if Instant::now() >= deadline {
                reason = Some(KillReason::TimedOut);
                break;
            }
            if output_too_large(&output_path, self.limits.max_output_bytes) {
                reason = Some(KillReason::OutputLimit);
                break;
            }
            if let Some(exit) = child.try_wait().map_err(ReferenceError::Wait)? {
                status = Some(exit_status(exit.code(), exit.success()));
                break;
            }
            match rx.recv_timeout(Duration::from_millis(10)) {
                Ok(event) => streams.accept(event, &mut reason),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        if reason.is_some() {
            terminate_process_tree(&mut child);
        }
        collect_until_done(&rx, &mut streams, &mut reason);
        stdout_thread
            .join()
            .map_err(|_| ReferenceError::ReaderPanic)?;
        stderr_thread
            .join()
            .map_err(|_| ReferenceError::ReaderPanic)?;
        let final_status = match reason {
            Some(KillReason::Cancelled) => ReferenceStatus::Cancelled,
            Some(KillReason::TimedOut) => ReferenceStatus::TimedOut,
            Some(KillReason::OutputLimit) => {
                return Err(ReferenceError::OutputLimit(self.limits.max_output_bytes));
            }
            Some(KillReason::StreamLimit(kind, limit)) => {
                return Err(ReferenceError::StreamLimit(kind, limit));
            }
            None => ReferenceStatus::Completed(status.unwrap_or(ExitStatus {
                code: None,
                success: false,
            })),
        };
        let stdout = streams.stdout;
        let stderr = streams.stderr;
        let output = match fs::metadata(&output_path) {
            Ok(_) => read_output(&output_path, self.limits.max_output_bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                return Err(ReferenceError::MissingOutputIo {
                    path: output_path.clone(),
                    message: error.to_string(),
                });
            }
        };
        if matches!(
            final_status,
            ReferenceStatus::Completed(ExitStatus { success: true, .. })
        ) && output.is_empty()
        {
            return Err(ReferenceError::MissingOutput(output_path));
        }
        let receipt = receipt(
            request,
            &self.identity,
            final_status,
            &stdout,
            &stderr,
            &output,
        );
        Ok(ReferenceRun {
            receipt,
            artifacts: ReferenceArtifacts {
                stdout,
                stderr,
                output,
            },
        })
    }
}

fn validate_request(
    request: &ReferenceRequest,
    limits: &ReferenceLimits,
) -> Result<(), ReferenceError> {
    if request.source_fixture_id.trim().is_empty()
        || request.source_fixture_id.contains(['/', '\\'])
    {
        return Err(ReferenceError::InvalidRequest(
            "source fixture ID must be a non-path value".to_owned(),
        ));
    }
    require_file(&request.source_path, "source")?;
    if let Some(path) = &request.xmp_path {
        require_file(path, "XMP")?;
    }
    if let Some(path) = &request.config_path {
        require_file(path, "config")?;
    }
    if request.dimensions.width == 0 || request.dimensions.height == 0 {
        return Err(ReferenceError::InvalidRequest(
            "dimensions must be non-zero".to_owned(),
        ));
    }
    if request.timeout_ms == 0 {
        return Err(ReferenceError::InvalidRequest(
            "timeout must be non-zero".to_owned(),
        ));
    }
    if limits.max_stdout_bytes == 0 || limits.max_stderr_bytes == 0 || limits.max_output_bytes == 0
    {
        return Err(ReferenceError::InvalidRequest(
            "resource limits must be non-zero".to_owned(),
        ));
    }
    Ok(())
}

fn require_file(path: &Path, label: &str) -> Result<(), ReferenceError> {
    if !path.is_file() {
        return Err(ReferenceError::MissingInput {
            label: label.to_owned(),
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn command_arguments(
    identity: &ReferenceIdentity,
    request: &ReferenceRequest,
    environment: &TempEnvironment,
    output: &Path,
) -> Result<Vec<String>, ReferenceError> {
    let mut args = isolation_arguments(
        &identity.required_flags,
        &identity.data_dir,
        &environment.config,
        &environment.cache,
        &environment.library,
    )
    .map_err(|error| ReferenceError::UnsupportedFlag(error.to_string()))?;
    for pair in args.chunks_exact_mut(2) {
        if pair[0] == "--width" {
            pair[1] = request.dimensions.width.to_string();
        }
        if pair[0] == "--height" {
            pair[1] = request.dimensions.height.to_string();
        }
        if pair[0] == "--icc-type" {
            request.output_profile.cli_name().clone_into(&mut pair[1]);
        }
        if pair[0] == "--icc" {
            request.output_profile.cli_name().clone_into(&mut pair[1]);
        }
        if pair[0] == "--out-ext" {
            request.output_format.cli_name().clone_into(&mut pair[1]);
        }
    }
    if request.execution_mode == ExecutionMode::Gpu {
        args.retain(|arg| arg != "--disable-opencl");
    }
    args.extend([
        request.source_path.display().to_string(),
        output.display().to_string(),
    ]);
    if let Some(xmp) = &request.xmp_path {
        args.push(xmp.display().to_string());
    }
    Ok(args)
}

fn spawn(
    identity: &ReferenceIdentity,
    request: &ReferenceRequest,
    environment: &TempEnvironment,
    arguments: &[String],
    output: &Path,
) -> Result<Child, ReferenceError> {
    let mut command = Command::new(&identity.executable);
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &environment.home)
        .env("XDG_CONFIG_HOME", &environment.config)
        .env("XDG_CACHE_HOME", &environment.cache)
        .env("XDG_DATA_HOME", &environment.data)
        .env("RUSTTABLE_OPENCL_CACHE", &environment.opencl)
        .env(
            "RUSTTABLE_REFERENCE_CONFIG",
            request.config_path.as_deref().unwrap_or(Path::new("")),
        )
        .env("RUSTTABLE_REFERENCE_OUTPUT", output)
        .env("RUSTTABLE_REFERENCE_ROOT", &environment.root)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("TZ", "UTC")
        .current_dir(&environment.root)
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.spawn().map_err(|error| ReferenceError::Spawn {
        path: identity.executable.clone(),
        message: error.to_string(),
    })
}

#[derive(Debug, Clone, Copy)]
enum StreamKind {
    Stdout,
    Stderr,
}

impl fmt::Display for StreamKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        })
    }
}

struct StreamEvent {
    kind: StreamKind,
    bytes: Vec<u8>,
    error: Option<String>,
}

fn spawn_reader<R: Read + Send + 'static>(
    mut reader: R,
    kind: StreamKind,
    limit: u64,
    tx: mpsc::Sender<StreamEvent>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; READ_CHUNK];
        let mut error = None;
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    bytes.extend_from_slice(&buffer[..count]);
                    if bytes.len() as u64 > limit {
                        error = Some(format!("{kind} exceeded {limit} bytes"));
                        break;
                    }
                }
                Err(read_error) => {
                    error = Some(read_error.to_string());
                    break;
                }
            }
        }
        let _ = tx.send(StreamEvent { kind, bytes, error });
    })
}

#[derive(Default)]
struct Streams {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_done: bool,
    stderr_done: bool,
}

impl Streams {
    fn accept(&mut self, event: StreamEvent, reason: &mut Option<KillReason>) {
        if let Some(error) = event.error {
            let limit = error
                .split_whitespace()
                .last()
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            *reason = Some(KillReason::StreamLimit(event.kind.to_string(), limit));
        }
        match event.kind {
            StreamKind::Stdout => {
                self.stdout = event.bytes;
                self.stdout_done = true;
            }
            StreamKind::Stderr => {
                self.stderr = event.bytes;
                self.stderr_done = true;
            }
        }
    }
}

fn collect_events(
    rx: &mpsc::Receiver<StreamEvent>,
    streams: &mut Streams,
    reason: &mut Option<KillReason>,
) {
    while let Ok(event) = rx.try_recv() {
        streams.accept(event, reason);
    }
}

fn collect_until_done(
    rx: &mpsc::Receiver<StreamEvent>,
    streams: &mut Streams,
    reason: &mut Option<KillReason>,
) {
    while !(streams.stdout_done && streams.stderr_done) {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(event) => streams.accept(event, reason),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
        }
    }
}

#[derive(Debug, Clone)]
enum KillReason {
    Cancelled,
    TimedOut,
    OutputLimit,
    StreamLimit(String, u64),
}

fn output_too_large(path: &Path, limit: u64) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.len() > limit)
}

fn read_output(path: &Path, limit: u64) -> Result<Vec<u8>, ReferenceError> {
    let metadata = fs::metadata(path).map_err(|error| ReferenceError::MissingOutputIo {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    if metadata.len() > limit {
        return Err(ReferenceError::OutputLimit(limit));
    }
    fs::read(path).map_err(|error| ReferenceError::MissingOutputIo {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn terminate_process_tree(child: &mut Child) {
    if let Some(pid) = child.id().checked_add(0) {
        kill_descendants(pid, false);
    }
    let _ = child.kill();
    if let Some(pid) = child.id().checked_add(0) {
        kill_descendants(pid, true);
    }
    let _ = child.wait();
}

#[cfg(unix)]
fn kill_descendants(pid: u32, force: bool) {
    let output = Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output();
    let children = output
        .ok()
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| line.trim().parse().ok())
                .collect::<Vec<u32>>()
        })
        .unwrap_or_default();
    for child in &children {
        kill_descendants(*child, force);
    }
    for child in children {
        let signal = if force { "-KILL" } else { "-TERM" };
        let _ = Command::new("kill")
            .args([signal, &child.to_string()])
            .status();
    }
}

#[cfg(windows)]
fn kill_descendants(pid: u32, _force: bool) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status();
}

#[cfg(not(any(unix, windows)))]
fn kill_descendants(_pid: u32, _force: bool) {}

fn exit_status(code: Option<i32>, success: bool) -> ExitStatus {
    ExitStatus { code, success }
}

fn receipt(
    request: &ReferenceRequest,
    identity: &ReferenceIdentity,
    status: ReferenceStatus,
    stdout: &[u8],
    stderr: &[u8],
    output: &[u8],
) -> ReferenceReceipt {
    ReferenceReceipt {
        source_fixture_id: request.source_fixture_id.clone(),
        xmp_path: request.xmp_path.clone(),
        config_path: request.config_path.clone(),
        output_format: request.output_format,
        output_profile: request.output_profile,
        dimensions: request.dimensions,
        timeout_ms: request.timeout_ms,
        status,
        stdout_hash: hash(&normalize_logs(stdout, identity.normalized_log_ruleset)),
        stderr_hash: hash(&normalize_logs(stderr, identity.normalized_log_ruleset)),
        output_hash: hash(output),
        reference_identity: identity.receipt(),
        normalized_log_ruleset: identity.normalized_log_ruleset,
        execution_mode: request.execution_mode,
    }
}

fn normalize_logs(bytes: &[u8], ruleset: u32) -> Vec<u8> {
    if ruleset != 1 {
        return bytes.to_vec();
    }
    let text = String::from_utf8_lossy(bytes);
    let mut result = String::with_capacity(text.len());
    for token in text.split_inclusive(|character: char| character.is_whitespace()) {
        let end = token.find(char::is_whitespace).unwrap_or(token.len());
        let word = &token[..end];
        let replacement = ["timestamp=", "pid=", "thread="].iter().find_map(|prefix| {
            word.strip_prefix(prefix)
                .map(|_| format!("{prefix}<normalized>"))
        });
        result.push_str(replacement.as_deref().unwrap_or(word));
        result.push_str(&token[end..]);
    }
    result.into_bytes()
}

fn hash(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        let _ = write!(result, "{byte:02x}");
    }
    result
}

struct TempEnvironment {
    root: PathBuf,
    home: PathBuf,
    config: PathBuf,
    cache: PathBuf,
    data: PathBuf,
    opencl: PathBuf,
    output: PathBuf,
    library: PathBuf,
}

impl TempEnvironment {
    fn new() -> Result<Self, ReferenceError> {
        let number = NEXT_ENVIRONMENT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "rusttable-reference-run-{}-{number}",
            std::process::id()
        ));
        let environment = Self {
            home: root.join("home"),
            config: root.join("config"),
            cache: root.join("cache"),
            data: root.join("data"),
            opencl: root.join("opencl"),
            output: root.join("output"),
            library: root.join("library.db"),
            root,
        };
        for path in [
            &environment.home,
            &environment.config,
            &environment.cache,
            &environment.data,
            &environment.opencl,
            &environment.output,
        ] {
            fs::create_dir_all(path).map_err(|error| ReferenceError::Temp {
                path: path.clone(),
                message: error.to_string(),
            })?;
        }
        Ok(environment)
    }
}

impl Drop for TempEnvironment {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[derive(Debug)]
pub enum ReferenceError {
    InvalidRequest(String),
    MissingInput { label: String, path: PathBuf },
    MissingOutput(PathBuf),
    MissingOutputIo { path: PathBuf, message: String },
    Temp { path: PathBuf, message: String },
    Spawn { path: PathBuf, message: String },
    Wait(std::io::Error),
    MissingPipe(&'static str),
    ReaderPanic,
    UnsupportedFlag(String),
    StreamLimit(String, u64),
    OutputLimit(u64),
}

impl fmt::Display for ReferenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(message) => {
                write!(formatter, "invalid reference request: {message}")
            }
            Self::MissingInput { label, path } => {
                write!(formatter, "missing {label} input: {}", path.display())
            }
            Self::MissingOutput(path) => write!(
                formatter,
                "reference produced no output: {}",
                path.display()
            ),
            Self::MissingOutputIo { path, message } => write!(
                formatter,
                "cannot read reference output {}: {message}",
                path.display()
            ),
            Self::Temp { path, message } => write!(
                formatter,
                "reference temporary environment at {}: {message}",
                path.display()
            ),
            Self::Spawn { path, message } => write!(
                formatter,
                "cannot spawn reference {}: {message}",
                path.display()
            ),
            Self::Wait(error) => write!(formatter, "cannot wait for reference: {error}"),
            Self::MissingPipe(stream) => {
                write!(formatter, "reference did not provide {stream} pipe")
            }
            Self::ReaderPanic => formatter.write_str("reference output reader panicked"),
            Self::UnsupportedFlag(message) => formatter.write_str(message),
            Self::StreamLimit(kind, limit) => {
                write!(formatter, "reference {kind} exceeded {limit} bytes")
            }
            Self::OutputLimit(limit) => {
                write!(formatter, "reference output exceeded {limit} bytes")
            }
        }
    }
}

impl std::error::Error for ReferenceError {}
