use std::str::FromStr;

use rusttable_core::{AssetId, EditId, IdParseError, OperationId, PhotoId};

#[test]
fn ids_are_nonzero_and_nominally_distinct() {
    let photo = PhotoId::new(1).expect("one is nonzero");
    let asset = AssetId::new(1).expect("one is nonzero");

    assert_eq!(photo.get(), 1);
    assert_eq!(asset.get(), 1);
    assert_ne!(photo.to_string(), "00000000000000000000000000000000");
    assert_eq!(EditId::new(2).expect("two is nonzero").get(), 2);
    assert_eq!(OperationId::new(3).expect("three is nonzero").get(), 3);
    assert!(PhotoId::new(0).is_none());
}

#[test]
fn ids_display_as_canonical_lowercase_hex() {
    let id = PhotoId::new(0x1234).expect("nonzero");

    assert_eq!(id.to_string(), "00000000000000000000000000001234");
    assert_eq!(PhotoId::from_str(&id.to_string()), Ok(id));
}

#[test]
fn id_parsing_rejects_invalid_values() {
    assert_eq!(PhotoId::from_str(""), Err(IdParseError::Empty));
    assert_eq!(PhotoId::from_str("1"), Err(IdParseError::WrongLength));
    assert_eq!(
        PhotoId::from_str(&"g".repeat(32)),
        Err(IdParseError::NonHex)
    );
    assert_eq!(
        PhotoId::from_str(&"A".repeat(32)),
        Err(IdParseError::NonCanonical)
    );
    assert_eq!(PhotoId::from_str(&"0".repeat(32)), Err(IdParseError::Zero));
}

#[test]
fn each_id_type_round_trips_with_the_same_invariant() {
    let value = "0000000000000000000000000000000f";

    assert_eq!(
        PhotoId::from_str(value)
            .expect("valid photo ID")
            .to_string(),
        value
    );
    assert_eq!(
        AssetId::from_str(value)
            .expect("valid asset ID")
            .to_string(),
        value
    );
    assert_eq!(
        EditId::from_str(value).expect("valid edit ID").to_string(),
        value
    );
    assert_eq!(
        OperationId::from_str(value)
            .expect("valid operation ID")
            .to_string(),
        value
    );
}
