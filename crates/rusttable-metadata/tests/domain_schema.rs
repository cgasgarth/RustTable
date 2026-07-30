use std::collections::BTreeMap;

use rusttable_metadata::{
    CanonicalField, Confidence, DomainValue, GpsCoordinate, HierarchicalKeywords,
    LanguageAlternative, LanguageTag, MetadataDocument, MetadataKey, MetadataNamespace,
    MetadataProvenance, MetadataRecord, MetadataSource, NormalizationWarning, PrivacyClass,
    Rational, RawRepresentation, StructuredValue,
};

fn provenance() -> MetadataProvenance {
    MetadataProvenance::new(
        MetadataSource::Xmp,
        Confidence::new(87).expect("confidence is bounded"),
        PrivacyClass::Public,
    )
    .with_raw(RawRepresentation::new("application/rdf+xml", vec![0, 1, 2]))
    .with_warning(NormalizationWarning::UnicodeNfc)
}

#[test]
fn canonical_fields_cover_photographic_domain_without_parser_types() {
    assert_eq!(CanonicalField::CameraMake.as_key(), "camera.make");
    assert_eq!(
        CanonicalField::CaptureDateTimeOriginal.as_key(),
        "capture.datetime"
    );
    assert_eq!(CanonicalField::GpsLatitude.as_key(), "gps.latitude");
    assert_eq!(
        CanonicalField::HierarchicalKeywords.as_key(),
        "keywords.hierarchical"
    );
    assert_eq!(CanonicalField::People.as_key(), "people");
    assert_eq!(CanonicalField::Rights.as_key(), "rights");
    assert_eq!(CanonicalField::Orientation.as_key(), "orientation");
}

#[test]
fn unknown_namespace_and_key_are_preserved_under_explicit_bounds() {
    let key = MetadataKey::new(
        MetadataNamespace::unknown("urn:example:vendor").expect("bounded namespace"),
        "Vendor:NewTag",
    )
    .expect("bounded key");
    let record = MetadataRecord::new(
        key.clone(),
        DomainValue::Opaque(
            RawRepresentation::new("application/vendor", vec![9, 8, 7]).expect("bounded raw value"),
        ),
        provenance(),
    );
    let document = MetadataDocument::from_records([record]).expect("valid document");
    let stored = document.get(&key).expect("unknown record is retained");
    assert_eq!(stored.key(), &key);
    assert_eq!(stored.provenance().source(), MetadataSource::Xmp);
    assert_eq!(
        stored.provenance().raw().expect("raw retained").bytes(),
        &[0, 1, 2]
    );
    assert_eq!(
        stored.provenance().warnings(),
        &[NormalizationWarning::UnicodeNfc]
    );
}

#[test]
fn structured_language_and_hierarchical_values_have_typed_shapes() {
    let mut alternatives = BTreeMap::new();
    alternatives.insert(
        LanguageTag::new("en-US").expect("language tag"),
        "A caption".to_owned(),
    );
    alternatives.insert(
        LanguageTag::new("fr").expect("language tag"),
        "Une légende".to_owned(),
    );
    let mut fields = BTreeMap::new();
    fields.insert("artist".to_owned(), DomainValue::Text("Ada".to_owned()));
    fields.insert("rating".to_owned(), DomainValue::Unsigned(5));
    let value =
        DomainValue::Structure(StructuredValue::new(fields).expect("bounded structured value"));
    assert!(matches!(
        value,
        DomainValue::Structure(StructuredValue { .. })
    ));
    assert_eq!(
        LanguageAlternative::new(alternatives)
            .expect("bounded alternatives")
            .get(&LanguageTag::new("en-us").expect("normalized language tag")),
        Some("A caption")
    );
    let keywords = HierarchicalKeywords::new(vec![
        vec!["Places".to_owned(), "Chicago".to_owned()],
        vec!["People".to_owned(), "Ada".to_owned()],
    ])
    .expect("hierarchical keywords");
    assert_eq!(keywords.paths()[0], &["People", "Ada"]);
}

#[test]
fn exact_rationals_and_gps_ranges_are_validated_without_float_conversion() {
    let rational = Rational::new(-50, 100).expect("nonzero denominator");
    assert_eq!(rational.numerator(), -1);
    assert_eq!(rational.denominator(), 2);
    assert_eq!(
        GpsCoordinate::new(
            Rational::new(41, 1).expect("latitude"),
            Rational::new(-87, 1).expect("longitude"),
            Some(Rational::new(-3, 2).expect("altitude")),
        )
        .expect("valid GPS"),
        GpsCoordinate::new(
            Rational::new(41, 1).expect("latitude"),
            Rational::new(-87, 1).expect("longitude"),
            Some(Rational::new(-3, 2).expect("altitude")),
        )
        .expect("same exact GPS")
    );
    assert!(
        GpsCoordinate::new(
            Rational::new(181, 1).expect("latitude"),
            Rational::new(0, 1).expect("longitude"),
            None,
        )
        .is_err()
    );
}
