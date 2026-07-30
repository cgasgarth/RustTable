use std::collections::BTreeMap;
use std::fmt;

use super::asset::{Asset, AssetRole};
use crate::{AssetId, PhotoId, Revision};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Photo {
    id: PhotoId,
    revision: Revision,
    primary_asset_id: AssetId,
    assets: BTreeMap<AssetId, Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhotoBuildError {
    NoAssets,
    DuplicateAssetId { id: AssetId },
    MissingPrimaryAsset,
    MultiplePrimaryAssets { ids: Vec<AssetId> },
}

impl fmt::Display for PhotoBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAssets => formatter.write_str("a photo must contain at least one asset"),
            Self::DuplicateAssetId { id } => {
                write!(formatter, "asset ID {id} was supplied more than once")
            }
            Self::MissingPrimaryAsset => {
                formatter.write_str("a photo must contain one primary asset")
            }
            Self::MultiplePrimaryAssets { ids } => {
                write!(formatter, "a photo has multiple primary assets: ")?;
                for (index, id) in ids.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{id}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for PhotoBuildError {}

impl Photo {
    /// Builds a photo with [`Revision::ZERO`].
    ///
    /// # Errors
    ///
    /// Returns a [`PhotoBuildError`] when the asset sequence violates the photo invariants.
    pub fn new<I>(id: PhotoId, assets: I) -> Result<Self, PhotoBuildError>
    where
        I: IntoIterator<Item = Asset>,
    {
        Self::from_parts(id, Revision::ZERO, assets)
    }

    /// Reconstructs a photo with an explicitly supplied revision.
    ///
    /// # Errors
    ///
    /// Returns a [`PhotoBuildError`] when the asset sequence violates the photo invariants.
    pub fn from_parts<I>(
        id: PhotoId,
        revision: Revision,
        assets: I,
    ) -> Result<Self, PhotoBuildError>
    where
        I: IntoIterator<Item = Asset>,
    {
        let assets = collect_assets(assets)?;
        let primary_ids = assets
            .values()
            .filter(|asset| asset.role() == AssetRole::Primary)
            .map(Asset::id)
            .collect::<Vec<_>>();
        let primary_asset_id = match primary_ids.as_slice() {
            [] => return Err(PhotoBuildError::MissingPrimaryAsset),
            [id] => *id,
            _ => return Err(PhotoBuildError::MultiplePrimaryAssets { ids: primary_ids }),
        };

        Ok(Self {
            id,
            revision,
            primary_asset_id,
            assets,
        })
    }

    #[must_use]
    pub const fn id(&self) -> PhotoId {
        self.id
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn primary_asset_id(&self) -> AssetId {
        self.primary_asset_id
    }

    #[must_use]
    /// Returns the asset whose role is [`AssetRole::Primary`].
    ///
    /// # Panics
    ///
    /// Panics only if the private aggregate invariant is violated; validated constructors make
    /// that state unreachable.
    pub fn primary_asset(&self) -> &Asset {
        self.assets
            .get(&self.primary_asset_id)
            .expect("Photo invariant guarantees a primary asset")
    }

    #[must_use]
    pub fn asset(&self, id: AssetId) -> Option<&Asset> {
        self.assets.get(&id)
    }

    pub fn assets(&self) -> impl Iterator<Item = &Asset> {
        self.assets.values()
    }
}

fn collect_assets<I>(assets: I) -> Result<BTreeMap<AssetId, Asset>, PhotoBuildError>
where
    I: IntoIterator<Item = Asset>,
{
    let mut collected = BTreeMap::new();
    for asset in assets {
        let id = asset.id();
        if collected.insert(id, asset).is_some() {
            return Err(PhotoBuildError::DuplicateAssetId { id });
        }
    }
    if collected.is_empty() {
        return Err(PhotoBuildError::NoAssets);
    }
    Ok(collected)
}
