use postcard::{from_bytes, to_allocvec};
use rusttable_catalog::{PhotoGroup, PhotoGroupId};
use rusttable_core::PhotoId;
use serde::{Deserialize, Serialize};

const PHOTO_GROUP_FORMAT_VERSION: u8 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct StoredPhotoGroup {
    version: u8,
    id: [u8; 16],
    members: Vec<[u8; 16]>,
    representative: Option<[u8; 16]>,
}

pub(crate) fn encode(group: &PhotoGroup) -> Result<Vec<u8>, ()> {
    to_allocvec(&StoredPhotoGroup {
        version: PHOTO_GROUP_FORMAT_VERSION,
        id: group.id().get().to_be_bytes(),
        members: group
            .members()
            .iter()
            .map(|photo_id| photo_id.get().to_be_bytes())
            .collect(),
        representative: group
            .representative()
            .map(|photo_id| photo_id.get().to_be_bytes()),
    })
    .map_err(|_| ())
}

pub(crate) fn decode(bytes: &[u8]) -> Result<PhotoGroup, ()> {
    let stored: StoredPhotoGroup = from_bytes(bytes).map_err(|_| ())?;
    if stored.version != PHOTO_GROUP_FORMAT_VERSION {
        return Err(());
    }
    let id = PhotoGroupId::new(u128::from_be_bytes(stored.id)).ok_or(())?;
    let members = stored
        .members
        .into_iter()
        .map(|bytes| PhotoId::new(u128::from_be_bytes(bytes)).ok_or(()))
        .collect::<Result<Vec<_>, _>>()?;
    let representative = stored
        .representative
        .map(|bytes| PhotoId::new(u128::from_be_bytes(bytes)).ok_or(()))
        .transpose()?;
    PhotoGroup::new(id, members, representative).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};
    use rusttable_catalog::{PhotoGroup, PhotoGroupId};
    use rusttable_core::PhotoId;

    #[test]
    fn group_codec_round_trips_canonical_members() {
        let group = PhotoGroup::new(
            PhotoGroupId::new(9).unwrap(),
            [PhotoId::new(3).unwrap(), PhotoId::new(1).unwrap()],
            Some(PhotoId::new(3).unwrap()),
        )
        .unwrap();
        assert_eq!(decode(&encode(&group).unwrap()).unwrap(), group);
    }
}
