#![allow(clippy::missing_errors_doc)]

use rusttable_core::ContentHash;

use crate::{CacheError, CacheStore, ThumbnailKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheChangeEvent {
    SourceChanged {
        content: ContentHash,
    },
    EditChanged {
        photo_id: rusttable_core::PhotoId,
        edit_id: rusttable_core::EditId,
        edit_revision: rusttable_core::Revision,
    },
    ProfileChanged {
        identity: [u8; 32],
        version: u32,
    },
    DecoderChanged {
        version: u32,
    },
    RendererChanged {
        version: u32,
    },
    ConfigurationChanged {
        identity: [u8; 32],
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheInvalidationReport {
    pub matched: usize,
    pub removed: usize,
    pub protected: usize,
}

#[derive(Debug)]
pub struct CacheLifecycle {
    store: CacheStore,
    generation: u64,
}

#[derive(Debug)]
pub enum CacheLifecycleError {
    Cache(CacheError),
    GenerationOverflow,
}

impl From<CacheError> for CacheLifecycleError {
    fn from(error: CacheError) -> Self {
        Self::Cache(error)
    }
}

impl CacheLifecycle {
    #[must_use]
    pub const fn new(store: CacheStore) -> Self {
        Self {
            store,
            generation: 0,
        }
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn store(&self) -> &CacheStore {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut CacheStore {
        &mut self.store
    }

    pub fn apply(
        &mut self,
        event: CacheChangeEvent,
    ) -> Result<CacheInvalidationReport, CacheLifecycleError> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(CacheLifecycleError::GenerationOverflow)?;
        let keys = self.store.keys().collect::<Vec<_>>();
        let mut report = CacheInvalidationReport::default();
        for key in keys {
            if affected(key, event) {
                report.matched += 1;
                match self.store.invalidate(key) {
                    Ok(true) => report.removed += 1,
                    Ok(false) => {}
                    Err(CacheError::Protected) => report.protected += 1,
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(report)
    }
}

fn affected(key: ThumbnailKey, event: CacheChangeEvent) -> bool {
    match event {
        CacheChangeEvent::SourceChanged { content } => key.source_content() == content,
        CacheChangeEvent::EditChanged {
            photo_id,
            edit_id,
            edit_revision,
        } => {
            key.photo_id() == photo_id
                && key.edit_id() == edit_id
                && key.edit_revision() == edit_revision
        }
        CacheChangeEvent::ProfileChanged { identity, version } => {
            key.profile_identity() == identity && key.profile_version() == version
        }
        CacheChangeEvent::DecoderChanged { version } => key.decoder_version() == version,
        CacheChangeEvent::RendererChanged { version } => key.renderer_version() == version,
        CacheChangeEvent::ConfigurationChanged { identity } => {
            key.configuration_identity() == identity
        }
    }
}
