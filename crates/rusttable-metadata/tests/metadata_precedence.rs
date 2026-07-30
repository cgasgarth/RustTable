use std::collections::BTreeMap;

use rusttable_metadata::{
    CanonicalField, Confidence, DatePrecision, DecisionRule, DomainValue, EvidenceDisposition,
    HierarchicalKeywords, LanguageAlternative, LanguageTag, MetadataAssertion, MetadataDateTime,
    MetadataKey, MetadataNamespace, MetadataPrecedencePolicy, MetadataProvenance, MetadataRecord,
    MetadataResolver, MetadataSource, MetadataSourceClass, PrivacyClass, ReceiptValue,
};

fn key(name: &str) -> MetadataKey {
    MetadataKey::new(MetadataNamespace::Xmp, name).expect("fixture key")
}

fn provenance(source: MetadataSource, privacy: PrivacyClass) -> MetadataProvenance {
    MetadataProvenance::new(
        source,
        Confidence::new(100).expect("fixture confidence"),
        privacy,
    )
}

fn record(
    name: &str,
    value: DomainValue,
    source: MetadataSource,
    privacy: PrivacyClass,
) -> MetadataRecord {
    MetadataRecord::new(key(name), value, provenance(source, privacy))
}

fn text(name: &str, value: &str, source: MetadataSource) -> MetadataAssertion {
    record(
        name,
        DomainValue::Text(value.to_owned()),
        source,
        PrivacyClass::Public,
    )
    .into()
}

fn effective_value<'a>(
    resolved: &'a rusttable_metadata::ResolvedMetadata,
    name: &str,
) -> &'a DomainValue {
    resolved
        .effective()
        .get(&key(name))
        .expect("effective field")
        .value()
}

fn shuffled<T: Clone>(values: &[T], seed: u64) -> Vec<T> {
    let mut values = values.to_vec();
    let mut state = seed;
    for index in (1..values.len()).rev() {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let swap = usize::try_from(state % u64::try_from(index + 1).expect("small fixture"))
            .expect("small fixture");
        values.swap(index, swap);
    }
    values
}

#[test]
fn source_priority_and_classes_are_explicit() {
    let ordered = [
        MetadataSource::ImportDefault,
        MetadataSource::Container,
        MetadataSource::Exif,
        MetadataSource::MakerNote,
        MetadataSource::Iptc,
        MetadataSource::EmbeddedXmp,
        MetadataSource::SidecarXmp,
        MetadataSource::CatalogValue,
        MetadataSource::UserOverride,
        MetadataSource::ExportOverride,
        MetadataSource::GeneratedTechnical,
    ];
    assert!(
        ordered
            .windows(2)
            .all(|sources| { sources[0].precedence() < sources[1].precedence() })
    );
    assert_eq!(
        MetadataSource::SidecarXmp.class(),
        MetadataSourceClass::Extracted
    );
    assert_eq!(
        MetadataSource::CatalogValue.class(),
        MetadataSourceClass::Catalog
    );
    assert_eq!(
        MetadataSource::UserOverride.class(),
        MetadataSourceClass::UserOverride
    );
    assert_eq!(
        MetadataSource::ExportOverride.class(),
        MetadataSourceClass::ExportOnly
    );
}

#[test]
fn resolution_is_order_independent_for_many_enumerations() {
    let caption = CanonicalField::Caption.as_key();
    let assertions = vec![
        text(caption, "EXIF caption", MetadataSource::Exif),
        text(caption, "IPTC caption", MetadataSource::Iptc),
        text(caption, "Embedded caption", MetadataSource::EmbeddedXmp),
        text(caption, "Sidecar caption", MetadataSource::SidecarXmp),
        text(caption, "Catalog caption", MetadataSource::CatalogValue),
        text(caption, "User caption", MetadataSource::UserOverride),
    ];
    let resolver = MetadataResolver::default();
    let expected = resolver.resolve(assertions.clone()).expect("baseline");
    for seed in 0..512 {
        assert_eq!(
            resolver
                .resolve(shuffled(&assertions, seed))
                .expect("permuted resolution"),
            expected,
            "seed {seed}"
        );
    }
    assert_eq!(
        effective_value(&expected, caption),
        &DomainValue::Text("User caption".to_owned())
    );
    assert_eq!(expected.retained(&key(caption)).len(), assertions.len());
}

