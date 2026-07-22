//! Background thumbnail generation over the immutable render and cache contracts.

use std::collections::BTreeMap;
use std::path::PathBuf;

use directories::ProjectDirs;
use rusttable_catalog::{EditRepository, ImportRepository};
use rusttable_catalog_store::RedbCatalogRepository;
use rusttable_core::{Edit, EditId, PhotoId, Revision};
use rusttable_image::{CancellationToken, ColorEncoding, DecodedImage};
use rusttable_import::RASTER_DECODER_IDENTITY_VERSION;
use rusttable_render::{
    CacheLifecycle, CacheLimits, CacheStore, CacheTime, MipmapLevel, ThumbnailGenerator,
    ThumbnailKey, ThumbnailRequest, ThumbnailSize,
};
use rusttable_ui::{PresentationText, PreviewDimensions, Rgba8PreviewMetadata};
use sha2::{Digest, Sha256};

use crate::PreviewOutputTransform;

const THUMBNAIL_WIDTH: u32 = 180;
const THUMBNAIL_HEIGHT: u32 = 120;
const MAX_THUMBNAIL_BYTES: u64 = 2 * 1024 * 1024;
const CACHE_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
const RENDERER_VERSION: u32 = 1;
const PROFILE_VERSION: u32 = 2;

/// Whether the visible thumbnail came from durable cache or a fresh bounded render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GtkThumbnailSource {
    Cache,
    Render,
}

/// One display-safe completed thumbnail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GtkThumbnail {
    photo_id: PhotoId,
    metadata: Rgba8PreviewMetadata,
    source: GtkThumbnailSource,
    cache_identity: [u8; 32],
    render_receipt_identity: Option<[u8; 32]>,
    output_transform: PreviewOutputTransform,
    edit_id: EditId,
    edit_revision: Revision,
}

impl GtkThumbnail {
    #[must_use]
    pub const fn photo_id(&self) -> PhotoId {
        self.photo_id
    }

    #[must_use]
    pub const fn metadata(&self) -> &Rgba8PreviewMetadata {
        &self.metadata
    }

    #[must_use]
    pub const fn source(&self) -> GtkThumbnailSource {
        self.source
    }

    #[must_use]
    pub const fn cache_identity(&self) -> [u8; 32] {
        self.cache_identity
    }

    #[must_use]
    pub const fn render_receipt_identity(&self) -> Option<[u8; 32]> {
        self.render_receipt_identity
    }

    #[must_use]
    pub const fn output_transform(&self) -> PreviewOutputTransform {
        self.output_transform
    }

    #[must_use]
    pub const fn edit_id(&self) -> EditId {
        self.edit_id
    }

