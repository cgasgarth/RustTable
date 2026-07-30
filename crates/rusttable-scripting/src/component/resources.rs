use std::collections::BTreeMap;

use super::errors::{ErrorCode, ScriptError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub enum ResourceKind {
    CatalogSnapshot,
    SelectionSnapshot,
    Storage,
    ExportProposal,
    Notification,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct ResourceHandle {
    pub id: u32,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy)]
struct ResourceEntry {
    kind: ResourceKind,
    parent: Option<ResourceHandle>,
    generation: u64,
}

#[derive(Debug, Default)]
pub struct ResourceRegistry {
    next: u32,
    generation: u64,
    entries: BTreeMap<ResourceHandle, ResourceEntry>,
    limit: usize,
}

impl ResourceRegistry {
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self {
            next: 1,
            generation: 1,
            entries: BTreeMap::new(),
            limit,
        }
    }

    /// Allocates a typed handle with an optional live parent.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when the quota is exhausted, the parent is stale, or the id space is exhausted.
    pub fn insert(
        &mut self,
        kind: ResourceKind,
        parent: Option<ResourceHandle>,
    ) -> Result<ResourceHandle, ScriptError> {
        if self.entries.len() >= self.limit {
            return Err(ScriptError::new(
                ErrorCode::LimitExceeded,
                "resource handle limit exceeded",
            ));
        }
        if let Some(parent) = parent {
            self.get(parent)?;
        }
        let handle = ResourceHandle {
            id: self.next,
            generation: self.generation,
        };
        self.next = self.next.checked_add(1).ok_or_else(|| {
            ScriptError::new(ErrorCode::LimitExceeded, "resource handle id exhausted")
        })?;
        self.entries.insert(
            handle,
            ResourceEntry {
                kind,
                parent,
                generation: self.generation,
            },
        );
        Ok(handle)
    }

    /// Resolves a handle while checking generation and parent lifetime.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when the handle or one of its parents is stale.
    pub fn get(&self, handle: ResourceHandle) -> Result<ResourceKind, ScriptError> {
        let entry = self.entries.get(&handle).ok_or_else(|| {
            ScriptError::new(
                ErrorCode::InvalidHandle,
                "resource handle is stale or unknown",
            )
        })?;
        if entry.generation != self.generation
            || entry
                .parent
                .is_some_and(|parent| !self.entries.contains_key(&parent))
        {
            return Err(ScriptError::new(
                ErrorCode::InvalidHandle,
                "resource parent is stale",
            ));
        }
        Ok(entry.kind)
    }

    /// Drops a handle after all child handles have been released.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when the handle is stale or still owns children.
    pub fn drop_handle(&mut self, handle: ResourceHandle) -> Result<(), ScriptError> {
        self.get(handle)?;
        if self
            .entries
            .values()
            .any(|entry| entry.parent == Some(handle))
        {
            return Err(ScriptError::new(
                ErrorCode::InvalidHandle,
                "resource has live children",
            ));
        }
        self.entries.remove(&handle);
        Ok(())
    }

    pub fn reset(&mut self) {
        self.entries.clear();
        self.generation = self.generation.saturating_add(1);
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
