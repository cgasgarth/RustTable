use rusttable_core::{ImageMetadata, MetadataEntry, MetadataText};
use rusttable_metadata::{CanonicalExifOutput, EncodedExif, MetadataOutput, MetadataOutputLimits};

fn limits() -> MetadataOutputLimits {
    MetadataOutputLimits::new(4096, 10, 4096, 4096).expect("valid output limits")
}

#[test]
fn metadata_output_is_object_safe_and_bounded() {
    let output: Box<dyn MetadataOutput + Send + Sync> =
        Box::new(CanonicalExifOutput::new(limits()));
    assert_eq!(output.encode_exif(&ImageMetadata::empty()).unwrap(), None);
}

#[test]
fn encoded_exif_exposes_only_borrowed_immutable_bytes() {
    let metadata =
        ImageMetadata::from_entries([MetadataEntry::CameraMake(MetadataText::new("A").unwrap())])
            .unwrap();
    let encoded: EncodedExif = CanonicalExifOutput::new(limits())
        .encode_exif(&metadata)
        .unwrap()
        .unwrap();
    assert!(!encoded.as_bytes().is_empty());
    assert_eq!(encoded.len(), encoded.as_bytes().len());
}
