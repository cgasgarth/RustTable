use std::fs;
use std::io::ErrorKind;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use rusttable_catalog::{ImportRecord, ImportRepository, RepositoryError, SourcePath};
use rusttable_catalog_store::RedbImportRepository;
use rusttable_core::PhotoId;
use rusttable_image::InputFormat;
use rusttable_import::decode_reference_source;

use crate::library::{LibraryFailureKind, LibraryState};
use rusttable_ui::{
    PhotoCardViewModel, PhotoDetailViewModel, PhotoFactViewModel, PhotoWorkspaceViewModel,
    PhotoWorkspaceViewModelError, PresentationText, PresentationTextError,
};

const CATALOG_FILENAME: &str = "catalog.redb";
const MAX_BROWSER_PHOTOS: usize = 200;
const MAX_PHOTO_TITLE_BYTES: usize = 128;
const GENERIC_PHOTO_TITLE: &str = "Image";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LibraryLoadRequestId(NonZeroU64);

impl LibraryLoadRequestId {
    pub const fn first() -> Self {
        Self(NonZeroU64::MIN)
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }

    pub fn next(self) -> Option<Self> {
        self.get()
            .checked_add(1)
            .and_then(NonZeroU64::new)
            .map(Self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryLoadResult {
    Empty,
    Ready(PhotoWorkspaceViewModel),
    Failed(LibraryFailureKind),
}

impl LibraryLoadResult {
    pub fn into_library_state(self) -> LibraryState {
        match self {
            Self::Empty => LibraryState::Empty,
            Self::Ready(workspace) => LibraryState::Ready(workspace),
            Self::Failed(kind) => LibraryState::Failed(kind),
        }
    }
}

#[derive(Debug)]
enum LibraryLoadError {
    CatalogLocation(ErrorKind),
    Repository(RepositoryError),
    Presentation(CatalogPresentationError),
}

impl LibraryLoadError {
    fn failure_kind(&self) -> LibraryFailureKind {
        match self {
            Self::CatalogLocation(error_kind) => {
                let _ = error_kind;
                LibraryFailureKind::CatalogLocationUnavailable
            }
            Self::Repository(RepositoryError::CorruptPersistedData) => {
                LibraryFailureKind::CorruptPersistedCatalog
            }
            Self::Repository(_) => LibraryFailureKind::RepositoryUnavailable,
            Self::Presentation(CatalogPresentationError::InvalidText {
                photo_id,
                source,
                error,
            }) => {
                let _ = (photo_id, source, error);
                LibraryFailureKind::PresentationConversionFailed
            }
            Self::Presentation(CatalogPresentationError::Workspace(error)) => {
                let _ = error;
                LibraryFailureKind::PresentationConversionFailed
            }
        }
    }
}

pub fn catalog_path() -> Result<PathBuf, LibraryFailureKind> {
    let override_path = std::env::var_os("RUSTTABLE_CATALOG_PATH").map(PathBuf::from);
    let default_data_directory = ProjectDirs::from("com", "cgasgarth", "RustTable")
        .map(|project_dirs| project_dirs.data_local_dir().to_path_buf());
    select_catalog_path(override_path.as_deref(), default_data_directory.as_deref())
}

pub fn source_root(catalog_path: &Path) -> Result<PathBuf, LibraryFailureKind> {
    let override_path = std::env::var_os("RUSTTABLE_SOURCE_ROOT").map(PathBuf::from);
    if let Some(path) = override_path.filter(|path| !path.as_os_str().is_empty()) {
        return Ok(path);
    }
    catalog_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .ok_or(LibraryFailureKind::CatalogLocationUnavailable)
}

fn select_catalog_path(
    override_path: Option<&Path>,
    default_data_directory: Option<&Path>,
) -> Result<PathBuf, LibraryFailureKind> {
    if let Some(path) = override_path.filter(|path| !path.as_os_str().is_empty()) {
        return Ok(path.to_path_buf());
    }
    default_data_directory
        .filter(|path| !path.as_os_str().is_empty())
        .map(|directory| directory.join(CATALOG_FILENAME))
        .ok_or(LibraryFailureKind::CatalogLocationUnavailable)
}

pub fn load_catalog(path: &Path) -> LibraryLoadResult {
    match load_catalog_detailed(path) {
        Ok(workspace) if workspace.cards().next().is_none() => LibraryLoadResult::Empty,
        Ok(workspace) => LibraryLoadResult::Ready(workspace),
        Err(error) => LibraryLoadResult::Failed(error.failure_kind()),
    }
}

fn load_catalog_detailed(path: &Path) -> Result<PhotoWorkspaceViewModel, LibraryLoadError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| LibraryLoadError::CatalogLocation(error.kind()))?;
    let repository = RedbImportRepository::open(path).map_err(LibraryLoadError::Repository)?;
    let records = repository.list().map_err(LibraryLoadError::Repository)?;
    present_records(records).map_err(LibraryLoadError::Presentation)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CatalogPresentationError {
    InvalidText {
        photo_id: PhotoId,
        source: SourcePath,
        error: PresentationTextError,
    },
    Workspace(PhotoWorkspaceViewModelError),
}

fn present_records(
    mut records: Vec<ImportRecord>,
) -> Result<PhotoWorkspaceViewModel, CatalogPresentationError> {
    records.sort_by(|left, right| left.source().cmp(right.source()));
    records.truncate(MAX_BROWSER_PHOTOS);
    let mut cards = Vec::with_capacity(records.len());
    let mut details = Vec::with_capacity(records.len());

    for record in records {
        let photo_id = record.photo().id();
        let source = record.source().clone();
        let title = photo_title(&source);
        let format = format_label(record.probe().format());
        let dimensions = record.probe().dimensions();
        let dimension_text = format!("{} × {}", dimensions.width(), dimensions.height());
        let file_size = format_file_size(record.photo().primary_asset().byte_length().get());
        let secondary =
            presentation_text(&format!("{format} · {dimension_text}"), photo_id, &source)?;
        let facts = vec![
            fact("Format", format, photo_id, &source)?,
            fact("Dimensions", &dimension_text, photo_id, &source)?,
            fact("File size", &file_size, photo_id, &source)?,
        ];
        cards.push(PhotoCardViewModel::new(
            photo_id,
            title.clone(),
            Some(secondary),
        ));
        details.push(PhotoDetailViewModel::new(photo_id, title, facts));
    }

    PhotoWorkspaceViewModel::new(cards, details).map_err(CatalogPresentationError::Workspace)
}

fn photo_title(source: &SourcePath) -> PresentationText {
    decode_reference_source(source)
        .ok()
        .and_then(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        })
        .as_deref()
        .and_then(bounded_title)
        .or_else(|| {
            (!source.as_str().starts_with("reference-v1/"))
                .then(|| source.components().last())
                .flatten()
                .and_then(bounded_title)
        })
        .unwrap_or_else(|| PresentationText::new(GENERIC_PHOTO_TITLE).expect("constant title"))
}

fn bounded_title(value: &str) -> Option<PresentationText> {
    let mut bounded = String::new();
    for character in value.chars().filter(|character| !character.is_control()) {
        let next_length = bounded.len().checked_add(character.len_utf8())?;
        if next_length > MAX_PHOTO_TITLE_BYTES {
            break;
        }
        bounded.push(character);
    }
    PresentationText::new(bounded).ok()
}

fn presentation_text(
    value: &str,
    photo_id: PhotoId,
    source: &SourcePath,
) -> Result<PresentationText, CatalogPresentationError> {
    PresentationText::new(value).map_err(|error| CatalogPresentationError::InvalidText {
        photo_id,
        source: source.clone(),
        error,
    })
}

fn fact(
    label: &str,
    value: &str,
    photo_id: PhotoId,
    source: &SourcePath,
) -> Result<PhotoFactViewModel, CatalogPresentationError> {
    Ok(PhotoFactViewModel::new(
        presentation_text(label, photo_id, source)?,
        presentation_text(value, photo_id, source)?,
    ))
}

fn format_label(format: InputFormat) -> &'static str {
    match format {
        InputFormat::Jpeg => "JPEG",
        InputFormat::Png => "PNG",
        InputFormat::Tiff => "TIFF",
    }
}

