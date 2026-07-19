use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::errors::{ErrorCode, ScriptError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageReceipt {
    pub version: u64,
    pub schema_version: u32,
    pub key_count: usize,
    pub byte_count: usize,
}

#[derive(Debug, Clone)]
pub struct ScriptStorage {
    schema_version: u32,
    version: u64,
    max_bytes: usize,
    max_keys: usize,
    values: BTreeMap<String, Vec<u8>>,
}

impl ScriptStorage {
    #[must_use]
    pub fn new(schema_version: u32, max_bytes: usize, max_keys: usize) -> Self {
        Self {
            schema_version,
            version: 0,
            max_bytes,
            max_keys,
            values: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&[u8]> {
        self.values.get(key).map(Vec::as_slice)
    }

    #[must_use]
    pub fn begin(&self) -> StorageTransaction {
        StorageTransaction {
            base_version: self.version,
            schema_version: self.schema_version,
            values: self.values.clone(),
            max_bytes: self.max_bytes,
            max_keys: self.max_keys,
        }
    }

    /// # Errors
    ///
    /// Returns a migration, conflict, or quota error without partially applying the transaction.
    pub fn migrate<F>(&mut self, target: u32, migration: F) -> Result<StorageReceipt, ScriptError>
    where
        F: FnOnce(&mut BTreeMap<String, Vec<u8>>) -> Result<(), ScriptError>,
    {
        let mut transaction = self.begin();
        migration(&mut transaction.values)?;
        transaction.schema_version = target;
        transaction.commit(self)
    }

    #[must_use]
    pub fn receipt(&self) -> StorageReceipt {
        StorageReceipt {
            version: self.version,
            schema_version: self.schema_version,
            key_count: self.values.len(),
            byte_count: 0,
        }
    }

    fn apply(&mut self, transaction: StorageTransaction) -> Result<StorageReceipt, ScriptError> {
        if transaction.base_version != self.version {
            return Err(ScriptError::new(
                ErrorCode::StorageConflict,
                "storage version changed",
            ));
        }
        let bytes = transaction.values.values().map(Vec::len).sum::<usize>();
        if transaction.values.len() > transaction.max_keys || bytes > transaction.max_bytes {
            return Err(ScriptError::new(
                ErrorCode::LimitExceeded,
                "storage quota exceeded",
            ));
        }
        self.version = self.version.saturating_add(1);
        self.schema_version = transaction.schema_version;
        self.values = transaction.values;
        Ok(StorageReceipt {
            version: self.version,
            schema_version: self.schema_version,
            key_count: self.values.len(),
            byte_count: bytes,
        })
    }
}

#[derive(Debug, Clone)]
pub struct StorageTransaction {
    base_version: u64,
    schema_version: u32,
    values: BTreeMap<String, Vec<u8>>,
    max_bytes: usize,
    max_keys: usize,
}

impl StorageTransaction {
    pub fn put(&mut self, key: impl Into<String>, value: Vec<u8>) {
        self.values.insert(key.into(), value);
    }

    pub fn remove(&mut self, key: &str) {
        self.values.remove(key);
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&[u8]> {
        self.values.get(key).map(Vec::as_slice)
    }

    /// # Errors
    ///
    /// Returns `StorageConflict` or `LimitExceeded` when the transaction cannot be committed.
    pub fn commit(self, storage: &mut ScriptStorage) -> Result<StorageReceipt, ScriptError> {
        storage.apply(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_migration_is_transactional_and_bounded() {
        let mut storage = ScriptStorage::new(1, 8, 2);
        let mut transaction = storage.begin();
        transaction.put("key", b"value".to_vec());
        assert_eq!(
            transaction.commit(&mut storage).expect("commit").byte_count,
            5
        );
        let receipt = storage
            .migrate(2, |values| {
                values.insert("migrated".to_owned(), b"ok".to_vec());
                Ok(())
            })
            .expect("migration");
        assert_eq!(receipt.schema_version, 2);
        assert!(
            storage
                .migrate(3, |values| {
                    values.insert("too-large".to_owned(), b"nope".to_vec());
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(storage.get("migrated"), Some(b"ok".as_slice()));
    }
}
