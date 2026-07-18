use std::path::{Path, PathBuf};

use rusttable_catalog::{ImportCandidate, ImportRecord, SourcePath};
use rusttable_core::{
    Asset, AssetId, AssetRole, ByteLength, ContentHash, ImageMetadata, Photo, PhotoId,
};
use rusttable_image::{ImageDimensions, ImageProbe, InputFormat};

#[allow(dead_code)]
pub fn record(source: &str, photo_id: u128, asset_id: u128, byte: u8) -> ImportRecord {
    let candidate = ImportCandidate::new(
        PhotoId::new(photo_id).expect("photo ID"),
        AssetId::new(asset_id).expect("asset ID"),
        SourcePath::new(source).expect("source"),
        ContentHash::Sha256([byte; 32]),
        ByteLength::from_bytes(8),
        ImageProbe::new(InputFormat::Png, ImageDimensions::new(2, 1).unwrap()),
        ImageMetadata::empty(),
    )
    .unwrap();
    let asset = Asset::new(
        candidate.asset_id(),
        AssetRole::Primary,
        candidate.content_hash(),
        candidate.byte_length(),
    );
    ImportRecord::new(
        &candidate,
        Photo::new(candidate.photo_id(), [asset]).unwrap(),
    )
    .unwrap()
}

pub fn temp_path(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("rusttable-catalog-store-{name}.redb"));
    let _ = std::fs::remove_file(&path);
    path
}

pub fn remove(path: &Path) {
    let _ = std::fs::remove_file(path);
}
