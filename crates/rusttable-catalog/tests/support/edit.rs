use std::collections::BTreeMap;

use rusttable_catalog::{CatalogCommand, CatalogState, EditRepository, EditRepositoryError};
use rusttable_core::{
    Asset, AssetId, AssetRole, ByteLength, ContentHash, Edit, EditId, Photo, PhotoId, Revision,
};

pub fn photo(id: u128, byte: u8) -> Photo {
    Photo::new(
        PhotoId::new(id).unwrap(),
        [Asset::new(
            AssetId::new(id).unwrap(),
            AssetRole::Primary,
            ContentHash::Sha256([byte; 32]),
            ByteLength::from_bytes(8),
        )],
    )
    .unwrap()
}

pub fn edit(id: u128, photo_id: u128, revision: u64) -> Edit {
    Edit::from_parts(
        EditId::new(id).unwrap(),
        PhotoId::new(photo_id).unwrap(),
        Revision::ZERO,
        Revision::from_u64(revision),
        [],
    )
    .unwrap()
}

pub fn state_with_photo() -> CatalogState {
    let mut state = CatalogState::new();
    state
        .apply(state.revision(), CatalogCommand::RegisterPhoto(photo(1, 1)))
        .unwrap();
    state
}

pub fn state_with_edit(revision: u64) -> CatalogState {
    let mut state = state_with_photo();
    state
        .apply(
            state.revision(),
            CatalogCommand::CreateEdit(edit(2, 1, revision)),
        )
        .unwrap();
    state
}

#[derive(Default)]
pub struct FakeEditRepository {
    pub edits: BTreeMap<EditId, Edit>,
    pub calls: Vec<&'static str>,
    pub lookup_error: Option<EditRepositoryError>,
    pub commit_error: Option<EditRepositoryError>,
}

impl EditRepository for FakeEditRepository {
    fn find_by_edit_id(&self, edit_id: EditId) -> Result<Option<Edit>, EditRepositoryError> {
        if let Some(error) = self.lookup_error.clone() {
            return Err(error);
        }
        Ok(self.edits.get(&edit_id).cloned())
    }

    fn list(&self) -> Result<Vec<Edit>, EditRepositoryError> {
        Ok(self.edits.values().cloned().collect())
    }

    fn commit_new(&mut self, edit: &Edit) -> Result<(), EditRepositoryError> {
        self.calls.push("commit_new");
        if let Some(error) = self.commit_error.clone() {
            return Err(error);
        }
        if self.edits.contains_key(&edit.id()) {
            return Err(EditRepositoryError::NewEditIdConflict { edit_id: edit.id() });
        }
        self.edits.insert(edit.id(), edit.clone());
        Ok(())
    }

    fn commit_replacement(
        &mut self,
        expected_edit_revision: Revision,
        edit: &Edit,
    ) -> Result<(), EditRepositoryError> {
        self.calls.push("commit_replacement");
        if let Some(error) = self.commit_error.clone() {
            return Err(error);
        }
        let current = self
            .edits
            .get(&edit.id())
            .ok_or(EditRepositoryError::UnknownEdit { edit_id: edit.id() })?;
        if current.revision() != expected_edit_revision {
            return Err(EditRepositoryError::EditRevisionConflict {
                edit_id: edit.id(),
                expected: expected_edit_revision,
                actual: current.revision(),
            });
        }
        self.edits.insert(edit.id(), edit.clone());
        Ok(())
    }
}
