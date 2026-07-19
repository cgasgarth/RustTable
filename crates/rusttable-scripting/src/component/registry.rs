use std::collections::BTreeMap;

use super::{
    api::{ExtensionId, ExtensionManifest},
    errors::{ErrorCode, ScriptError},
    receipt::digest,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PackageProvenance {
    pub component_hash: String,
    pub signature: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ExtensionPackage {
    pub manifest: ExtensionManifest,
    pub component: Vec<u8>,
    pub provenance: PackageProvenance,
}

impl ExtensionPackage {
    /// Creates a package and records its content hash before host validation.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when the manifest or component byte bound is invalid.
    pub fn new(manifest: ExtensionManifest, component: Vec<u8>) -> Result<Self, ScriptError> {
        manifest.validate()?;
        if component.is_empty() || component.len() > 64 * 1024 * 1024 {
            return Err(ScriptError::new(
                ErrorCode::MalformedComponent,
                "component bytes are outside the package bound",
            ));
        }
        Ok(Self {
            provenance: PackageProvenance {
                component_hash: digest(&component),
                signature: None,
                source: "local-package".to_owned(),
            },
            manifest,
            component,
        })
    }
}

#[derive(Debug, Default)]
pub struct ExtensionRegistry {
    packages: BTreeMap<ExtensionId, ExtensionPackage>,
}

impl ExtensionRegistry {
    /// Installs a package exactly once by extension identifier.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when the identifier is already installed.
    pub fn install(&mut self, package: ExtensionPackage) -> Result<(), ScriptError> {
        let id = package.manifest.id.clone();
        if self.packages.insert(id, package).is_some() {
            return Err(ScriptError::new(
                ErrorCode::InvalidManifest,
                "extension is already installed",
            ));
        }
        Ok(())
    }

    /// Replaces a package for transactional reload.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when the replacement manifest is invalid.
    pub fn replace(&mut self, package: ExtensionPackage) -> Result<(), ScriptError> {
        package.manifest.validate()?;
        self.packages.insert(package.manifest.id.clone(), package);
        Ok(())
    }

    /// Gets an installed package.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when the identifier is unknown.
    pub fn get(&self, id: &ExtensionId) -> Result<&ExtensionPackage, ScriptError> {
        self.packages
            .get(id)
            .ok_or_else(|| ScriptError::new(ErrorCode::NotFound, "extension is not registered"))
    }

    #[must_use]
    pub fn contains(&self, id: &ExtensionId) -> bool {
        self.packages.contains_key(id)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.packages.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }
}
