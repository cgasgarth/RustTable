use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use rusttable_core::{
    ALL_FIELDS, MetadataField, MetadataText, MetadataTextError, Orientation, OrientationError,
    PositiveRational, PositiveRationalError,
};

fn hash<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn metadata_text_preserves_valid_utf8_without_normalization() {
    let text = MetadataText::new("  Café  ").expect("nonempty UTF-8 should work");

    assert_eq!(text.as_str(), "  Café  ");
    assert_eq!(text.byte_len(), "  Café  ".len());
}

#[test]
fn metadata_text_rejects_distinct_invalid_inputs() {
    assert_eq!(MetadataText::new(""), Err(MetadataTextError::Empty));
    assert_eq!(
        MetadataText::new("contains\0nul"),
        Err(MetadataTextError::ContainsNul)
    );
    assert_eq!(
        MetadataText::new(&"x".repeat(4_097)),
        Err(MetadataTextError::TooLong)
    );
    assert_eq!(
        MetadataText::from_bytes(vec![0xff]),
        Err(MetadataTextError::InvalidUtf8)
    );
}

#[test]
fn positive_rationals_reduce_and_hash_equal() {
    let half = PositiveRational::new(1, 2).expect("positive rational");
    let equivalent = PositiveRational::new(50, 100).expect("positive rational");

    assert_eq!(half, equivalent);
    assert_eq!(half.numerator(), 1);
    assert_eq!(half.denominator(), 2);
    assert_eq!(hash(&half), hash(&equivalent));
}

#[test]
fn positive_rationals_reject_zero_components() {
    assert_eq!(
        PositiveRational::new(0, 1),
        Err(PositiveRationalError::ZeroNumerator)
    );
    assert_eq!(
        PositiveRational::new(1, 0),
        Err(PositiveRationalError::ZeroDenominator)
    );
}

#[test]
fn orientations_round_trip_only_standard_codes() {
    for code in 1..=8 {
        let orientation = Orientation::from_u8(code).expect("standard orientation");
        assert_eq!(orientation.code(), code);
    }
    for code in [0, 9, u8::MAX] {
        assert_eq!(
            Orientation::from_u8(code),
            Err(OrientationError::InvalidCode(code))
        );
    }
}

#[test]
fn metadata_fields_have_explicit_stable_ranks() {
    assert_eq!(
        ALL_FIELDS,
        [
            MetadataField::CameraMake,
            MetadataField::CameraModel,
            MetadataField::LensModel,
            MetadataField::CaptureDateTimeOriginal,
            MetadataField::Orientation,
            MetadataField::ExposureTime,
            MetadataField::FNumber,
            MetadataField::IsoSpeed,
            MetadataField::FocalLength,
        ]
    );
    for (rank, field) in ALL_FIELDS.into_iter().enumerate() {
        assert_eq!(field.rank(), u8::try_from(rank).expect("field ranks fit"));
    }
    assert!(MetadataField::CameraMake < MetadataField::FocalLength);
}
