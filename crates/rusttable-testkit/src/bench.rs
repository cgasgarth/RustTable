use std::fmt;
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};

use process_wrap::std::CommandWrap;
#[cfg(windows)]
use process_wrap::std::JobObject;
#[cfg(unix)]
use process_wrap::std::ProcessSession;
use serde::{Deserialize, Serialize};

#[path = "bench_helpers.rs"]
mod helpers;
#[path = "bench_scenarios.rs"]
mod scenarios;
use helpers::{
    csv_env, delta, env_bool, env_optional, env_or, env_u32, env_u64, environment_check,
    metric_delta, metric_value_delta, sha256, summarize, terminate_process_tree, validate_hash,
};
pub use scenarios::initial_scenarios;

pub const BENCH_SCHEMA_VERSION: u32 = 2;
const SHA256_LENGTH: usize = 64;
const MIN_REPETITIONS: u32 = 5;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ScenarioState {
    InactivePlaceholder,
    Qualification,
    Active,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct BenchmarkScenario {
    pub id: String,
    pub state: ScenarioState,
    pub fixture_ids: Vec<String>,
    pub fixture_content_sha256: Vec<String>,
    pub width: u32,
    pub height: u32,
    pub operations: Vec<String>,
    pub thread_count: u32,
    pub memory_cap_bytes: u64,
    pub timeout_ms: u64,
    /// Exact expected output identity, required only once a scenario is active.
    pub expected_output_sha256: Option<String>,
    pub warmup_iterations: u32,
    pub repetitions: u32,
    pub requires_gpu: bool,
    pub blocking_issue: Option<u32>,
}

impl BenchmarkScenario {
    /// Validates scenario identity and prevents placeholders from becoming gates.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed identity, bounds, hashes, or state.
    pub fn validate(&self) -> Result<(), BenchmarkError> {
        if self.id.trim().is_empty()
            || self.fixture_ids.is_empty()
            || self.fixture_ids.len() != self.fixture_content_sha256.len()
            || self.operations.is_empty()
        {
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
        {
            return Err(BenchmarkError::InvalidScenario(
                self.id.clone(),
                "resource or dimension bound is zero".to_owned(),
            ));
        }
        for hash in &self.fixture_content_sha256 {
            validate_hash(hash, "fixture")?;
        }
        match self.state {
            ScenarioState::Active => {
                if self.width == 1 && self.height == 1 {
                    return Err(BenchmarkError::InvalidScenario(
                        self.id.clone(),
                        "active scenario cannot use a 1x1 placeholder".to_owned(),
                    ));
                }
                if self.repetitions < MIN_REPETITIONS {
                    return Err(BenchmarkError::InvalidScenario(
                        self.id.clone(),
                        format!("active scenario needs {MIN_REPETITIONS} repetitions"),
                    ));
                }
                let expected_output_sha256 =
                    self.expected_output_sha256.as_deref().ok_or_else(|| {
                        BenchmarkError::InvalidScenario(
                            self.id.clone(),
                            "active scenario needs an expected output identity".to_owned(),
                        )
                    })?;
                validate_hash(expected_output_sha256, "expected output")?;
                if expected_output_sha256 == "0".repeat(SHA256_LENGTH) {
                    return Err(BenchmarkError::InvalidScenario(
                        self.id.clone(),
                        "expected output cannot be an all-zero placeholder".to_owned(),
                    ));
                }
            }
            ScenarioState::Qualification => {
                if self.blocking_issue.is_none() {
                    return Err(BenchmarkError::InvalidScenario(
                        self.id.clone(),
                        "qualification scenario needs a blocking issue".to_owned(),
                    ));
                }
            }
            ScenarioState::InactivePlaceholder => {
                if self.blocking_issue.is_none() {
                    return Err(BenchmarkError::InvalidScenario(
                        self.id.clone(),
                        "inactive scenario needs a blocking issue".to_owned(),
                    ));
                }
            }
        }
        if self.state == ScenarioState::Active && self.blocking_issue.is_some() {
            return Err(BenchmarkError::InvalidScenario(
                self.id.clone(),
                "active scenario cannot retain a blocking issue".to_owned(),
            ));
        }
        if self.repetitions == 0 {
            return Err(BenchmarkError::InvalidScenario(
                self.id.clone(),
                "repetitions must be nonzero".to_owned(),
            ));
        }
        Ok(())
    }

    /// Returns the canonical identity used to key reviewed baselines.
    #[must_use]
    pub fn workload(&self) -> WorkloadIdentity {
        WorkloadIdentity {
            scenario_id: self.id.clone(),
            fixture_ids: self.fixture_ids.clone(),
            fixture_content_sha256: self.fixture_content_sha256.clone(),
            width: self.width,
            height: self.height,
            operations: self.operations.clone(),
            thread_count: self.thread_count,
            memory_cap_bytes: self.memory_cap_bytes,
            timeout_ms: self.timeout_ms,
            expected_output_sha256: self.expected_output_sha256.clone(),
            warmup_iterations: self.warmup_iterations,
            repetitions: self.repetitions,
            harness_schema: BENCH_SCHEMA_VERSION,
        }
    }

    #[must_use]
    pub fn workload_identity(&self) -> String {
        self.workload().sha256()
    }

    /// Verifies output before a timing sample can be accepted.
    ///
    /// # Errors
    ///
    /// Returns an error when the output hash is malformed or differs from the scenario.
    pub fn validate_output(&self, output_sha256: &str) -> Result<(), BenchmarkError> {
        validate_hash(output_sha256, "output")?;
        let expected_output_sha256 = self.expected_output_sha256.as_deref().ok_or_else(|| {
            BenchmarkError::InvalidScenario(
                self.id.clone(),
                "scenario has no expected output identity".to_owned(),
            )
        })?;
        if output_sha256 != expected_output_sha256 {
            return Err(BenchmarkError::OutputMismatch {
                expected: expected_output_sha256.to_owned(),
                actual: output_sha256.to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct WorkloadIdentity {
    pub scenario_id: String,
    pub fixture_ids: Vec<String>,
    pub fixture_content_sha256: Vec<String>,
    pub width: u32,
    pub height: u32,
    pub operations: Vec<String>,
    pub thread_count: u32,
    pub memory_cap_bytes: u64,
    pub timeout_ms: u64,
    pub expected_output_sha256: Option<String>,
    pub warmup_iterations: u32,
    pub repetitions: u32,
    pub harness_schema: u32,
}

impl WorkloadIdentity {
    /// Returns the digest of the canonical, serialized workload identity.
    ///
    /// # Panics
    ///
    /// Panics only if the type's infallible serde representation becomes
    /// unserializable, which would indicate a programming error.
    #[must_use]
    pub fn sha256(&self) -> String {
        sha256(
            serde_json::to_vec(self)
                .expect("workload identity is serializable")
                .as_slice(),
        )
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct HostIdentity {
    pub schema_version: u32,
    pub target_triple: String,
    pub operating_system: String,
    pub kernel: String,
    pub cpu_model: String,
    pub cpu_microcode: String,
    pub cpu_features: Vec<String>,
    pub physical_cores: u32,
    pub logical_cores: u32,
    pub thread_policy: String,
    pub memory_bytes: Option<u64>,
    pub gpu_adapter: Option<String>,
    pub gpu_driver: Option<String>,
    pub gpu_backend: Option<String>,
    pub gpu_features: Vec<String>,
    pub power_mode: String,
    pub benchmark_runner_version: String,
}

impl HostIdentity {
    #[must_use]
    pub fn local() -> Self {
        let logical_cores = std::thread::available_parallelism()
            .map_or(1, |count| u32::try_from(count.get()).unwrap_or(u32::MAX));
        Self {
            schema_version: BENCH_SCHEMA_VERSION,
            target_triple: env_or(
                "RUSTTABLE_TARGET_TRIPLE",
                &env_or("TARGET", std::env::consts::ARCH),
            ),
            operating_system: std::env::consts::OS.to_owned(),
            kernel: env_or("RUSTTABLE_KERNEL", "unknown"),
            cpu_model: env_or("RUSTTABLE_CPU_MODEL", "unknown"),
            cpu_microcode: env_or("RUSTTABLE_CPU_MICROCODE", "unknown"),
            cpu_features: csv_env("RUSTTABLE_CPU_FEATURES"),
            physical_cores: env_u32("RUSTTABLE_PHYSICAL_CORES").unwrap_or(logical_cores),
            logical_cores,
            thread_policy: env_or("RUSTTABLE_THREAD_POLICY", "available-parallelism"),
            memory_bytes: env_u64("RUSTTABLE_MEMORY_BYTES"),
            gpu_adapter: env_optional("RUSTTABLE_GPU_ADAPTER"),
            gpu_driver: env_optional("RUSTTABLE_GPU_DRIVER"),
            gpu_backend: env_optional("RUSTTABLE_GPU_BACKEND"),
            gpu_features: csv_env("RUSTTABLE_GPU_FEATURES"),
            power_mode: env_or("RUSTTABLE_POWER_MODE", "unknown"),
            benchmark_runner_version: env_or("RUSTTABLE_BENCHMARK_RUNNER", "rusttable-testkit"),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct BuildIdentity {
    pub schema_version: u32,
    pub source_commit: String,
    pub dirty: bool,
    pub rustc: String,
    pub llvm: String,
    pub cargo_lock_sha256: String,
    pub feature_flags: Vec<String>,
    pub profile: String,
    pub codegen_units: Option<u32>,
    pub lto: String,
    pub panic: String,
    pub shader_hash: Option<String>,
    pub dependencies: Vec<String>,
}

impl BuildIdentity {
    #[must_use]
    pub fn local(source_commit: impl Into<String>, profile: impl Into<String>) -> Self {
        Self {
            schema_version: BENCH_SCHEMA_VERSION,
            source_commit: source_commit.into(),
            dirty: env_bool("RUSTTABLE_BUILD_DIRTY"),
            rustc: env_or("RUSTTABLE_RUSTC", "unknown"),
            llvm: env_or("RUSTTABLE_LLVM", "unknown"),
            cargo_lock_sha256: env_or("RUSTTABLE_CARGO_LOCK_SHA256", "unknown"),
            feature_flags: csv_env("RUSTTABLE_FEATURES"),
            profile: profile.into(),
            codegen_units: env_u32("RUSTTABLE_CODEGEN_UNITS"),
            lto: env_or("RUSTTABLE_LTO", "off"),
            panic: env_or("RUSTTABLE_PANIC", "unwind"),
            shader_hash: env_optional("RUSTTABLE_SHADER_HASH"),
            dependencies: csv_env("RUSTTABLE_BUILD_DEPENDENCIES"),
        }
    }

    /// Validates build identity fields required for a reviewed receipt.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty source commit or a dirty build.
    pub fn validate(&self) -> Result<(), BenchmarkError> {
        if self.source_commit.trim().is_empty() {
            return Err(BenchmarkError::InvalidReceipt(
                "build source commit is missing".to_owned(),
            ));
        }
        if self.dirty {
            return Err(BenchmarkError::InvalidReceipt(
                "dirty builds cannot produce reviewed benchmark receipts".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum QualificationOutcome {
    Passed,
    Unavailable,
    Failed,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct QualificationCheck {
    pub id: String,
    pub outcome: QualificationOutcome,
    pub detail: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct EnvironmentQualification {
    pub checks: Vec<QualificationCheck>,
    pub unstable: bool,
    pub concurrent_load_percent_milli: Option<u32>,
    pub memory_pressure_percent_milli: Option<u32>,
}

impl EnvironmentQualification {
    /// Returns the explicit host checks available from the local runner.
    #[must_use]
    pub fn local() -> Self {
        let concurrent_load = env_u32("RUSTTABLE_CONCURRENT_LOAD_PERCENT_MILLI");
        let memory_pressure = env_u32("RUSTTABLE_MEMORY_PRESSURE_PERCENT_MILLI");
        let thermal = env_optional("RUSTTABLE_THERMAL_STATE");
        let unstable = env_bool("RUSTTABLE_BENCH_UNSTABLE")
            || concurrent_load.is_some_and(|value| value > 10_000)
            || memory_pressure.is_some_and(|value| value > 10_000)
            || thermal.as_deref().is_some_and(|value| value != "stable");
        Self {
            checks: vec![
                environment_check("cpu-governor", "RUSTTABLE_CPU_GOVERNOR"),
                environment_check("power-mode", "RUSTTABLE_POWER_MODE"),
                environment_check("thermal-state", "RUSTTABLE_THERMAL_STATE"),
                environment_check("concurrent-load", "RUSTTABLE_CONCURRENT_LOAD_PERCENT_MILLI"),
                environment_check("memory-pressure", "RUSTTABLE_MEMORY_PRESSURE_PERCENT_MILLI"),
                environment_check("timer-resolution", "RUSTTABLE_TIMER_RESOLUTION_NS"),
                environment_check("gpu", "RUSTTABLE_GPU_ADAPTER"),
            ],
            unstable,
            concurrent_load_percent_milli: concurrent_load,
            memory_pressure_percent_milli: memory_pressure,
        }
    }

    #[must_use]
    pub fn qualified() -> Self {
        Self {
            checks: [
                "cpu-governor",
                "power-mode",
                "thermal-state",
                "concurrent-load",
                "memory-pressure",
                "timer-resolution",
            ]
            .into_iter()
            .map(|id| QualificationCheck {
                id: id.to_owned(),
                outcome: QualificationOutcome::Passed,
                detail: "test qualification environment".to_owned(),
            })
            .chain([QualificationCheck {
                id: "gpu".to_owned(),
                outcome: QualificationOutcome::Unavailable,
                detail: "GPU qualification is not required for CPU workloads".to_owned(),
            }])
            .collect(),
            unstable: false,
            concurrent_load_percent_milli: Some(0),
            memory_pressure_percent_milli: Some(0),
        }
    }

    #[must_use]
    pub fn is_qualified(&self) -> bool {
        !self.unstable
            && !self.checks.is_empty()
            && self.checks.iter().all(|check| {
                check.outcome == QualificationOutcome::Passed
                    || (check.id == "gpu" && check.outcome == QualificationOutcome::Unavailable)
            })
    }

    #[must_use]
    pub fn gpu_is_qualified(&self) -> bool {
        self.is_qualified()
            && self
                .checks
                .iter()
                .any(|check| check.id == "gpu" && check.outcome == QualificationOutcome::Passed)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RunEnvironment {
    pub host: HostIdentity,
    pub build: BuildIdentity,
    pub qualification: EnvironmentQualification,
}

impl RunEnvironment {
    #[must_use]
    pub fn local(source_commit: impl Into<String>, profile: impl Into<String>) -> Self {
        Self {
            host: HostIdentity::local(),
            build: BuildIdentity::local(source_commit, profile),
            qualification: EnvironmentQualification::local(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub enum MetricValue {
    Measured(u64),
    Unavailable,
}

impl MetricValue {
    fn add_all<'a, I>(values: I) -> Result<Self, BenchmarkError>
    where
        I: IntoIterator<Item = &'a MetricValue>,
    {
        let mut total = 0_u64;
        for value in values {
            let MetricValue::Measured(value) = value else {
                return Ok(Self::Unavailable);
            };
            total = total.checked_add(*value).ok_or(BenchmarkError::Overflow)?;
        }
        Ok(Self::Measured(total))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct MetricSample {
    pub sequence: u32,
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
    pub gpu_upload_bytes: MetricValue,
    pub gpu_download_bytes: MetricValue,
    pub gpu_dispatch_ns: MetricValue,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Percentiles {
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Uncertainty {
    pub minimum_ns: u64,
    pub maximum_ns: u64,
    pub median_absolute_deviation_ns: u64,
    pub noise_percent_milli: u64,
    pub stable: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct BenchmarkSummary {
    pub samples: u32,
    pub latency: Percentiles,
    pub cpu_time: Percentiles,
    pub preview_latency: Option<Percentiles>,
    pub uncertainty: Uncertainty,
    pub peak_resident_bytes: u64,
    pub allocated_bytes: u64,
    pub allocation_count: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub decoded_megapixels: u64,
    pub processed_megapixels: u64,
    pub throughput_pixels_per_second: u64,
    pub gpu_upload_bytes: MetricValue,
    pub gpu_download_bytes: MetricValue,
    pub gpu_dispatch_ns: MetricValue,
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
    pub scenario: BenchmarkScenario,
    pub scenario_id: String,
    pub workload_identity: String,
    pub output_sha256: String,
    pub environment: RunEnvironment,
    pub status: ReceiptStatus,
    pub samples: Vec<MetricSample>,
    pub summary: BenchmarkSummary,
}

impl BenchmarkReceipt {
    /// Returns stable, machine-readable receipt JSON after validating it.
    ///
    /// # Errors
    ///
    /// Returns an error when receipt validation or serialization fails.
    pub fn stable_json(&self) -> Result<String, BenchmarkError> {
        self.validate()?;
        serde_json::to_string(self)
            .map_err(|error| BenchmarkError::Serialization(error.to_string()))
    }

    /// Checks receipt identity, output qualification, sample uniqueness, and summary invariants.
    ///
    /// # Errors
    ///
    /// Returns an error when the receipt is incomplete, dirty, unstable, or
    /// inconsistent with its scenario, samples, or summary.
    pub fn validate(&self) -> Result<(), BenchmarkError> {
        self.scenario.validate()?;
        self.environment.build.validate()?;
        if self.schema_version != BENCH_SCHEMA_VERSION
            || self.status != ReceiptStatus::Completed
            || self.scenario.state != ScenarioState::Active
            || self.scenario_id != self.scenario.id
            || self.workload_identity != self.scenario.workload_identity()
            || !self.environment.qualification.is_qualified()
        {
            return Err(BenchmarkError::Incomplete);
        }
        self.scenario.validate_output(&self.output_sha256)?;
        if self.samples.len() != usize::try_from(self.scenario.repetitions).unwrap_or(0)
            || self.samples.iter().any(|sample| sample.wall_time_ns == 0)
        {
            return Err(BenchmarkError::InvalidReceipt(
                "sample count or wall time is invalid".to_owned(),
            ));
        }
        let mut sequences = self
            .samples
            .iter()
            .map(|sample| sample.sequence)
            .collect::<Vec<_>>();
        sequences.sort_unstable();
        if sequences.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(BenchmarkError::InvalidReceipt(
                "duplicate sample sequence".to_owned(),
            ));
        }
        if self.summary != summarize(&self.samples)? {
            return Err(BenchmarkError::InvalidReceipt(
                "summary does not match samples".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub enum ComparisonStatus {
    Comparable,
    NotComparable { reason: String },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct MetricDelta {
    pub p50_ns: i128,
    pub p95_ns: i128,
    pub p99_ns: i128,
    pub p50_percent_milli: i64,
    pub p95_percent_milli: i64,
    pub p99_percent_milli: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct BaselineComparison {
    pub scenario_id: String,
    pub workload_identity: String,
    pub status: ComparisonStatus,
    pub baseline_host: HostIdentity,
    pub current_host: HostIdentity,
    pub baseline_build: BuildIdentity,
    pub current_build: BuildIdentity,
    pub latency: Option<MetricDelta>,
    pub peak_memory_delta_bytes: Option<i128>,
    pub throughput_delta_pixels_per_second: Option<i128>,
    pub allocation_delta_bytes: Option<i128>,
    pub allocation_count_delta: Option<i128>,
    pub gpu_upload_delta_bytes: Option<i128>,
    pub gpu_download_delta_bytes: Option<i128>,
    pub gpu_dispatch_delta_ns: Option<i128>,
    pub compared: bool,
}

#[derive(Debug, Clone)]
pub struct CancellationToken(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl CancellationToken {
    #[must_use]
    pub fn new() -> Self {
        Self(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
            false,
        )))
    }
    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
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
    /// Builds a completed receipt only for an active, qualified scenario whose output passed.
    ///
    /// # Errors
    ///
    /// Returns an error when the scenario is inactive, output validation fails,
    /// qualification fails, samples are incomplete, or metrics overflow.
    pub fn receipt(
        &self,
        scenario: &BenchmarkScenario,
        environment: RunEnvironment,
        output_sha256: String,
        samples: Vec<MetricSample>,
        status: ReceiptStatus,
    ) -> Result<BenchmarkReceipt, BenchmarkError> {
        scenario.validate()?;
        if scenario.state != ScenarioState::Active {
            return Err(BenchmarkError::ScenarioNotActive(scenario.id.clone()));
        }
        if scenario.requires_gpu && !environment.qualification.gpu_is_qualified() {
            return Err(BenchmarkError::QualificationFailed(scenario.id.clone()));
        }
        scenario.validate_output(&output_sha256)?;
        if status != ReceiptStatus::Completed
            || samples.len() != usize::try_from(scenario.repetitions).unwrap_or(0)
        {
            return Err(BenchmarkError::Incomplete);
        }
        let summary = summarize(&samples)?;
        let receipt = BenchmarkReceipt {
            schema_version: BENCH_SCHEMA_VERSION,
            scenario: scenario.clone(),
            scenario_id: scenario.id.clone(),
            workload_identity: scenario.workload_identity(),
            output_sha256,
            environment,
            status,
            samples,
            summary,
        };
        receipt.validate()?;
        Ok(receipt)
    }

    /// Runs a bounded child with process-session/job ownership from #452.
    ///
    /// # Errors
    ///
    /// Returns an error when the child cannot be spawned, exits unsuccessfully,
    /// is cancelled, or exceeds its timeout.
    pub fn run_isolated_command(
        &self,
        program: &str,
        args: &[String],
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<ProcessReceipt, BenchmarkError> {
        let started = Instant::now();
        let mut command = CommandWrap::with_new(program, |command| {
            command
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("LANG", "C")
                .env("LC_ALL", "C")
                .env("TZ", "UTC")
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        });
        #[cfg(unix)]
        command.wrap(ProcessSession);
        #[cfg(windows)]
        command.wrap(JobObject);
        let mut child = command
            .spawn()
            .map_err(|error| BenchmarkError::Process(error.to_string()))?;
        loop {
            if cancellation.is_cancelled() {
                terminate_process_tree(child.as_mut());
                return Err(BenchmarkError::Interrupted);
            }
            if started.elapsed() > timeout {
                terminate_process_tree(child.as_mut());
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
                    cleanup: "process-tree-owned".to_owned(),
                });
            }
            thread::sleep(self.poll_interval);
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProcessReceipt {
    pub duration_ns: u64,
    pub cleanup: String,
}

/// Compares two valid receipts. Host/workload mismatches are typed non-comparable results.
///
/// # Errors
///
/// Returns an error when either receipt is invalid or metric aggregation overflows.
pub fn compare_baseline(
    current: &BenchmarkReceipt,
    baseline: &BenchmarkReceipt,
) -> Result<BaselineComparison, BenchmarkError> {
    current.validate()?;
    baseline.validate()?;
    let mut comparison = BaselineComparison {
        scenario_id: current.scenario_id.clone(),
        workload_identity: current.workload_identity.clone(),
        status: ComparisonStatus::Comparable,
        baseline_host: baseline.environment.host.clone(),
        current_host: current.environment.host.clone(),
        baseline_build: baseline.environment.build.clone(),
        current_build: current.environment.build.clone(),
        latency: None,
        peak_memory_delta_bytes: None,
        throughput_delta_pixels_per_second: None,
        allocation_delta_bytes: None,
        allocation_count_delta: None,
        gpu_upload_delta_bytes: None,
        gpu_download_delta_bytes: None,
        gpu_dispatch_delta_ns: None,
        compared: false,
    };
    let reason = if current.environment.host != baseline.environment.host {
        Some("host identity differs".to_owned())
    } else if current.workload_identity != baseline.workload_identity
        || current.scenario_id != baseline.scenario_id
    {
        Some("workload identity differs".to_owned())
    } else if !current.environment.qualification.is_qualified()
        || !baseline.environment.qualification.is_qualified()
    {
        Some("environment is unstable or unqualified".to_owned())
    } else if !current.summary.uncertainty.stable || !baseline.summary.uncertainty.stable {
        Some("sample uncertainty is unstable".to_owned())
    } else {
        None
    };
    if let Some(reason) = reason {
        comparison.status = ComparisonStatus::NotComparable { reason };
        return Ok(comparison);
    }
    comparison.latency = Some(metric_delta(
        &current.summary.latency,
        &baseline.summary.latency,
    )?);
    comparison.peak_memory_delta_bytes = Some(delta(
        current.summary.peak_resident_bytes,
        baseline.summary.peak_resident_bytes,
    ));
    comparison.throughput_delta_pixels_per_second = Some(delta(
        current.summary.throughput_pixels_per_second,
        baseline.summary.throughput_pixels_per_second,
    ));
    comparison.allocation_delta_bytes = Some(delta(
        current.summary.allocated_bytes,
        baseline.summary.allocated_bytes,
    ));
    comparison.allocation_count_delta = Some(delta(
        current.summary.allocation_count,
        baseline.summary.allocation_count,
    ));
    comparison.gpu_upload_delta_bytes = metric_value_delta(
        &current.summary.gpu_upload_bytes,
        &baseline.summary.gpu_upload_bytes,
    );
    comparison.gpu_download_delta_bytes = metric_value_delta(
        &current.summary.gpu_download_bytes,
        &baseline.summary.gpu_download_bytes,
    );
    comparison.gpu_dispatch_delta_ns = metric_value_delta(
        &current.summary.gpu_dispatch_ns,
        &baseline.summary.gpu_dispatch_ns,
    );
    comparison.compared = true;
    Ok(comparison)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BenchmarkError {
    InvalidScenario(String, String),
    InvalidReceipt(String),
    Incomplete,
    ScenarioNotActive(String),
    QualificationFailed(String),
    OutputMismatch { expected: String, actual: String },
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
            Self::ScenarioNotActive(id) => write!(formatter, "scenario is not active: {id}"),
            Self::QualificationFailed(id) => write!(formatter, "scenario is not qualified: {id}"),
            Self::OutputMismatch { expected, actual } => write!(
                formatter,
                "output hash mismatch: expected {expected}, got {actual}"
            ),
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
