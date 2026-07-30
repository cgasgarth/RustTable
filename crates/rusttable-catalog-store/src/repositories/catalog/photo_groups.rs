use std::collections::BTreeSet;

use redb::{ReadableDatabase, ReadableTable, WriteTransaction};
use rusttable_catalog::{
    CatalogChangeEvent, ImportRepository, PhotoGroup, PhotoGroupCommand, PhotoGroupId,
    PhotoGroupProjection, PhotoGroupState,
};
use rusttable_core::PhotoId;

use super::RedbCatalogRepository;
use crate::schema;

impl RedbCatalogRepository {
    /// Returns every durable group in stable group-ID order.
    ///
    /// # Errors
    ///
    /// Returns a typed storage or corruption error when a group row or its
    /// reverse membership index is not internally consistent.
    pub fn photo_group_projections(
        &self,
    ) -> Result<Vec<PhotoGroupProjection>, super::AtomicCatalogStoreError> {
        Ok(self.load_photo_group_state()?.projections())
    }

    /// Finds one durable group by its stable identity.
    ///
    /// # Errors
    ///
    /// Returns a typed storage or corruption error.
    pub fn photo_group(
        &self,
        group_id: PhotoGroupId,
    ) -> Result<Option<PhotoGroup>, super::AtomicCatalogStoreError> {
        self.load_photo_group_state()
            .map(|state| state.group(group_id).cloned())
    }

    /// Finds the explicit group owning a photo, if any.
    ///
    /// # Errors
    ///
    /// Returns a typed storage or corruption error.
    pub fn photo_group_for(
        &self,
        photo_id: PhotoId,
    ) -> Result<Option<PhotoGroupId>, super::AtomicCatalogStoreError> {
        self.load_photo_group_state()
            .map(|state| state.group_for_photo(photo_id))
    }

    /// Applies one explicit group command in the same durable revision stream
    /// as organization mutations.
    ///
    /// # Errors
    ///
    /// Returns before commit on validation, storage, or hook failure. No
    /// projection or change event is published on failure.
    pub fn apply_photo_group_command(
        &mut self,
        command: &PhotoGroupCommand,
    ) -> Result<CatalogChangeEvent, super::AtomicCatalogStoreError> {
        let current = self.load_photo_group_state()?;
        let known_photos = self
            .imports
            .list()
            .map_err(|error| super::map_repository_error(&error))?
            .into_iter()
            .map(|record| record.photo().id())
            .collect::<BTreeSet<_>>();
        let mut next = current;
        let changed = next
            .apply(command.clone(), &known_photos)
            .map_err(|_| super::AtomicCatalogStoreError::Conflict)?;
        let current_revision = self.organization_revision()?;
        let next_revision = current_revision
            .checked_increment()
            .map_err(|_| super::AtomicCatalogStoreError::Conflict)?;
        let transaction = self
            .database
            .begin_write()
            .map_err(|_| super::AtomicCatalogStoreError::Unavailable)?;
        stage_photo_group_state(&transaction, &next)?;
        let mut revisions = transaction
            .open_table(schema::ORGANIZATION_REVISION_TABLE)
            .map_err(|_| super::AtomicCatalogStoreError::Unavailable)?;
        revisions
            .insert(
                schema::ORGANIZATION_REVISION_KEY,
                next_revision.get().to_be_bytes().as_slice(),
            )
            .map_err(|_| super::AtomicCatalogStoreError::Unavailable)?;
        drop(revisions);
        if let Some(hook) = &self.before_commit {
            hook()?;
        }
        transaction
            .commit()
            .map_err(|_| super::AtomicCatalogStoreError::CommitFailed)?;
        let event = CatalogChangeEvent::new(next_revision, changed);
        if let Some(listener) = &self.change_listener {
            listener(&event);
        }
        Ok(event)
    }

    pub(crate) fn prepare_import_photo_group(
        &self,
        group_id: PhotoGroupId,
        photo_id: PhotoId,
    ) -> Result<PhotoGroup, super::AtomicCatalogStoreError> {
        let state = self.load_photo_group_state()?;
        let group = state
            .group(group_id)
            .ok_or(super::AtomicCatalogStoreError::Conflict)?;
        let known = self
            .imports
            .find_by_photo_id(photo_id)
            .map_err(|error| super::map_repository_error(&error))?;
        if known.is_some() || state.group_for_photo(photo_id).is_some() {
            return Err(super::AtomicCatalogStoreError::Conflict);
        }
        group
            .add_members([photo_id])
            .map_err(|_| super::AtomicCatalogStoreError::Conflict)
    }

    pub(crate) fn stage_photo_group_membership(
        transaction: &WriteTransaction,
        group: &PhotoGroup,
        photo_id: PhotoId,
    ) -> Result<(), super::AtomicCatalogStoreError> {
        let mut groups = transaction
            .open_table(schema::PHOTO_GROUPS_TABLE)
            .map_err(|_| super::AtomicCatalogStoreError::Unavailable)?;
        let encoded = crate::photo_group_codec::encode(group)
            .map_err(|()| super::AtomicCatalogStoreError::Corrupt)?;
        groups
            .insert(
                group.id().get().to_be_bytes().as_slice(),
                encoded.as_slice(),
            )
            .map_err(|_| super::AtomicCatalogStoreError::Unavailable)?;
        drop(groups);
        let mut members = transaction
            .open_table(schema::PHOTO_GROUP_MEMBER_INDEX_TABLE)
            .map_err(|_| super::AtomicCatalogStoreError::Unavailable)?;
        members
            .insert(
                photo_id.get().to_be_bytes().as_slice(),
                group.id().get().to_be_bytes().as_slice(),
            )
            .map_err(|_| super::AtomicCatalogStoreError::Unavailable)?;
        Ok(())
    }

