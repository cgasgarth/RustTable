use std::path::{Path, PathBuf};

use rusttable_catalog::SourcePath;
use rusttable_core::{AssetId, PhotoId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceImportRequest {
    photo_id: PhotoId,
    asset_id: AssetId,
    source: SourcePath,
    physical_path: PathBuf,
}

impl SourceImportRequest {
    #[must_use]
    pub fn new(
        photo_id: PhotoId,
        asset_id: AssetId,
        source: SourcePath,
        physical_path: PathBuf,
    ) -> Self {
        Self {
            photo_id,
            asset_id,
            source,
            physical_path,
        }
    }

    #[must_use]
    pub const fn photo_id(&self) -> PhotoId {
        self.photo_id
    }

    #[must_use]
    pub const fn asset_id(&self) -> AssetId {
        self.asset_id
    }

    #[must_use]
    pub fn source(&self) -> &SourcePath {
        &self.source
    }

    #[must_use]
    pub fn physical_path(&self) -> &Path {
        &self.physical_path
    }
}
