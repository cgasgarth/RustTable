mod support;

use rusttable_core::Orientation;
use rusttable_image::InputFormat;
use rusttable_metadata::{
    DomainValue, ExifMetadataInput, MetadataKey, MetadataLimits, MetadataNamespace,
};

fn key(name: &str) -> MetadataKey {
    MetadataKey::new(MetadataNamespace::Exif, name).expect("valid EXIF key")
}

#[test]
fn reads_bounded_tiff_into_typed_canonical_domain_values() {
    let input =
        ExifMetadataInput::new(MetadataLimits::new(4096, 2048, 16, 16, 4, 32, 128).unwrap());
    let document = input
        .read_domain(InputFormat::Tiff, &support::tiff_with_metadata())
        .expect("canonical EXIF document");

    assert!(matches!(
        document.get(&key("camera.make")).unwrap().value(),
        DomainValue::Text(value) if value == "Canon"
    ));
    assert!(matches!(
        document.get(&key("orientation")).unwrap().value(),
        DomainValue::Orientation(Orientation::TopLeft)
    ));
    assert!(matches!(
        document.get(&key("camera.exposure-time")).unwrap().value(),
        DomainValue::Rational(value) if value.numerator() == 1 && value.denominator() == 125
    ));
    assert_eq!(
        document
            .get(&key("camera.make"))
            .unwrap()
            .provenance()
            .source(),
        rusttable_metadata::MetadataSource::Exif
    );
}