fn format_file_size(bytes: u64) -> String {
    format!("{bytes} bytes")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use rusttable_catalog::{
        ImportCandidate, ImportRecord, ImportRepository, RepositoryError, SourcePath,
    };
    use rusttable_catalog_store::RedbImportRepository;
    use rusttable_core::{
        Asset, AssetId, AssetRole, ByteLength, ContentHash, ImageMetadata, Photo, PhotoId,
    };
    use rusttable_image::{ImageDimensions, ImageProbe, InputFormat};
    use rusttable_import::encode_reference_source;

    use super::{
        CatalogPresentationError, LibraryLoadError, LibraryLoadRequestId, LibraryLoadResult,
        MAX_BROWSER_PHOTOS, format_file_size, load_catalog, present_records, select_catalog_path,
        source_root,
    };
    use crate::library::LibraryFailureKind;
    use rusttable_ui::PhotoWorkspaceViewModelError;

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempCatalog {
        directory: PathBuf,
        path: PathBuf,
    }

    impl TempCatalog {
        fn new(name: &str) -> Self {
            let number = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let directory = std::env::temp_dir().join(format!(
                "rusttable-library-loader-{}-{number}-{name}",
                std::process::id()
            ));
            let path = directory.join("nested").join("catalog.redb");
            let _ = fs::remove_dir_all(&directory);
            Self { directory, path }
        }
    }

    impl Drop for TempCatalog {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    fn record(source: &str, photo_id: u128, asset_id: u128, format: InputFormat) -> ImportRecord {
        let candidate = ImportCandidate::new(
            PhotoId::new(photo_id).expect("photo ID"),
            AssetId::new(asset_id).expect("asset ID"),
            SourcePath::new(source).expect("source"),
            ContentHash::Sha256([u8::try_from(photo_id).expect("test hash byte"); 32]),
            ByteLength::from_bytes(1_025),
            ImageProbe::new(
                format,
                ImageDimensions::new(6000, 4000).expect("dimensions"),
            ),
            ImageMetadata::empty(),
        )
        .expect("candidate");
        let asset = Asset::new(
            candidate.asset_id(),
            AssetRole::Primary,
            candidate.content_hash(),
            candidate.byte_length(),
        );
        ImportRecord::new(
            &candidate,
            Photo::new(candidate.photo_id(), [asset]).expect("photo"),
        )
        .expect("record")
    }

    #[test]
    fn path_selection_prefers_nonempty_override_and_uses_exact_default_filename() {
        let override_path = Path::new("/explicit/catalog.redb");
        let default_directory = Path::new("/platform/rusttable");
        assert_eq!(
            select_catalog_path(Some(override_path), Some(default_directory)),
            Ok(override_path.to_path_buf())
        );
        assert_eq!(
            select_catalog_path(Some(Path::new("")), Some(default_directory)),
            Ok(default_directory.join("catalog.redb"))
        );
        assert_eq!(
            select_catalog_path(None, None),
            Err(LibraryFailureKind::CatalogLocationUnavailable)
        );
    }

    #[test]
    fn source_root_uses_the_catalog_parent_without_an_override() {
        let catalog = Path::new("/tmp/rusttable/catalog.redb");
        assert_eq!(source_root(catalog), Ok(PathBuf::from("/tmp/rusttable")));
    }

    #[test]
    fn a_new_real_store_loads_as_empty() {
        let catalog = TempCatalog::new("empty");

        assert_eq!(load_catalog(&catalog.path), LibraryLoadResult::Empty);
        assert!(catalog.path.is_file());
    }

    #[test]
    fn corrupt_real_store_maps_to_safe_corrupt_failure() {
        let catalog = TempCatalog::new("corrupt");
        fs::create_dir_all(catalog.path.parent().expect("parent")).expect("parent");
        fs::write(&catalog.path, b"not a redb database").expect("corrupt catalog");

        assert_eq!(
            load_catalog(&catalog.path),
            LibraryLoadResult::Failed(LibraryFailureKind::CorruptPersistedCatalog)
        );
    }

    #[test]
    fn a_populated_real_store_loads_in_source_order_as_ready_model() {
        let catalog = TempCatalog::new("ready");
        let first = record("zeta/photo.png", 1, 11, InputFormat::Png);
        let second = record("alpha/photo.jpg", 2, 12, InputFormat::Jpeg);
        {
            fs::create_dir_all(catalog.path.parent().expect("parent")).expect("parent");
            let mut repository = RedbImportRepository::open(&catalog.path).expect("open");
            repository.commit(&first).expect("first commit");
            repository.commit(&second).expect("second commit");
        }

        let LibraryLoadResult::Ready(workspace) = load_catalog(&catalog.path) else {
            panic!("expected ready library")
        };
        assert_eq!(
            workspace
                .cards()
                .map(|card| card.id().get())
                .collect::<Vec<_>>(),
            [2, 1]
        );
        let first_card = workspace.cards().next().expect("card");
        assert_eq!(first_card.title().as_str(), "photo.jpg");
        assert_eq!(
            first_card.secondary().expect("secondary").as_str(),
            "JPEG · 6000 × 4000"
        );
        let first_detail = workspace.detail(first_card.id()).expect("detail");
        assert_eq!(
            first_detail
                .facts()
                .map(|fact| (fact.label().as_str(), fact.value().as_str()))
                .collect::<Vec<_>>(),
            [
                ("Format", "JPEG"),
                ("Dimensions", "6000 × 4000"),
                ("File size", "1025 bytes"),
            ]
        );
    }

    #[test]
    fn lower_layer_failures_map_to_closed_safe_kinds() {
        assert_eq!(
            LibraryLoadError::CatalogLocation(std::io::ErrorKind::PermissionDenied).failure_kind(),
            LibraryFailureKind::CatalogLocationUnavailable
        );
        assert_eq!(
            LibraryLoadError::Repository(RepositoryError::Unavailable).failure_kind(),
            LibraryFailureKind::RepositoryUnavailable
        );
        assert_eq!(
            LibraryLoadError::Repository(RepositoryError::CorruptPersistedData).failure_kind(),
            LibraryFailureKind::CorruptPersistedCatalog
        );
        assert_eq!(
            LibraryLoadError::Presentation(CatalogPresentationError::Workspace(
                PhotoWorkspaceViewModelError::OrphanDetail {
                    id: PhotoId::new(1).expect("id"),
                },
            ))
            .failure_kind(),
            LibraryFailureKind::PresentationConversionFailed
        );
    }

    #[test]
    fn request_ids_are_checked_and_overflow_is_rejected() {
        let first = LibraryLoadRequestId::first();
        assert_eq!(first.get(), 1);
        assert_eq!(first.next().expect("next").get(), 2);
        let maximum = LibraryLoadRequestId(std::num::NonZeroU64::MAX);
        assert_eq!(maximum.next(), None);
    }

    #[test]
    fn reference_source_titles_use_the_original_filename_without_exposing_its_path() {
        let source_path = Path::new("/private/photos/2026/long-name.png");
        let source = encode_reference_source(source_path, [7; 32]).expect("opaque source");
        let record = record(source.as_str(), 4, 14, InputFormat::Png);

        let workspace = present_records(vec![record]).expect("safe title");
        let card = workspace.cards().next().expect("card");

        assert_eq!(card.title().as_str(), "long-name.png");
        assert!(!card.title().as_str().contains("private"));
        assert!(!card.title().as_str().contains("reference-v1"));
    }

    #[test]
    fn malformed_reference_source_uses_a_safe_generic_title() {
        let malformed = record(
            "reference-v1/not-a-hash/not-a-path",
            5,
            15,
            InputFormat::Png,
        );
        let workspace = present_records(vec![malformed]).expect("safe fallback title");
        assert_eq!(
            workspace.cards().next().expect("card").title().as_str(),
            "Image"
        );
    }

    #[test]
    fn presentation_is_bounded_to_the_first_two_hundred_sorted_sources() {
        let records = (1..=201)
            .map(|number| {
                record(
                    &format!("collection/{number:03}.png"),
                    number,
                    number + 1_000,
                    InputFormat::Png,
                )
            })
            .rev()
            .collect();

        let workspace = present_records(records).expect("bounded presentation");

        assert_eq!(workspace.cards().len(), MAX_BROWSER_PHOTOS);
        assert_eq!(
            workspace
                .cards()
                .map(|card| card.id().get())
                .collect::<Vec<_>>(),
            (1..=u128::try_from(MAX_BROWSER_PHOTOS).expect("fits test IDs")).collect::<Vec<_>>()
        );
        assert!(
            workspace
                .detail(PhotoId::new(201).expect("test photo ID"))
                .is_none()
        );
    }

    #[test]
    fn file_size_is_exact_integer_text() {
        assert_eq!(format_file_size(0), "0 bytes");
        assert_eq!(format_file_size(1023), "1023 bytes");
        assert_eq!(format_file_size(1024), "1024 bytes");
        assert_eq!(format_file_size(u64::MAX), format!("{} bytes", u64::MAX));
    }
}
