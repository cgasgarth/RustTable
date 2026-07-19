use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

#[cfg(windows)]
use process_wrap::std::JobObject;
#[cfg(unix)]
use process_wrap::std::ProcessSession;
use process_wrap::std::{ChildWrapper, CommandWrap};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod artifact_receipts;

const READ_CHUNK: usize = 8192;
const DEFAULT_OUTPUT_LIMIT: usize = 64 * 1024;
const DEFAULT_TIMEOUT: Duration = Duration::from_mins(5);
const CLEANUP_GRACE: Duration = Duration::from_secs(1);
const DRAIN_DEADLINE: Duration = Duration::from_secs(2);
const ARTIFACT_LIMIT: usize = 256 * 1024;
const ARTIFACT_TAIL_LIMIT: usize = 64 * 1024;
const ARTIFACT_MARKER: &[u8] =
    b"\n[output truncated; inspect the command receipt for the bounded artifact]\n";
const ARTIFACT_HEAD_LIMIT: usize = ARTIFACT_LIMIT - ARTIFACT_TAIL_LIMIT - ARTIFACT_MARKER.len();

/// Named environment policies prevent accidental inheritance from the parent process.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EnvironmentProfile {
    #[default]
    Empty,
    RustTool,
    GitTool,
    GitHubApi,
    ReferenceTool,
    PlatformTool,
    TestFixture,
}

impl EnvironmentProfile {
    const fn name(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::RustTool => "rust-tool",
            Self::GitTool => "git-tool",
            Self::GitHubApi => "github-api",
            Self::ReferenceTool => "reference-tool",
            Self::PlatformTool => "platform-tool",
            Self::TestFixture => "test-fixture",
        }
    }
}

/// Secret values are intentionally not serializable or printable.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(String);

impl SecretValue {
    #[must_use]
    #[allow(dead_code)]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NetworkPolicy {
    #[default]
    None,
    Read,
}

