use rusttable_core::{
    Asset, AssetId, AssetRole, ByteLength, ContentHash, Photo, PhotoBuildError, PhotoId, Revision,
};

fn asset(id: u128, role: AssetRole, length: u64) -> Asset {
    Asset::new(
        AssetId::new(id).expect("test IDs are nonzero"),
        role,
        ContentHash::Sha256([id.to_le_bytes()[0]; 32]),
        ByteLength::from_bytes(length),
    )
}

#[test]
fn asset_values_expose_only_their_immutable_inputs() {
    let hash = ContentHash::Sha256([7; 32]);
    let value = asset(1, AssetRole::Primary, 42);

    assert_eq!(ByteLength::ZERO.get(), 0);
    assert_eq!(ByteLength::from_bytes(u64::MAX).get(), u64::MAX);
    assert_eq!(value.id().get(), 1);
    assert_eq!(value.role(), AssetRole::Primary);
    assert_eq!(value.byte_length().get(), 42);
    assert_eq!(hash.algorithm(), rusttable_core::HashAlgorithm::Sha256);
    assert_eq!(hash.bytes(), &[7; 32]);
    assert_eq!(
        value.content_hash().algorithm(),
        rusttable_core::HashAlgorithm::Sha256
    );
}

#[test]
fn valid_photo_derives_primary_and_canonicalizes_asset_iteration() {
    let photo_id = PhotoId::new(9).expect("test ID is nonzero");
    let primary = asset(2, AssetRole::Primary, 2);
    let sidecar = asset(1, AssetRole::Sidecar, 1);
    let photo = Photo::new(photo_id, vec![primary, sidecar]).expect("valid photo");

    assert_eq!(photo.id(), photo_id);
    assert_eq!(photo.revision(), Revision::ZERO);
    assert_eq!(photo.primary_asset_id(), primary.id());
    assert_eq!(photo.primary_asset(), &primary);
    assert_eq!(photo.asset(sidecar.id()), Some(&sidecar));
    assert_eq!(
        photo.assets().map(Asset::id).collect::<Vec<_>>(),
        vec![sidecar.id(), primary.id()]
    );
}

#[test]
fn reconstruction_preserves_revision_and_value_equality_is_order_independent() {
    let photo_id = PhotoId::new(9).expect("test ID is nonzero");
    let primary = asset(2, AssetRole::Primary, 2);
    let sidecar = asset(1, AssetRole::Sidecar, 1);
    let first = Photo::from_parts(photo_id, Revision::from_u64(4), vec![primary, sidecar])
        .expect("valid photo");
    let second = Photo::from_parts(photo_id, Revision::from_u64(4), vec![sidecar, primary])
        .expect("valid photo");

    assert_eq!(first, second);
    assert_eq!(first.revision(), Revision::from_u64(4));
}

#[test]
fn photo_validation_has_deterministic_error_order() {
    let photo_id = PhotoId::new(9).expect("test ID is nonzero");
    let primary_one = asset(1, AssetRole::Primary, 1);
    let primary_two = asset(2, AssetRole::Primary, 2);

    assert!(matches!(
        Photo::new(photo_id, Vec::<Asset>::new()),
        Err(PhotoBuildError::NoAssets)
    ));
    assert!(matches!(
        Photo::new(photo_id, vec![primary_one, primary_one]),
        Err(PhotoBuildError::DuplicateAssetId { id }) if id == primary_one.id()
    ));
    assert!(matches!(
        Photo::new(photo_id, vec![asset(3, AssetRole::Sidecar, 3)]),
        Err(PhotoBuildError::MissingPrimaryAsset)
    ));

    let error = Photo::new(photo_id, vec![primary_two, primary_one])
        .expect_err("two primaries are invalid");
    assert_eq!(
        error,
        PhotoBuildError::MultiplePrimaryAssets {
            ids: vec![AssetId::new(1).unwrap(), AssetId::new(2).unwrap()]
        }
    );
}

#[test]
fn duplicate_validation_precedes_primary_validation() {
    let photo_id = PhotoId::new(9).expect("test ID is nonzero");
    let duplicate = asset(1, AssetRole::Sidecar, 1);

    assert!(matches!(
        Photo::new(photo_id, vec![duplicate, duplicate]),
        Err(PhotoBuildError::DuplicateAssetId { .. })
    ));
}
