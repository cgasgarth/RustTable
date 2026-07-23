use postcard::{from_bytes, to_allocvec};
use serde::{Deserialize, Serialize};

use rusttable_catalog::{
    SourceAssetIdentity, VIRTUAL_COPY_FORMAT_VERSION, VirtualCopy, VirtualCopyId,
};
use rusttable_core::{AssetId, Edit, PhotoId};

#[derive(Debug, Serialize, Deserialize)]
struct StoredVirtualCopy {
    version: u8,
    id: [u8; 16],
    source_photo_id: [u8; 16],
    source_asset_id: [u8; 16],
    order: [u8; 8],
    deleted: bool,
    current_edit: Vec<u8>,
    history: Vec<Vec<u8>>,
}

pub(crate) fn encode(copy: &VirtualCopy) -> Result<Vec<u8>, ()> {
    let history = copy
        .history()
        .map(crate::edit_codec::encode)
        .collect::<Result<Vec<_>, _>>()?;
    to_allocvec(&StoredVirtualCopy {
        version: VIRTUAL_COPY_FORMAT_VERSION,
        id: copy.id().get().to_be_bytes(),
        source_photo_id: copy.source().photo_id().get().to_be_bytes(),
        source_asset_id: copy.source().asset_id().get().to_be_bytes(),
        order: copy.order().to_be_bytes(),
        deleted: copy.is_deleted(),
        current_edit: crate::edit_codec::encode(copy.current_edit())?,
        history,
    })
    .map_err(|_| ())
}

pub(crate) fn decode(bytes: &[u8]) -> Result<VirtualCopy, ()> {
    let stored: StoredVirtualCopy = from_bytes(bytes).map_err(|_| ())?;
    if stored.version != VIRTUAL_COPY_FORMAT_VERSION {
        return Err(());
    }
    let id = VirtualCopyId::new(u128::from_be_bytes(stored.id)).ok_or(())?;
    let source = SourceAssetIdentity::new(
        PhotoId::new(u128::from_be_bytes(stored.source_photo_id)).ok_or(())?,
        AssetId::new(u128::from_be_bytes(stored.source_asset_id)).ok_or(())?,
    );
    let current_edit = crate::edit_codec::decode(&stored.current_edit)?;
    let history = stored
        .history
        .iter()
        .map(|bytes| crate::edit_codec::decode(bytes))
        .collect::<Result<Vec<Edit>, _>>()?;
    VirtualCopy::from_parts(
        id,
        source,
        u64::from_be_bytes(stored.order),
        stored.deleted,
        current_edit,
        history,
    )
    .map_err(|_| ())
}
