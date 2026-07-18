use std::fmt;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const BENCH_SCHEMA_VERSION: u32 = 1;
const SHA256_LENGTH: usize = 64;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct BenchmarkScenario {
    pub id: String,
    pub fixture_ids: Vec<String>,
    pub width: u32,
    pub height: u32,
    pub operations: Vec<String>,
    pub thread_count: u32,
    pub memory_cap_bytes: u64,
    pub timeout_ms: u64,
    pub expected_output_sha256: String,
    pub warmup_iterations: u32,
    pub repetitions: u32,
}

impl BenchmarkScenario {
    /// Validates one benchmark workload before it can produce a receipt.
    ///
    /// # Errors
    ///
    /// Returns an error for empty identities, dimensions, resource bounds, or
    /// malformed expected output hashes.
    pub fn validate(&self) -> Result<(), BenchmarkError> {
        if self.id.trim().is_empty() || self.fixture_ids.is_empty() || self.operations.is_empty() {
            return Err(BenchmarkError::InvalidScenario(
                self.id.clone(),
                "identity is incomplete".to_owned(),
            ));
        }
        if self.width == 0
            || self.height == 0
            || self.thread_count == 0
            || self.memory_cap_bytes == 0
            || self.timeout_ms == 0
            || self.repetitions == 0
        {
            return Err(BenchmarkError::InvalidScenario(
                self.id.clone(),
                "resource or dimension bound is zero".to_owned(),
            ));
        }
        validate_hash(&self.expected_output_sha256, "expected output")
    }

