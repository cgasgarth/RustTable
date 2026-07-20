//! Background lighttable thumbnail projection over the bounded render cache.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusttable_catalog::{ImportRecord, ImportRepository};
use rusttable_catalog_store::RedbImportRepository;
use rusttable_core::{EditId, Revision};
use rusttable_image::{CancellationToken, DecodeLimits, ImageInput, Orientation};
use rusttable_image_io::FileImageInput;
use rusttable_import::{
    FileSourceSnapshotReader, ImportSourceLimits, SourceSnapshotReader, decode_reference_source,
};
use rusttable_render::{
    CacheLifecycle, CacheLimits, CacheStore, CacheTime, MipmapLevel, PrefetchPriority,
    PrefetchRequest, PrefetchScheduler, ThumbnailGenerator, ThumbnailKey, ThumbnailProvenance,
    ThumbnailRequest, ThumbnailSize,
};
use rusttable_ui::{PhotoThumbnailViewModel, PhotoWorkspaceViewModel, PreviewDimensions};

const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DECODED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_THUMBNAIL_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CACHE_BYTES: u64 = 512 * 1024 * 1024;
const THUMBNAIL_WIDTH: u32 = 256;
const THUMBNAIL_HEIGHT: u32 = 170;

#[derive(Debug, Clone)]
pub struct ThumbnailLoadRequest {
    catalog_path: PathBuf,
    source_root: PathBuf,
    workspace: PhotoWorkspaceViewModel,
}

impl ThumbnailLoadRequest {
    #[must_use]
    pub fn new(
        catalog_path: impl Into<PathBuf>,
        source_root: impl Into<PathBuf>,
        workspace: PhotoWorkspaceViewModel,
    ) -> Self {
        Self {
            catalog_path: catalog_path.into(),
            source_root: source_root.into(),
            workspace,
        }
    }
}

/// Loads cached thumbnails or generates them from immutable source snapshots.
///
/// This function performs file I/O and image work and must run away from GTK's main context.
#[must_use]
pub fn load_lighttable_thumbnails(request: ThumbnailLoadRequest) -> PhotoWorkspaceViewModel {
    load_lighttable_thumbnails_checked(&request).unwrap_or(request.workspace)
}

fn load_lighttable_thumbnails_checked(
    request: &ThumbnailLoadRequest,
) -> Result<PhotoWorkspaceViewModel, ThumbnailLoadError> {
    let repository = RedbImportRepository::open(&request.catalog_path)
        .map_err(|_| ThumbnailLoadError::Catalog)?;
    let records = repository.list().map_err(|_| ThumbnailLoadError::Catalog)?;
    let records = records
        .into_iter()
        .map(|record| (record.photo().id(), record))
        .collect::<BTreeMap<_, _>>();
    let cache_root = request
        .catalog_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("thumbnails-v1");
    let limits = CacheLimits::new(MAX_CACHE_BYTES, MAX_THUMBNAIL_BYTES)
        .map_err(|_| ThumbnailLoadError::Cache)?;
    let (store, _) = CacheStore::open(cache_root, limits).map_err(|_| ThumbnailLoadError::Cache)?;
    let mut lifecycle = CacheLifecycle::new(store);
    let mut scheduler = PrefetchScheduler::new(records.len().max(1), 2)
        .map_err(|_| ThumbnailLoadError::Schedule)?;
    let request_shape = ThumbnailRequest::new(
        MipmapLevel::zero(),
        ThumbnailSize::fit(THUMBNAIL_WIDTH, THUMBNAIL_HEIGHT)
            .map_err(|_| ThumbnailLoadError::Schedule)?,
    )
    .with_orientation(Orientation::Normal)
    .with_provenance(ThumbnailProvenance::RawFallback);
    for record in records.values() {
        let key = thumbnail_key(record, request_shape);
        let _ = scheduler.submit(PrefetchRequest {
            key,
            priority: PrefetchPriority::Visible,
        });
    }

    let mut workspace = request.workspace.clone();
    while let Some(job) = scheduler.next() {
        let key = job.request().key;
        let result = records.get(&key.photo_id()).and_then(|record| {
            thumbnail_for_record(record, &request.source_root, key, &mut lifecycle).ok()
        });
        if scheduler.complete(job) != rusttable_render::PrefetchCompletion::Publish {
            continue;
        }
        if let Some(thumbnail) = result
            && let Some(updated) = workspace.clone().with_thumbnail(key.photo_id(), thumbnail)
        {
            workspace = updated;
        }
    }
    Ok(workspace)
}

fn thumbnail_for_record(
    record: &ImportRecord,
    source_root: &Path,
    key: ThumbnailKey,
    lifecycle: &mut CacheLifecycle,
) -> Result<PhotoThumbnailViewModel, ThumbnailLoadError> {
    let now = CacheTime::now().map_err(|_| ThumbnailLoadError::Cache)?;
    if let Some(entry) = lifecycle
        .store_mut()
        .get(key, now)
        .map_err(|_| ThumbnailLoadError::Cache)?
    {
        return thumbnail_view_model(entry.image());
    }
    let source = decode_reference_source(record.source())
        .unwrap_or_else(|_| source_root.join(record.source().as_str()));
    let source_limits =
        ImportSourceLimits::new(MAX_SOURCE_BYTES).map_err(|_| ThumbnailLoadError::Source)?;
    let reader = FileSourceSnapshotReader;
    let snapshot = reader
        .read_snapshot(&source, source_limits)
        .map_err(|_| ThumbnailLoadError::Source)?;
    let bytes = snapshot
        .materialize(source_limits)
        .map_err(|_| ThumbnailLoadError::Source)?;
    let decoded = FileImageInput::new(decode_limits())
        .decode_bytes(&bytes)
        .map_err(|_| ThumbnailLoadError::Decode)?;
    let image = ThumbnailGenerator::generate(
        &decoded,
        key.request(),
        MAX_THUMBNAIL_BYTES,
        &CancellationToken::new(),
    )
    .map_err(|_| ThumbnailLoadError::Generate)?;
    reader
        .revalidate(&snapshot, source_limits)
        .map_err(|_| ThumbnailLoadError::Source)?;
    lifecycle
        .store_mut()
        .put(key, &image, now)
        .map_err(|_| ThumbnailLoadError::Cache)?;
    thumbnail_view_model(&image)
}

