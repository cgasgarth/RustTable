use std::collections::BTreeMap;

use rusttable_metadata::{
    CanonicalCodec, CanonicalField, Confidence, DatePrecision, DomainValue, MetadataDateTime,
    MetadataDocument, MetadataKey, MetadataNamespace, MetadataProvenance, MetadataRecord,
    MetadataSource, PrivacyClass, Rational, RawRepresentation,
};

fn record(namespace: MetadataNamespace, key: &str, value: DomainValue) -> MetadataRecord {
    MetadataRecord::new(
        MetadataKey::new(namespace, key).expect("valid key"),
        value,
        MetadataProvenance::new(
            MetadataSource::Exif,
            Confidence::new(100).expect("confidence"),
            PrivacyClass::Public,
        ),
    )
}

fn document_in_order(reverse: bool) -> MetadataDocument {
    let mut records = vec![
        record(
            MetadataNamespace::Xmp,
            "description",
            DomainValue::Text("Cafe\u{301}".to_owned()),
        ),
        record(
            MetadataNamespace::Exif,
            "f-number",
            DomainValue::Rational(Rational::new(50, 100).expect("rational")),
        ),
        record(
            MetadataNamespace::unknown("urn:vendor").expect("namespace"),
            "opaque",
            DomainValue::Opaque(
                RawRepresentation::new("application/vendor", vec![3, 1, 4]).expect("raw value"),
            ),
        ),
    ];
    if reverse {
        records.reverse();
    }
    MetadataDocument::from_records(records).expect("document")
}

#[test]
fn canonical_codec_is_order_independent_and_round_trips_exact_values() {
    let first = CanonicalCodec::encode(&document_in_order(false)).expect("encode");
    let second = CanonicalCodec::encode(&document_in_order(true)).expect("encode");
    assert_eq!(first, second);
    let decoded = CanonicalCodec::decode(&first).expect("decode");
    assert_eq!(decoded, document_in_order(false));
    assert!(first.starts_with(b"RustTableMetadata\0"));
}

#[test]
fn dates_preserve_precision_and_absent_timezone() {
    let date = MetadataDateTime::new(
        2024,
        2,
        29,
        3,
        4,
        5,
        123_000_000,
        None,
        DatePrecision::Millisecond,
    )
    .expect("valid local date");
    assert_eq!(date.timezone(), None);
    assert_eq!(date.precision(), DatePrecision::Millisecond);
    let document = MetadataDocument::from_records([record(
        MetadataNamespace::Exif,
        CanonicalField::CaptureDateTimeOriginal.as_key(),
        DomainValue::DateTime(date),
    )])
    .expect("document");
    let decoded = CanonicalCodec::decode(&CanonicalCodec::encode(&document).expect("encode"))
        .expect("decode");
    assert_eq!(decoded, document);
}

#[test]
fn malformed_codec_inputs_fail_closed() {
    for bytes in [
        b"".as_slice(),
        b"RustTableMetadata\0\0\x02".as_slice(),
        b"RustTableMetadata\0\0\x01\0\0\0\0\xff".as_slice(),
    ] {
        assert!(CanonicalCodec::decode(bytes).is_err(), "input: {bytes:?}");
    }
    let duplicate = MetadataDocument::from_records([
        record(MetadataNamespace::Exif, "same", DomainValue::Boolean(true)),
        record(MetadataNamespace::Exif, "same", DomainValue::Boolean(false)),
    ]);
    assert!(duplicate.is_err());
}

#[test]
fn nested_maps_are_canonically_sorted() {
    let mut fields = BTreeMap::new();
    fields.insert("z".to_owned(), DomainValue::Integer(1));
    fields.insert("a".to_owned(), DomainValue::Integer(2));
    let value = DomainValue::Structure(
        rusttable_metadata::StructuredValue::new(fields).expect("structure"),
    );
    let document = MetadataDocument::from_records([record(MetadataNamespace::Xmp, "s", value)])
        .expect("document");
    let bytes = CanonicalCodec::encode(&document).expect("encode");
    assert_eq!(CanonicalCodec::decode(&bytes).expect("decode"), document);
}
