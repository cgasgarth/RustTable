use rusttable_catalog::{
    DuplicateClassification, DuplicateEvidence, DuplicateSearchResult, EmbeddedPhotoIdentity,
    ExactContentIdentity, MAX_DUPLICATE_MATCHES, ReferencePathIdentity, VisualFingerprint,
    classify_duplicate,
};
use rusttable_core::PhotoId;

fn evidence(
    photo: u128,
    source: u8,
    content: u8,
    length: u64,
    embedded: Option<u8>,
    visual: Option<VisualFingerprint>,
) -> DuplicateEvidence {
    DuplicateEvidence::new(
        PhotoId::new(photo).unwrap(),
        ReferencePathIdentity::new([source; 32]),
        ExactContentIdentity::new([content; 32], length),
        embedded.map(|value| EmbeddedPhotoIdentity::new([value; 32])),
        visual,
    )
}

fn visual(bits: u64, width: u32, height: u32) -> VisualFingerprint {
    VisualFingerprint::new(bits, 0, width, height).unwrap()
}

#[test]
fn source_is_strongest_even_when_content_changed() {
    let candidate = evidence(9, 1, 2, 200, None, None);
    let existing = evidence(1, 1, 3, 300, None, None);

    let duplicate = classify_duplicate(candidate, existing).unwrap();

    assert_eq!(duplicate.classification(), DuplicateClassification::Source);
    assert_eq!(duplicate.confidence_millis(), 1_000);
}

#[test]
fn exact_content_requires_both_digest_and_length() {
    let candidate = evidence(9, 9, 2, 200, None, None);
    let exact = classify_duplicate(candidate, evidence(1, 1, 2, 200, None, None)).unwrap();
    let digest_collision = classify_duplicate(candidate, evidence(2, 2, 2, 201, None, None));

    assert_eq!(
        exact.classification(),
        DuplicateClassification::ExactContent
    );
    assert!(digest_collision.is_none());
}

#[test]
fn embedded_identity_matches_distinct_encodings_without_raw_metadata() {
    let candidate = evidence(9, 9, 2, 200, Some(7), None);
    let duplicate = classify_duplicate(candidate, evidence(1, 1, 3, 300, Some(7), None)).unwrap();

    assert_eq!(
        duplicate.classification(),
        DuplicateClassification::EmbeddedIdentity
    );
    assert_eq!(duplicate.visual_distance(), None);
}

#[test]
fn probable_visual_threshold_and_aspect_boundary_are_explicit() {
    let candidate = evidence(9, 9, 2, 200, None, Some(visual(0, 4_000, 3_000)));
    let at_threshold = classify_duplicate(
        candidate,
        evidence(1, 1, 3, 300, None, Some(visual(0b11_1111, 2_000, 1_500))),
    )
    .unwrap();
    let over_threshold = classify_duplicate(
        candidate,
        evidence(2, 2, 4, 400, None, Some(visual(0b111_1111, 2_000, 1_500))),
    );
    let wrong_aspect = classify_duplicate(
        candidate,
        evidence(3, 3, 5, 500, None, Some(visual(0, 1_600, 1_200 + 20))),
    );

    assert_eq!(
        at_threshold.classification(),
        DuplicateClassification::ProbableVisual
    );
    assert_eq!(at_threshold.visual_distance(), Some(6));
    assert!(over_threshold.is_none());
    assert!(wrong_aspect.is_none());
}

#[test]
fn result_deduplicates_by_strongest_class_and_orders_deterministically() {
    let candidate = evidence(99, 99, 9, 900, Some(9), Some(visual(0, 100, 100)));
    let exact = classify_duplicate(candidate, evidence(3, 3, 9, 900, None, None)).unwrap();
    let probable = classify_duplicate(
        candidate,
        evidence(1, 1, 1, 100, None, Some(visual(1, 100, 100))),
    )
    .unwrap();
    let embedded = classify_duplicate(
        candidate,
        evidence(2, 2, 2, 200, Some(9), Some(visual(0, 100, 100))),
    )
    .unwrap();
    let weaker_for_exact_photo = classify_duplicate(
        candidate,
        evidence(3, 8, 8, 800, Some(9), Some(visual(0, 100, 100))),
    )
    .unwrap();

    let result = DuplicateSearchResult::from_candidates(
        [probable, weaker_for_exact_photo, exact, embedded],
        false,
    );
    let matches = result.matches().copied().collect::<Vec<_>>();

    assert_eq!(matches.len(), 3);
    assert_eq!(
        matches[0].classification(),
        DuplicateClassification::ExactContent
    );
    assert_eq!(
        matches[1].classification(),
        DuplicateClassification::EmbeddedIdentity
    );
    assert_eq!(
        matches[2].classification(),
        DuplicateClassification::ProbableVisual
    );
}

#[test]
fn result_bound_is_reviewable() {
    let candidate = evidence(999, 99, 9, 900, None, None);
    let matches = (1..=MAX_DUPLICATE_MATCHES + 1).map(|photo| {
        classify_duplicate(
            candidate,
            evidence(
                u128::try_from(photo).unwrap(),
                u8::try_from(photo).unwrap(),
                9,
                900,
                None,
                None,
            ),
        )
        .unwrap()
    });

    let result = DuplicateSearchResult::from_candidates(matches, false);

    assert_eq!(result.matches().len(), MAX_DUPLICATE_MATCHES);
    assert!(result.truncated());
}
