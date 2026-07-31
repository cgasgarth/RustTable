use rusttable_catalog::{
    NATIVE_COLLECTION_MODE_AND, NATIVE_COLLECTION_MODE_AND_NOT, NATIVE_COLLECTION_MODE_OR,
    NativeCollectionError, NativeCollectionMode, NativeCollectionRule, NativeCollectionRules,
    NativeCollectionSortRule, NativeCollectionSorts, save_native_collection_history,
};

#[test]
fn native_query_inputs_preserve_rule_order_modes_and_filter_disable_state() {
    let collect = NativeCollectionRules::collect(
        (0..11)
            .map(|index| {
                NativeCollectionRule::collect(
                    match index % 3 {
                        0 => NATIVE_COLLECTION_MODE_AND,
                        1 => NATIVE_COLLECTION_MODE_OR,
                        _ => NATIVE_COLLECTION_MODE_AND_NOT,
                    },
                    index,
                    format!("value-{index}").into_bytes(),
                )
            })
            .collect(),
    )
    .expect("collect rules");
    let query_rules = collect.query_rules();
    assert_eq!(query_rules.len(), 10);
    assert_eq!(query_rules[0].mode_kind(), NativeCollectionMode::And);
    assert_eq!(query_rules[1].mode_kind(), NativeCollectionMode::Or);
    assert_eq!(query_rules[2].mode_kind(), NativeCollectionMode::AndNot);
    assert_eq!(query_rules[9].item(), 9);

    let filtering = NativeCollectionRules::filtering(vec![
        NativeCollectionRule::filtering(1, 4, 0, 5, b"enabled".to_vec()),
        NativeCollectionRule::filtering(2, 5, 1, 6, b"disabled".to_vec()),
    ])
    .expect("filtering rules");
    let query_filters = filtering.query_rules();
    assert_eq!(query_filters.len(), 2);
    assert_eq!(query_filters[0].value(), b"enabled");
    assert_eq!(query_filters[1].value(), b"");
    assert_eq!(query_filters[1].off(), 1);
    assert_eq!(query_filters[1].top(), 6);
}

#[test]
fn native_collect_zero_rules_has_the_source_default_but_filtering_does_not() {
    let collect = NativeCollectionRules::from_parts(false, 0, Vec::new()).expect("collect zero");
    let filters = NativeCollectionRules::from_parts(true, 0, Vec::new()).expect("filter zero");

    let defaults = collect.query_rules();
    assert_eq!(defaults.len(), 1);
    assert_eq!(defaults[0].mode(), 0);
    assert_eq!(defaults[0].item(), 0);
    assert_eq!(defaults[0].value(), b"%");
    assert!(filters.query_rules().is_empty());
}

#[test]
fn native_from_parts_rejects_declared_counts_beyond_available_records() {
    let rule = NativeCollectionRule::collect(0, 1, b"value".to_vec());
    assert_eq!(
        NativeCollectionRules::from_parts(false, 2, vec![rule]),
        Err(NativeCollectionError::RulePrefixExceedsDeclaredCount)
    );

    let sort = NativeCollectionSortRule::new(3, 1);
    assert_eq!(
        NativeCollectionSorts::from_parts(2, vec![sort]),
        Err(NativeCollectionError::SortPrefixExceedsDeclaredCount)
    );
}

#[test]
fn native_history_removes_duplicates_compacts_positions_then_shifts_backwards() {
    let history = vec![
        rusttable_catalog::NativeCollectionHistoryEntry::new(b"old", 100),
        rusttable_catalog::NativeCollectionHistoryEntry::new(b"current", 101),
        rusttable_catalog::NativeCollectionHistoryEntry::new(b"other", 102),
        rusttable_catalog::NativeCollectionHistoryEntry::new(b"current", 103),
        rusttable_catalog::NativeCollectionHistoryEntry::new(b"tail", 104),
    ];
    let updated = save_native_collection_history(&history, b"current", 5, 4);

    assert_eq!(updated[0].query(), b"current");
    assert_eq!(updated[0].position(), 100);
    assert_eq!(updated[1].query(), b"old");
    assert_eq!(updated[1].position(), 100);
    assert_eq!(updated[2].query(), b"other");
    assert_eq!(updated[2].position(), 102);
    assert_eq!(updated[3].query(), b"tail");
    assert_eq!(updated[3].position(), 104);
    assert_eq!(updated[4].query(), b"");

    let unchanged = save_native_collection_history(&updated, b"current", 5, 4);
    assert_eq!(unchanged, updated);
}
