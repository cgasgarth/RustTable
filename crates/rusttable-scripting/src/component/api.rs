use std::collections::BTreeSet;
use std::fmt;

use super::{
    errors::{ErrorCode, ScriptError},
    limits::ScriptLimits,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
pub struct ExtensionId(String);

impl ExtensionId {
    /// Creates a bounded, stable extension identifier.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when the identifier is empty, too long, or contains an unsupported byte.
    pub fn new(value: impl Into<String>) -> Result<Self, ScriptError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            return Err(ScriptError::new(
                ErrorCode::InvalidManifest,
                "extension id is not a bounded identifier",
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExtensionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
pub struct WorldVersion {
    pub major: u16,
    pub minor: u16,
}

impl WorldVersion {
    pub const CURRENT: Self = Self { major: 1, minor: 0 };

    #[must_use]
    pub const fn compatible_with(self, requested: Self) -> bool {
        self.major == requested.major && self.minor >= requested.minor
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
pub enum Permission {
    CatalogRead,
    SelectionRead,
    MetadataProposal,
    ExportProposal,
    Notifications,
    Storage,
    FileSystem,
    Network,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct CapabilitySet(BTreeSet<Permission>);

impl CapabilitySet {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn grant(&mut self, permission: Permission) {
        self.0.insert(permission);
    }

    #[must_use]
    pub fn contains(&self, permission: Permission) -> bool {
        self.0.contains(&permission)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Permission> {
        self.0.iter()
    }

    /// Requires a granted capability.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] with [`ErrorCode::PermissionDenied`] when the capability is absent.
    pub fn require(&self, permission: Permission) -> Result<(), ScriptError> {
        self.contains(permission).then_some(()).ok_or_else(|| {
            ScriptError::new(
                ErrorCode::PermissionDenied,
                format!("permission {permission:?} was not granted"),
            )
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ExtensionManifest {
    pub id: ExtensionId,
    pub name: String,
    pub version: String,
    pub world: WorldVersion,
    pub requested_permissions: CapabilitySet,
    pub limits: ScriptLimits,
}

impl ExtensionManifest {
    /// Validates manifest identity, world compatibility, and resource policy.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when a manifest field, world version, or limit is invalid.
    pub fn validate(&self) -> Result<(), ScriptError> {
        if self.name.is_empty()
            || self.name.len() > 256
            || self.version.is_empty()
            || self.version.len() > 64
        {
            return Err(ScriptError::new(
                ErrorCode::InvalidManifest,
                "manifest text fields are out of bounds",
            ));
        }
        if !WorldVersion::CURRENT.compatible_with(self.world) {
            return Err(ScriptError::new(
                ErrorCode::IncompatibleWorld,
                "required WIT world is not compatible",
            ));
        }
        self.limits.validate()
    }
}
