#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

use redb::{Database, ReadableDatabase, ReadableTable};
use rusttable_export::{ExportRecipe, ImportConflictPolicy, RecipeError, RecipeId, RecipeRevision};

use crate::schema;

/// Durable, revisioned export recipe persistence over the catalog's redb database.
pub struct RedbRecipeRepository {
    database: Arc<Database>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecipeStoreError {
    Unavailable,
    Corrupt,
    Conflict,
    OptimisticConcurrency {
        expected: RecipeRevision,
        actual: RecipeRevision,
    },
    BuiltInImmutable,
    Referenced,
    InvalidRecipe(RecipeError),
    CommitFailed,
}

impl RedbRecipeRepository {
    /// Opens a recipe repository, sharing the same schema migration as the catalog store.
    pub fn open(path: &Path) -> Result<Self, RecipeStoreError> {
        Ok(Self {
            database: schema::open(path).map_err(|error| map_schema_error(&error))?,
        })
    }

    pub(crate) const fn from_database(database: Arc<Database>) -> Self {
        Self { database }
    }

    pub fn create(&self, recipe: &ExportRecipe) -> Result<(), RecipeStoreError> {
        recipe.validate().map_err(RecipeStoreError::InvalidRecipe)?;
        if recipe.revision() != RecipeRevision::FIRST {
            return Err(RecipeStoreError::Conflict);
        }
        let transaction = self.write_transaction()?;
        let key = recipe_key(recipe);
        let id = recipe.id().as_str().as_bytes();
        {
            let mut recipes = transaction
                .open_table(schema::RECIPES_TABLE)
                .map_err(|_| RecipeStoreError::Unavailable)?;
            let mut heads = transaction
                .open_table(schema::RECIPE_HEADS_TABLE)
                .map_err(|_| RecipeStoreError::Unavailable)?;
            if heads
                .get(id)
                .map_err(|_| RecipeStoreError::Unavailable)?
                .is_some()
            {
                return Err(RecipeStoreError::Conflict);
            }
            let encoded = recipe
                .canonical_json()
                .map_err(RecipeStoreError::InvalidRecipe)?;
            recipes
                .insert(key.as_slice(), encoded.as_bytes())
                .map_err(|_| RecipeStoreError::Unavailable)?;
            let revision = recipe.revision().get().to_be_bytes();
            heads
                .insert(id, revision.as_slice())
                .map_err(|_| RecipeStoreError::Unavailable)?;
        }
        transaction
            .commit()
            .map_err(|_| RecipeStoreError::CommitFailed)
    }

