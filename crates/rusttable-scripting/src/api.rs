use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::errors::{ErrorCode, ScriptError};

pub const API_VERSION: ApiVersion = ApiVersion { major: 1, minor: 0 };

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ApiVersion {
    pub major: u16,
    pub minor: u16,
}

impl ApiVersion {
    #[must_use]
    pub const fn compatible_with(self, other: Self) -> bool {
        self.major == other.major && self.minor >= other.minor
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    CatalogRead,
    MetadataPropose,
    SelectionRead,
    ExportSubmit,
    Notifications,
    Storage,
    NetworkRoot(String),
    FileRoot(String),
}

impl Permission {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        match self {
            Self::NetworkRoot(root) | Self::FileRoot(root) => {
                !root.is_empty() && !root.contains("..") && !root.contains('\0')
            }
            _ => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    ApplicationVersion,
    CatalogSnapshot,
    MetadataProposal,
    Selection,
    ExportQueue,
    Notifications,
    ExtensionStorage,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySet(BTreeSet<Capability>);

impl CapabilitySet {
    #[must_use]
    pub fn from_permissions(permissions: &BTreeSet<Permission>) -> Self {
        let mut capabilities = BTreeSet::from([Capability::ApplicationVersion]);
        for permission in permissions {
            let capability = match permission {
                Permission::CatalogRead => Some(Capability::CatalogSnapshot),
                Permission::MetadataPropose => Some(Capability::MetadataProposal),
                Permission::SelectionRead => Some(Capability::Selection),
                Permission::ExportSubmit => Some(Capability::ExportQueue),
                Permission::Notifications => Some(Capability::Notifications),
                Permission::Storage => Some(Capability::ExtensionStorage),
                Permission::NetworkRoot(_) | Permission::FileRoot(_) => None,
            };
            if let Some(capability) = capability {
                capabilities.insert(capability);
            }
        }
        Self(capabilities)
    }

    #[must_use]
    pub fn contains(&self, capability: Capability) -> bool {
        self.0.contains(&capability)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ScriptId(String);

impl ScriptId {
    /// # Errors
    ///
    /// Returns `InvalidManifest` when the identifier is empty, oversized, or contains
    /// characters outside the stable script identifier alphabet.
    pub fn new(value: impl Into<String>) -> Result<Self, ScriptError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 96
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(ScriptError::new(
                ErrorCode::InvalidManifest,
                "script id must be 1..96 ASCII identifier characters",
            ));
        }
        Ok(Self(value))
    }
}

impl AsRef<str> for ScriptId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ScriptId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptManifest {
    pub id: ScriptId,
    pub name: String,
    pub api_min: ApiVersion,
    pub api_max: ApiVersion,
    pub permissions: BTreeSet<Permission>,
    pub storage_version: u32,
    pub enabled: bool,
    pub source_sha256: String,
}

impl ScriptManifest {
    /// # Errors
    ///
    /// Returns a stable manifest or compatibility error when metadata, permissions, or the
    /// source hash is invalid.
    pub fn validate(&self, source: &[u8]) -> Result<(), ScriptError> {
        if self.name.trim().is_empty() || self.name.len() > 128 {
            return Err(ScriptError::new(
                ErrorCode::InvalidManifest,
                "script name is empty or oversized",
            ));
        }
        if self.api_min > self.api_max || !self.api_max.compatible_with(API_VERSION) {
            return Err(ScriptError::new(
                ErrorCode::IncompatibleApi,
                "manifest API range does not include the host API",
            ));
        }
        if self
            .permissions
            .iter()
            .any(|permission| !permission.is_valid())
        {
            return Err(ScriptError::new(
                ErrorCode::InvalidManifest,
                "permission root is invalid",
            ));
        }
        let expected = source_hash(source);
        if self.source_sha256 != expected {
            return Err(ScriptError::new(
                ErrorCode::InvalidManifest,
                "source hash does not match manifest",
            ));
        }
        Ok(())
    }
}

#[must_use]
pub fn source_hash(source: &[u8]) -> String {
    use std::fmt::Write;

    let digest = Sha256::digest(source);
    let mut hash = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(hash, "{byte:02x}").expect("writing to String cannot fail");
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(source: &[u8]) -> ScriptManifest {
        ScriptManifest {
            id: ScriptId::new("fixture").expect("valid id"),
            name: "Fixture".to_owned(),
            api_min: API_VERSION,
            api_max: API_VERSION,
            permissions: BTreeSet::new(),
            storage_version: 1,
            enabled: true,
            source_sha256: source_hash(source),
        }
    }

    #[test]
    fn manifest_validation_is_hash_and_range_bound() {
        let source = b"return 1";
        assert!(manifest(source).validate(source).is_ok());
        assert_eq!(
            ScriptId::new("../escape").unwrap_err().code,
            ErrorCode::InvalidManifest
        );
    }

    #[test]
    fn permissions_map_only_to_explicit_capabilities() {
        let permissions = BTreeSet::from([Permission::CatalogRead, Permission::Storage]);
        let capabilities = CapabilitySet::from_permissions(&permissions);
        assert!(capabilities.contains(Capability::CatalogSnapshot));
        assert!(capabilities.contains(Capability::ExtensionStorage));
        assert!(!capabilities.contains(Capability::ExportQueue));
    }
}