    #[must_use]
    pub fn workload_identity(&self) -> String {
        let canonical = format!(
            "{}\0{}\0{}x{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
            self.id,
            self.fixture_ids.join(","),
            self.width,
            self.height,
            self.operations.join(","),
            self.thread_count,
            self.memory_cap_bytes,
            self.timeout_ms,
            self.expected_output_sha256,
            self.warmup_iterations,
            self.repetitions,
            BENCH_SCHEMA_VERSION,
        );
        sha256(canonical.as_bytes())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct HostFingerprint {
    pub target: String,
    pub cpu_model: String,
    pub cpu_features: Vec<String>,
    pub gpu_adapter: Option<String>,
    pub gpu_driver: Option<String>,
    pub gpu_backend: Option<String>,
    pub thread_count: u32,
    pub build_profile: String,
    pub source_sha: String,
}

impl HostFingerprint {
    #[must_use]
    pub fn local(source_sha: impl Into<String>, build_profile: impl Into<String>) -> Self {
        Self {
            target: std::env::var("TARGET").unwrap_or_else(|_| std::env::consts::ARCH.to_owned()),
            cpu_model: std::env::var("RUSTTABLE_CPU_MODEL")
                .unwrap_or_else(|_| "unknown".to_owned()),
            cpu_features: std::env::var("RUSTTABLE_CPU_FEATURES").map_or_else(
                |_| Vec::new(),
                |value| value.split(',').map(str::to_owned).collect(),
            ),
            gpu_adapter: None,
            gpu_driver: None,
            gpu_backend: None,
            thread_count: std::thread::available_parallelism()
                .map_or(1, |count| u32::try_from(count.get()).unwrap_or(u32::MAX)),
            build_profile: build_profile.into(),
            source_sha: source_sha.into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct MetricSample {
    pub wall_time_ns: u64,
    pub cpu_time_ns: u64,
    pub peak_resident_bytes: u64,
    pub allocated_bytes: u64,
    pub allocation_count: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub decoded_megapixels: u64,
    pub processed_megapixels: u64,
    pub preview_latency_ns: Option<u64>,
    pub gpu_upload_bytes: u64,
    pub gpu_download_bytes: u64,
    pub gpu_dispatch_ns: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Percentiles {
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
}

impl Percentiles {
    fn from_samples(samples: &[MetricSample]) -> Self {
        let mut values = samples
            .iter()
            .map(|sample| sample.wall_time_ns)
            .collect::<Vec<_>>();
        values.sort_unstable();
        Self {
            p50_ns: percentile(&values, 50),
            p95_ns: percentile(&values, 95),
            p99_ns: percentile(&values, 99),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct BenchmarkSummary {
    pub samples: u32,
    pub latency: Percentiles,
    pub peak_resident_bytes: u64,
    pub allocated_bytes: u64,
    pub allocation_count: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub decoded_megapixels: u64,
    pub processed_megapixels: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum ReceiptStatus {
    Completed,
    Interrupted,
    TimedOut,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct BenchmarkReceipt {
    pub schema_version: u32,
    pub scenario_id: String,
    pub workload_identity: String,
    pub output_sha256: String,
    pub host: HostFingerprint,
    pub status: ReceiptStatus,
    pub samples: Vec<MetricSample>,
    pub summary: BenchmarkSummary,
}

impl BenchmarkReceipt {
    /// Returns stable, machine-readable receipt JSON.
    ///
    /// # Errors
    ///
    /// Returns an error if a receipt is not complete or cannot be serialized.
    pub fn stable_json(&self) -> Result<String, BenchmarkError> {
        self.validate()?;
        serde_json::to_string(self)
            .map_err(|error| BenchmarkError::Serialization(error.to_string()))
    }

    /// Checks receipt identity, sample counts, and the success invariant.
    ///
    /// # Errors
    ///
    /// Returns an error when the receipt is interrupted, timed out, malformed,
    /// or inconsistent with its summary.
    pub fn validate(&self) -> Result<(), BenchmarkError> {
        if self.schema_version != BENCH_SCHEMA_VERSION || self.status != ReceiptStatus::Completed {
            return Err(BenchmarkError::Incomplete);
        }
        if self.samples.is_empty()
            || self.scenario_id.is_empty()
            || self.workload_identity.len() != SHA256_LENGTH
        {
            return Err(BenchmarkError::InvalidReceipt(
                "identity or samples are incomplete".to_owned(),
            ));
        }
        validate_hash(&self.output_sha256, "receipt output")
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct BaselineComparison {
    pub scenario_id: String,
    pub workload_identity: String,
    pub host: HostFingerprint,
    pub wall_time_delta_ns: i128,
    pub wall_time_delta_percent_milli: i64,
    pub memory_delta_bytes: i128,
    pub compared: bool,
}

#[derive(Debug, Clone)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BenchmarkRunner {
    pub poll_interval: Duration,
}

impl Default for BenchmarkRunner {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(2),
        }
    }
}

impl BenchmarkRunner {
    /// Builds a deterministic receipt from measured samples.
    ///
    /// # Errors
    ///
    /// Returns an error when the scenario, samples, output hash, cancellation,
    /// or timeout state is invalid.
    pub fn receipt(
        &self,
        scenario: &BenchmarkScenario,
        host: HostFingerprint,
        output_sha256: String,
        samples: Vec<MetricSample>,
        status: ReceiptStatus,
    ) -> Result<BenchmarkReceipt, BenchmarkError> {
        scenario.validate()?;
        validate_hash(&output_sha256, "output")?;
        if status != ReceiptStatus::Completed
            || samples.len() != usize::try_from(scenario.repetitions).unwrap_or(0)
        {
            return Err(BenchmarkError::Incomplete);
        }
        let summary = summarize(&samples)?;
        let receipt = BenchmarkReceipt {
            schema_version: BENCH_SCHEMA_VERSION,
            scenario_id: scenario.id.clone(),
            workload_identity: scenario.workload_identity(),
            output_sha256,
            host,
            status,
            samples,
            summary,
        };
        receipt.validate()?;
        Ok(receipt)
    }

    /// Runs a bounded child command for process-isolated benchmark setup.
    ///
    /// # Errors
    ///
    /// Returns a timeout, cancellation, spawn, or non-zero-exit error. The
    /// child is killed before any non-success result is returned.
    pub fn run_isolated_command(
        &self,
        program: &str,
        args: &[String],
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<ProcessReceipt, BenchmarkError> {
        let started = Instant::now();
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| BenchmarkError::Process(error.to_string()))?;
        loop {
            if cancellation.is_cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(BenchmarkError::Interrupted);
            }
            if started.elapsed() > timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err(BenchmarkError::TimedOut);
            }
            if let Some(status) = child
                .try_wait()
                .map_err(|error| BenchmarkError::Process(error.to_string()))?
            {
                if !status.success() {
                    return Err(BenchmarkError::Process(format!(
                        "child exited with {status}"
                    )));
                }
                return Ok(ProcessReceipt {
                    duration_ns: u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
                });
            }
            thread::sleep(self.poll_interval);
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProcessReceipt {
    pub duration_ns: u64,
}

/// Compares receipts only when host and workload identities match exactly.
///
/// # Errors
///
/// Returns an error for invalid receipts, different hosts, or different workloads.
pub fn compare_baseline(
    current: &BenchmarkReceipt,
    baseline: &BenchmarkReceipt,
) -> Result<BaselineComparison, BenchmarkError> {
    current.validate()?;
    baseline.validate()?;
    if current.host != baseline.host {
        return Err(BenchmarkError::HostMismatch);
    }
    if current.workload_identity != baseline.workload_identity
        || current.scenario_id != baseline.scenario_id
    {
        return Err(BenchmarkError::WorkloadMismatch);
    }
    let current_time = i128::from(current.summary.latency.p50_ns);
    let baseline_time = i128::from(baseline.summary.latency.p50_ns);
    let delta = current_time - baseline_time;
    let percent = if baseline_time == 0 {
        0
    } else {
        i64::try_from((delta * 100_000) / baseline_time).unwrap_or(i64::MAX)
    };
    Ok(BaselineComparison {
        scenario_id: current.scenario_id.clone(),
        workload_identity: current.workload_identity.clone(),
        host: current.host.clone(),
        wall_time_delta_ns: delta,
        wall_time_delta_percent_milli: percent,
        memory_delta_bytes: i128::from(current.summary.peak_resident_bytes)
            - i128::from(baseline.summary.peak_resident_bytes),
        compared: true,
    })
}

#[must_use]
pub fn initial_scenarios() -> Vec<BenchmarkScenario> {
    [
        ("catalog-open-checkpoint", "catalog.fixture", "catalog.open"),
        ("import-registration", "catalog.fixture", "import.register"),
        ("raster-decode", "raster.png.16-alpha", "decode.raster"),
        ("raw-decode-placeholder", "raw.bayer.12-2row", "decode.raw"),
        (
            "thumbnail-generation",
            "raster.png.16-alpha",
            "thumbnail.generate",
        ),
        (
            "minimal-cpu-pipeline",
            "raster.png.16-alpha",
            "pipeline.cpu",
        ),
        (
            "minimal-wgpu-pipeline-placeholder",
            "raster.png.16-alpha",
            "pipeline.wgpu",
        ),
        ("preview-update", "raster.png.16-alpha", "preview.update"),
        ("full-export", "raster.png.16-alpha", "export.full"),
        (
            "library-projection-10k",
            "catalog.fixture",
            "library.project",
        ),
    ]
    .into_iter()
    .map(|(id, fixture, operation)| BenchmarkScenario {
        id: id.to_owned(),
        fixture_ids: vec![fixture.to_owned()],
        width: 1,
        height: 1,
        operations: vec![operation.to_owned()],
        thread_count: 1,
        memory_cap_bytes: 64 * 1024 * 1024,
        timeout_ms: 30_000,
        expected_output_sha256: "0".repeat(SHA256_LENGTH),
        warmup_iterations: 1,
        repetitions: 3,
    })
    .collect()
}

fn summarize(samples: &[MetricSample]) -> Result<BenchmarkSummary, BenchmarkError> {
    if samples.is_empty() {
        return Err(BenchmarkError::InvalidReceipt("no samples".to_owned()));
    }
    Ok(BenchmarkSummary {
        samples: u32::try_from(samples.len()).map_err(|_| BenchmarkError::Overflow)?,
        latency: Percentiles::from_samples(samples),
        peak_resident_bytes: samples
            .iter()
            .map(|sample| sample.peak_resident_bytes)
            .max()
            .unwrap_or(0),
        allocated_bytes: samples.iter().map(|sample| sample.allocated_bytes).sum(),
        allocation_count: samples.iter().map(|sample| sample.allocation_count).sum(),
        cache_hits: samples.iter().map(|sample| sample.cache_hits).sum(),
        cache_misses: samples.iter().map(|sample| sample.cache_misses).sum(),
        decoded_megapixels: samples.iter().map(|sample| sample.decoded_megapixels).sum(),
        processed_megapixels: samples
            .iter()
            .map(|sample| sample.processed_megapixels)
            .sum(),
    })
}

fn percentile(values: &[u64], percentile: usize) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let index = ((values.len() - 1) * percentile).div_ceil(100);
    values[index.min(values.len() - 1)]
}

fn validate_hash(value: &str, label: &str) -> Result<(), BenchmarkError> {
    if value.len() != SHA256_LENGTH || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BenchmarkError::InvalidReceipt(format!(
            "{label} is not a SHA-256"
        )));
    }
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BenchmarkError {
    InvalidScenario(String, String),
    InvalidReceipt(String),
    Incomplete,
    HostMismatch,
    WorkloadMismatch,
    Interrupted,
    TimedOut,
    Process(String),
    Serialization(String),
    Overflow,
}

impl fmt::Display for BenchmarkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidScenario(id, message) => {
                write!(formatter, "invalid scenario {id}: {message}")
            }
            Self::InvalidReceipt(message) => {
                write!(formatter, "invalid benchmark receipt: {message}")
            }
            Self::Incomplete => formatter.write_str("benchmark did not complete successfully"),
            Self::HostMismatch => formatter.write_str("baseline host fingerprint differs"),
            Self::WorkloadMismatch => formatter.write_str("benchmark workload identity differs"),
            Self::Interrupted => formatter.write_str("benchmark was interrupted"),
            Self::TimedOut => formatter.write_str("benchmark timed out"),
            Self::Process(message) => write!(formatter, "benchmark process failed: {message}"),
            Self::Serialization(message) => write!(
                formatter,
                "benchmark receipt serialization failed: {message}"
            ),
            Self::Overflow => formatter.write_str("benchmark metric overflowed"),
        }
    }
}

impl std::error::Error for BenchmarkError {}