    pub fn find(
        &self,
        id: &RecipeId,
        revision: Option<RecipeRevision>,
    ) -> Result<Option<ExportRecipe>, RecipeStoreError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| RecipeStoreError::Unavailable)?;
        let heads = transaction
            .open_table(schema::RECIPE_HEADS_TABLE)
            .map_err(|_| RecipeStoreError::Corrupt)?;
        let revision = if let Some(revision) = revision {
            revision
        } else {
            let Some(value) = heads
                .get(id.as_str().as_bytes())
                .map_err(|_| RecipeStoreError::Corrupt)?
            else {
                return Ok(None);
            };
            RecipeRevision::new(read_u64(value.value())?).ok_or(RecipeStoreError::Corrupt)?
        };
        let recipes = transaction
            .open_table(schema::RECIPES_TABLE)
            .map_err(|_| RecipeStoreError::Corrupt)?;
        let Some(value) = recipes
            .get(recipe_key_parts(id, revision).as_slice())
            .map_err(|_| RecipeStoreError::Corrupt)?
        else {
            return Ok(None);
        };
        ExportRecipe::from_canonical_json(
            std::str::from_utf8(value.value()).map_err(|_| RecipeStoreError::Corrupt)?,
        )
        .map(Some)
        .map_err(RecipeStoreError::InvalidRecipe)
    }

    pub fn list(&self) -> Result<Vec<ExportRecipe>, RecipeStoreError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| RecipeStoreError::Unavailable)?;
        let heads = transaction
            .open_table(schema::RECIPE_HEADS_TABLE)
            .map_err(|_| RecipeStoreError::Corrupt)?;
        let mut recipes = Vec::new();
        for entry in heads.iter().map_err(|_| RecipeStoreError::Corrupt)? {
            let (id, revision) = entry.map_err(|_| RecipeStoreError::Corrupt)?;
            let id = RecipeId::new(
                std::str::from_utf8(id.value()).map_err(|_| RecipeStoreError::Corrupt)?,
            )
            .map_err(RecipeStoreError::InvalidRecipe)?;
            let revision = RecipeRevision::new(read_u64(revision.value())?)
                .ok_or(RecipeStoreError::Corrupt)?;
            let table = transaction
                .open_table(schema::RECIPES_TABLE)
                .map_err(|_| RecipeStoreError::Corrupt)?;
            let value = table
                .get(recipe_key_parts(&id, revision).as_slice())
                .map_err(|_| RecipeStoreError::Corrupt)?
                .ok_or(RecipeStoreError::Corrupt)?;
            recipes.push(
                ExportRecipe::from_canonical_json(
                    std::str::from_utf8(value.value()).map_err(|_| RecipeStoreError::Corrupt)?,
                )
                .map_err(RecipeStoreError::InvalidRecipe)?,
            );
        }
        Ok(recipes)
    }

    pub fn update(
        &self,
        expected: RecipeRevision,
        recipe: &ExportRecipe,
    ) -> Result<(), RecipeStoreError> {
        recipe.validate().map_err(RecipeStoreError::InvalidRecipe)?;
        let current = self
            .current_revision(recipe.id())?
            .ok_or(RecipeStoreError::Conflict)?;
        if self
            .find(recipe.id(), Some(current))?
            .is_some_and(|current_recipe| current_recipe.built_in())
        {
            return Err(RecipeStoreError::BuiltInImmutable);
        }
        if current != expected {
            return Err(RecipeStoreError::OptimisticConcurrency {
                expected,
                actual: current,
            });
        }
        let next = expected.next().map_err(RecipeStoreError::InvalidRecipe)?;
        if recipe.revision() != next {
            return Err(RecipeStoreError::Conflict);
        }
        self.insert_revision(recipe, expected)
    }

    pub fn disable(
        &self,
        expected: RecipeRevision,
        id: &RecipeId,
    ) -> Result<ExportRecipe, RecipeStoreError> {
        let recipe = self.find(id, None)?.ok_or(RecipeStoreError::Conflict)?;
        if recipe.built_in() {
            return Err(RecipeStoreError::BuiltInImmutable);
        }
        if recipe.revision() != expected {
            return Err(RecipeStoreError::OptimisticConcurrency {
                expected,
                actual: recipe.revision(),
            });
        }
        let disabled = recipe.disabled().map_err(RecipeStoreError::InvalidRecipe)?;
        self.insert_revision(&disabled, expected)?;
        Ok(disabled)
    }

    pub fn clone_recipe(
        &self,
        source: &RecipeId,
        new_id: RecipeId,
    ) -> Result<ExportRecipe, RecipeStoreError> {
        let recipe = self.find(source, None)?.ok_or(RecipeStoreError::Conflict)?;
        let cloned = recipe
            .with_id(new_id)
            .map_err(RecipeStoreError::InvalidRecipe)?;
        self.create(&cloned)?;
        Ok(cloned)
    }

    pub fn delete(&self, id: &RecipeId) -> Result<(), RecipeStoreError> {
        let recipe = self.find(id, None)?.ok_or(RecipeStoreError::Conflict)?;
        if recipe.built_in() {
            return Err(RecipeStoreError::BuiltInImmutable);
        }
        let transaction = self.write_transaction()?;
        {
            let references = transaction
                .open_table(schema::RECIPE_REFERENCES_TABLE)
                .map_err(|_| RecipeStoreError::Unavailable)?;
            if references
                .iter()
                .map_err(|_| RecipeStoreError::Unavailable)?
                .filter_map(Result::ok)
                .any(|(key, _)| key.value().starts_with(id.as_str().as_bytes()))
            {
                return Err(RecipeStoreError::Referenced);
            }
        }
        let prefix = recipe_prefix(id);
        let keys = {
            let recipes = transaction
                .open_table(schema::RECIPES_TABLE)
                .map_err(|_| RecipeStoreError::Unavailable)?;
            recipes
                .iter()
                .map_err(|_| RecipeStoreError::Unavailable)?
                .filter_map(Result::ok)
                .filter(|(key, _)| key.value().starts_with(&prefix))
                .map(|(key, _)| key.value().to_vec())
                .collect::<Vec<_>>()
        };
        {
            let mut recipes = transaction
                .open_table(schema::RECIPES_TABLE)
                .map_err(|_| RecipeStoreError::Unavailable)?;
            for key in keys {
                recipes
                    .remove(key.as_slice())
                    .map_err(|_| RecipeStoreError::Unavailable)?;
            }
        }
        {
            let mut heads = transaction
                .open_table(schema::RECIPE_HEADS_TABLE)
                .map_err(|_| RecipeStoreError::Unavailable)?;
            heads
                .remove(id.as_str().as_bytes())
                .map_err(|_| RecipeStoreError::Unavailable)?;
        }
        transaction
            .commit()
            .map_err(|_| RecipeStoreError::CommitFailed)
    }

    pub fn mark_reference(&self, id: &RecipeId, reference: &str) -> Result<(), RecipeStoreError> {
        if self.find(id, None)?.is_none() {
            return Err(RecipeStoreError::Conflict);
        }
        let transaction = self.write_transaction()?;
        {
            let mut table = transaction
                .open_table(schema::RECIPE_REFERENCES_TABLE)
                .map_err(|_| RecipeStoreError::Unavailable)?;
            let key = reference_key(id, reference);
            table
                .insert(key.as_slice(), &[1][..])
                .map_err(|_| RecipeStoreError::Unavailable)?;
        }
        transaction
            .commit()
            .map_err(|_| RecipeStoreError::CommitFailed)
    }

    pub fn import_json(
        &self,
        json: &str,
        policy: ImportConflictPolicy,
    ) -> Result<ExportRecipe, RecipeStoreError> {
        let recipe =
            ExportRecipe::from_canonical_json(json).map_err(RecipeStoreError::InvalidRecipe)?;
        match policy {
            ImportConflictPolicy::Reject => {
                if self.find(recipe.id(), None)?.is_some() {
                    return Err(RecipeStoreError::Conflict);
                }
                self.create(&recipe)?;
                Ok(recipe)
            }
            ImportConflictPolicy::CreateNewId => {
                let mut id = RecipeId::new(format!("import-{}", hex(&recipe.content_hash()[..8])))
                    .map_err(RecipeStoreError::InvalidRecipe)?;
                let mut suffix = 0_u32;
                while self.find(&id, None)?.is_some() {
                    suffix += 1;
                    id = RecipeId::new(format!(
                        "import-{}-{suffix}",
                        hex(&recipe.content_hash()[..8])
                    ))
                    .map_err(RecipeStoreError::InvalidRecipe)?;
                }
                let recipe = recipe
                    .with_id(id)
                    .map_err(RecipeStoreError::InvalidRecipe)?;
                self.create(&recipe)?;
                Ok(recipe)
            }
            ImportConflictPolicy::ReplaceMatchingRevision => {
                let current = self.find(recipe.id(), None)?;
                if current
                    .as_ref()
                    .is_some_and(|current| current.revision() != recipe.revision())
                {
                    return Err(RecipeStoreError::OptimisticConcurrency {
                        expected: recipe.revision(),
                        actual: current.expect("checked above").revision(),
                    });
                }
                if current.is_none() {
                    return Err(RecipeStoreError::Conflict);
                }
                let transaction = self.write_transaction()?;
                {
                    let mut table = transaction
                        .open_table(schema::RECIPES_TABLE)
                        .map_err(|_| RecipeStoreError::Unavailable)?;
                    let encoded = recipe
                        .canonical_json()
                        .map_err(RecipeStoreError::InvalidRecipe)?;
                    table
                        .insert(recipe_key(&recipe).as_slice(), encoded.as_bytes())
                        .map_err(|_| RecipeStoreError::Unavailable)?;
                }
                transaction
                    .commit()
                    .map_err(|_| RecipeStoreError::CommitFailed)?;
                Ok(recipe)
            }
        }
    }

    fn current_revision(&self, id: &RecipeId) -> Result<Option<RecipeRevision>, RecipeStoreError> {
        self.find(id, None)
            .map(|recipe| recipe.map(|recipe| recipe.revision()))
    }

    fn insert_revision(
        &self,
        recipe: &ExportRecipe,
        expected: RecipeRevision,
    ) -> Result<(), RecipeStoreError> {
        let transaction = self.write_transaction()?;
        let actual = {
            let heads = transaction
                .open_table(schema::RECIPE_HEADS_TABLE)
                .map_err(|_| RecipeStoreError::Unavailable)?;
            let actual = heads
                .get(recipe.id().as_str().as_bytes())
                .map_err(|_| RecipeStoreError::Unavailable)?
                .ok_or(RecipeStoreError::Conflict)?;
            read_u64(actual.value())?
        };
        if actual != expected.get() {
            return Err(RecipeStoreError::OptimisticConcurrency {
                expected,
                actual: RecipeRevision::new(actual).ok_or(RecipeStoreError::Corrupt)?,
            });
        }
        {
            let mut recipes = transaction
                .open_table(schema::RECIPES_TABLE)
                .map_err(|_| RecipeStoreError::Unavailable)?;
            let mut heads = transaction
                .open_table(schema::RECIPE_HEADS_TABLE)
                .map_err(|_| RecipeStoreError::Unavailable)?;
            let encoded = recipe
                .canonical_json()
                .map_err(RecipeStoreError::InvalidRecipe)?;
            recipes
                .insert(recipe_key(recipe).as_slice(), encoded.as_bytes())
                .map_err(|_| RecipeStoreError::Unavailable)?;
            let revision = recipe.revision().get().to_be_bytes();
            heads
                .insert(recipe.id().as_str().as_bytes(), revision.as_slice())
                .map_err(|_| RecipeStoreError::Unavailable)?;
        }
        transaction
            .commit()
            .map_err(|_| RecipeStoreError::CommitFailed)
    }

    fn write_transaction(&self) -> Result<redb::WriteTransaction, RecipeStoreError> {
        self.database
            .begin_write()
            .map_err(|_| RecipeStoreError::Unavailable)
    }
}