fn thumbnail_key(record: &ImportRecord, request: ThumbnailRequest) -> ThumbnailKey {
    let photo = record.photo();
    let asset = photo.primary_asset();
    let edit_id = EditId::new(photo.id().get()).expect("photo identifiers are nonzero");
    ThumbnailKey::new(
        asset.content_hash(),
        photo.id(),
        asset.id(),
        edit_id,
        photo.revision(),
        Revision::ZERO,
        1,
        1,
        [0; 32],
        1,
        [0; 32],
        request,
    )
}

fn thumbnail_view_model(
    image: &rusttable_image::DecodedImage,
) -> Result<PhotoThumbnailViewModel, ThumbnailLoadError> {
    let dimensions =
        PreviewDimensions::new(image.dimensions().width(), image.dimensions().height())
            .map_err(|_| ThumbnailLoadError::Presentation)?;
    PhotoThumbnailViewModel::new(dimensions, image.pixels().to_vec())
        .map_err(|_| ThumbnailLoadError::Presentation)
}

fn decode_limits() -> DecodeLimits {
    DecodeLimits::new(
        MAX_SOURCE_BYTES,
        32_768,
        32_768,
        MAX_DECODED_BYTES / 4,
        MAX_DECODED_BYTES,
    )
    .expect("constant thumbnail decode limits are consistent")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThumbnailLoadError {
    Catalog,
    Cache,
    Schedule,
    Source,
    Decode,
    Generate,
    Presentation,
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use rusttable_catalog::ImportCandidate;
    use rusttable_core::{
        Asset, AssetId, AssetRole, ByteLength, ContentHash, ImageMetadata, Photo,
    };
    use rusttable_image::{ColorEncoding, DecodedImage, ImageDimensions};
    use rusttable_import::encode_reference_source;

    use super::*;

    #[test]
    fn thumbnail_projection_preserves_generated_dimensions_and_pixels() {
        let dimensions = ImageDimensions::new(2, 1).expect("dimensions");
        let image = DecodedImage::new_with_color_encoding(
            dimensions,
            vec![1, 2, 3, 255, 4, 5, 6, 255],
            ColorEncoding::Srgb,
        )
        .expect("image");
        let thumbnail = thumbnail_view_model(&image).expect("presentation");
        assert_eq!(thumbnail.dimensions().width(), 2);
        assert_eq!(thumbnail.dimensions().height(), 1);
        assert_eq!(thumbnail.pixels(), image.pixels());
    }

    #[test]
    fn decode_and_cache_envelopes_are_bounded() {
        let limits = decode_limits();
        assert_eq!(limits.max_source_bytes(), MAX_SOURCE_BYTES);
        assert_eq!(limits.max_decoded_bytes(), MAX_DECODED_BYTES);
        assert!(limits.max_decoded_bytes() > MAX_THUMBNAIL_BYTES);
    }

    #[test]
    fn immutable_source_thumbnail_is_generated_then_reused_from_cache() {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "rusttable-thumbnail-loader-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).expect("temp root");
        let source = fs::canonicalize(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join("fixtures/corpus/assets/raster-png-16-alpha.png"),
        )
        .expect("canonical fixture");
        let source_bytes = fs::read(&source).expect("fixture");
        let photo_id = rusttable_core::PhotoId::new(7).expect("photo ID");
        let asset_id = AssetId::new(9).expect("asset ID");
        let source_path = encode_reference_source(&source, [7; 32]).expect("reference source");
        let candidate = ImportCandidate::new(
            photo_id,
            asset_id,
            source_path,
            ContentHash::Sha256([7; 32]),
            ByteLength::from_bytes(u64::try_from(source_bytes.len()).expect("fixture length")),
            FileImageInput::new(decode_limits())
                .probe_bytes(&source_bytes)
                .expect("probe"),
            ImageMetadata::empty(),
        )
        .expect("candidate");
        let photo = Photo::new(
            photo_id,
            [Asset::new(
                asset_id,
                AssetRole::Primary,
                candidate.content_hash(),
                candidate.byte_length(),
            )],
        )
        .expect("photo");
        let record = ImportRecord::new(&candidate, photo).expect("record");
        let limits = CacheLimits::new(MAX_CACHE_BYTES, MAX_THUMBNAIL_BYTES).expect("limits");
        let (store, _) = CacheStore::open(root.join("cache"), limits).expect("cache");
        let mut lifecycle = CacheLifecycle::new(store);
        let request = ThumbnailRequest::new(
            MipmapLevel::zero(),
            ThumbnailSize::fit(THUMBNAIL_WIDTH, THUMBNAIL_HEIGHT).expect("size"),
        )
        .with_provenance(ThumbnailProvenance::RawFallback);
        let key = thumbnail_key(&record, request);

        let first = thumbnail_for_record(&record, &root, key, &mut lifecycle).expect("generated");
        let second = thumbnail_for_record(&record, &root, key, &mut lifecycle).expect("cached");
        assert_eq!(first, second);
        assert_eq!(lifecycle.store().len(), 1);
        assert!(!first.pixels().is_empty());
        let _ = fs::remove_dir_all(root);
    }
}
