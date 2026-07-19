use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use tokio::time;
use wasmtime::{
    Config, Engine, Store, StoreLimits, StoreLimitsBuilder, WasmBacktraceDetails,
    component::{Component, Linker, ResourceTable},
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use super::{
    api::{CapabilitySet, ExtensionId, Permission, WorldVersion},
    cache::{CacheKey, CacheReceipt, CompiledArtifactCache},
    errors::{ErrorCode, ScriptError},
    lifecycle::{ExtensionState, Lifecycle, LifecycleReceipt},
    limits::ScriptLimits,
    receipt::{ReceiptStatus, digest},
    registry::{ExtensionPackage, ExtensionRegistry},
    resources::ResourceRegistry,
};

#[derive(Debug, Clone)]
pub struct HostConfig {
    pub limits: ScriptLimits,
    pub cache_root: PathBuf,
    pub target: String,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            limits: ScriptLimits::default(),
            cache_root: std::env::temp_dir().join("rusttable-extension-cache"),
            target: std::env::consts::ARCH.to_owned(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct InvocationCancellation(Arc<AtomicBool>);

impl InvocationCancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone)]
pub struct InvocationRequest {
    pub extension: ExtensionId,
    pub payload: Vec<u8>,
    pub grants: CapabilitySet,
    pub cancellation: InvocationCancellation,
}

impl InvocationRequest {
    #[must_use]
    pub fn new(extension: ExtensionId) -> Self {
        Self {
            extension,
            payload: Vec::new(),
            grants: CapabilitySet::new(),
            cancellation: InvocationCancellation::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InvocationReceipt {
    pub receipt_version: u32,
    pub receipt_id: String,
    pub extension: ExtensionId,
    pub generation: u64,
    pub component_hash: String,
    pub cache_key: String,
    pub status: ReceiptStatus,
    pub error: Option<ErrorCode>,
    pub payload_hash: String,
    pub fuel_remaining: Option<u64>,
    pub host_calls: usize,
    pub event_count: usize,
}

struct ReceiptMetrics {
    status: ReceiptStatus,
    error: Option<ErrorCode>,
    payload_hash: String,
    fuel_remaining: Option<u64>,
    host_calls: usize,
    event_count: usize,
}

struct StoreState {
    wasi: WasiCtx,
    table: ResourceTable,
    limits: StoreLimits,
    resources: ResourceRegistry,
    grants: CapabilitySet,
    host_calls: usize,
    event_count: usize,
    cancelled: InvocationCancellation,
    host_call_limit: usize,
    output_limit: usize,
    storage: BTreeMap<String, String>,
    storage_bytes: usize,
    storage_limit: usize,
}

impl WasiView for StoreState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl super::wit::rusttable::extension::host::Host for StoreState {
    async fn get_catalog_snapshot(
        &mut self,
    ) -> Result<
        super::wit::rusttable::extension::host::CatalogSnapshot,
        super::wit::rusttable::extension::host::CapabilityError,
    > {
        tokio::task::yield_now().await;
        self.require_call(Permission::CatalogRead)?;
        Ok(super::wit::rusttable::extension::host::CatalogSnapshot {
            revision: 0,
            json: "{}".to_owned(),
        })
    }

    async fn notify(
        &mut self,
        kind: String,
        payload: String,
    ) -> Result<(), super::wit::rusttable::extension::host::CapabilityError> {
        tokio::task::yield_now().await;
        self.require_call(Permission::Notifications)?;
        if kind.len().saturating_add(payload.len()) > self.output_limit {
            return Err(super::wit::rusttable::extension::host::CapabilityError::Limit);
        }
        self.event_count = self.event_count.saturating_add(1);
        Ok(())
    }

    async fn storage_get(
        &mut self,
        key: String,
    ) -> Result<Option<String>, super::wit::rusttable::extension::host::CapabilityError> {
        tokio::task::yield_now().await;
        self.require_call(Permission::Storage)?;
        Ok(self.storage.get(&key).cloned())
    }

    async fn storage_put(
        &mut self,
        key: String,
        value: String,
    ) -> Result<(), super::wit::rusttable::extension::host::CapabilityError> {
        tokio::task::yield_now().await;
        self.require_call(Permission::Storage)?;
        let previous = self.storage.get(&key).map_or(0, String::len);
        let next = self
            .storage_bytes
            .saturating_sub(previous)
            .saturating_add(key.len())
            .saturating_add(value.len());
        if next > self.storage_limit {
            return Err(super::wit::rusttable::extension::host::CapabilityError::Limit);
        }
        self.storage_bytes = next;
        self.storage.insert(key, value);
        Ok(())
    }

    async fn submit_export(
        &mut self,
        proposal: String,
    ) -> Result<(), super::wit::rusttable::extension::host::CapabilityError> {
        tokio::task::yield_now().await;
        self.require_call(Permission::ExportProposal)?;
        if proposal.len() > self.output_limit {
            return Err(super::wit::rusttable::extension::host::CapabilityError::Limit);
        }
        Ok(())
    }
}

impl StoreState {
    fn require_call(
        &mut self,
        permission: Permission,
    ) -> Result<(), super::wit::rusttable::extension::host::CapabilityError> {
        if self.host_calls >= self.host_call_limit {
            return Err(super::wit::rusttable::extension::host::CapabilityError::Limit);
        }
        self.host_calls = self.host_calls.saturating_add(1);
        self.grants
            .require(permission)
            .map_err(|_| super::wit::rusttable::extension::host::CapabilityError::Denied)
    }
}

#[derive(Clone)]
pub struct WasmtimeHost {
    engine: Engine,
    config: HostConfig,
    cache: CompiledArtifactCache,
    registry: Arc<Mutex<ExtensionRegistry>>,
    lifecycle: Arc<Mutex<Lifecycle>>,
}

impl WasmtimeHost {
    /// Builds a Wasmtime engine with the product’s minimal component feature policy.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when limits, engine configuration, or the cache directory is invalid.
    pub fn new(config: HostConfig) -> Result<Self, ScriptError> {
        config.limits.validate()?;
        let mut engine_config = Config::new();
        engine_config
            .wasm_component_model(true)
            .wasm_component_model_async(true)
            .consume_fuel(true)
            .epoch_interruption(true)
            .wasm_backtrace_details(WasmBacktraceDetails::Disable);
        let engine = Engine::new(&engine_config).map_err(|error| {
            ScriptError::new(
                ErrorCode::MalformedComponent,
                format!("engine configuration failed: {error}"),
            )
        })?;
        let cache = CompiledArtifactCache::new(config.cache_root.clone())?;
        Ok(Self {
            engine,
            config,
            cache,
            registry: Arc::new(Mutex::new(ExtensionRegistry::default())),
            lifecycle: Arc::new(Mutex::new(Lifecycle::default())),
        })
    }

    /// Validates, compiles, caches, and registers an extension package.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when validation, compilation, cache persistence, or registry activation fails.
    pub fn install(&self, package: ExtensionPackage) -> Result<CacheReceipt, ScriptError> {
        let extension = package.manifest.id.clone();
        self.validate_component(&package)?;
        let cache_key = self.cache_key(&package);
        let component = Component::new(&self.engine, &package.component).map_err(|error| {
            ScriptError::new(
                ErrorCode::MalformedComponent,
                format!("component validation failed: {error}"),
            )
        })?;
        let artifact = component.serialize().map_err(|error| {
            ScriptError::new(
                ErrorCode::CacheRejected,
                format!("component serialization failed: {error}"),
            )
        })?;
        let receipt = self.cache.write(cache_key, &artifact)?;
        self.registry
            .lock()
            .map_err(|_| {
                ScriptError::new(
                    ErrorCode::HostCallFailed,
                    "extension registry lock poisoned",
                )
            })?
            .install(package)?;
        self.lifecycle
            .lock()
            .map_err(|_| {
                ScriptError::new(
                    ErrorCode::HostCallFailed,
                    "extension lifecycle lock poisoned",
                )
            })?
            .install(&extension);
        Ok(receipt)
    }

    /// Revalidates and transactionally replaces an extension package.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when validation, registry, or lifecycle transition fails.
    pub fn replace(&self, package: ExtensionPackage) -> Result<LifecycleReceipt, ScriptError> {
        self.validate_component(&package)?;
        let extension = package.manifest.id.clone();
        self.registry
            .lock()
            .map_err(|_| {
                ScriptError::new(
                    ErrorCode::HostCallFailed,
                    "extension registry lock poisoned",
                )
            })?
            .replace(package)?;
        let mut lifecycle = self.lifecycle.lock().map_err(|_| {
            ScriptError::new(
                ErrorCode::HostCallFailed,
                "extension lifecycle lock poisoned",
            )
        })?;
        if lifecycle.generation(&extension).is_none() {
            lifecycle.install(&extension);
        }
        lifecycle.reload(&extension)
    }

    /// Enables a validated extension generation.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when the extension is unknown, disabled, or quarantined.
    pub fn enable(&self, extension: &ExtensionId) -> Result<LifecycleReceipt, ScriptError> {
        self.lifecycle
            .lock()
            .map_err(|_| {
                ScriptError::new(
                    ErrorCode::HostCallFailed,
                    "extension lifecycle lock poisoned",
                )
            })?
            .enable(extension)
    }
    /// Disables an extension and invalidates pending generations.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when the extension is unknown.
    pub fn disable(&self, extension: &ExtensionId) -> Result<LifecycleReceipt, ScriptError> {
        self.lifecycle
            .lock()
            .map_err(|_| {
                ScriptError::new(
                    ErrorCode::HostCallFailed,
                    "extension lifecycle lock poisoned",
                )
            })?
            .disable(extension)
    }

    /// Runs one extension in a short-lived, independently limited store.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when permissions, cancellation, deadlines, traps, or lifecycle checks fail.
    pub async fn invoke(
        &self,
        request: InvocationRequest,
    ) -> Result<InvocationReceipt, ScriptError> {
        let package = self
            .registry
            .lock()
            .map_err(|_| {
                ScriptError::new(
                    ErrorCode::HostCallFailed,
                    "extension registry lock poisoned",
                )
            })?
            .get(&request.extension)?
            .clone();
        for permission in package.manifest.requested_permissions.iter() {
            request.grants.require(*permission)?;
        }
        let generation = self
            .lifecycle
            .lock()
            .map_err(|_| {
                ScriptError::new(
                    ErrorCode::HostCallFailed,
                    "extension lifecycle lock poisoned",
                )
            })?
            .generation(&request.extension)
            .ok_or_else(|| ScriptError::new(ErrorCode::NotFound, "extension is not registered"))?;
        self.lifecycle
            .lock()
            .map_err(|_| {
                ScriptError::new(
                    ErrorCode::HostCallFailed,
                    "extension lifecycle lock poisoned",
                )
            })?
            .check(&request.extension, generation)?;
        if request.cancellation.is_cancelled() {
            return Err(ScriptError::new(
                ErrorCode::Cancelled,
                "invocation was cancelled before execution",
            ));
        }
        let cache_key = self.cache_key(&package);
        let payload_hash = digest(&request.payload);
        let future = self.run_component(&package, &request, generation);
        let result = time::timeout(
            Duration::from_millis(package.manifest.limits.deadline_ms),
            future,
        )
        .await;
        match result {
            Ok(Ok((fuel_remaining, host_calls, event_count))) => Ok(Self::receipt(
                &package,
                generation,
                &cache_key,
                ReceiptMetrics {
                    status: ReceiptStatus::Completed,
                    error: None,
                    payload_hash,
                    fuel_remaining,
                    host_calls,
                    event_count,
                },
            )),
            Ok(Err(error)) => {
                if matches!(error.code, ErrorCode::Trap | ErrorCode::LimitExceeded) {
                    let _ = self
                        .lifecycle
                        .lock()
                        .ok()
                        .and_then(|mut lifecycle| lifecycle.fault(&request.extension).ok());
                }
                Err(error)
            }
            Err(_) => {
                let _ = self
                    .lifecycle
                    .lock()
                    .ok()
                    .and_then(|mut lifecycle| lifecycle.fault(&request.extension).ok());
                Err(ScriptError::new(
                    ErrorCode::DeadlineExpired,
                    "invocation wall-clock deadline expired",
                ))
            }
        }
    }

    async fn run_component(
        &self,
        package: &ExtensionPackage,
        request: &InvocationRequest,
        generation: u64,
    ) -> Result<(Option<u64>, usize, usize), ScriptError> {
        let limits = &package.manifest.limits;
        let state = StoreState {
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::with_capacity(limits.resource_handles),
            limits: StoreLimitsBuilder::new()
                .memory_size(limits.memory_bytes)
                .table_elements(limits.table_elements)
                .instances(limits.instances)
                .tables(1)
                .memories(1)
                .trap_on_grow_failure(true)
                .build(),
            resources: ResourceRegistry::new(limits.resource_handles),
            grants: request.grants.clone(),
            host_calls: 0,
            event_count: 0,
            cancelled: request.cancellation.clone(),
            host_call_limit: limits.host_calls,
            output_limit: limits.output_bytes,
            storage: BTreeMap::new(),
            storage_bytes: 0,
            storage_limit: limits.storage_bytes,
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|state| &mut state.limits);
        store.set_fuel(limits.fuel).map_err(|error| {
            ScriptError::new(
                ErrorCode::LimitExceeded,
                format!("fuel setup failed: {error}"),
            )
        })?;
        store
            .fuel_async_yield_interval(Some(10_000))
            .map_err(|error| {
                ScriptError::new(
                    ErrorCode::LimitExceeded,
                    format!("fuel yielding setup failed: {error}"),
                )
            })?;
        store.set_epoch_deadline(1);
        let component = Component::new(&self.engine, &package.component).map_err(|error| {
            ScriptError::new(
                ErrorCode::MalformedComponent,
                format!("component validation failed: {error}"),
            )
        })?;
        let mut linker = Linker::new(&self.engine);
        if request.grants.contains(Permission::FileSystem) {
            wasmtime_wasi::p2::add_to_linker_async(&mut linker).map_err(|error| {
                ScriptError::new(
                    ErrorCode::HostCallFailed,
                    format!("WASIp2 linker setup failed: {error}"),
                )
            })?;
        }
        super::wit::rusttable::extension::host::add_to_linker::<
            _,
            wasmtime::component::HasSelf<_>,
        >(&mut linker, |state| state)
        .map_err(|error| {
            ScriptError::new(
                ErrorCode::HostCallFailed,
                format!("typed extension linker setup failed: {error}"),
            )
        })?;
        let instance = linker
            .instantiate_async(&mut store, &component)
            .await
            .map_err(|error| classify_trap(&error.to_string()))?;
        let function = instance.get_func(&mut store, "run").ok_or_else(|| {
            ScriptError::new(
                ErrorCode::IncompatibleWorld,
                "component does not export the extension run function",
            )
        })?;
        function
            .call_async(&mut store, &[], &mut [])
            .await
            .map_err(|error| classify_trap(&error.to_string()))?;
        if store.data().cancelled.is_cancelled() {
            return Err(ScriptError::new(
                ErrorCode::Cancelled,
                "invocation was cancelled",
            ));
        }
        let fuel_remaining = store.get_fuel().ok();
        let _ = store.data().resources.len();
        let _ = store.data().grants.iter().count();
        Ok((
            fuel_remaining,
            store.data().host_calls,
            store
                .data()
                .event_count
                .saturating_add(usize::from(generation == 0)),
        ))
    }

    fn validate_component(&self, package: &ExtensionPackage) -> Result<(), ScriptError> {
        if package.manifest.world != WorldVersion::CURRENT {
            return Err(ScriptError::new(
                ErrorCode::IncompatibleWorld,
                "only the current extension world is activatable",
            ));
        }
        if package.component.len() > 64 * 1024 * 1024 {
            return Err(ScriptError::new(
                ErrorCode::LimitExceeded,
                "component exceeds package byte limit",
            ));
        }
        let component = Component::new(&self.engine, &package.component).map_err(|error| {
            ScriptError::new(
                ErrorCode::MalformedComponent,
                format!("component validation failed: {error}"),
            )
        })?;
        if component.get_export_index(None, "run").is_none() {
            return Err(ScriptError::new(
                ErrorCode::IncompatibleWorld,
                "component does not export the required extension run function",
            ));
        }
        if let Some(resources) = component.resources_required() {
            if resources.num_memories > 1 || resources.num_tables > 1 {
                return Err(ScriptError::new(
                    ErrorCode::LimitExceeded,
                    "component declares resources above policy",
                ));
            }
            if resources
                .max_initial_memory_size
                .is_some_and(|size| size > package.manifest.limits.memory_bytes as u64)
            {
                return Err(ScriptError::new(
                    ErrorCode::LimitExceeded,
                    "component initial memory exceeds policy",
                ));
            }
        }
        Ok(())
    }

    fn cache_key(&self, package: &ExtensionPackage) -> CacheKey {
        CacheKey {
            wasmtime: "46.0.1".to_owned(),
            target: self.config.target.clone(),
            engine_config: digest(
                b"async|component-model|component-model-async|fuel|epoch|no-backtrace",
            ),
            component: package.provenance.component_hash.clone(),
            world: package.manifest.world,
            feature_policy: "component-model-1|wasip2-explicit|no-pooling".to_owned(),
        }
    }

    fn receipt(
        package: &ExtensionPackage,
        generation: u64,
        cache_key: &CacheKey,
        metrics: ReceiptMetrics,
    ) -> InvocationReceipt {
        let cache_key = cache_key.stable_id();
        let receipt_id = digest(
            format!(
                "{}|{}|{}|{}|{}",
                package.manifest.id,
                generation,
                package.provenance.component_hash,
                cache_key,
                metrics.payload_hash
            )
            .as_bytes(),
        );
        InvocationReceipt {
            receipt_version: 1,
            receipt_id,
            extension: package.manifest.id.clone(),
            generation,
            component_hash: package.provenance.component_hash.clone(),
            cache_key,
            status: metrics.status,
            error: metrics.error,
            payload_hash: metrics.payload_hash,
            fuel_remaining: metrics.fuel_remaining,
            host_calls: metrics.host_calls,
            event_count: metrics.event_count,
        }
    }

    #[must_use]
    pub fn state(&self, extension: &ExtensionId) -> Option<ExtensionState> {
        self.lifecycle
            .lock()
            .ok()
            .and_then(|lifecycle| lifecycle.state(extension))
    }
}

fn classify_trap(message: &str) -> ScriptError {
    let code =
        if message.contains("fuel") || message.contains("resource") || message.contains("memory") {
            ErrorCode::LimitExceeded
        } else {
            ErrorCode::Trap
        };
    ScriptError::new(code, "guest invocation trapped")
}