#[test]
fn equal_values_are_not_conflicts_but_different_values_have_complete_evidence() {
    let caption = CanonicalField::Caption.as_key();
    let resolver = MetadataResolver::default();
    let equal = resolver
        .resolve([
            text(caption, "Same", MetadataSource::EmbeddedXmp),
            text(caption, "Same", MetadataSource::SidecarXmp),
        ])
        .expect("equal values");
    assert_eq!(equal.receipt().conflicts().count(), 0);
    assert_eq!(
        equal
            .receipt()
            .decisions()
            .next()
            .expect("caption decision")
            .effective_source(),
        Some(MetadataSource::SidecarXmp)
    );

    let conflict = resolver
        .resolve([
            text(caption, "Embedded", MetadataSource::EmbeddedXmp),
            text(caption, "Sidecar", MetadataSource::SidecarXmp),
        ])
        .expect("conflicting values");
    let decision = conflict
        .receipt()
        .conflicts()
        .next()
        .expect("conflict receipt");
    assert_eq!(decision.rules(), &[DecisionRule::LanguagePreference]);
    assert_eq!(decision.evidence().len(), 2);
    assert!(decision.evidence().iter().any(|evidence| {
        evidence.source() == MetadataSource::EmbeddedXmp
            && evidence.value() == &ReceiptValue::Public(DomainValue::Text("Embedded".to_owned()))
    }));
    assert!(decision.evidence().iter().any(|evidence| {
        evidence.source() == MetadataSource::SidecarXmp
            && evidence.disposition() == EvidenceDisposition::Selected
    }));
}

#[test]
fn clear_reveals_lower_priority_value_and_retains_source_records() {
    let caption = CanonicalField::Caption.as_key();
    let assertions = [
        text(caption, "Extracted", MetadataSource::EmbeddedXmp),
        text(caption, "Catalog", MetadataSource::CatalogValue),
        text(caption, "Override", MetadataSource::UserOverride),
        MetadataAssertion::clear(
            key(caption),
            provenance(MetadataSource::UserOverride, PrivacyClass::Public),
        ),
    ];
    let resolved = MetadataResolver::default()
        .resolve(assertions)
        .expect("cleared override");
    assert_eq!(
        effective_value(&resolved, caption),
        &DomainValue::Text("Catalog".to_owned())
    );
    assert_eq!(resolved.retained(&key(caption)).len(), 3);
    let decision = resolved
        .receipt()
        .decisions()
        .next()
        .expect("caption decision");
    assert!(
        decision
            .rules()
            .contains(&DecisionRule::ClearRevealsLowerPriority)
    );
    assert!(decision.evidence().iter().any(|evidence| {
        evidence.source() == MetadataSource::UserOverride
            && evidence.value() == &ReceiptValue::Clear
    }));
}

#[test]
fn private_conflicts_expose_hashes_not_values() {
    let gps = CanonicalField::GpsLatitude.as_key();
    let resolved = MetadataResolver::default()
        .resolve([
            record(
                gps,
                DomainValue::Text("41.8819".to_owned()),
                MetadataSource::Exif,
                PrivacyClass::Location,
            ),
            record(
                gps,
                DomainValue::Text("41.9000".to_owned()),
                MetadataSource::SidecarXmp,
                PrivacyClass::Location,
            ),
        ])
        .expect("private conflict");
    let decision = resolved.receipt().conflicts().next().expect("GPS conflict");
    assert!(decision.evidence().iter().all(|evidence| {
        evidence.privacy() == PrivacyClass::Location
            && matches!(evidence.value(), ReceiptValue::Sha256(_))
    }));
    assert!(!format!("{decision:?}").contains("41.8819"));
    assert!(!format!("{decision:?}").contains("41.9000"));
}

