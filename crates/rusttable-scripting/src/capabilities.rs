use std::sync::Arc;

use crate::api::{Capability, CapabilitySet};
use crate::dto::{CatalogSnapshot, EditProposal, ExportProposal};
use crate::errors::{ErrorCode, ScriptError};

pub trait CatalogPort: Send + Sync {
    fn snapshot(&self) -> Result<CatalogSnapshot, ScriptError>;
}

pub trait CommandPort: Send + Sync {
    /// # Errors
    ///
    /// Returns a validated application-port error when the proposal is rejected.
    fn propose_edit(&self, proposal: EditProposal) -> Result<String, ScriptError>;
    /// # Errors
    ///
    /// Returns a validated application-port error when queue submission is rejected.
    fn submit_export(&self, proposal: ExportProposal) -> Result<String, ScriptError>;
}

pub trait NotificationPort: Send + Sync {
    /// # Errors
    ///
    /// Returns an application-port error when delivery is rejected.
    fn notify(&self, title: String, body: String) -> Result<(), ScriptError>;
}

#[derive(Clone, Default)]
pub struct HostPorts {
    pub catalog: Option<Arc<dyn CatalogPort>>,
    pub commands: Option<Arc<dyn CommandPort>>,
    pub notifications: Option<Arc<dyn NotificationPort>>,
}

#[derive(Clone, Default)]
pub struct ScriptCapabilities {
    permissions: CapabilitySet,
    ports: HostPorts,
}

impl ScriptCapabilities {
    #[must_use]
    pub fn new(
        permissions: &std::collections::BTreeSet<crate::Permission>,
        ports: HostPorts,
    ) -> Self {
        Self {
            permissions: CapabilitySet::from_permissions(permissions),
            ports,
        }
    }

    /// # Errors
    ///
    /// Returns `PermissionDenied` when the manifest did not grant the capability.
    pub fn require(&self, capability: Capability) -> Result<(), ScriptError> {
        self.permissions
            .contains(capability)
            .then_some(())
            .ok_or_else(|| {
                ScriptError::new(
                    ErrorCode::PermissionDenied,
                    format!("capability denied: {capability:?}"),
                )
            })
    }

    /// # Errors
    ///
    /// Returns `PermissionDenied` when catalog read is not granted or no port is installed.
    pub fn catalog(&self) -> Result<&dyn CatalogPort, ScriptError> {
        self.require(Capability::CatalogSnapshot)?;
        self.ports.catalog.as_deref().ok_or_else(|| {
            ScriptError::new(ErrorCode::PermissionDenied, "catalog port is unavailable")
        })
    }

    /// # Errors
    ///
    /// Returns `PermissionDenied` when metadata proposals are not granted or no port is installed.
    pub fn commands(&self) -> Result<&dyn CommandPort, ScriptError> {
        self.require(Capability::MetadataProposal)?;
        self.ports.commands.as_deref().ok_or_else(|| {
            ScriptError::new(ErrorCode::PermissionDenied, "command port is unavailable")
        })
    }

    /// # Errors
    ///
    /// Returns `PermissionDenied` when export submission is not granted or no port is installed.
    pub fn exports(&self) -> Result<&dyn CommandPort, ScriptError> {
        self.require(Capability::ExportQueue)?;
        self.ports.commands.as_deref().ok_or_else(|| {
            ScriptError::new(ErrorCode::PermissionDenied, "command port is unavailable")
        })
    }

    /// # Errors
    ///
    /// Returns `PermissionDenied` when notifications are not granted or no port is installed.
    pub fn notifications(&self) -> Result<&dyn NotificationPort, ScriptError> {
        self.require(Capability::Notifications)?;
        self.ports.notifications.as_deref().ok_or_else(|| {
            ScriptError::new(
                ErrorCode::PermissionDenied,
                "notification port is unavailable",
            )
        })
    }
}
