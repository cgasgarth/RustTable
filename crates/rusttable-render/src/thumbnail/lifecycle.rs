#![allow(clippy::missing_errors_doc)]

use std::collections::HashSet;
use std::sync::Arc;

use rusttable_core::ContentHash;
use rusttable_image::DecodedImage;
use rusttable_image::common::cache::{
    CacheLease as CommonCacheLease, CacheMode as CommonCacheMode,
};

use crate::thumbnail::cache::{ResidentCacheEntry, ResidentCacheSlot};
use crate::{CacheError, CacheStore, CacheTime, ThumbnailKey, ThumbnailMemoryCache};

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

impl CacheChangeEvent {
    pub(crate) fn affects(self, key: ThumbnailKey) -> bool {
        match self {
            Self::SourceChanged { content } => key.source_content() == content,
            Self::EditChanged {
                photo_id,
                edit_id,
                edit_revision,
            } => {
                key.photo_id() == photo_id
                    && key.edit_id() == edit_id
                    && key.edit_revision() == edit_revision
            }
            Self::ProfileChanged { identity, version } => {
                key.profile_identity() == identity && key.profile_version() == version
            }
            Self::DecoderChanged { version } => key.decoder_version() == version,
            Self::RendererChanged { version } => key.renderer_version() == version,
            Self::ConfigurationChanged { identity } => key.configuration_identity() == identity,
        }
    }
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
    memory: Option<ThumbnailMemoryCache>,
    generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheResolutionSource {
    Memory,
    Durable,
    Produced,
}

#[derive(Debug, Clone)]
pub struct CacheResolution {
    image: Arc<DecodedImage>,
    created_at: CacheTime,
    source: CacheResolutionSource,
}

impl CacheResolution {
    #[must_use]
    pub fn image(&self) -> &DecodedImage {
        &self.image
    }

    #[must_use]
    pub const fn created_at(&self) -> CacheTime {
        self.created_at
    }

