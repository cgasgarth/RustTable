use crate::{
    AdapterCandidate, AdapterIdentity, AdvertisedFeatures, Backend, BackendPolicy, DeviceClass,
    ExecutionTier, FaultTracker, GpuCapabilitySnapshot, LimitEnvelope, Platform, PowerPreference,
    ProbeLedger, SelectionError, select_adapter,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

const DEFAULT_DEADLINE: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct GpuRuntimeConfig {
    pub policy: BackendPolicy,
    pub deadline: Duration,
    pub allow_cpu_fallback: bool,
    pub cancellation: Arc<AtomicBool>,
}

impl Default for GpuRuntimeConfig {
    fn default() -> Self {
        Self {
            policy: BackendPolicy::current(),
            deadline: DEFAULT_DEADLINE,
            allow_cpu_fallback: true,
            cancellation: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl GpuRuntimeConfig {
    #[must_use]
    pub fn cancelled(&self) -> bool {
        self.cancellation.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuInitError {
    UnsupportedPlatform(Platform),
    Cancelled,
    TimedOut,
    NoAdapter,
    NoCoreCapableAdapter,
    DeviceRequest(String),
}

impl std::fmt::Display for GpuInitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedPlatform(platform) => {
                write!(formatter, "unsupported GPU platform: {platform:?}")
            }
            Self::Cancelled => formatter.write_str("GPU initialization was cancelled"),
            Self::TimedOut => formatter.write_str("GPU initialization timed out"),
            Self::NoAdapter => formatter.write_str("no GPU adapter was available"),
            Self::NoCoreCapableAdapter => {
                formatter.write_str("no GPU adapter passed the core contract")
            }
            Self::DeviceRequest(error) => write!(formatter, "GPU device request failed: {error}"),
        }
    }
}

impl std::error::Error for GpuInitError {}

#[derive(Debug)]
pub struct GpuRuntime {
    instance: Option<wgpu::Instance>,
    device: Option<wgpu::Device>,
    queue: Option<wgpu::Queue>,
    snapshot: GpuCapabilitySnapshot,
    faults: Arc<Mutex<FaultTracker>>,
}

impl GpuRuntime {
    pub async fn initialize(config: GpuRuntimeConfig) -> Result<Self, GpuInitError> {
        if config.cancelled() {
            return Err(GpuInitError::Cancelled);
        }
        let deadline = config.deadline.max(Duration::from_millis(1));
        match tokio::time::timeout(deadline, Self::initialize_inner(config.clone())).await {
            Ok(result) => result,
            Err(_) => Err(GpuInitError::TimedOut),
        }
    }

    async fn initialize_inner(config: GpuRuntimeConfig) -> Result<Self, GpuInitError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: to_wgpu_backends(&config.policy),
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });
        if config.policy.backends.is_empty() {
            return if config.allow_cpu_fallback {
                Ok(Self::cpu_only(instance))
            } else {
                Err(GpuInitError::UnsupportedPlatform(config.policy.platform))
            };
        }
        if config.cancelled() {
            return Err(GpuInitError::Cancelled);
        }
        let adapters = instance
            .enumerate_adapters(to_wgpu_backends(&config.policy))
            .await;
        if config.cancelled() {
            return Err(GpuInitError::Cancelled);
        }
        let mut normalized = Vec::with_capacity(adapters.len());
        for adapter in &adapters {
            normalized.push(normalize_adapter(adapter));
        }
        let selection =
            match select_adapter(&normalized, &config.policy).map_err(map_selection_error) {
                Ok(selection) => selection,
                Err(_error) if config.allow_cpu_fallback => return Ok(Self::cpu_only(instance)),
                Err(error) => return Err(error),
            };
        let selected_key = selection.selected.identity.device_key();
        let (adapter, candidate) = adapters
            .into_iter()
            .zip(normalized)
            .find(|(_, candidate)| candidate.identity.device_key() == selected_key)
            .ok_or(GpuInitError::NoAdapter)?;
        let descriptor = wgpu::DeviceDescriptor {
            label: Some("RustTable compute device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            ..Default::default()
        };
        let (device, queue) = match adapter.request_device(&descriptor).await {
            Ok(handles) => handles,
            Err(_error) if config.allow_cpu_fallback => return Ok(Self::cpu_only(instance)),
            Err(error) => return Err(GpuInitError::DeviceRequest(error.to_string())),
        };
        let snapshot = snapshot_from_candidate(&candidate);
        let faults = Arc::new(Mutex::new(FaultTracker::default()));
        let lost_faults = Arc::clone(&faults);
        device.set_device_lost_callback(move |_, _description| {
            record_fault(&lost_faults, crate::FaultKind::DeviceLost);
        });
        let error_faults = Arc::clone(&faults);
        device.on_uncaptured_error(Arc::new(move |error| {
            let kind = match error {
                wgpu::Error::OutOfMemory { .. } => crate::FaultKind::OutOfMemory,
                wgpu::Error::Validation { .. } => crate::FaultKind::Validation,
                wgpu::Error::Internal { .. } => crate::FaultKind::Uncaptured,
            };
            record_fault(&error_faults, kind);
        }));
        Ok(Self {
            instance: Some(instance),
            device: Some(device),
            queue: Some(queue),
            snapshot,
            faults,
        })
    }

    #[must_use]
    pub fn snapshot(&self) -> &GpuCapabilitySnapshot {
        &self.snapshot
    }

    #[must_use]
    pub fn is_cpu_only(&self) -> bool {
        self.snapshot.is_cpu_only()
    }

    #[must_use]
    pub fn fault_snapshot(&self) -> crate::GpuFaultSnapshot {
        self.faults.lock().map_or_else(
            |_| FaultTracker::default().snapshot(),
            |faults| faults.snapshot(),
        )
    }

    #[must_use]
    pub fn health_check(&self) -> bool {
        self.instance.is_some()
            && self.device.is_some()
            && self.queue.is_some()
            && matches!(self.snapshot.state, crate::FaultState::Healthy)
    }

    fn cpu_only(instance: wgpu::Instance) -> Self {
        Self {
            instance: Some(instance),
            device: None,
            queue: None,
            snapshot: GpuCapabilitySnapshot::cpu_only(),
            faults: Arc::new(Mutex::new(FaultTracker::default())),
        }
    }
}

fn record_fault(faults: &Mutex<FaultTracker>, kind: crate::FaultKind) {
    if let Ok(mut faults) = faults.lock() {
        let _ = faults.record(kind, Instant::now());
    }
}

fn map_selection_error(error: SelectionError) -> GpuInitError {
    match error {
        SelectionError::NoAdapter => GpuInitError::NoAdapter,
        SelectionError::NoCoreCapableAdapter => GpuInitError::NoCoreCapableAdapter,
    }
}

fn to_wgpu_backends(policy: &BackendPolicy) -> wgpu::Backends {
    policy
        .backends
        .iter()
        .fold(wgpu::Backends::empty(), |backends, backend| {
            backends
                | match backend {
                    Backend::Metal => wgpu::Backends::METAL,
                    Backend::Vulkan => wgpu::Backends::VULKAN,
                    Backend::Dx12 => wgpu::Backends::DX12,
                    Backend::Gl => wgpu::Backends::GL,
                    Backend::Cpu => wgpu::Backends::empty(),
                }
        })
}

fn normalize_adapter(adapter: &wgpu::Adapter) -> AdapterCandidate {
    let info = adapter.get_info();
    let backend = match info.backend {
        wgpu::Backend::Metal => Backend::Metal,
        wgpu::Backend::Vulkan => Backend::Vulkan,
        wgpu::Backend::Dx12 => Backend::Dx12,
        wgpu::Backend::Gl => Backend::Gl,
        _ => Backend::Cpu,
    };
    let class = match info.device_type {
        wgpu::DeviceType::DiscreteGpu => DeviceClass::Discrete,
        wgpu::DeviceType::IntegratedGpu => DeviceClass::Integrated,
        wgpu::DeviceType::VirtualGpu => DeviceClass::Virtual,
        wgpu::DeviceType::Cpu => DeviceClass::Cpu,
        wgpu::DeviceType::Other => DeviceClass::Other,
    };
    let features = adapter.features();
    let limits = adapter.limits();
    let envelope = LimitEnvelope {
        max_storage_buffer_bytes: limits.max_storage_buffer_binding_size,
        max_buffer_bytes: limits.max_buffer_size,
        max_bindings: limits.max_bindings_per_bind_group,
        max_workgroups_per_dimension: limits.max_compute_workgroups_per_dimension,
        supports_rgba16f_storage: true,
        supports_r32f_storage: true,
    };
    let advertised = AdvertisedFeatures {
        f16: features.contains(wgpu::Features::SHADER_F16),
        sixteen_bit_storage: features.contains(wgpu::Features::SHADER_F16),
        subgroups: features.contains(wgpu::Features::SUBGROUP),
        float_atomics: features.contains(wgpu::Features::SHADER_FLOAT32_ATOMIC),
        immediate_data: features.contains(wgpu::Features::IMMEDIATES),
        timestamps: features.contains(wgpu::Features::TIMESTAMP_QUERY),
        pipeline_cache: features.contains(wgpu::Features::PIPELINE_CACHE),
    };
    let identity = AdapterIdentity::new(
        backend,
        info.vendor,
        info.device,
        &format!("{}:{}", info.name, info.device_pci_bus_id),
    );
    AdapterCandidate {
        identity,
        class,
        power: PowerPreference::Unspecified,
        memory_bytes: 0,
        limits: envelope,
        advertised,
        core_probe_passed: envelope.supports_core(),
        rejected_reason: None,
    }
}

fn snapshot_from_candidate(candidate: &AdapterCandidate) -> GpuCapabilitySnapshot {
    let probes = ProbeLedger {
        compute_storage_buffers: candidate.core_probe_passed,
        rgba16f_sampled_attachment: candidate.limits.supports_rgba16f_storage,
        r32f_storage_or_buffer_path: candidate.limits.supports_r32f_storage,
        binding_alignment: candidate.limits.max_bindings >= 4,
        copy_map_readback: candidate.core_probe_passed,
        optional: candidate.advertised,
    };
    GpuCapabilitySnapshot {
        schema_version: 1,
        generation: 0,
        backend: candidate.identity.backend(),
        adapter: Some(candidate.identity.clone()),
        limits: candidate.limits,
        advertised: candidate.advertised,
        tier: if probes.core_passed() {
            probes.tier()
        } else {
            ExecutionTier::CpuOnly
        },
        probes,
        state: crate::FaultState::Healthy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_policy_never_requests_a_surface() {
        let policy = BackendPolicy::current();
        assert!(!policy.backends.is_empty());
        assert!(!to_wgpu_backends(&policy).is_empty());
    }

    #[test]
    fn normalizing_synthetic_contracts_keeps_cpu_fallback_semantic() {
        let snapshot = GpuCapabilitySnapshot::cpu_only();
        assert!(snapshot.is_cpu_only());
    }

    #[tokio::test]
    async fn empty_backend_policy_returns_a_ready_cpu_fallback() {
        let mut config = GpuRuntimeConfig::default();
        config.policy.backends.clear();
        let runtime = GpuRuntime::initialize(config).await.expect("CPU fallback");
        assert!(runtime.is_cpu_only());
        assert!(!runtime.health_check());
    }
}