#[test]
fn invalid_high_priority_values_do_not_hide_valid_extracted_values() {
    let rating = CanonicalField::Rating.as_key();
    let resolved = MetadataResolver::default()
        .resolve([
            record(
                rating,
                DomainValue::Unsigned(4),
                MetadataSource::EmbeddedXmp,
                PrivacyClass::Public,
            ),
            record(
                rating,
                DomainValue::Text("eleven".to_owned()),
                MetadataSource::UserOverride,
                PrivacyClass::Public,
            ),
        ])
        .expect("invalid rating is evidence, not a fatal error");
    assert_eq!(
        effective_value(&resolved, rating),
        &DomainValue::Unsigned(4)
    );
    let decision = resolved
        .receipt()
        .decisions()
        .next()
        .expect("rating decision");
    assert!(decision.rules().contains(&DecisionRule::RatingMapping));
    assert!(
        decision
            .rules()
            .contains(&DecisionRule::InvalidValueIgnored)
    );
    assert!(decision.evidence().iter().any(|evidence| {
        evidence.source() == MetadataSource::UserOverride
            && evidence.disposition() == EvidenceDisposition::Invalid
    }));

    let caption = CanonicalField::Caption.as_key();
    let invalid_domain = MetadataResolver::default()
        .resolve([
            text(caption, "Extracted caption", MetadataSource::EmbeddedXmp),
            text(caption, "invalid\0caption", MetadataSource::UserOverride),
        ])
        .expect("invalid canonical text is ignored");
    assert_eq!(
        effective_value(&invalid_domain, caption),
        &DomainValue::Text("Extracted caption".to_owned())
    );
    assert!(
        invalid_domain
            .receipt()
            .decisions()
            .next()
            .expect("caption decision")
            .rules()
            .contains(&DecisionRule::InvalidValueIgnored)
    );
}

#[test]
fn date_precision_breaks_equal_priority_ties_deterministically() {
    let date = CanonicalField::CaptureDateTimeOriginal.as_key();
    let year = MetadataDateTime::new(2026, 1, 1, 0, 0, 0, 0, None, DatePrecision::Year)
        .expect("year precision");
    let second = MetadataDateTime::new(2026, 1, 1, 0, 0, 1, 0, None, DatePrecision::Second)
        .expect("second precision");
    let resolved = MetadataResolver::default()
        .resolve([
            record(
                date,
                DomainValue::DateTime(second.clone()),
                MetadataSource::Xmp,
                PrivacyClass::Public,
            ),
            record(
                date,
                DomainValue::DateTime(year),
                MetadataSource::EmbeddedXmp,
                PrivacyClass::Public,
            ),
        ])
        .expect("date resolution");
    assert_eq!(
        effective_value(&resolved, date),
        &DomainValue::DateTime(second)
    );
    assert_eq!(
        resolved
            .receipt()
            .decisions()
            .next()
            .expect("date decision")
            .rules(),
        &[DecisionRule::DatePrecision]
    );
}

