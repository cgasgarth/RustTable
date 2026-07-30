use rusttable_metadata::{
    DomainValue, IptcMetadataInput, MetadataKey, MetadataNamespace, MetadataPacketLimits,
    MetadataSource,
};

fn limits() -> MetadataPacketLimits {
    MetadataPacketLimits::new(4096, 4096, 64, 8, 16, 16, 4096).expect("valid fixture limits")
}

fn dataset(record: u8, number: u8, value: &[u8]) -> Vec<u8> {
    let length = u16::try_from(value.len()).expect("fixture dataset length");
    let mut bytes = vec![0x1c, record, number];
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(value);
    bytes
}

fn iptc_fixture() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(dataset(1, 90, b"\x1b%G"));
    bytes.extend(dataset(2, 80, "Ada Lovelace".as_bytes()));
    bytes.extend(dataset(2, 25, "rust".as_bytes()));
    bytes.extend(dataset(2, 25, "metadata".as_bytes()));
    bytes.extend(dataset(2, 120, "A bounded caption".as_bytes()));
    bytes.extend(dataset(2, 200, &[0xde, 0xad]));
    bytes
}

fn key(name: &str) -> MetadataKey {
    MetadataKey::new(MetadataNamespace::Iptc, name).expect("valid IPTC key")
}

#[test]
fn iptc_fixture_maps_datasets_and_keeps_unknown_bytes() {
    let document = IptcMetadataInput::new(limits())
        .read_domain(&iptc_fixture())
        .expect("bounded IPTC fixture parses");

    let keywords = document.get(&key("keywords")).expect("keywords");
    assert!(matches!(
        keywords.value(),
        DomainValue::List(values) if values.len() == 2
    ));
    assert_eq!(
        document.get(&key("caption")).expect("caption").value(),
        &DomainValue::Text("A bounded caption".to_owned())
    );
    let unknown = document
        .get(&key("dataset-002-200"))
        .expect("unknown dataset");
    assert!(matches!(unknown.value(), DomainValue::Opaque(_)));
    assert_eq!(unknown.provenance().source(), MetadataSource::Iptc);
}
