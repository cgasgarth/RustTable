use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;
use std::fmt::Write as _;

use crate::descriptor::OperationFlags;
use crate::{OperationDefinition, RegistrySnapshot};

pub const REGISTRY_CLOSURE_SCHEMA: &str = "rusttable.operation-capabilities.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationClassification {
    Implemented,
    DeprecatedImplemented,
    IntentionallyUnsupportedBlocking,
    ReferenceOnlyNonProduct,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryClosureEntry {
    pub identity: String,
    pub compatibility_name: String,
    pub rust_id: Option<String>,
    pub reference_path: String,
    pub descriptor_version: u16,
    pub parameter_versions: Vec<u16>,
    pub implementation_crate: String,
    pub issue_sequence_id: u16,
    pub order: usize,
    pub cpu_supported: bool,
    pub gpu_supported: bool,
    pub cpu_fallback: bool,
    pub status: OperationClassification,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryClosure {
    pub schema: String,
    pub entries: Vec<RegistryClosureEntry>,
    pub identity_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryClosureError {
    EmptyIdentity,
    DuplicateIdentity(String),
    MissingRustIdentity(String),
    MissingImplementation(String),
    MissingIssue(String),
    MissingVersion(String),
    MissingReason(String),
    InvalidStatus(String),
    Serialization(String),
}

impl fmt::Display for RegistryClosureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyIdentity => formatter.write_str("operation closure identity is empty"),
            Self::DuplicateIdentity(identity) => {
                write!(
                    formatter,
                    "operation closure identity is duplicated: {identity}"
                )
            }
            Self::MissingRustIdentity(identity) => {
                write!(
                    formatter,
                    "implemented operation has no Rust identity: {identity}"
                )
            }
            Self::MissingImplementation(identity) => {
                write!(
                    formatter,
                    "implemented operation has no implementation owner: {identity}"
                )
            }
            Self::MissingIssue(identity) => {
                write!(
                    formatter,
                    "blocking operation has no issue sequence ID: {identity}"
                )
            }
            Self::MissingVersion(identity) => {
                write!(
                    formatter,
                    "operation has no descriptor or parameter version: {identity}"
                )
            }
            Self::MissingReason(identity) => {
                write!(formatter, "unsupported operation has no reason: {identity}")
            }
            Self::InvalidStatus(identity) => {
                write!(formatter, "operation status is inconsistent: {identity}")
            }
            Self::Serialization(error) => {
                write!(formatter, "operation closure serialization failed: {error}")
            }
        }
    }
}

impl std::error::Error for RegistryClosureError {}

impl RegistryClosure {
    /// Builds the canonical, sorted closure and rejects incomplete entries.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate identities, incomplete unsupported entries, or serialization
    /// failure while computing the closure hash.
    pub fn new(mut entries: Vec<RegistryClosureEntry>) -> Result<Self, RegistryClosureError> {
        entries.sort_by(|left, right| left.identity.cmp(&right.identity));
        let mut identities = BTreeSet::new();
        for entry in &entries {
            validate_entry(entry)?;
            if !identities.insert(entry.identity.clone()) {
                return Err(RegistryClosureError::DuplicateIdentity(
                    entry.identity.clone(),
                ));
            }
        }
        let mut closure = Self {
            schema: REGISTRY_CLOSURE_SCHEMA.to_owned(),
            entries,
            identity_hash: String::new(),
        };
        closure.identity_hash = closure.compute_hash()?;
        Ok(closure)
    }

    /// Creates implemented entries directly from the first-party registry.
    ///
    /// # Errors
    ///
    /// Returns an error if a registry definition cannot satisfy the closure contract.
    pub fn from_registry(registry: &RegistrySnapshot) -> Result<Self, RegistryClosureError> {
        let entries = registry
            .definitions()
            .iter()
            .enumerate()
            .map(|(order, definition)| entry_from_definition(definition, order))
            .collect();
        Self::new(entries)
    }

    ///
    /// # Errors
    ///
    /// Returns an error if the closure cannot be serialized canonically.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, RegistryClosureError> {
        let mut copy = self.clone();
        copy.identity_hash.clear();
        serde_json::to_vec(&copy)
            .map_err(|error| RegistryClosureError::Serialization(error.to_string()))
    }

    #[must_use]
    pub fn identity_hash(&self) -> &str {
        &self.identity_hash
    }

    fn compute_hash(&self) -> Result<String, RegistryClosureError> {
        let digest = Sha256::digest(self.canonical_bytes()?);
        let mut output = String::with_capacity(digest.len() * 2);
        for byte in digest {
            write!(output, "{byte:02x}").expect("writing to a String cannot fail");
        }
        Ok(output)
    }
}

