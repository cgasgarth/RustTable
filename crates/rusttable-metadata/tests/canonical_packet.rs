use rusttable_metadata::{
    CanonicalMetadataPolicy, FormatViewKind, MetadataAction, MetadataCategory,
    MetadataPacketBuilder, MetadataProperty, MetadataSource, MetadataValue,
};

fn property(
    name: &str,
    category: MetadataCategory,
    source: MetadataSource,
    value: MetadataValue,
) -> MetadataProperty {
    MetadataProperty::new("xmp", name, category, source, value)
}

#[test]
fn default_policy_fails_closed_for_sensitive_categories_and_correlated_aliases() {
    let packet = MetadataPacketBuilder::new(CanonicalMetadataPolicy::default())
        .add_property(property(
            "GPSLatitude",
            MetadataCategory::GpsLocation,
            MetadataSource::Imported,
            MetadataValue::Text("41.9".into()),
        ))
        .add_property(property(
            "GPSLongitude",
            MetadataCategory::GpsLocation,
            MetadataSource::Imported,
            MetadataValue::Text("-87.6".into()),
        ))
        .add_property(property(
            "RegionName",
            MetadataCategory::PeopleRegions,
            MetadataSource::Imported,
            MetadataValue::Text("Alice".into()),
        ))
        .add_property(property(
            "RegionBounds",
            MetadataCategory::PeopleRegions,
            MetadataSource::Imported,
            MetadataValue::Region {
                x: 1,
                y: 2,
                width: 3,
                height: 4,
            },
        ))
        .add_property(property(
            "SourceFileName",
            MetadataCategory::SourceIdentity,
            MetadataSource::Imported,
            MetadataValue::Text("/private/original.nef".into()),
        ))
        .add_property(property(
            "OperationParameters",
            MetadataCategory::EditHistory,
            MetadataSource::Imported,
            MetadataValue::Binary(vec![1, 2, 3]),
        ))
        .build()
        .expect("bounded packet");
    assert!(
        packet
            .properties()
            .iter()
            .all(|property| !property.name.contains("GPS"))
    );
    assert!(
        packet
            .properties()
            .iter()
            .all(|property| property.category == MetadataCategory::Technical)
    );
    assert!(
        !packet
            .canonical_bytes()
            .windows(b"private".len())
            .any(|window| window == b"private")
    );
}

#[test]
fn normalization_precedence_and_explicit_clear_are_deterministic() {
    let policy = CanonicalMetadataPolicy {
        gps_location: MetadataAction::Redact,
        ..Default::default()
    };
    let packet = MetadataPacketBuilder::new(policy)
        .add_property(property(
            "Caption",
            MetadataCategory::DescriptionRights,
            MetadataSource::Imported,
            MetadataValue::Text("Cafe\u{301}".into()),
        ))
        .add_property(property(
            "Caption",
            MetadataCategory::DescriptionRights,
            MetadataSource::CatalogEdit,
            MetadataValue::Text("Edited".into()),
        ))
        .add_property(property(
            "GPSReference",
            MetadataCategory::GpsLocation,
            MetadataSource::Imported,
            MetadataValue::Text("N".into()),
        ))
        .clear("xmp", "GPSReference", MetadataSource::RecipeOverride)
        .build()
        .expect("packet");
    let caption = packet
        .properties()
        .iter()
        .find(|value| value.name == "Caption")
        .expect("caption");
    assert_eq!(caption.value, MetadataValue::Text("Edited".into()));
    assert!(!packet.property_names().contains("xmp:GPSReference"));
    assert!(packet.view(FormatViewKind::Exif).is_some());
    assert!(packet.view(FormatViewKind::Jp2Boxes).is_some());
    assert_eq!(
        packet.canonical_hash(),
        MetadataPacketBuilder::new(policy)
            .add_property(property(
                "Caption",
                MetadataCategory::DescriptionRights,
                MetadataSource::CatalogEdit,
                MetadataValue::Text("Edited".into())
            ))
            .build()
            .expect("same packet")
            .canonical_hash()
    );
}