    #[must_use]
    pub const fn source(&self) -> CacheResolutionSource {
        self.source
    }
}

#[derive(Debug)]
pub enum CacheResolutionError<E> {
    Cache(CacheError),
    Producer(E),
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
            memory: None,
            generation: 0,
        }
    }

    #[must_use]
    pub fn with_memory(store: CacheStore, memory: ThumbnailMemoryCache) -> Self {
        Self {
            store,
            memory: Some(memory),
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

    pub fn resolve_with<E>(
        &mut self,
        key: ThumbnailKey,
        now: CacheTime,
        producer: impl FnOnce() -> Result<DecodedImage, E>,
    ) -> Result<CacheResolution, CacheResolutionError<E>> {
        let Some(memory) = self.memory.clone() else {
            return self.resolve_without_memory(key, now, producer);
        };
        let resolution_guard = memory
            .begin_resolution(key)
            .map_err(CacheResolutionError::Cache)?;
        let mut producer = Some(producer);
        let mut requested_mode = CommonCacheMode::Read;
        loop {
            let lease = memory
                .acquire(key, requested_mode)
                .map_err(CacheResolutionError::Cache)?;
            match lease {
                CommonCacheLease::Read(read) => {
                    let resident = read
                        .with_value(|slot| match slot {
                            ResidentCacheSlot::Ready(entry) => Some(entry.clone()),
                            ResidentCacheSlot::Vacant => None,
                        })
                        .map_err(|_| CacheResolutionError::Cache(CacheError::Memory))?;
                    drop(read);
                    if let Some(resident) = resident {
                        return resolution_guard
                            .commit(|| {
                                self.store.touch(key, now);
                                Ok(resolution(&resident, CacheResolutionSource::Memory))
                            })
                            .map_err(CacheResolutionError::Cache);
                    }
                    requested_mode = CommonCacheMode::Write;
                }
                CommonCacheLease::Write(write) => {
                    let resident = write
                        .with_value(|slot| match slot {
                            ResidentCacheSlot::Ready(entry) => Some(entry.clone()),
                            ResidentCacheSlot::Vacant => None,
                        })
                        .map_err(|_| CacheResolutionError::Cache(CacheError::Memory))?;
                    if let Some(resident) = resident {
                        return resolution_guard
                            .commit(|| {
                                let read = write.demote().map_err(|_| CacheError::Memory)?;
                                drop(read);
                                self.store.touch(key, now);
                                Ok(resolution(&resident, CacheResolutionSource::Memory))
                            })
                            .map_err(CacheResolutionError::Cache);
                    }

                    let (resident, source) = if let Some(entry) = self
                        .store
                        .get(key, now)
                        .map_err(CacheResolutionError::Cache)?
                    {
                        let created_at = entry.created_at();
                        (
                            ResidentCacheEntry::new(entry.into_image(), created_at),
                            CacheResolutionSource::Durable,
                        )
                    } else {
                        let Some(produce) = producer.take() else {
                            return Err(CacheResolutionError::Cache(CacheError::Memory));
                        };
                        let image = produce().map_err(CacheResolutionError::Producer)?;
                        (
                            ResidentCacheEntry::new(image, now),
                            CacheResolutionSource::Produced,
                        )
                    };
                    resolution_guard
                        .commit(|| {
                            if source == CacheResolutionSource::Produced {
                                let image = resident.image();
                                self.store.put(key, &image, now)?;
                            }
                            write
                                .with_value_mut(|slot| {
                                    *slot = ResidentCacheSlot::Ready(resident.clone());
                                })
                                .map_err(|_| CacheError::Memory)
                        })
                        .map_err(CacheResolutionError::Cache)?;
                    let read = write
                        .demote()
                        .map_err(|_| CacheResolutionError::Cache(CacheError::Memory))?;
                    drop(read);
                    return Ok(resolution(&resident, source));
                }
            }
        }
    }

    fn resolve_without_memory<E>(
        &mut self,
        key: ThumbnailKey,
        now: CacheTime,
        producer: impl FnOnce() -> Result<DecodedImage, E>,
    ) -> Result<CacheResolution, CacheResolutionError<E>> {
        if let Some(entry) = self
            .store
            .get(key, now)
            .map_err(CacheResolutionError::Cache)?
        {
            let created_at = entry.created_at();
            return Ok(CacheResolution {
                image: Arc::new(entry.into_image()),
                created_at,
                source: CacheResolutionSource::Durable,
            });
        }
        let image = producer().map_err(CacheResolutionError::Producer)?;
        self.store
            .put(key, &image, now)
            .map_err(CacheResolutionError::Cache)?;
        Ok(CacheResolution {
            image: Arc::new(image),
            created_at: now,
            source: CacheResolutionSource::Produced,
        })
    }

    pub fn apply(
        &mut self,
        event: CacheChangeEvent,
    ) -> Result<CacheInvalidationReport, CacheLifecycleError> {
        let next_generation = self
            .generation
            .checked_add(1)
            .ok_or(CacheLifecycleError::GenerationOverflow)?;
        let memory = self.memory.clone();
        let memory_invalidation = memory
            .as_ref()
            .map(|memory| memory.begin_invalidation(event))
            .transpose()?;
        self.generation = next_generation;
        let mut keys = self.store.keys().collect::<HashSet<_>>();
        if let Some(memory) = &memory {
            keys.extend(memory.keys()?);
        }
        if let Some(invalidation) = &memory_invalidation {
            keys.extend(invalidation.active_keys());
        }
        let mut report = CacheInvalidationReport::default();
        for key in keys {
            if event.affects(key) {
                report.matched += 1;
                match self.store.invalidate(key) {
                    Ok(durable_removed) => {
                        let memory_removed = match &memory {
                            Some(memory) => memory.remove(&key)?,
                            None => false,
                        };
                        if durable_removed || memory_removed {
                            report.removed += 1;
                        }
                    }
                    Err(CacheError::Protected) => report.protected += 1,
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(report)
    }
}

fn resolution(resident: &ResidentCacheEntry, source: CacheResolutionSource) -> CacheResolution {
    CacheResolution {
        image: resident.image(),
        created_at: resident.created_at(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use rusttable_core::{AssetId, EditId, PhotoId, Revision};
    use rusttable_image::{ColorEncoding, ImageDimensions, Orientation};

    use super::*;
    use crate::{CacheLimits, MipmapLevel, ThumbnailProvenance, ThumbnailRequest, ThumbnailSize};

    #[test]
    fn concurrent_invalidation_rejects_an_in_flight_publication() {
        let directory = temporary_directory("concurrent-invalidation");
        let cache_key = thumbnail_key();
        let memory = ThumbnailMemoryCache::new(64 * 1024);
        let (producer_store, _) = CacheStore::open(&directory, cache_limits()).expect("open first");
        let (invalidator_store, _) =
            CacheStore::open(&directory, cache_limits()).expect("open second");
        let (producer_started_tx, producer_started_rx) = mpsc::channel();
        let (release_producer_tx, release_producer_rx) = mpsc::channel();

        let producer_memory = memory.clone();
        let producer = thread::spawn(move || {
            let mut lifecycle = CacheLifecycle::with_memory(producer_store, producer_memory);
            let result = lifecycle.resolve_with(cache_key, CacheTime::from_seconds(20), || {
                producer_started_tx.send(()).expect("signal producer");
                release_producer_rx.recv().expect("release producer");
                Ok::<_, ()>(test_image(1))
            });
            (lifecycle, result)
        });
        producer_started_rx.recv().expect("producer started");

        let invalidator_memory = memory.clone();
        let invalidator = thread::spawn(move || {
            let mut lifecycle = CacheLifecycle::with_memory(invalidator_store, invalidator_memory);
            let report = lifecycle.apply(CacheChangeEvent::EditChanged {
                photo_id: cache_key.photo_id(),
                edit_id: cache_key.edit_id(),
                edit_revision: cache_key.edit_revision(),
            });
            (lifecycle, report)
        });

        let deadline = Instant::now() + Duration::from_secs(5);
        while !memory.is_invalidating() {
            assert!(
                Instant::now() < deadline,
                "invalidation did not reach the resident-cache barrier"
            );
            thread::yield_now();
        }
        release_producer_tx.send(()).expect("release producer");

        let (mut producer_lifecycle, result) = producer.join().expect("producer thread");
        assert!(matches!(
            result,
            Err(CacheResolutionError::Cache(CacheError::Invalidated))
        ));
        let (_, report) = invalidator.join().expect("invalidator thread");
        assert_eq!(
            report.expect("invalidation"),
            CacheInvalidationReport {
                matched: 1,
                removed: 1,
                protected: 0,
            }
        );

        let (reopened, reconciliation) =
            CacheStore::open(&directory, cache_limits()).expect("reopen");
        assert_eq!(reconciliation.valid_entries, 0);
        assert!(reopened.is_empty());
        drop(reopened);

        let rebuilt = producer_lifecycle
            .resolve_with(cache_key, CacheTime::from_seconds(21), || {
                Ok::<_, ()>(test_image(2))
            })
            .expect("rebuild after invalidation");
        assert_eq!(rebuilt.source(), CacheResolutionSource::Produced);
        assert_eq!(rebuilt.image().pixels(), test_image(2).pixels());
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn concurrent_invalidation_rejects_a_ready_memory_hit_before_return() {
        let directory = temporary_directory("concurrent-ready-invalidation");
        let cache_key = thumbnail_key();
        let memory = ThumbnailMemoryCache::new(64 * 1024);
        let (ready_store, _) = CacheStore::open(&directory, cache_limits()).expect("open first");
        let (invalidator_store, _) =
            CacheStore::open(&directory, cache_limits()).expect("open second");
        let mut ready_lifecycle = CacheLifecycle::with_memory(ready_store, memory.clone());
        assert_eq!(
            ready_lifecycle
                .resolve_with(cache_key, CacheTime::from_seconds(20), || {
                    Ok::<_, ()>(test_image(1))
                })
                .expect("populate")
                .source(),
            CacheResolutionSource::Produced
        );

        let (commit_reached, resume_commit) = memory.pause_next_commit();
        let ready = thread::spawn(move || {
            ready_lifecycle.resolve_with(
                cache_key,
                CacheTime::from_seconds(21),
                || -> Result<_, ()> { panic!("ready hit must not produce") },
            )
        });
        commit_reached.recv().expect("ready commit reached");

        let mut invalidator_lifecycle =
            CacheLifecycle::with_memory(invalidator_store, memory.clone());
        let report = invalidator_lifecycle
            .apply(CacheChangeEvent::EditChanged {
                photo_id: cache_key.photo_id(),
                edit_id: cache_key.edit_id(),
                edit_revision: cache_key.edit_revision(),
            })
            .expect("invalidation");
        assert_eq!(
            report,
            CacheInvalidationReport {
                matched: 1,
                removed: 1,
                protected: 0,
            }
        );

        resume_commit.send(()).expect("resume ready commit");
        assert!(matches!(
            ready.join().expect("ready thread"),
            Err(CacheResolutionError::Cache(CacheError::Invalidated))
        ));
        let (reopened, reconciliation) =
            CacheStore::open(&directory, cache_limits()).expect("reopen");
        assert_eq!(reconciliation.valid_entries, 0);
        assert!(reopened.is_empty());
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn invalidating_one_key_does_not_reject_an_unrelated_in_flight_fill() {
        let directory = temporary_directory("unrelated-concurrent-invalidation");
        let affected = thumbnail_key_with_revision(5);
        let unrelated = thumbnail_key_with_revision(6);
        let memory = ThumbnailMemoryCache::new(128 * 1024);
        let (producer_store, _) = CacheStore::open(&directory, cache_limits()).expect("open first");
        let (invalidator_store, _) =
            CacheStore::open(&directory, cache_limits()).expect("open second");
        let mut producer_lifecycle = CacheLifecycle::with_memory(producer_store, memory.clone());
        producer_lifecycle
            .resolve_with(affected, CacheTime::from_seconds(20), || {
                Ok::<_, ()>(test_image(1))
            })
            .expect("populate affected");
        let held_read = memory
            .acquire(affected, CommonCacheMode::Read)
            .expect("hold affected resident");
        assert!(matches!(held_read, CommonCacheLease::Read(_)));

        let (commit_reached, resume_commit) = memory.pause_next_commit();
        let producer = thread::spawn(move || {
            producer_lifecycle.resolve_with(unrelated, CacheTime::from_seconds(21), || {
                Ok::<_, ()>(test_image(2))
            })
        });
        commit_reached.recv().expect("unrelated commit reached");

        let invalidator_memory = memory.clone();
        let invalidator = thread::spawn(move || {
            let mut lifecycle = CacheLifecycle::with_memory(invalidator_store, invalidator_memory);
            lifecycle.apply(CacheChangeEvent::EditChanged {
                photo_id: affected.photo_id(),
                edit_id: affected.edit_id(),
                edit_revision: affected.edit_revision(),
            })
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while !memory.is_invalidating() {
            assert!(
                Instant::now() < deadline,
                "invalidation did not reach the resident-cache barrier"
            );
            thread::yield_now();
        }

        resume_commit.send(()).expect("resume unrelated commit");
        assert_eq!(
            producer
                .join()
                .expect("producer thread")
                .expect("unrelated production")
                .source(),
            CacheResolutionSource::Produced
        );
        drop(held_read);
        assert_eq!(
            invalidator
                .join()
                .expect("invalidator thread")
                .expect("invalidation"),
            CacheInvalidationReport {
                matched: 1,
                removed: 1,
                protected: 0,
            }
        );
        fs::remove_dir_all(directory).expect("cleanup");
    }

    fn thumbnail_key() -> ThumbnailKey {
        thumbnail_key_with_revision(5)
    }

    fn thumbnail_key_with_revision(edit_revision: u64) -> ThumbnailKey {
        let request = ThumbnailRequest::new(
            MipmapLevel::zero(),
            ThumbnailSize::fit(128, 96).expect("size"),
        )
        .with_orientation(Orientation::Rotate90)
        .with_provenance(ThumbnailProvenance::PipelineRender);
        ThumbnailKey::new(
            ContentHash::Sha256([7; 32]),
            PhotoId::new(1).expect("photo"),
            AssetId::new(2).expect("asset"),
            EditId::new(3).expect("edit"),
            Revision::from_u64(4),
            Revision::from_u64(edit_revision),
            [15; 32],
            10,
            11,
            [12; 32],
            13,
            [14; 32],
            request,
        )
    }

    fn test_image(value: u8) -> DecodedImage {
        DecodedImage::new_with_color_encoding(
            ImageDimensions::new(2, 2).expect("dimensions"),
            vec![value; 16],
            ColorEncoding::Srgb,
        )
        .expect("image")
    }

    fn cache_limits() -> CacheLimits {
        CacheLimits::new(16 * 1024, 8 * 1024).expect("limits")
    }

    fn temporary_directory(label: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rusttable-{label}-{}-{:?}",
            std::process::id(),
            thread::current().id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("temporary directory");
        path
    }
}
