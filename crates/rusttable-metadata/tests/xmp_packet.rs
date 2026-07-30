use rusttable_metadata::{
    DomainValue, LanguageTag, MetadataKey, MetadataNamespace, MetadataPacketLimits, MetadataSource,
    XmpMetadataInput,
};

fn limits() -> MetadataPacketLimits {
    MetadataPacketLimits::new(16 * 1024, 16 * 1024, 256, 16, 64, 64, 4096)
        .expect("valid fixture limits")
}

fn key(namespace: MetadataNamespace, name: &str) -> MetadataKey {
    MetadataKey::new(namespace, name).expect("valid metadata key")
}

#[test]
fn xmp_fixture_preserves_typed_values_unknown_namespace_and_provenance() {
    let document = XmpMetadataInput::new(limits())
        .read_domain(include_bytes!("fixtures/canonical.xmp"))
        .expect("bounded XMP fixture parses");

    let title = document
        .get(&key(MetadataNamespace::DublinCore, "title"))
        .expect("localized title");
    let DomainValue::LanguageAlternative(title) = title.value() else {
        panic!("title should preserve language alternatives");
    };
    assert_eq!(
        title.get(&LanguageTag::new("fr-fr").expect("language tag")),
        Some("Un titre")
    );

    let subject = document
        .get(&key(MetadataNamespace::DublinCore, "subject"))
        .expect("subject bag");
    assert!(matches!(
        subject.value(),
        DomainValue::List(values) if values.len() == 2
    ));
    let rating = document
        .get(&key(MetadataNamespace::Photoshop, "Rating"))
        .expect("rating");
    assert_eq!(rating.value(), &DomainValue::Integer(5));

    let hierarchical = document
        .get(&key(
            MetadataNamespace::unknown("http://ns.adobe.com/lightroom/1.0/")
                .expect("lightroom namespace"),
            "hierarchicalSubject",
        ))
        .expect("hierarchical subject");
    assert!(matches!(hierarchical.value(), DomainValue::Keywords(_)));

    let unknown = document
        .get(&key(
            MetadataNamespace::unknown("urn:example:vendor").expect("vendor namespace"),
            "opaque",
        ))
        .expect("unknown property");
    assert!(matches!(unknown.value(), DomainValue::Structure(_)));
    assert_eq!(unknown.provenance().source(), MetadataSource::Xmp);
    assert_eq!(
        unknown
            .provenance()
            .raw()
            .expect("raw packet provenance")
            .media_type(),
        "application/rdf+xml"
    );
}

#[test]
fn xmp_entity_and_depth_limits_fail_closed() {
    let source = br#"<!DOCTYPE x [<!ENTITY bomb "expanded">]><x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description/></rdf:RDF></x:xmpmeta>"#;
    assert!(XmpMetadataInput::new(limits()).read_domain(source).is_err());
}

#[test]
fn xmp_packet_limit_fails_before_tree_construction() {
    let limits = MetadataPacketLimits::new(16 * 1024, 32, 256, 16, 64, 64, 4096)
        .expect("valid small packet limits");
    let error = XmpMetadataInput::new(limits)
        .read_domain(include_bytes!("fixtures/canonical.xmp"))
        .expect_err("fixture exceeds packet cap");
    assert!(matches!(
        error,
        rusttable_metadata::MetadataPacketError::PacketTooLarge { .. }
    ));
}
