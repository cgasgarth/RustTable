use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::num::NonZeroU32;

use rusttable_core::{
    ImageMetadata, MetadataEntry, MetadataField, MetadataModelError, MetadataText, Orientation,
    PositiveRational,
};

fn hash<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn make_entries(reverse: bool) -> Vec<MetadataEntry> {
    let mut entries = vec![
        MetadataEntry::CameraMake(MetadataText::new("RustTable").expect("text")),
        MetadataEntry::Orientation(Orientation::from_u8(6).expect("orientation")),
        MetadataEntry::ExposureTime(PositiveRational::new(2, 4).expect("rational")),
        MetadataEntry::IsoSpeed(NonZeroU32::new(100).expect("nonzero")),
    ];
    if reverse {
        entries.reverse();
    }
    entries
}

#[test]
fn empty_metadata_is_valid_and_read_only() {
    let metadata = ImageMetadata::empty();

    assert!(metadata.is_empty());
    assert_eq!(metadata.len(), 0);
    assert_eq!(metadata.get(MetadataField::CameraMake), None);
    assert_eq!(metadata.iter().count(), 0);
}

#[test]
fn metadata_is_canonical_across_input_order_and_rational_spelling() {
    let first = ImageMetadata::from_entries(make_entries(false)).expect("unique entries");
    let second = ImageMetadata::from_entries(make_entries(true)).expect("unique entries");

    assert_eq!(first, second);
    assert_eq!(hash(&first), hash(&second));
    assert_eq!(
        first.iter().map(|(field, _)| *field).collect::<Vec<_>>(),
        vec![
            MetadataField::CameraMake,
            MetadataField::Orientation,
            MetadataField::ExposureTime,
            MetadataField::IsoSpeed,
        ]
    );
}

#[test]
fn duplicate_fields_are_rejected_with_the_exact_field() {
    let make = || MetadataEntry::CameraMake(MetadataText::new("RustTable").expect("text"));

    assert_eq!(
        ImageMetadata::from_entries([make(), make()]),
        Err(MetadataModelError::DuplicateField(
            MetadataField::CameraMake
        ))
    );
}

#[test]
fn entry_types_expose_their_matching_fields() {
    let entries = make_entries(false);

    assert_eq!(entries[0].field(), MetadataField::CameraMake);
    assert_eq!(entries[1].field(), MetadataField::Orientation);
    assert_eq!(entries[2].field(), MetadataField::ExposureTime);
    assert_eq!(entries[3].field(), MetadataField::IsoSpeed);
}

#[test]
fn capture_datetime_is_stored_as_exact_source_text() {
    let source = "2026:07:18 00:38:42+04:00";
    let entry = MetadataEntry::CaptureDateTimeOriginal(
        MetadataText::new(source).expect("source text should be valid"),
    );
    let metadata = ImageMetadata::from_entries([entry]).expect("unique entry");

    assert_eq!(
        metadata.get(MetadataField::CaptureDateTimeOriginal),
        Some(&MetadataEntry::CaptureDateTimeOriginal(
            MetadataText::new(source).expect("source text should be valid")
        ))
    );
}
