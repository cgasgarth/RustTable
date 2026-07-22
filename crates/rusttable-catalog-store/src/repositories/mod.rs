use std::path::Path;
use std::sync::Arc;

use redb::{Database, ReadableDatabase, ReadableTable};
use rusttable_catalog::{ImportRecord, ImportRepository, RepositoryError, SourcePath};
use rusttable_core::{AssetId, PhotoId};

pub(crate) mod catalog;
pub(crate) mod collection;
pub(crate) mod edit;
pub(crate) mod history;
pub(crate) mod metadata;
pub(crate) mod recipe;
pub(crate) mod tags;

use crate::codecs as codec;
use crate::schema::{self, ASSET_INDEX_TABLE, PHOTO_INDEX_TABLE, RECORDS_TABLE};

pub struct RedbImportRepository {
    database: Arc<Database>,
}

impl RedbImportRepository {
    /// Opens a checked schema-versioned store, initializing a new database.
    ///
    /// # Errors
    ///
    /// Returns a typed unavailable or corrupt-persisted-data error.
    pub fn open(path: &Path) -> Result<Self, RepositoryError> {
        Ok(Self {
            database: schema::open(path)?,
        })
    }

    pub(crate) const fn from_database(database: Arc<Database>) -> Self {
        Self { database }
    }
}

impl ImportRepository for RedbImportRepository {
    fn find_by_source(&self, source: &SourcePath) -> Result<Option<ImportRecord>, RepositoryError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| RepositoryError::Unavailable)?;
        let table = transaction
            .open_table(RECORDS_TABLE)
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
        let value = table
            .get(source.as_str().as_bytes())
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
        value
            .map(|value| {
                codec::decode(value.value()).map_err(|()| RepositoryError::CorruptPersistedData)
            })
            .transpose()
    }

    fn find_by_photo_id(&self, photo_id: PhotoId) -> Result<Option<ImportRecord>, RepositoryError> {
        self.find_by_index(PHOTO_INDEX_TABLE, &photo_id.get().to_be_bytes())
    }

    fn find_by_asset_id(&self, asset_id: AssetId) -> Result<Option<ImportRecord>, RepositoryError> {
        self.find_by_index(ASSET_INDEX_TABLE, &asset_id.get().to_be_bytes())
    }

    fn commit(&mut self, record: &ImportRecord) -> Result<(), RepositoryError> {
        let encoded = codec::encode(record).map_err(|()| RepositoryError::CorruptPersistedData)?;
        let source = record.source().as_str().as_bytes();
        let photo_id = record.photo().id().get().to_be_bytes();
        let asset_id = record.photo().primary_asset_id().get().to_be_bytes();
        let transaction = self
            .database
            .begin_write()
            .map_err(|_| RepositoryError::Unavailable)?;
        {
            let mut records = transaction
                .open_table(RECORDS_TABLE)
                .map_err(|_| RepositoryError::Unavailable)?;
            let mut photos = transaction
                .open_table(PHOTO_INDEX_TABLE)
                .map_err(|_| RepositoryError::Unavailable)?;
            let mut assets = transaction
                .open_table(ASSET_INDEX_TABLE)
                .map_err(|_| RepositoryError::Unavailable)?;
            if records
                .get(source)
                .map_err(|_| RepositoryError::Unavailable)?
                .is_some()
            {
                return Err(RepositoryError::SourceConflict {
                    source: record.source().clone(),
                });
            }
            if photos
                .get(photo_id.as_slice())
                .map_err(|_| RepositoryError::Unavailable)?
                .is_some()
            {
                return Err(RepositoryError::PhotoIdConflict {
                    photo_id: record.photo().id(),
                });
            }
            if assets
                .get(asset_id.as_slice())
                .map_err(|_| RepositoryError::Unavailable)?
                .is_some()
            {
                return Err(RepositoryError::AssetIdConflict {
                    asset_id: record.photo().primary_asset_id(),
                });
            }
            records
                .insert(source, encoded.as_slice())
                .map_err(|_| RepositoryError::Unavailable)?;
            photos
                .insert(photo_id.as_slice(), source)
                .map_err(|_| RepositoryError::Unavailable)?;
            assets
                .insert(asset_id.as_slice(), source)
                .map_err(|_| RepositoryError::Unavailable)?;
        }
        transaction
            .commit()
            .map_err(|_| RepositoryError::CommitFailure)
    }

    fn list(&self) -> Result<Vec<ImportRecord>, RepositoryError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| RepositoryError::Unavailable)?;
        let table = transaction
            .open_table(RECORDS_TABLE)
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
        let mut records = Vec::new();
        for entry in table
            .iter()
            .map_err(|_| RepositoryError::CorruptPersistedData)?
        {
            let (_, value) = entry.map_err(|_| RepositoryError::CorruptPersistedData)?;
            records.push(
                codec::decode(value.value()).map_err(|()| RepositoryError::CorruptPersistedData)?,
            );
        }
        Ok(records)
    }
}

impl RedbImportRepository {
    fn find_by_index(
        &self,
        table_definition: redb::TableDefinition<&[u8], &[u8]>,
        key: &[u8],
    ) -> Result<Option<ImportRecord>, RepositoryError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| RepositoryError::Unavailable)?;
        let index = transaction
            .open_table(table_definition)
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
        let source = index
            .get(key)
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
        let Some(source) = source else {
            return Ok(None);
        };
        let records = transaction
            .open_table(RECORDS_TABLE)
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
        let value = records
            .get(source.value())
            .map_err(|_| RepositoryError::CorruptPersistedData)?
            .ok_or(RepositoryError::CorruptPersistedData)?;
        Ok(Some(
            codec::decode(value.value()).map_err(|()| RepositoryError::CorruptPersistedData)?,
        ))
    }
}