    #[must_use]
    pub const fn edit_revision(&self) -> Revision {
        self.edit_revision
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GtkThumbnailError {
    Catalog,
    MissingPhoto,
    MissingEdit,
    Preview,
    Cache,
    Render,
    Presentation,
}

/// Serial background worker that shares one bounded cache lifecycle across a visible batch.
pub struct GtkThumbnailController {
    source_root: PathBuf,
    repository: RedbCatalogRepository,
    cache: CacheLifecycle,
    records: BTreeMap<PhotoId, rusttable_catalog::ImportRecord>,
    edits: BTreeMap<PhotoId, Edit>,
}

impl GtkThumbnailController {
    /// Opens catalog identity and cache state before any GTK update is scheduled.
    ///
    /// # Errors
    ///
    /// Returns a closed catalog/cache error when durable inputs cannot be opened or validated.
    pub fn open(
        catalog_path: impl Into<PathBuf>,
        source_root: impl Into<PathBuf>,
        cache_root: impl Into<PathBuf>,
    ) -> Result<Self, GtkThumbnailError> {
        let catalog_path = catalog_path.into();
        let catalog =
            RedbCatalogRepository::open(&catalog_path).map_err(|_| GtkThumbnailError::Catalog)?;
        let records = ImportRepository::list(&catalog)
            .map_err(|_| GtkThumbnailError::Catalog)?
            .into_iter()
            .map(|record| (record.photo().id(), record))
            .collect();
        let edits = EditRepository::list(&catalog)
            .map_err(|_| GtkThumbnailError::Catalog)?
            .into_iter()
            .map(|edit| (edit.photo_id(), edit))
            .collect();
        let limits = CacheLimits::new(CACHE_TOTAL_BYTES, MAX_THUMBNAIL_BYTES)
            .map_err(|_| GtkThumbnailError::Cache)?;
        let (cache, _) =
            CacheStore::open(cache_root, limits).map_err(|_| GtkThumbnailError::Cache)?;
        Ok(Self {
            source_root: source_root.into(),
            repository: catalog,
            cache: CacheLifecycle::new(cache),
            records,
            edits,
        })
    }

    /// Loads or renders one visible thumbnail with no GTK object access.
    ///
    /// # Errors
    ///
    /// Returns a closed error when catalog identity, preview rendering, cache publication, or
    /// presentation validation fails.
    pub fn render(&mut self, photo_id: PhotoId) -> Result<GtkThumbnail, GtkThumbnailError> {
        self.render_with_generation(photo_id, 0)
    }

    /// Loads or renders one visible thumbnail for a checked publication generation.
    ///
    /// # Errors
    ///
    /// Returns a typed catalog, cache, render, or presentation error.
    pub fn render_with_generation(
        &mut self,
        photo_id: PhotoId,
        generation: u64,
    ) -> Result<GtkThumbnail, GtkThumbnailError> {
        let edit = self
            .edits
            .get(&photo_id)
            .ok_or(GtkThumbnailError::MissingEdit)?;
        self.render_for_edit(photo_id, edit.id(), edit.revision(), generation)
    }

    /// Loads or renders one thumbnail for the exact edit identity captured by the preview.
    ///
    /// The worker never reselects a newer edit while it is rendering. If the persisted edit
    /// changed between scheduling and execution, the request fails and the caller keeps the
    /// shared surfaces in their explicit loading state until a new request is scheduled.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the captured photo/edit, cache, render, or presentation cannot
    /// be resolved.
    pub fn render_with_generation_for_edit(
        &mut self,
        photo_id: PhotoId,
        edit_id: EditId,
        edit_revision: Revision,
        generation: u64,
    ) -> Result<GtkThumbnail, GtkThumbnailError> {
        self.render_for_edit(photo_id, edit_id, edit_revision, generation)
    }

    fn render_for_edit(
        &mut self,
        photo_id: PhotoId,
        edit_id: EditId,
        edit_revision: Revision,
        generation: u64,
    ) -> Result<GtkThumbnail, GtkThumbnailError> {
        let record = self
            .records
            .get(&photo_id)
            .ok_or(GtkThumbnailError::MissingPhoto)?;
        let edit = self
            .edits
            .values()
            .find(|edit| {
                edit.photo_id() == photo_id
                    && edit.id() == edit_id
                    && edit.revision() == edit_revision
            })
            .ok_or(GtkThumbnailError::MissingEdit)?;
        let request = thumbnail_request()?;
        let key = thumbnail_key(record, edit, request);
        let now = CacheTime::now().map_err(|_| GtkThumbnailError::Cache)?;
        if let Some(entry) = self
            .cache
            .store_mut()
            .get(key, now)
            .map_err(|_| GtkThumbnailError::Cache)?
        {
            return present(
                photo_id,
                entry.image(),
                GtkThumbnailSource::Cache,
                key.digest(),
                None,
                key.edit_id(),
                key.edit_revision(),
            );
        }

        let preview = crate::workspace::preview_loader::load_selected_preview_from_repository_with_generation(
            &self.repository,
            &self.source_root,
            photo_id,
            generation,
        )
        .map_err(|_| GtkThumbnailError::Preview)?;
        let (_, dimensions, pixels, receipt) = preview.into_render_parts();
        let render_receipt_identity = Some(receipt.identity_hash());
        let source = DecodedImage::new_with_color_encoding(dimensions, pixels, ColorEncoding::Srgb)
            .map_err(|_| GtkThumbnailError::Preview)?;
        let thumbnail = ThumbnailGenerator::generate(
            &source,
            request,
            MAX_THUMBNAIL_BYTES,
            &CancellationToken::new(),
        )
        .map_err(|_| GtkThumbnailError::Render)?;
        self.cache
            .store_mut()
            .put(key, &thumbnail, now)
            .map_err(|_| GtkThumbnailError::Cache)?;
        present(
            photo_id,
            &thumbnail,
            GtkThumbnailSource::Render,
            key.digest(),
            render_receipt_identity,
            key.edit_id(),
            key.edit_revision(),
        )
    }
}

#[must_use]
pub fn default_thumbnail_cache_root() -> PathBuf {
    ProjectDirs::from("com", "cgasgarth", "RustTable").map_or_else(
        || std::env::temp_dir().join("rusttable-thumbnails"),
        |directories| directories.cache_dir().join("thumbnails"),
    )
}

fn thumbnail_request() -> Result<ThumbnailRequest, GtkThumbnailError> {
    let size = ThumbnailSize::fit(THUMBNAIL_WIDTH, THUMBNAIL_HEIGHT)
        .map_err(|_| GtkThumbnailError::Render)?;
    Ok(ThumbnailRequest::new(MipmapLevel::zero(), size))
}

fn thumbnail_key(
    record: &rusttable_catalog::ImportRecord,
    edit: &Edit,
    request: ThumbnailRequest,
) -> ThumbnailKey {
    let photo = record.photo();
    let asset = photo.primary_asset();
    ThumbnailKey::new(
        asset.content_hash(),
        photo.id(),
        asset.id(),
        edit.id(),
        photo.revision(),
        edit.revision(),
        u32::from(RASTER_DECODER_IDENTITY_VERSION),
        RENDERER_VERSION,
        output_transform_identity(),
        PROFILE_VERSION,
        configuration_identity(request),
        request,
    )
}

fn configuration_identity(request: ThumbnailRequest) -> [u8; 32] {
    let (width, height) = request.size().dimensions();
    let mut bytes = Vec::with_capacity(16);
    bytes.extend_from_slice(b"GTKTHUMB2:sRGB-display-fallback");
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    Sha256::digest(bytes).into()
}

fn output_transform_identity() -> [u8; 32] {
    Sha256::digest(b"rusttable.srgb-display-fallback.v1").into()
}

fn present(
    photo_id: PhotoId,
    image: &DecodedImage,
    source: GtkThumbnailSource,
    cache_identity: [u8; 32],
    render_receipt_identity: Option<[u8; 32]>,
    edit_id: EditId,
    edit_revision: Revision,
) -> Result<GtkThumbnail, GtkThumbnailError> {
    let dimensions =
        PreviewDimensions::new(image.dimensions().width(), image.dimensions().height())
            .map_err(|_| GtkThumbnailError::Presentation)?;
    let status =
        PresentationText::new("thumbnail ready").map_err(|_| GtkThumbnailError::Presentation)?;
    let metadata = Rgba8PreviewMetadata::new(dimensions, status, image.pixels().to_vec())
        .map_err(|_| GtkThumbnailError::Presentation)?;
    Ok(GtkThumbnail {
        photo_id,
        metadata,
        source,
        cache_identity,
        render_receipt_identity,
        output_transform: PreviewOutputTransform::SrgbDisplayFallback,
        edit_id,
        edit_revision,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use rusttable_catalog::EditRepository;
    use rusttable_catalog_store::RedbCatalogRepository;
    use rusttable_import::RasterImportCancellation;

    use super::{
        GtkThumbnailController, GtkThumbnailSource, configuration_identity, thumbnail_request,
    };
    use crate::workspace::run_raster_import;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let number = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "rusttable-gtk-thumbnail-{}-{number}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("thumbnail test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn decode_base64(value: &str) -> Vec<u8> {
        let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut output = Vec::new();
        let mut quartet = [0_u8; 4];
        let mut length = 0;
        for byte in value.bytes().filter(|byte| !byte.is_ascii_whitespace()) {
            if byte == b'=' {
                break;
            }
            quartet[length] = u8::try_from(
                alphabet
                    .iter()
                    .position(|candidate| *candidate == byte)
                    .expect("fixture base64 character"),
            )
            .expect("base64 alphabet index");
            length += 1;
            if length == quartet.len() {
                output.push((quartet[0] << 2) | (quartet[1] >> 4));
                output.push((quartet[1] << 4) | (quartet[2] >> 2));
                output.push((quartet[2] << 6) | quartet[3]);
                length = 0;
            }
        }
        if length >= 2 {
            output.push((quartet[0] << 2) | (quartet[1] >> 4));
        }
        if length >= 3 {
            output.push((quartet[1] << 4) | (quartet[2] >> 2));
        }
        output
    }

    #[test]
    fn visible_thumbnail_contract_is_stable_and_bounded() {
        let request = thumbnail_request().expect("constant request");
        assert_eq!(request.size().dimensions(), (180, 120));
        assert_eq!(
            configuration_identity(request),
            configuration_identity(request)
        );
    }

    #[test]
    fn imported_photo_renders_then_reuses_the_thumbnail_cache() {
        let directory = TestDirectory::new();
        let source = directory.0.join("visible.png");
        let catalog = directory.0.join("catalog.redb");
        let cache = directory.0.join("cache");
        fs::write(
            &source,
            decode_base64(include_str!(
                "../../../rusttable-image-io/tests/fixtures/rgba-2x1.png.b64"
            )),
        )
        .expect("fixture source");
        let batch = run_raster_import(
            &catalog,
            vec![source],
            &RasterImportCancellation::default(),
            &|_| {},
        );
        let photo_id = batch.first_selected_photo().expect("imported photo");

        let mut first = GtkThumbnailController::open(&catalog, &directory.0, &cache)
            .expect("open first controller");
        let rendered = first.render(photo_id).expect("render thumbnail");
        assert_eq!(rendered.source(), GtkThumbnailSource::Render);
        assert_eq!(rendered.metadata().dimensions().width(), 2);
        drop(first);

        let mut reopened =
            GtkThumbnailController::open(&catalog, &directory.0, &cache).expect("reopen cache");
        let cached = reopened.render(photo_id).expect("read cached thumbnail");
        assert_eq!(cached.source(), GtkThumbnailSource::Cache);
        assert_eq!(cached.metadata(), rendered.metadata());
        assert_eq!(cached.cache_identity(), rendered.cache_identity());
        assert!(rendered.render_receipt_identity().is_some());
        assert!(cached.render_receipt_identity().is_none());
        assert_eq!(
            cached.output_transform(),
            crate::PreviewOutputTransform::SrgbDisplayFallback
        );
    }

    #[test]
    fn changed_edit_revision_cannot_reuse_the_previous_thumbnail_cache_entry() {
        let directory = TestDirectory::new();
        let source = directory.0.join("edited.png");
        let catalog = directory.0.join("catalog.redb");
        let cache = directory.0.join("cache");
        fs::write(
            &source,
            decode_base64(include_str!(
                "../../../rusttable-image-io/tests/fixtures/rgba-2x1.png.b64"
            )),
        )
        .expect("fixture source");
        let batch = run_raster_import(
            &catalog,
            vec![source],
            &RasterImportCancellation::default(),
            &|_| {},
        );
        let photo_id = batch.first_selected_photo().expect("imported photo");

        let mut first = GtkThumbnailController::open(&catalog, &directory.0, &cache)
            .expect("open first controller");
        let old = first.render(photo_id).expect("old thumbnail");
        let old_identity = (old.edit_id(), old.edit_revision(), old.cache_identity());
        drop(first);

        let mut repository = RedbCatalogRepository::open(&catalog).expect("open catalog");
        let persisted = repository
            .list()
            .expect("list edits")
            .into_iter()
            .find(|edit| edit.photo_id() == photo_id)
            .expect("persisted edit");
        let replacement = persisted
            .revised(persisted.operations().cloned().collect::<Vec<_>>())
            .expect("advance edit revision");
        repository
            .commit_replacement(persisted.revision(), &replacement)
            .expect("persist replacement");
        drop(repository);

        let mut second = GtkThumbnailController::open(&catalog, &directory.0, &cache)
            .expect("reopen edited controller");
        let current = second.render(photo_id).expect("current thumbnail");
        assert_eq!(current.source(), GtkThumbnailSource::Render);
        assert_ne!(
            (
                current.edit_id(),
                current.edit_revision(),
                current.cache_identity()
            ),
            old_identity
        );
        assert_eq!(current.edit_id(), old.edit_id());
        assert!(current.edit_revision() > old.edit_revision());
    }
}