fn entry_from_definition(definition: &OperationDefinition, order: usize) -> RegistryClosureEntry {
    let id = &definition.descriptor().id;
    RegistryClosureEntry {
        identity: id.rust_id.clone(),
        compatibility_name: id.compatibility_name.clone(),
        rust_id: Some(id.rust_id.clone()),
        reference_path: String::new(),
        descriptor_version: id.schema_version,
        parameter_versions: definition.descriptor().migration.source_versions.clone(),
        implementation_crate: "rusttable-processing".to_owned(),
        issue_sequence_id: 395,
        order,
        cpu_supported: definition.cpu().is_some(),
        gpu_supported: definition.gpu().is_some(),
        cpu_fallback: definition.cpu().is_some() && definition.gpu().is_some(),
        status: if definition.availability().is_available() {
            if definition
                .descriptor()
                .flags
                .contains(OperationFlags::DEPRECATED)
            {
                OperationClassification::DeprecatedImplemented
            } else {
                OperationClassification::Implemented
            }
        } else {
            OperationClassification::IntentionallyUnsupportedBlocking
        },
        reason: definition
            .availability()
            .reason()
            .map(str::to_owned)
            .or_else(|| {
                (definition
                    .descriptor()
                    .flags
                    .contains(OperationFlags::DEPRECATED))
                .then(|| {
                    "deprecated compatibility operation; hidden from new-edit discovery".to_owned()
                })
            }),
    }
}

fn validate_entry(entry: &RegistryClosureEntry) -> Result<(), RegistryClosureError> {
    if entry.identity.trim().is_empty() {
        return Err(RegistryClosureError::EmptyIdentity);
    }
    if entry.descriptor_version == 0 || entry.parameter_versions.is_empty() {
        return Err(RegistryClosureError::MissingVersion(entry.identity.clone()));
    }
    if matches!(
        entry.status,
        OperationClassification::Implemented | OperationClassification::DeprecatedImplemented
    ) {
        if entry.rust_id.is_none() {
            return Err(RegistryClosureError::MissingRustIdentity(
                entry.identity.clone(),
            ));
        }
        if entry.implementation_crate.trim().is_empty() {
            return Err(RegistryClosureError::MissingImplementation(
                entry.identity.clone(),
            ));
        }
        if !entry.cpu_supported && !entry.gpu_supported {
            return Err(RegistryClosureError::InvalidStatus(entry.identity.clone()));
        }
    }
    if matches!(
        entry.status,
        OperationClassification::IntentionallyUnsupportedBlocking
    ) && entry.issue_sequence_id == 0
    {
        return Err(RegistryClosureError::MissingIssue(entry.identity.clone()));
    }
    if !matches!(entry.status, OperationClassification::Implemented)
        && entry
            .reason
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
    {
        return Err(RegistryClosureError::MissingReason(entry.identity.clone()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin_registry;

    #[test]
    fn registry_closure_is_sorted_and_hashable() {
        let closure = RegistryClosure::from_registry(builtin_registry()).expect("closure");
        assert_eq!(closure.entries.len(), 26);
        assert!(
            closure
                .entries
                .windows(2)
                .all(|entries| entries[0].identity < entries[1].identity)
        );
        assert_eq!(closure.identity_hash().len(), 64);
        assert_eq!(
            closure.canonical_bytes().expect("canonical"),
            closure.canonical_bytes().expect("canonical")
        );
    }

    #[test]
    fn closure_rejects_duplicate_and_unclassified_entries() {
        let base = RegistryClosure::from_registry(builtin_registry()).expect("closure");
        let mut entries = base.entries.clone();
        entries.push(entries[0].clone());
        assert!(matches!(
            RegistryClosure::new(entries),
            Err(RegistryClosureError::DuplicateIdentity(_))
        ));
        let mut entry = base.entries[0].clone();
        entry.status = OperationClassification::ReferenceOnlyNonProduct;
        entry.reason = None;
        assert!(matches!(
            RegistryClosure::new(vec![entry]),
            Err(RegistryClosureError::MissingReason(_))
        ));
    }
}