    fn load_photo_group_state(&self) -> Result<PhotoGroupState, super::AtomicCatalogStoreError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| super::AtomicCatalogStoreError::Unavailable)?;
        let groups = transaction
            .open_table(schema::PHOTO_GROUPS_TABLE)
            .map_err(|_| super::AtomicCatalogStoreError::Corrupt)?;
        let decoded = groups
            .iter()
            .map_err(|_| super::AtomicCatalogStoreError::Corrupt)?
            .map(|entry| {
                let (key, value) = entry.map_err(|_| super::AtomicCatalogStoreError::Corrupt)?;
                let group_id = PhotoGroupId::new(u128::from_be_bytes(
                    key.value()
                        .try_into()
                        .map_err(|_| super::AtomicCatalogStoreError::Corrupt)?,
                ))
                .ok_or(super::AtomicCatalogStoreError::Corrupt)?;
                let group = crate::photo_group_codec::decode(value.value())
                    .map_err(|()| super::AtomicCatalogStoreError::Corrupt)?;
                if group.id() != group_id {
                    return Err(super::AtomicCatalogStoreError::Corrupt);
                }
                Ok(group)
            })
            .collect::<Result<Vec<_>, _>>()?;
        drop(groups);
        let state = PhotoGroupState::from_groups(decoded)
            .map_err(|_| super::AtomicCatalogStoreError::Corrupt)?;
        let index = transaction
            .open_table(schema::PHOTO_GROUP_MEMBER_INDEX_TABLE)
            .map_err(|_| super::AtomicCatalogStoreError::Corrupt)?;
        let mut indexed = BTreeSet::new();
        for entry in index
            .iter()
            .map_err(|_| super::AtomicCatalogStoreError::Corrupt)?
        {
            let (key, value) = entry.map_err(|_| super::AtomicCatalogStoreError::Corrupt)?;
            let photo_id = PhotoId::new(u128::from_be_bytes(
                key.value()
                    .try_into()
                    .map_err(|_| super::AtomicCatalogStoreError::Corrupt)?,
            ))
            .ok_or(super::AtomicCatalogStoreError::Corrupt)?;
            let group_id = PhotoGroupId::new(u128::from_be_bytes(
                value
                    .value()
                    .try_into()
                    .map_err(|_| super::AtomicCatalogStoreError::Corrupt)?,
            ))
            .ok_or(super::AtomicCatalogStoreError::Corrupt)?;
            if state.group_for_photo(photo_id) != Some(group_id) || !indexed.insert(photo_id) {
                return Err(super::AtomicCatalogStoreError::Corrupt);
            }
        }
        let member_count = state
            .groups()
            .map(|group| group.members().len())
            .sum::<usize>();
        if indexed.len() != member_count {
            return Err(super::AtomicCatalogStoreError::Corrupt);
        }
        Ok(state)
    }
}

fn stage_photo_group_state(
    transaction: &WriteTransaction,
    state: &PhotoGroupState,
) -> Result<(), super::AtomicCatalogStoreError> {
    let mut groups = transaction
        .open_table(schema::PHOTO_GROUPS_TABLE)
        .map_err(|_| super::AtomicCatalogStoreError::Unavailable)?;
    let group_keys = groups
        .iter()
        .map_err(|_| super::AtomicCatalogStoreError::Unavailable)?
        .map(|entry| {
            entry
                .map(|(key, _)| key.value().to_vec())
                .map_err(|_| super::AtomicCatalogStoreError::Unavailable)
        })
        .collect::<Result<Vec<_>, _>>()?;
    for key in group_keys {
        groups
            .remove(key.as_slice())
            .map_err(|_| super::AtomicCatalogStoreError::Unavailable)?;
    }
    for group in state.groups() {
        let encoded = crate::photo_group_codec::encode(group)
            .map_err(|()| super::AtomicCatalogStoreError::Corrupt)?;
        groups
            .insert(
                group.id().get().to_be_bytes().as_slice(),
                encoded.as_slice(),
            )
            .map_err(|_| super::AtomicCatalogStoreError::Unavailable)?;
    }
    drop(groups);
    let mut members = transaction
        .open_table(schema::PHOTO_GROUP_MEMBER_INDEX_TABLE)
        .map_err(|_| super::AtomicCatalogStoreError::Unavailable)?;
    let member_keys = members
        .iter()
        .map_err(|_| super::AtomicCatalogStoreError::Unavailable)?
        .map(|entry| {
            entry
                .map(|(key, _)| key.value().to_vec())
                .map_err(|_| super::AtomicCatalogStoreError::Unavailable)
        })
        .collect::<Result<Vec<_>, _>>()?;
    for key in member_keys {
        members
            .remove(key.as_slice())
            .map_err(|_| super::AtomicCatalogStoreError::Unavailable)?;
    }
    for group in state.groups() {
        for photo_id in group.members() {
            members
                .insert(
                    photo_id.get().to_be_bytes().as_slice(),
                    group.id().get().to_be_bytes().as_slice(),
                )
                .map_err(|_| super::AtomicCatalogStoreError::Unavailable)?;
        }
    }
    Ok(())
}
