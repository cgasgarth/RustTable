use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use futures::FutureExt;
use mlua::{
    DebugEvent, Function, HookTriggers, Lua, LuaOptions, LuaSerdeExt, StdLib, Value, VmState,
};
use serde::{Deserialize, Serialize};

use crate::api::{API_VERSION, Capability, ScriptId, ScriptManifest};
use crate::capabilities::{HostPorts, ScriptCapabilities};
use crate::dto::{EditProposal, ExportProposal};
use crate::errors::{ErrorCode, ScriptError};
use crate::events::{EventBus, EventFilter, EventKind, EventSubscription};
use crate::limits::{QuotaLedger, ScriptLimits};
use crate::storage::ScriptStorage;

#[derive(Debug, Clone)]
pub struct HostConfig {
    pub limits: ScriptLimits,
    pub max_faults: u32,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            limits: ScriptLimits::default(),
            max_faults: 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScriptState {
    Enabled,
    Disabled,
    Quarantined,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvocationReceipt {
    pub script_id: String,
    pub generation: u64,
    pub status: String,
    pub result_bytes: usize,
    pub error_code: Option<ErrorCode>,
}

struct ScriptRecord {
    source: String,
    lua: Lua,
    generation: Arc<AtomicU64>,
    cancelled: Arc<AtomicBool>,
    ledger: Arc<Mutex<QuotaLedger>>,
    storage: Arc<Mutex<ScriptStorage>>,
    state: ScriptState,
    faults: u32,
    events: EventSubscription,
}

pub struct LuaHost {
    config: HostConfig,
    ports: HostPorts,
    scripts: BTreeMap<ScriptId, ScriptRecord>,
    event_bus: EventBus,
}

impl LuaHost {
    #[must_use]
    pub fn new(config: HostConfig, ports: HostPorts) -> Self {
        let max_queue = config.limits.max_event_queue;
        Self {
            config,
            ports,
            scripts: BTreeMap::new(),
            event_bus: EventBus::new(max_queue),
        }
    }

    /// # Errors
    ///
    /// Returns a stable manifest, permission, source, or Lua setup error when installation fails.
    pub fn install(&mut self, manifest: &ScriptManifest, source: &str) -> Result<(), ScriptError> {
        manifest.validate(source.as_bytes())?;
        let ledger = Arc::new(Mutex::new(QuotaLedger::new(self.config.limits)));
        ledger
            .lock()
            .map_err(|_| ScriptError::new(ErrorCode::PanicContained, "quota ledger poisoned"))?
            .check_source(source.len())?;
        let generation = Arc::new(AtomicU64::new(1));
        let cancelled = Arc::new(AtomicBool::new(false));
        let storage = Arc::new(Mutex::new(ScriptStorage::new(
            manifest.storage_version,
            self.config.limits.max_storage_bytes,
            self.config.limits.max_storage_keys,
        )));
        let lua = create_lua(
            manifest,
            self.ports.clone(),
            &generation,
            &cancelled,
            &ledger,
            &storage,
        )?;
        ledger
            .lock()
            .map_err(|_| ScriptError::new(ErrorCode::PanicContained, "quota ledger poisoned"))?
            .reset_invocation();
        let id = manifest.id.clone();
        let events = self.event_bus.subscribe(EventFilter::default());
        self.scripts.insert(
            id,
            ScriptRecord {
                source: source.to_owned(),
                lua,
                generation,
                cancelled,
                ledger,
                storage,
                state: ScriptState::Enabled,
                faults: 0,
                events,
            },
        );
        Ok(())
    }

    /// # Errors
    ///
    /// Returns a stable cancellation, quota, serialization, Lua, or quarantine error.
    pub async fn execute(&mut self, id: &ScriptId) -> Result<InvocationReceipt, ScriptError> {
        let record = self.scripts.get_mut(id).ok_or_else(|| {
            ScriptError::new(ErrorCode::InvalidManifest, "script is not installed")
        })?;
        if record.state != ScriptState::Enabled {
            return Err(ScriptError::new(
                ErrorCode::ScriptQuarantined,
                "script is disabled or quarantined",
            ));
        }
        let generation = record.generation.load(Ordering::Acquire);
        record.cancelled.store(false, Ordering::Release);
        record
            .ledger
            .lock()
            .map_err(|_| ScriptError::new(ErrorCode::PanicContained, "quota ledger poisoned"))?
            .reset_invocation();
        record
            .ledger
            .lock()
            .map_err(|_| ScriptError::new(ErrorCode::PanicContained, "quota ledger poisoned"))?
            .enter_call()?;
        let future = record.lua.load(&record.source).eval_async::<Value>();
        let result = std::panic::AssertUnwindSafe(future).catch_unwind().await;
        record
            .ledger
            .lock()
            .map_err(|_| ScriptError::new(ErrorCode::PanicContained, "quota ledger poisoned"))?
            .leave_call();
        let value = match result {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => return self.record_fault(id, error.into()),
            Err(_) => {
                return self.record_fault(
                    id,
                    ScriptError::new(ErrorCode::PanicContained, "script panic contained"),
                );
            }
        };
        let value: serde_json::Value = record
            .lua
            .from_value(value)
            .map_err(|_| ScriptError::new(ErrorCode::SerializationBound, "result is not a DTO"))?;
        let bytes = serde_json::to_vec(&value).map_err(|_| {
            ScriptError::new(ErrorCode::SerializationBound, "result encoding failed")
        })?;
        record
            .ledger
            .lock()
            .map_err(|_| ScriptError::new(ErrorCode::PanicContained, "quota ledger poisoned"))?
            .check_result(bytes.len())?;
        Ok(InvocationReceipt {
            script_id: id.to_string(),
            generation,
            status: "ok".to_owned(),
            result_bytes: bytes.len(),
            error_code: None,
        })
    }

    /// # Errors
    ///
    /// Returns a stable manifest, quota, or Lua setup error when replacement fails.
    pub fn reload(
        &mut self,
        id: &ScriptId,
        manifest: &ScriptManifest,
        source: &str,
    ) -> Result<(), ScriptError> {
        let record = self.scripts.get_mut(id).ok_or_else(|| {
            ScriptError::new(ErrorCode::InvalidManifest, "script is not installed")
        })?;
        manifest.validate(source.as_bytes())?;
        record.cancelled.store(true, Ordering::Release);
        record.generation.fetch_add(1, Ordering::AcqRel);
        let cancelled = Arc::new(AtomicBool::new(false));
        record.lua = create_lua(
            manifest,
            self.ports.clone(),
            &record.generation,
            &cancelled,
            &record.ledger,
            &record.storage,
        )?;
        source.clone_into(&mut record.source);
        record.state = ScriptState::Enabled;
        record.faults = 0;
        record.cancelled = cancelled;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns `InvalidManifest` when the script is not installed.
    pub fn disable(&mut self, id: &ScriptId) -> Result<(), ScriptError> {
        let record = self.scripts.get_mut(id).ok_or_else(|| {
            ScriptError::new(ErrorCode::InvalidManifest, "script is not installed")
        })?;
        record.cancelled.store(true, Ordering::Release);
        record.generation.fetch_add(1, Ordering::AcqRel);
        record.state = ScriptState::Disabled;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns `EventBackpressure` when the bounded event queue cannot accept the event.
    pub fn publish_event(&mut self, kind: EventKind, payload: String) -> Result<u64, ScriptError> {
        self.event_bus.publish(kind, payload)
    }

    /// # Errors
    ///
    /// Returns a stable event, callback, serialization, or reentrancy error.
    pub async fn deliver_events(&mut self, id: &ScriptId) -> Result<usize, ScriptError> {
        let events = {
            let record = self.scripts.get_mut(id).ok_or_else(|| {
                ScriptError::new(ErrorCode::InvalidManifest, "script is not installed")
            })?;
            self.event_bus.drain(&mut record.events)?
        };
        let record = self.scripts.get(id).ok_or_else(|| {
            ScriptError::new(ErrorCode::InvalidManifest, "script is not installed")
        })?;
        let callback = record.lua.globals().get::<Option<Function>>("on_event")?;
        let Some(callback) = callback else {
            return Ok(events.len());
        };
        for event in &events {
            let value = record.lua.to_value(&event.payload)?;
            callback.call_async::<()>(value).await?;
        }
        Ok(events.len())
    }

    #[must_use]
    pub fn state(&self, id: &ScriptId) -> Option<ScriptState> {
        self.scripts.get(id).map(|record| record.state)
    }

    #[must_use]
    pub fn storage(&self, id: &ScriptId) -> Option<Arc<Mutex<ScriptStorage>>> {
        self.scripts
            .get(id)
            .map(|record| Arc::clone(&record.storage))
    }

    fn record_fault(
        &mut self,
        id: &ScriptId,
        error: ScriptError,
    ) -> Result<InvocationReceipt, ScriptError> {
        let record = self.scripts.get_mut(id).ok_or_else(|| {
            ScriptError::new(ErrorCode::InvalidManifest, "script is not installed")
        })?;
        record.faults = record.faults.saturating_add(1);
        if record.faults >= self.config.max_faults {
            record.state = ScriptState::Quarantined;
        }
        Err(error)
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "Lua module registration remains one auditable capability boundary."
)]
fn create_lua(
    manifest: &ScriptManifest,
    ports: HostPorts,
    generation: &Arc<AtomicU64>,
    cancelled: &Arc<AtomicBool>,
    ledger: &Arc<Mutex<QuotaLedger>>,
    storage: &Arc<Mutex<ScriptStorage>>,
) -> Result<Lua, ScriptError> {
    let lua = Lua::new_with(
        StdLib::COROUTINE | StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8,
        LuaOptions::default(),
    )?;
    let limits = ledger
        .lock()
        .map_err(|_| ScriptError::new(ErrorCode::PanicContained, "quota ledger poisoned"))?
        .limits();
    lua.set_memory_limit(limits.max_memory_bytes)?;
    let module = lua.create_table()?;
    let version =
        lua.create_function(|_, ()| Ok(format!("{}.{}", API_VERSION.major, API_VERSION.minor)))?;
    module.set("application_version", version)?;
    let capabilities = ScriptCapabilities::new(&manifest.permissions, ports.clone());
    let catalog = lua.create_table()?;
    let catalog_capabilities = capabilities.clone();
    let catalog_ports = ports.catalog.clone();
    catalog.set(
        "snapshot",
        lua.create_async_function(move |lua, ()| {
            let capabilities = catalog_capabilities.clone();
            let port = catalog_ports.clone();
            async move {
                capabilities
                    .require(Capability::CatalogSnapshot)
                    .map_err(mlua::Error::external)?;
                let port = port.ok_or_else(|| mlua::Error::external("catalog port unavailable"))?;
                let snapshot = port.snapshot().map_err(mlua::Error::external)?;
                lua.to_value(&snapshot)
            }
        })?,
    )?;
    module.set("catalog", catalog)?;
    let metadata = lua.create_table()?;
    let metadata_capabilities = capabilities.clone();
    let command_ports = ports.commands.clone();
    metadata.set(
        "propose",
        lua.create_async_function(move |lua, value: Value| {
            let capabilities = metadata_capabilities.clone();
            let commands = command_ports.clone();
            async move {
                capabilities
                    .require(Capability::MetadataProposal)
                    .map_err(mlua::Error::external)?;
                let proposal: EditProposal = lua.from_value(value)?;
                if !proposal.bounded(128) || proposal.reason.len() > 512 {
                    return Err(mlua::Error::external("proposal DTO exceeds bounds"));
                }
                let commands =
                    commands.ok_or_else(|| mlua::Error::external("command port unavailable"))?;
                commands
                    .propose_edit(proposal)
                    .map_err(mlua::Error::external)
            }
        })?,
    )?;
    module.set("metadata", metadata)?;
    let export = lua.create_table()?;
    let export_capabilities = capabilities.clone();
    let export_ports = ports.commands;
    export.set(
        "submit",
        lua.create_async_function(move |lua, value: Value| {
            let capabilities = export_capabilities.clone();
            let commands = export_ports.clone();
            async move {
                capabilities
                    .require(Capability::ExportQueue)
                    .map_err(mlua::Error::external)?;
                let proposal: ExportProposal = lua.from_value(value)?;
                if proposal.item_ids.len() > 256
                    || proposal.destination.len() > 512
                    || proposal.profile.len() > 128
                {
                    return Err(mlua::Error::external("export DTO exceeds bounds"));
                }
                let commands =
                    commands.ok_or_else(|| mlua::Error::external("command port unavailable"))?;
                commands
                    .submit_export(proposal)
                    .map_err(mlua::Error::external)
            }
        })?,
    )?;
    module.set("export", export)?;
    let storage_module = lua.create_table()?;
    let storage_capabilities = capabilities.clone();
    let storage_for_get = Arc::clone(storage);
    storage_module.set(
        "get",
        lua.create_function(move |_, key: String| {
            storage_capabilities
                .require(Capability::ExtensionStorage)
                .map_err(mlua::Error::external)?;
            let storage = storage_for_get
                .lock()
                .map_err(|_| mlua::Error::external("storage poisoned"))?;
            Ok(storage
                .get(&key)
                .map(|value| String::from_utf8_lossy(value).into_owned()))
        })?,
    )?;
    let storage_capabilities = capabilities.clone();
    let storage_for_set = Arc::clone(storage);
    storage_module.set(
        "set",
        lua.create_function(move |_, (key, value): (String, String)| {
            storage_capabilities
                .require(Capability::ExtensionStorage)
                .map_err(mlua::Error::external)?;
            if key.len() > 256 || value.len() > 64 * 1024 {
                return Err(mlua::Error::external("storage value exceeds bounds"));
            }
            let mut storage = storage_for_set
                .lock()
                .map_err(|_| mlua::Error::external("storage poisoned"))?;
            let mut transaction = storage.begin();
            transaction.put(key, value.into_bytes());
            transaction
                .commit(&mut storage)
                .map_err(mlua::Error::external)?;
            Ok(())
        })?,
    )?;
    module.set("storage", storage_module)?;
    let notifications = lua.create_table()?;
    let notification_capabilities = capabilities.clone();
    let notification_ports = ports.notifications;
    notifications.set(
        "send",
        lua.create_async_function(move |_, (title, body): (String, String)| {
            let capabilities = notification_capabilities.clone();
            let ports = notification_ports.clone();
            async move {
                capabilities
                    .require(Capability::Notifications)
                    .map_err(mlua::Error::external)?;
                if title.len() > 128 || body.len() > 4096 {
                    return Err(mlua::Error::external("notification exceeds bounds"));
                }
                let ports =
                    ports.ok_or_else(|| mlua::Error::external("notification port unavailable"))?;
                ports.notify(title, body).map_err(mlua::Error::external)
            }
        })?,
    )?;
    module.set("notifications", notifications)?;
    lua.globals().set("rusttable", module)?;
    install_limits(&lua, limits, generation, cancelled, ledger)?;
    Ok(lua)
}

fn install_limits(
    lua: &Lua,
    limits: ScriptLimits,
    generation: &Arc<AtomicU64>,
    cancelled: &Arc<AtomicBool>,
    ledger: &Arc<Mutex<QuotaLedger>>,
) -> Result<(), ScriptError> {
    let recursion = Arc::new(Mutex::new(0_u32));
    let hook_generation = Arc::clone(generation);
    let hook_cancelled = Arc::clone(cancelled);
    let hook_ledger = Arc::clone(ledger);
    let hook_recursion = Arc::clone(&recursion);
    let recursion_limit = limits.max_recursion.min(limits.max_stack_slots);
    let triggers = HookTriggers::new()
        .every_nth_instruction(1_000)
        .on_calls()
        .on_returns();
    lua.set_global_hook(triggers, move |_lua, debug| {
        if hook_cancelled.load(Ordering::Acquire) {
            return Err(mlua::Error::RuntimeError("script cancelled".to_owned()));
        }
        let mut ledger = hook_ledger
            .lock()
            .map_err(|_| mlua::Error::RuntimeError("quota ledger poisoned".to_owned()))?;
        ledger
            .tick(1_000)
            .map_err(|error| mlua::Error::RuntimeError(error.to_string()))?;
        let mut recursion = hook_recursion
            .lock()
            .map_err(|_| mlua::Error::RuntimeError("recursion counter poisoned".to_owned()))?;
        match debug.event() {
            DebugEvent::Call => {
                *recursion = recursion.saturating_add(1);
                if *recursion > recursion_limit {
                    return Err(mlua::Error::RuntimeError(
                        "quota exceeded: recursion".to_owned(),
                    ));
                }
            }
            DebugEvent::Ret => *recursion = recursion.saturating_sub(1),
            _ => {}
        }
        let _ = hook_generation.load(Ordering::Acquire);
        Ok(VmState::Continue)
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{Permission, source_hash};
    use std::collections::BTreeSet;

    fn manifest(source: &str, permissions: BTreeSet<Permission>) -> ScriptManifest {
        ScriptManifest {
            id: ScriptId::new("host-test").expect("id"),
            name: "Host test".to_owned(),
            api_min: API_VERSION,
            api_max: API_VERSION,
            permissions,
            storage_version: 1,
            enabled: true,
            source_sha256: source_hash(source.as_bytes()),
        }
    }

    #[test]
    fn safe_library_surface_and_quarantine_are_enforced() {
        let mut host = LuaHost::new(
            HostConfig {
                max_faults: 2,
                ..HostConfig::default()
            },
            HostPorts::default(),
        );
        let source = "return os.execute('bad')";
        let id = manifest(source, BTreeSet::new()).id.clone();
        let manifest = manifest(source, BTreeSet::new());
        host.install(&manifest, source).expect("install");
        assert!(futures::executor::block_on(host.execute(&id)).is_err());
        assert!(futures::executor::block_on(host.execute(&id)).is_err());
        assert_eq!(host.state(&id), Some(ScriptState::Quarantined));
    }

    #[test]
    fn reload_invalidates_generation_and_preserves_no_lua_handles() {
        let mut host = LuaHost::new(HostConfig::default(), HostPorts::default());
        let first = "return 1";
        let id = manifest(first, BTreeSet::new()).id.clone();
        let first_manifest = manifest(first, BTreeSet::new());
        host.install(&first_manifest, first).expect("install");
        let old = futures::executor::block_on(host.execute(&id)).expect("execute");
        let second = "return 2";
        let second_manifest = manifest(second, BTreeSet::new());
        host.reload(&id, &second_manifest, second).expect("reload");
        let new = futures::executor::block_on(host.execute(&id)).expect("execute");
        assert!(new.generation > old.generation);
    }

    #[test]
    fn instruction_limit_preempts_an_infinite_script() {
        let source = "while true do end";
        let mut host = LuaHost::new(
            HostConfig {
                limits: ScriptLimits {
                    max_instructions: 100_000,
                    max_wall_time_ms: 1_000,
                    ..ScriptLimits::default()
                },
                max_faults: 1,
            },
            HostPorts::default(),
        );
        let id = manifest(source, BTreeSet::new()).id.clone();
        let manifest = manifest(source, BTreeSet::new());
        host.install(&manifest, source).expect("install");
        let error = futures::executor::block_on(host.execute(&id)).unwrap_err();
        assert_eq!(error.code, ErrorCode::LimitExceeded);
    }
}