impl NetworkPolicy {
    const fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Read => "read",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CleanupReceipt {
    pub outcome: String,
    pub termination: String,
    pub grace_ms: u128,
    pub drain_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactReceipt {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub kind: String,
    pub fresh: bool,
    #[serde(default = "default_artifact_present")]
    pub present: bool,
}

fn default_artifact_present() -> bool {
    true
}

#[derive(Debug, Clone)]
pub struct ProcessRequest {
    program: String,
    args: Vec<String>,
    current_dir: Option<PathBuf>,
    environment: BTreeMap<String, String>,
    secrets: BTreeMap<String, SecretValue>,
    #[allow(dead_code)]
    secret_args: BTreeMap<usize, SecretValue>,
    profile: EnvironmentProfile,
    network: NetworkPolicy,
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
            environment: BTreeMap::new(),
            secrets: BTreeMap::new(),
            secret_args: BTreeMap::new(),
            profile: EnvironmentProfile::Empty,
            network: NetworkPolicy::None,
            limits: ProcessLimits::default(),
            cancellation: None,
            stdout_artifact: None,
            stderr_artifact: None,
        }
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn profile(mut self, profile: EnvironmentProfile) -> Self {
        self.profile = profile;
        self
    }

    #[allow(dead_code)]
    #[must_use]
    pub fn network(mut self, network: NetworkPolicy) -> Self {
        self.network = network;
        self
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.environment.insert(key.into(), value.into());
        self
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn secret(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.secrets.insert(key.into(), SecretValue::new(value));
        self
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn secret_arg(mut self, index: usize, value: impl Into<String>) -> Self {
        self.secret_args.insert(index, SecretValue::new(value));
        self
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

    fn effective_environment(&self) -> BTreeMap<String, String> {
        let mut environment = BTreeMap::from([
            ("LANG".to_owned(), "C".to_owned()),
            ("LC_ALL".to_owned(), "C".to_owned()),
            ("TZ".to_owned(), "UTC".to_owned()),
        ]);
        if let Ok(path) = std::env::var("PATH") {
            environment.insert("PATH".to_owned(), path);
        }
        match self.profile {
            EnvironmentProfile::RustTool => {
                environment.insert("CARGO_TERM_COLOR".to_owned(), "never".to_owned());
                environment.insert("CARGO_NET_OFFLINE".to_owned(), "true".to_owned());
                for key in [
                    "CARGO_BUILD_JOBS",
                    "CARGO_INCREMENTAL",
                    "CARGO_PROFILE_DEV_DEBUG",
                    "CARGO_PROFILE_TEST_DEBUG",
                ] {
                    if let Ok(value) = std::env::var(key) {
                        environment.insert(key.to_owned(), value);
                    }
                }
            }
            EnvironmentProfile::GitTool => {
                environment.insert("GIT_CONFIG_NOSYSTEM".to_owned(), "1".to_owned());
                environment.insert("GIT_TERMINAL_PROMPT".to_owned(), "0".to_owned());
            }
            EnvironmentProfile::GitHubApi => {
                environment.insert("GIT_CONFIG_NOSYSTEM".to_owned(), "1".to_owned());
            }
            EnvironmentProfile::ReferenceTool => {
                environment.insert("RUST_BACKTRACE".to_owned(), "0".to_owned());
            }
            EnvironmentProfile::PlatformTool
            | EnvironmentProfile::TestFixture
            | EnvironmentProfile::Empty => {}
        }
        if self.network == NetworkPolicy::None {
            environment.insert("CARGO_NET_OFFLINE".to_owned(), "true".to_owned());
        }
        environment.extend(self.environment.clone());
        environment.extend(
            self.secrets
                .iter()
                .map(|(key, value)| (key.clone(), value.0.clone())),
        );
        environment
    }

    fn public_environment(&self) -> BTreeMap<String, String> {
        let mut environment = self.effective_environment();
        for key in self.secrets.keys() {
            environment.remove(key);
        }
        environment
    }

    #[allow(dead_code)]
    fn effective_args(&self) -> Vec<String> {
        self.args
            .iter()
            .enumerate()
            .map(|(index, arg)| {
                self.secret_args
                    .get(&index)
                    .map_or_else(|| arg.clone(), |secret| secret.0.clone())
            })
            .collect()
    }

    #[allow(dead_code)]
    fn public_args(&self) -> Vec<String> {
        self.args
            .iter()
            .enumerate()
            .map(|(index, arg)| {
                if self.secret_args.contains_key(&index) {
                    "[REDACTED]".to_owned()
                } else {
                    arg.clone()
                }
            })
            .collect()
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
    #[serde(default)]
    pub logical_command_id: String,
    #[serde(default)]
    pub redacted_args: Vec<String>,
    #[serde(default)]
    pub path_aliases: BTreeMap<String, String>,
    #[serde(default)]
    pub environment_names: Vec<String>,
    #[serde(default)]
    pub environment_policy_hash: String,
    #[serde(default)]
    pub network_policy: String,
    #[serde(default)]
    pub process_ownership: String,
    #[serde(default)]
    pub process_id: u32,
    #[serde(default)]
    pub cleanup: CleanupReceipt,
    #[serde(default)]
    pub artifacts: Vec<ArtifactReceipt>,
    #[serde(default)]
    pub stdout_error: Option<String>,
    #[serde(default)]
    pub stderr_error: Option<String>,
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

    /// Runs a command with an allowlisted environment, owned process tree, and bounded pipes.
    pub fn run(&self, mut request: ProcessRequest) -> Result<ProcessResult, ProcessError> {
        if request.limits == ProcessLimits::default() {
            request.limits = self.default_limits;
        }
        validate_limits(request.limits)?;
        let started = SystemTime::now();
        let (mut child, receiver, stdout_thread, stderr_thread) = spawn_process(&request)?;
        let process_id = child.id();
        let mut streams = Streams::default();
        let (reason, exit_code, cleanup) =
            wait_for_exit(child.as_mut(), &receiver, &mut streams, &request)?;
        join_readers(
            &receiver,
            &mut streams,
            stdout_thread,
            stderr_thread,
            reason,
        )?;
        if let Some((path, message)) = streams.artifact_error {
            return Err(ProcessError::Artifact { path, message });
        }
        let artifacts = artifact_receipts::collect(
            &request,
            started,
            &streams.stdout.bytes,
            &streams.stderr.bytes,
        )?;
        let public_environment = request.public_environment();
        let receipt = CommandReceipt {
            schema_version: 2,
            program: stable_path(&request.program, request.current_dir.as_deref()),
            args: redact_args(
                &request.public_args(),
                &request.secrets,
                request.current_dir.as_deref(),
            ),
            current_dir: request
                .current_dir
                .as_deref()
                .map(|_| "<worktree>".to_owned()),
            environment_hash: hash_environment(&public_environment),
            status: reason.status().to_owned(),
            exit_code,
            stdout_hash: hash_bytes(&streams.stdout.bytes),
            stderr_hash: hash_bytes(&streams.stderr.bytes),
            stdout_truncated: streams.stdout.truncated,
            stderr_truncated: streams.stderr.truncated,
            logical_command_id: logical_command_id(&request),
            redacted_args: redact_args(
                &request.public_args(),
                &request.secrets,
                request.current_dir.as_deref(),
            ),
            path_aliases: path_aliases(&request),
            environment_names: public_environment.keys().cloned().collect(),
            environment_policy_hash: hash_policy(&request),
            network_policy: request.network.name().to_owned(),
            process_ownership: ownership_name(),
            process_id,
            cleanup,
            artifacts,
            stdout_error: streams.stdout.read_error,
            stderr_error: streams.stderr.read_error,
        };
        Ok(ProcessResult {
            receipt,
            stdout: streams.stdout.bytes,
            stderr: streams.stderr.bytes,
        })
    }
}

type RunningProcess = (
    Box<dyn ChildWrapper>,
    Receiver<StreamEvent>,
    JoinHandle<()>,
    JoinHandle<()>,
);

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
    let environment = request.effective_environment();
    let args = request.effective_args();
    let mut command = CommandWrap::with_new(&request.program, |command| {
        command
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear()
            .envs(&environment);
        if let Some(directory) = &request.current_dir {
            command.current_dir(directory);
        }
    });
    #[cfg(unix)]
    command.wrap(ProcessSession);
    #[cfg(windows)]
    command.wrap(JobObject);
    let mut child = command.spawn().map_err(|source| ProcessError::Spawn {
        program: request.program.clone(),
        message: source.to_string(),
    })?;
    let stdout = child
        .stdout()
        .take()
        .ok_or(ProcessError::MissingPipe("stdout"))?;
    let stderr = child
        .stderr()
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
    child: &mut dyn ChildWrapper,
    receiver: &Receiver<StreamEvent>,
    streams: &mut Streams,
    request: &ProcessRequest,
) -> Result<(StopReason, Option<i32>, CleanupReceipt), ProcessError> {
    let deadline = Instant::now() + request.limits.timeout;
    let cancellation = request
        .cancellation
        .clone()
        .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
    let mut reason = StopReason::None;
    let mut exit_code = None;
    while matches!(reason, StopReason::None) && exit_code.is_none() {
        drain_events(receiver, streams);
        if cancellation.load(Ordering::Acquire) {
            reason = StopReason::Cancelled;
        } else if Instant::now() >= deadline {
            reason = StopReason::TimedOut;
        } else if let Some(status) = child.try_wait().map_err(ProcessError::Wait)? {
            exit_code = status.code();
        } else {
            match receiver.recv_timeout(Duration::from_millis(10)) {
                Ok(event) => accept_event(event, streams),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {}
            }
        }
    }
    let cleanup = if matches!(reason, StopReason::None) {
        let status = child.wait().map_err(ProcessError::Wait)?;
        exit_code = status.code();
        CleanupReceipt {
            outcome: "reaped".to_owned(),
            ..CleanupReceipt::default()
        }
    } else {
        terminate_owned_process(child)?
    };
    Ok((reason, exit_code, cleanup))
}

fn terminate_owned_process(child: &mut dyn ChildWrapper) -> Result<CleanupReceipt, ProcessError> {
    let started = Instant::now();
    if child.try_wait().map_err(ProcessError::Wait)?.is_some() {
        let _ = child.wait().map_err(ProcessError::Wait)?;
        return Ok(CleanupReceipt {
            outcome: "reaped".to_owned(),
            termination: "already-exited".to_owned(),
            grace_ms: started.elapsed().as_millis(),
            drain_ms: 0,
        });
    }
    #[cfg(unix)]
    child.signal(15).map_err(ProcessError::Cleanup)?;
    let grace_deadline = Instant::now() + CLEANUP_GRACE;
    while Instant::now() < grace_deadline {
        if child.try_wait().map_err(ProcessError::Wait)?.is_some() {
            return Ok(CleanupReceipt {
                outcome: "reaped".to_owned(),
                termination: "term".to_owned(),
                grace_ms: started.elapsed().as_millis(),
                drain_ms: 0,
            });
        }
        thread::sleep(Duration::from_millis(10));
    }
    child.start_kill().map_err(ProcessError::Cleanup)?;
    let status = child.wait().map_err(ProcessError::Wait)?;
    Ok(CleanupReceipt {
        outcome: "reaped".to_owned(),
        termination: "kill".to_owned(),
        grace_ms: started.elapsed().as_millis(),
        drain_ms: u128::from(status.code().is_none()),
    })
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
    Spawn {
        program: String,
        message: String,
    },
    MissingPipe(&'static str),
    Wait(std::io::Error),
    Cleanup(std::io::Error),
    ReaderPanic,
    DrainTimeout,
    OutputRead {
        stream: &'static str,
        message: String,
    },
    Artifact {
        path: PathBuf,
        message: String,
    },
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
            Self::Cleanup(error) => write!(formatter, "cannot clean up child process: {error}"),
            Self::ReaderPanic => formatter.write_str("child output reader panicked"),
            Self::DrainTimeout => formatter.write_str("child output drain exceeded its deadline"),
            Self::OutputRead { stream, message } => {
                write!(formatter, "cannot read {stream}: {message}")
            }
            Self::Artifact { path, message } => write!(
                formatter,
                "cannot write process artifact {}: {message}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ProcessError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamKind {
    Stdout,
    Stderr,
}

impl StreamKind {
    const fn name(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

#[derive(Debug)]
enum StreamEvent {
    Data(StreamKind, Vec<u8>),
    Limit(StreamKind),
    Done(StreamKind),
    ReadError(StreamKind, String),
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
                Ok(0) => {
                    finish_reader(&sender, stream, bytes, truncated, artifact.take());
                    return;
                }
                Err(error) => {
                    let _ = sender.send(StreamEvent::ReadError(stream, error.to_string()));
                    finish_reader(&sender, stream, bytes, truncated, artifact.take());
                    return;
                }
                Ok(count) => {
                    let chunk = &buffer[..count];
                    if let Some(artifact) = artifact.as_mut() {
                        artifact.push(chunk);
                    }
                    let retained = bytes.len();
                    if retained < limit {
                        bytes.extend_from_slice(&chunk[..(limit - retained).min(count)]);
                    }
                    truncated |= retained.saturating_add(count) > limit;
                }
            }
        }
    })
}

fn finish_reader(
    sender: &Sender<StreamEvent>,
    stream: StreamKind,
    bytes: Vec<u8>,
    truncated: bool,
    artifact: Option<ArtifactWriter>,
) {
    let _ = sender.send(StreamEvent::Data(stream, bytes));
    if truncated {
        let _ = sender.send(StreamEvent::Limit(stream));
    }
    if let Some(artifact) = artifact {
        let path = artifact.path.clone();
        if let Err(message) = artifact.finish() {
            let _ = sender.send(StreamEvent::ArtifactError(stream, path, message));
        }
    }
    let _ = sender.send(StreamEvent::Done(stream));
}

#[derive(Debug, Default)]
struct StreamCapture {
    bytes: Vec<u8>,
    done: bool,
    truncated: bool,
    read_error: Option<String>,
}

#[derive(Debug, Default)]
struct Streams {
    stdout: StreamCapture,
    stderr: StreamCapture,
    artifact_error: Option<(PathBuf, String)>,
}

fn drain_events(receiver: &Receiver<StreamEvent>, streams: &mut Streams) {
    loop {
        match receiver.try_recv() {
            Ok(event) => accept_event(event, streams),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => return,
        }
    }
}

fn accept_event(event: StreamEvent, streams: &mut Streams) {
    match event {
        StreamEvent::Data(StreamKind::Stdout, bytes) => streams.stdout.bytes = bytes,
        StreamEvent::Data(StreamKind::Stderr, bytes) => streams.stderr.bytes = bytes,
        StreamEvent::Limit(StreamKind::Stdout) => streams.stdout.truncated = true,
        StreamEvent::Limit(StreamKind::Stderr) => streams.stderr.truncated = true,
        StreamEvent::Done(StreamKind::Stdout) => streams.stdout.done = true,
        StreamEvent::Done(StreamKind::Stderr) => streams.stderr.done = true,
        StreamEvent::ReadError(stream, message) => match stream {
            StreamKind::Stdout => streams.stdout.read_error = Some(message),
            StreamKind::Stderr => streams.stderr.read_error = Some(message),
        },
        StreamEvent::ArtifactError(stream, path, message) => {
            streams.artifact_error = Some((path, format!("{}: {message}", stream.name())));
        }
    }
}

fn join_readers(
    receiver: &Receiver<StreamEvent>,
    streams: &mut Streams,
    stdout_thread: JoinHandle<()>,
    stderr_thread: JoinHandle<()>,
    _reason: StopReason,
) -> Result<(), ProcessError> {
    let deadline = Instant::now() + DRAIN_DEADLINE;
    while !(streams.stdout.done && streams.stderr.done) && Instant::now() < deadline {
        match receiver.recv_timeout(Duration::from_millis(25)) {
            Ok(event) => accept_event(event, streams),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {}
        }
    }
    if !(streams.stdout.done && streams.stderr.done) {
        return Err(ProcessError::DrainTimeout);
    }
    stdout_thread
        .join()
        .map_err(|_| ProcessError::ReaderPanic)?;
    stderr_thread
        .join()
        .map_err(|_| ProcessError::ReaderPanic)?;
    if let Some(message) = streams.stdout.read_error.clone() {
        return Err(ProcessError::OutputRead {
            stream: "stdout",
            message,
        });
    }
    if let Some(message) = streams.stderr.read_error.clone() {
        return Err(ProcessError::OutputRead {
            stream: "stderr",
            message,
        });
    }
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

mod artifacts;
use artifacts::ArtifactWriter;
pub(crate) use artifacts::write_bounded_artifact;

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

fn hash_policy(request: &ProcessRequest) -> String {
    let names = request
        .public_environment()
        .keys()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    hash_bytes(
        format!(
            "{}\n{}\n{}",
            request.profile.name(),
            request.network.name(),
            names
        )
        .as_bytes(),
    )
}

fn logical_command_id(request: &ProcessRequest) -> String {
    hash_bytes(format!("{}\n{}", request.program, request.public_args().join("\n")).as_bytes())
        [..16]
        .to_owned()
}

fn stable_path(path: &str, current_dir: Option<&Path>) -> String {
    current_dir
        .and_then(|dir| path.strip_prefix(dir.to_string_lossy().as_ref()))
        .map_or_else(
            || {
                if Path::new(path).is_absolute() {
                    "<program>".to_owned()
                } else {
                    path.to_owned()
                }
            },
            |suffix| format!("<worktree>{suffix}"),
        )
}

fn redact_args(
    args: &[String],
    secrets: &BTreeMap<String, SecretValue>,
    current_dir: Option<&Path>,
) -> Vec<String> {
    args.iter()
        .map(|arg| {
            if secrets
                .values()
                .any(|secret| !secret.0.is_empty() && arg.contains(&secret.0))
            {
                return "[REDACTED]".to_owned();
            }
            if let Some(dir) = current_dir
                .and_then(|dir| dir.to_str())
                .filter(|dir| arg.starts_with(*dir))
            {
                return format!("<worktree>{}", &arg[dir.len()..]);
            }
            if Path::new(arg).is_absolute() {
                "<path>".to_owned()
            } else {
                arg.clone()
            }
        })
        .collect()
}

fn path_aliases(request: &ProcessRequest) -> BTreeMap<String, String> {
    request
        .current_dir
        .as_ref()
        .map_or_else(BTreeMap::new, |dir| {
            BTreeMap::from([
                (String::from("current_dir"), String::from("<worktree>")),
                (dir.display().to_string(), String::from("<worktree>")),
            ])
        })
}

fn ownership_name() -> String {
    #[cfg(unix)]
    {
        "unix-process-session".to_owned()
    }
    #[cfg(windows)]
    {
        return "windows-job-object".to_owned();
    }
    #[cfg(not(any(unix, windows)))]
    {
        "platform-process-wrapper".to_owned()
    }
}

#[cfg(test)]
mod tests;
