#![forbid(unsafe_code)]
#![doc = "A bounded Lua 5.4 host with typed, capability-gated `RustTable` APIs."]

mod api;
mod capabilities;
mod dto;
mod errors;
mod events;
mod host;
mod limits;
mod storage;

pub mod conformance;

pub use api::{
    ApiVersion, Capability, CapabilitySet, Permission, ScriptId, ScriptManifest, source_hash,
};
pub use capabilities::{CommandPort, HostPorts, NotificationPort, ScriptCapabilities};
pub use dto::{CatalogSnapshot, EditProposal, EventDto, ExportProposal, MetadataValue};
pub use errors::{ErrorCode, ScriptError};
pub use events::{EventBus, EventFilter, EventKind, EventRecord, EventSubscription};
pub use host::{HostConfig, InvocationReceipt, LuaHost, ScriptState};
pub use limits::{LimitKind, QuotaLedger, ScriptLimits};
pub use storage::{ScriptStorage, StorageReceipt, StorageTransaction};
