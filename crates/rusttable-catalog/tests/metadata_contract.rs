use rusttable_catalog::{
    CatalogMetadataCandidate, CatalogMetadataDocument, CatalogMetadataKey, CatalogMetadataPrivacy,
    CatalogMetadataProvenance, CatalogMetadataSource, CatalogMetadataValue, CatalogMetadataValues,
};
use rusttable_core::{ImageMetadata, MetadataEntry, MetadataText, PhotoId};

fn photo_id() -> PhotoId {
    PhotoId::new(7).expect("nonzero photo ID")
}

fn key() -> CatalogMetadataKey {
    CatalogMetadataKey::new("Xmp.dc", "subject").expect("valid key")
}

fn candidate(text: &str, source: CatalogMetadataSource, evidence: u8) -> CatalogMetadataCandidate {
    CatalogMetadataCandidate::new(
        CatalogMetadataValues::new(vec![CatalogMetadataValue::Text(text.to_owned())])
            .expect("values"),
        CatalogMetadataPrivacy::Public,
        CatalogMetadataProvenance::new(source, [evidence; 32]),
    )
}

#[test]
fn reconciliation_is_permutation_invariant_and_retains_conflicts() {
    let values = [
        candidate("import", CatalogMetadataSource::Imported, 1),
        candidate("recipe", CatalogMetadataSource::RecipeOverride, 2),
        candidate("edit-b", CatalogMetadataSource::CatalogEdit, 4),
        candidate("edit-a", CatalogMetadataSource::CatalogEdit, 3),
    ];
    let expected = CatalogMetadataDocument::reconcile(
        photo_id(),
        (0..4).map(|index| (key(), values[index].clone())),
    )
    .expect("reconcile");
    let mut permutation_count = 0;
    for first in 0..4 {
        for second in 0..4 {
            for third in 0..4 {
                for fourth in 0..4 {
                    let order = [first, second, third, fourth];
                    if order
                        .iter()
                        .copied()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        != 4
                    {
                        continue;
                    }
                    let actual = CatalogMetadataDocument::reconcile(
                        photo_id(),
                        order.map(|index| (key(), values[index].clone())),
                    )
                    .expect("reconcile");
                    assert_eq!(actual, expected);
                    assert_eq!(actual.canonical_sha256(), expected.canonical_sha256());
                    permutation_count += 1;
                }
            }
        }
    }
    assert_eq!(permutation_count, 24);
    let field = expected.fields().get(&key()).expect("field");
    assert_eq!(field.selected(), &values[3]);
    assert_eq!(field.conflict().expect("conflict").candidates().len(), 3);
}

#[test]
fn exact_unknown_values_and_raw_receipt_provenance_survive() {
    let decomposed = "Cafe\u{301}";
    let opaque = CatalogMetadataKey::new("vendor.future", "opaque-field").unwrap();
    let provenance = CatalogMetadataProvenance::raw_metadata_receipt([0x5a; 32]);
    let value = CatalogMetadataValue::Text(decomposed.to_owned());
    let document = CatalogMetadataDocument::reconcile(
        photo_id(),
        [(
            opaque.clone(),
            CatalogMetadataCandidate::new(
                CatalogMetadataValues::new(vec![value.clone()]).unwrap(),
                CatalogMetadataPrivacy::Private,
                provenance.clone(),
            ),
        )],
    )
    .unwrap();
    let selected = document.fields().get(&opaque).unwrap().selected();
    assert_eq!(selected.values().as_slice(), &[value]);
    assert_eq!(selected.provenance(), &provenance);
    assert_eq!(
        provenance.source(),
        CatalogMetadataSource::RawMetadataReceipt
    );
}

#[test]
fn sensitive_values_never_enter_indexes_or_diagnostics() {
    let secret = "private-place-123";
    let document = CatalogMetadataDocument::reconcile(
        photo_id(),
        [(
            CatalogMetadataKey::new("rusttable", "location").unwrap(),
            CatalogMetadataCandidate::new(
                CatalogMetadataValues::new(vec![CatalogMetadataValue::Text(secret.to_owned())])
                    .unwrap(),
                CatalogMetadataPrivacy::Sensitive,
                CatalogMetadataProvenance::new(CatalogMetadataSource::Imported, [1; 32]),
            ),
        )],
    )
    .unwrap();
    assert!(document.index_terms().is_empty());
    let diagnostic = format!("{:?}", document.diagnostic());
    assert!(!diagnostic.contains(secret));
    assert!(!format!("{document:?}").contains(secret));
    assert_eq!(document.diagnostic().field_count, 1);
}

#[test]
fn existing_core_metadata_maps_to_typed_catalog_values() {
    let metadata = ImageMetadata::from_entries([
        MetadataEntry::CameraMake(MetadataText::new("FUJIFILM").unwrap()),
        MetadataEntry::CameraModel(MetadataText::new("X-Pro2").unwrap()),
    ])
    .unwrap();
    let document = CatalogMetadataDocument::from_image_metadata(
        photo_id(),
        &metadata,
        &CatalogMetadataProvenance::raw_metadata_receipt([9; 32]),
    );
    let model = CatalogMetadataKey::new("exif", "Model").unwrap();
    assert_eq!(
        document
            .fields()
            .get(&model)
            .unwrap()
            .selected()
            .values()
            .as_slice(),
        &[CatalogMetadataValue::Text("X-Pro2".to_owned())]
    );
}