#[test]
fn lists_and_hierarchies_are_unioned_and_deduplicated() {
    let keywords = CanonicalField::Keywords.as_key();
    let hierarchy = CanonicalField::HierarchicalKeywords.as_key();
    let resolved = MetadataResolver::default()
        .resolve([
            record(
                keywords,
                DomainValue::List(vec![
                    DomainValue::Text("Chicago".to_owned()),
                    DomainValue::Text("Rust".to_owned()),
                ]),
                MetadataSource::Iptc,
                PrivacyClass::Public,
            ),
            record(
                keywords,
                DomainValue::List(vec![
                    DomainValue::Text("Rust".to_owned()),
                    DomainValue::Text("Travel".to_owned()),
                ]),
                MetadataSource::SidecarXmp,
                PrivacyClass::Public,
            ),
            record(
                hierarchy,
                DomainValue::Keywords(
                    HierarchicalKeywords::new(vec![vec![
                        "Places".to_owned(),
                        "Chicago".to_owned(),
                    ]])
                    .expect("IPTC hierarchy"),
                ),
                MetadataSource::Iptc,
                PrivacyClass::Public,
            ),
            record(
                hierarchy,
                DomainValue::Keywords(
                    HierarchicalKeywords::new(vec![
                        vec!["Places".to_owned(), "Chicago".to_owned()],
                        vec!["Topics".to_owned(), "Rust".to_owned()],
                    ])
                    .expect("sidecar hierarchy"),
                ),
                MetadataSource::SidecarXmp,
                PrivacyClass::Public,
            ),
        ])
        .expect("list merges");
    let DomainValue::List(values) = effective_value(&resolved, keywords) else {
        panic!("keywords must be a list");
    };
    assert_eq!(values.len(), 3);
    let DomainValue::Keywords(values) = effective_value(&resolved, hierarchy) else {
        panic!("hierarchy must retain its canonical type");
    };
    assert_eq!(values.paths().len(), 2);
}

#[test]
fn language_preference_and_rating_label_mapping_are_explicit() {
    let caption = CanonicalField::Caption.as_key();
    let rating = CanonicalField::Rating.as_key();
    let label = CanonicalField::ColorLabel.as_key();
    let mut alternatives = BTreeMap::new();
    alternatives.insert(
        LanguageTag::new("en").expect("English"),
        "English caption".to_owned(),
    );
    alternatives.insert(
        LanguageTag::new("fr").expect("French"),
        "Légende".to_owned(),
    );
    let policy = MetadataPrecedencePolicy::default().with_preferred_languages(vec![
        LanguageTag::new("fr").expect("French"),
        LanguageTag::new("en").expect("English"),
    ]);
    let resolved = MetadataResolver::new(policy)
        .resolve([
            record(
                caption,
                DomainValue::LanguageAlternative(
                    LanguageAlternative::new(alternatives).expect("alternatives"),
                ),
                MetadataSource::EmbeddedXmp,
                PrivacyClass::Public,
            ),
            record(
                rating,
                DomainValue::Text("*****".to_owned()),
                MetadataSource::SidecarXmp,
                PrivacyClass::Public,
            ),
            record(
                label,
                DomainValue::Unsigned(3),
                MetadataSource::CatalogValue,
                PrivacyClass::Public,
            ),
        ])
        .expect("mapped metadata");
    assert_eq!(
        effective_value(&resolved, caption),
        &DomainValue::Text("Légende".to_owned())
    );
    assert_eq!(
        effective_value(&resolved, rating),
        &DomainValue::Unsigned(5)
    );
    assert_eq!(
        effective_value(&resolved, label),
        &DomainValue::Text("green".to_owned())
    );
}

#[test]
fn fixture_covers_equal_conflicting_sidecar_change_and_clear_cases() {
    let caption = CanonicalField::Caption.as_key();
    for line in include_str!("fixtures/metadata_precedence.fixture").lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let columns = line.split('|').collect::<Vec<_>>();
        assert_eq!(columns.len(), 5, "fixture row: {line}");
        let mut assertions = vec![text(caption, columns[1], MetadataSource::EmbeddedXmp)];
        if columns[2] == "<clear>" {
            assertions.push(MetadataAssertion::clear(
                key(caption),
                provenance(MetadataSource::SidecarXmp, PrivacyClass::Public),
            ));
        } else {
            assertions.push(text(caption, columns[2], MetadataSource::SidecarXmp));
        }
        let resolved = MetadataResolver::default()
            .resolve(assertions)
            .unwrap_or_else(|error| panic!("fixture {}: {error}", columns[0]));
        assert_eq!(
            effective_value(&resolved, caption),
            &DomainValue::Text(columns[3].to_owned()),
            "fixture {}",
            columns[0]
        );
        assert_eq!(
            resolved.receipt().conflicts().count() == 1,
            columns[4] == "true",
            "fixture {}",
            columns[0]
        );
    }
}
