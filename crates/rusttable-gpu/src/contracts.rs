use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::fmt;
use std::time::{Duration, Instant};

const SNAPSHOT_SCHEMA: u16 = 1;
const MAX_REJECTION_REASON: usize = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Platform {
    MacOs,
    Linux,
    Windows,
    Other,
}

impl Platform {
    #[must_use]
    pub const fn current() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::MacOs
        }
        #[cfg(all(not(target_os = "macos"), target_os = "linux"))]
        {
            Self::Linux
        }
        #[cfg(all(
            not(target_os = "macos"),
            not(target_os = "linux"),
            target_os = "windows"
        ))]
        {
            Self::Windows
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            Self::Other
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Backend {
    Metal,
    Vulkan,
    Dx12,
    Gl,
    Cpu,
}

impl Backend {
    #[must_use]
    pub const fn platform_rank(self, platform: Platform) -> u8 {
        match (platform, self) {
            (Platform::MacOs, Self::Metal)
            | (Platform::Linux, Self::Vulkan)
            | (Platform::Windows, Self::Dx12) => 0,
            (Platform::Linux, Self::Gl) | (Platform::Windows, Self::Vulkan) => 1,
            (_, Self::Metal | Self::Dx12 | Self::Vulkan) => 2,
            (_, Self::Gl) => 3,
            (_, Self::Cpu) => u8::MAX,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DeviceClass {
    Discrete,
    Integrated,
    Virtual,
    Cpu,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PowerPreference {
    HighPerformance,
    LowPower,
    Unspecified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ExecutionTier {
    CoreCompute,
    EnhancedCompute,
    SubgroupCompute,
    CpuOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum FaultState {
    Healthy,
    Degraded,
    Lost,
    DisabledForSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum FaultKind {
    DeviceLost,
    OutOfMemory,
    Validation,
    Submission,
    Timeout,
    Cancelled,
    Uncaptured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LimitEnvelope {
    pub max_storage_buffer_bytes: u64,
    pub max_buffer_bytes: u64,
    pub max_bindings: u32,
    pub max_workgroups_per_dimension: u32,
    pub supports_rgba16f_storage: bool,
    pub supports_r32f_storage: bool,
}

impl LimitEnvelope {
    #[must_use]
    pub const fn minimum() -> Self {
        Self {
            max_storage_buffer_bytes: 16 * 1024,
            max_buffer_bytes: 64 * 1024,
            max_bindings: 4,
            max_workgroups_per_dimension: 1,
            supports_rgba16f_storage: false,
            supports_r32f_storage: false,
        }
    }

    #[must_use]
    pub const fn supports_core(self) -> bool {
        self.max_storage_buffer_bytes >= Self::minimum().max_storage_buffer_bytes
            && self.max_buffer_bytes >= Self::minimum().max_buffer_bytes
            && self.max_bindings >= Self::minimum().max_bindings
            && self.max_workgroups_per_dimension > 0
    }
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AdvertisedFeatures {
    pub f16: bool,
    pub sixteen_bit_storage: bool,
    pub subgroups: bool,
    pub float_atomics: bool,
    pub immediate_data: bool,
    pub timestamps: bool,
    pub pipeline_cache: bool,
}

impl AdvertisedFeatures {
    #[must_use]
    pub const fn none() -> Self {
        Self {
            f16: false,
            sixteen_bit_storage: false,
            subgroups: false,
            float_atomics: false,
            immediate_data: false,
            timestamps: false,
            pipeline_cache: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AdapterIdentity {
    backend: Backend,
    vendor_id: u32,
    device_id: u32,
    device_key: [u8; 32],
}

impl AdapterIdentity {
    #[must_use]
    pub fn new(backend: Backend, vendor_id: u32, device_id: u32, stable_key: &str) -> Self {
        let mut digest = Sha256::new();
        digest.update(stable_key.as_bytes());
        Self {
            backend,
            vendor_id,
            device_id,
            device_key: digest.finalize().into(),
        }
    }

    #[must_use]
    pub const fn backend(&self) -> Backend {
        self.backend
    }

    #[must_use]
    pub const fn vendor_id(&self) -> u32 {
        self.vendor_id
    }

    #[must_use]
    pub const fn device_id(&self) -> u32 {
        self.device_id
    }

    #[must_use]
    pub const fn device_key(&self) -> [u8; 32] {
        self.device_key
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterCandidate {
    pub identity: AdapterIdentity,
    pub class: DeviceClass,
    pub power: PowerPreference,
    pub memory_bytes: u64,
    pub limits: LimitEnvelope,
    pub advertised: AdvertisedFeatures,
    pub core_probe_passed: bool,
    pub rejected_reason: Option<String>,
}

impl AdapterCandidate {
    #[must_use]
    pub fn usable(&self) -> bool {
        self.identity.backend() != Backend::Cpu
            && self.rejected_reason.is_none()
            && self.limits.supports_core()
            && self.core_probe_passed
    }

    #[must_use]
    pub const fn with_core_defaults(identity: AdapterIdentity, class: DeviceClass) -> Self {
        Self {
            identity,
            class,
            power: PowerPreference::Unspecified,
            memory_bytes: 0,
            limits: LimitEnvelope::minimum(),
            advertised: AdvertisedFeatures::none(),
            core_probe_passed: false,
            rejected_reason: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendPolicy {
    pub platform: Platform,
    pub backends: Vec<Backend>,
    pub power: PowerPreference,
}

impl BackendPolicy {
    #[must_use]
    pub fn current() -> Self {
        let platform = Platform::current();
        let backends = match platform {
            Platform::MacOs => vec![Backend::Metal],
            Platform::Linux | Platform::Other => vec![Backend::Vulkan, Backend::Gl],
            Platform::Windows => vec![Backend::Dx12, Backend::Vulkan],
        };
        Self {
            platform,
            backends,
            power: PowerPreference::HighPerformance,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedAdapter {
    pub identity: AdapterIdentity,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterSelection {
    pub selected: AdapterCandidate,
    pub rejected: Vec<RejectedAdapter>,
    pub rank: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionError {
    NoAdapter,
    NoCoreCapableAdapter,
}

impl fmt::Display for SelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoAdapter => "no GPU adapter was enumerated",
            Self::NoCoreCapableAdapter => "no adapter passed the core compute contract",
        })
    }
}

impl std::error::Error for SelectionError {}

pub fn select_adapter(
    candidates: &[AdapterCandidate],
    policy: &BackendPolicy,
) -> Result<AdapterSelection, SelectionError> {
    if candidates.is_empty() {
        return Err(SelectionError::NoAdapter);
    }
    let mut usable: Vec<_> = candidates
        .iter()
        .filter(|candidate| candidate.usable())
        .collect();
    if usable.is_empty() {
        return Err(SelectionError::NoCoreCapableAdapter);
    }
    usable.sort_by(|left, right| compare_candidates(left, right, policy));
    let selected = (*usable[0]).clone();
    let selected_key = selected.identity.device_key();
    let rejected = candidates
        .iter()
        .filter(|candidate| candidate.identity.device_key() != selected_key)
        .map(|candidate| RejectedAdapter {
            identity: candidate.identity.clone(),
            reason: bounded_reason(&candidate_rejection(candidate)),
        })
        .collect();
    Ok(AdapterSelection {
        selected,
        rejected,
        rank: 0,
    })
}

fn compare_candidates(
    left: &AdapterCandidate,
    right: &AdapterCandidate,
    policy: &BackendPolicy,
) -> Ordering {
    let left_backend = policy
        .backends
        .iter()
        .position(|backend| *backend == left.identity.backend())
        .unwrap_or(u8::MAX as usize);
    let right_backend = policy
        .backends
        .iter()
        .position(|backend| *backend == right.identity.backend())
        .unwrap_or(u8::MAX as usize);
    left_backend
        .cmp(&right_backend)
        .then_with(|| device_class_rank(left.class).cmp(&device_class_rank(right.class)))
        .then_with(|| {
            power_rank(left.power, policy.power).cmp(&power_rank(right.power, policy.power))
        })
        .then_with(|| right.memory_bytes.cmp(&left.memory_bytes))
        .then_with(|| left.identity.device_key().cmp(&right.identity.device_key()))
}

const fn device_class_rank(class: DeviceClass) -> u8 {
    match class {
        DeviceClass::Discrete => 0,
        DeviceClass::Integrated => 1,
        DeviceClass::Virtual => 2,
        DeviceClass::Other => 3,
        DeviceClass::Cpu => 4,
    }
}

fn power_rank(actual: PowerPreference, requested: PowerPreference) -> u8 {
    match (actual, requested) {
        (a, r) if a == r => 0,
        (PowerPreference::Unspecified, _) => 2,
        _ => 1,
    }
}

fn candidate_rejection(candidate: &AdapterCandidate) -> String {
    if let Some(reason) = &candidate.rejected_reason {
        return reason.clone();
    }
    if !candidate.limits.supports_core() {
        return "core limits are not supported".to_owned();
    }
    if !candidate.core_probe_passed {
        return "core behavioral probe failed".to_owned();
    }
    "lower deterministic rank".to_owned()
}

fn bounded_reason(reason: &str) -> String {
    reason.chars().take(MAX_REJECTION_REASON).collect()
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeLedger {
    pub compute_storage_buffers: bool,
    pub rgba16f_sampled_attachment: bool,
    pub r32f_storage_or_buffer_path: bool,
    pub binding_alignment: bool,
    pub copy_map_readback: bool,
    pub optional: AdvertisedFeatures,
}

impl ProbeLedger {
    #[must_use]
    pub const fn core_passed(&self) -> bool {
        self.compute_storage_buffers
            && self.rgba16f_sampled_attachment
            && self.r32f_storage_or_buffer_path
            && self.binding_alignment
            && self.copy_map_readback
    }

    #[must_use]
    pub const fn tier(&self) -> ExecutionTier {
        if !self.core_passed() {
            return ExecutionTier::CpuOnly;
        }
        if self.optional.subgroups {
            return ExecutionTier::SubgroupCompute;
        }
        if self.optional.f16 || self.optional.sixteen_bit_storage {
            return ExecutionTier::EnhancedCompute;
        }
        ExecutionTier::CoreCompute
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuCapabilitySnapshot {
    pub schema_version: u16,
    pub generation: u64,
    pub backend: Backend,
    pub adapter: Option<AdapterIdentity>,
    pub limits: LimitEnvelope,
    pub advertised: AdvertisedFeatures,
    pub probes: ProbeLedger,
    pub tier: ExecutionTier,
    pub state: FaultState,
}

impl GpuCapabilitySnapshot {
    #[must_use]
    pub const fn cpu_only() -> Self {
        Self {
            schema_version: SNAPSHOT_SCHEMA,
            generation: 0,
            backend: Backend::Cpu,
            adapter: None,
            limits: LimitEnvelope::minimum(),
            advertised: AdvertisedFeatures::none(),
            probes: ProbeLedger {
                compute_storage_buffers: false,
                rgba16f_sampled_attachment: false,
                r32f_storage_or_buffer_path: false,
                binding_alignment: false,
                copy_map_readback: false,
                optional: AdvertisedFeatures::none(),
            },
            tier: ExecutionTier::CpuOnly,
            state: FaultState::Healthy,
        }
    }

    #[must_use]
    pub const fn is_cpu_only(&self) -> bool {
        matches!(self.tier, ExecutionTier::CpuOnly)
    }

    pub fn canonical_hash(&self) -> Result<[u8; 32], postcard::Error> {
        let bytes = postcard::to_allocvec(self)?;
        Ok(Sha256::digest(bytes).into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuFeaturePlan {
    pub tier: ExecutionTier,
    pub use_f16: bool,
    pub use_subgroups: bool,
    pub use_float_atomics: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelPlanError {
    CpuRequired,
    SnapshotNotHealthy,
}

impl GpuFeaturePlan {
    pub fn for_kernel(
        snapshot: &GpuCapabilitySnapshot,
        prefer_optional: bool,
    ) -> Result<Self, KernelPlanError> {
        if snapshot.is_cpu_only() {
            return Err(KernelPlanError::CpuRequired);
        }
        if !matches!(snapshot.state, FaultState::Healthy | FaultState::Degraded) {
            return Err(KernelPlanError::SnapshotNotHealthy);
        }
        Ok(Self {
            tier: snapshot.tier,
            use_f16: prefer_optional && snapshot.probes.optional.f16,
            use_subgroups: prefer_optional && snapshot.probes.optional.subgroups,
            use_float_atomics: prefer_optional && snapshot.probes.optional.float_atomics,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuFaultSnapshot {
    pub generation: u64,
    pub state: FaultState,
    pub last_fault: Option<FaultKind>,
    pub resource_epoch: u64,
}

#[derive(Debug)]
pub struct FaultTracker {
    generation: u64,
    state: FaultState,
    last_fault: Option<FaultKind>,
    resource_epoch: u64,
    last_recreation: Option<Instant>,
}

impl Default for FaultTracker {
    fn default() -> Self {
        Self {
            generation: 0,
            state: FaultState::Healthy,
            last_fault: None,
            resource_epoch: 0,
            last_recreation: None,
        }
    }
}

impl FaultTracker {
    pub fn record(&mut self, fault: FaultKind, _now: Instant) -> GpuFaultSnapshot {
        self.generation = self.generation.saturating_add(1);
        self.resource_epoch = self.resource_epoch.saturating_add(1);
        self.last_fault = Some(fault);
        self.state = match fault {
            FaultKind::DeviceLost | FaultKind::OutOfMemory => FaultState::Lost,
            FaultKind::Timeout | FaultKind::Cancelled => FaultState::Degraded,
            FaultKind::Validation | FaultKind::Submission | FaultKind::Uncaptured => {
                FaultState::Degraded
            }
        };
        GpuFaultSnapshot {
            generation: self.generation,
            state: self.state,
            last_fault: self.last_fault,
            resource_epoch: self.resource_epoch,
        }
    }

    pub fn recreate_once(&mut self, now: Instant) -> bool {
        if self.state != FaultState::Lost
            || self
                .last_recreation
                .is_some_and(|last| now.duration_since(last) < Duration::from_secs(5))
        {
            return false;
        }
        self.last_recreation = Some(now);
        self.state = FaultState::Healthy;
        true
    }

    pub fn disable_for_session(&mut self) {
        self.state = FaultState::DisabledForSession;
        self.generation = self.generation.saturating_add(1);
        self.resource_epoch = self.resource_epoch.saturating_add(1);
    }

    #[must_use]
    pub const fn snapshot(&self) -> GpuFaultSnapshot {
        GpuFaultSnapshot {
            generation: self.generation,
            state: self.state,
            last_fault: self.last_fault,
            resource_epoch: self.resource_epoch,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(key: &str, backend: Backend, class: DeviceClass, memory: u64) -> AdapterCandidate {
        AdapterCandidate {
            identity: AdapterIdentity::new(backend, 1, 2, key),
            class,
            power: PowerPreference::HighPerformance,
            memory_bytes: memory,
            limits: LimitEnvelope {
                max_storage_buffer_bytes: 1 << 30,
                max_buffer_bytes: 1 << 30,
                max_bindings: 8,
                max_workgroups_per_dimension: 65_535,
                supports_rgba16f_storage: true,
                supports_r32f_storage: true,
            },
            advertised: AdvertisedFeatures::none(),
            core_probe_passed: true,
            rejected_reason: None,
        }
    }

    #[test]
    fn selection_is_backend_first_then_hardware_then_memory() {
        let policy = BackendPolicy {
            platform: Platform::Linux,
            backends: vec![Backend::Vulkan, Backend::Gl],
            power: PowerPreference::HighPerformance,
        };
        let selected = select_adapter(
            &[
                candidate("gl", Backend::Gl, DeviceClass::Discrete, 4),
                candidate("vk-integrated", Backend::Vulkan, DeviceClass::Integrated, 8),
                candidate("vk-discrete", Backend::Vulkan, DeviceClass::Discrete, 2),
            ],
            &policy,
        )
        .expect("one candidate is valid");
        assert_eq!(selected.selected.identity.backend(), Backend::Vulkan);
        assert_eq!(selected.selected.class, DeviceClass::Discrete);
    }

    #[test]
    fn cpu_only_plan_is_explicit_and_hash_is_stable() {
        let snapshot = GpuCapabilitySnapshot::cpu_only();
        assert!(snapshot.is_cpu_only());
        assert_eq!(snapshot.canonical_hash(), snapshot.canonical_hash());
        assert_eq!(
            GpuFeaturePlan::for_kernel(&snapshot, true),
            Err(KernelPlanError::CpuRequired)
        );
    }

    #[test]
    fn faults_invalidate_resources_and_recreation_is_cooldown_bound() {
        let mut tracker = FaultTracker::default();
        let now = Instant::now();
        let first = tracker.record(FaultKind::DeviceLost, now);
        assert_eq!(first.state, FaultState::Lost);
        assert!(tracker.recreate_once(now + Duration::from_secs(5)));
        assert!(!tracker.recreate_once(now + Duration::from_secs(6)));
        tracker.disable_for_session();
        assert_eq!(tracker.snapshot().state, FaultState::DisabledForSession);
    }
}
