use rusttable_metadata::{
    CanonicalCodec, Confidence, DomainValue, MetadataDocument, MetadataKey, MetadataNamespace,
    MetadataProvenance, MetadataRecord, MetadataSource, PrivacyClass, Rational,
};

fn next(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1);
    *state
}

fn generated_document(seed: u64) -> MetadataDocument {
    let mut state = seed;
    let records = (0..32)
        .map(|index| {
            let namespace = if index % 3 == 0 {
                MetadataNamespace::unknown(format!("urn:generated:{}", index % 7))
                    .expect("generated namespace")
            } else {
                MetadataNamespace::Xmp
            };
            let key = format!("key-{index:02}");
            let value = match next(&mut state) % 5 {
                0 => DomainValue::Boolean(next(&mut state).is_multiple_of(2)),
                1 => DomainValue::Integer(next(&mut state).cast_signed()),
                2 => DomainValue::Unsigned(next(&mut state)),
                3 => DomainValue::Rational(
                    Rational::new(
                        next(&mut state).cast_signed().max(1),
                        (next(&mut state) % 4095).max(1),
                    )
                    .expect("generated rational"),
                ),
                _ => DomainValue::Text(format!("value-{}", next(&mut state) % 1000)),
            };
            MetadataRecord::new(
                MetadataKey::new(namespace, key).expect("generated key"),
                value,
                MetadataProvenance::new(
                    MetadataSource::Xmp,
                    Confidence::new((next(&mut state) % 101) as u8).expect("confidence"),
                    PrivacyClass::Public,
                ),
            )
        })
        .collect::<Vec<_>>();
    MetadataDocument::from_records(records).expect("generated document")
}

#[test]
fn deterministic_generated_documents_round_trip_for_many_seeds() {
    for seed in 0..256 {
        let document = generated_document(seed);
        let bytes = CanonicalCodec::encode(&document).expect("encode");
        assert_eq!(
            CanonicalCodec::encode(&document).expect("same encode"),
            bytes
        );
        assert_eq!(CanonicalCodec::decode(&bytes).expect("decode"), document);
    }
}