fn recipe_key(recipe: &ExportRecipe) -> Vec<u8> {
    recipe_key_parts(recipe.id(), recipe.revision())
}
fn recipe_key_parts(id: &RecipeId, revision: RecipeRevision) -> Vec<u8> {
    let mut key = id.as_str().as_bytes().to_vec();
    key.push(0);
    key.extend_from_slice(&revision.get().to_be_bytes());
    key
}
fn recipe_prefix(id: &RecipeId) -> Vec<u8> {
    let mut prefix = id.as_str().as_bytes().to_vec();
    prefix.push(0);
    prefix
}
fn reference_key(id: &RecipeId, reference: &str) -> Vec<u8> {
    let mut key = id.as_str().as_bytes().to_vec();
    key.push(0);
    key.extend_from_slice(reference.as_bytes());
    key
}
fn read_u64(bytes: &[u8]) -> Result<u64, RecipeStoreError> {
    bytes
        .try_into()
        .map(u64::from_be_bytes)
        .map_err(|_| RecipeStoreError::Corrupt)
}
fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}
fn map_schema_error(error: &rusttable_catalog::RepositoryError) -> RecipeStoreError {
    match error {
        rusttable_catalog::RepositoryError::Unavailable => RecipeStoreError::Unavailable,
        rusttable_catalog::RepositoryError::CommitFailure => RecipeStoreError::CommitFailed,
        rusttable_catalog::RepositoryError::CorruptPersistedData
        | rusttable_catalog::RepositoryError::SourceConflict { .. }
        | rusttable_catalog::RepositoryError::PhotoIdConflict { .. }
        | rusttable_catalog::RepositoryError::AssetIdConflict { .. } => RecipeStoreError::Corrupt,
    }
}
