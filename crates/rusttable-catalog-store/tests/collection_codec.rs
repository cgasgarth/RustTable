use rusttable_catalog::{
    NativeCollectionRule, NativeCollectionRules, NativeCollectionSortRule, NativeCollectionSorts,
};
use rusttable_catalog_store::RedbCollectionRepository;

#[test]
fn collect_and_filtering_records_match_the_native_bytes() {
    let collect = NativeCollectionRules::collect(vec![NativeCollectionRule::collect(
        0,
        7,
        b"camera".to_vec(),
    )])
    .expect("collect rules");
    assert_eq!(
        RedbCollectionRepository::encode_native_collection_rules(&collect),
        b"1:0:7:camera$"
    );

    let filtering = NativeCollectionRules::filtering(vec![NativeCollectionRule::filtering(
        1,
        4,
        2,
        9,
        Vec::new(),
    )])
    .expect("filtering rules");
    assert_eq!(
        RedbCollectionRepository::encode_native_collection_rules(&filtering),
        b"1:1:4:2:9:%$"
    );
}

#[test]
fn empty_strings_use_percent_and_embedded_delimiters_truncate_at_the_first_dollar() {
    let decoded =
        RedbCollectionRepository::decode_native_collection_rules(false, b"2:0:3:left$right");
    assert_eq!(decoded.num_rules(), 1);
    assert_eq!(decoded.rules()[0].value(), b"left");
}

#[test]
fn zero_rule_defaults_differ_between_collect_and_filtering() {
    let collect = RedbCollectionRepository::decode_native_collection_rules(false, b"0:");
    assert_eq!(collect.num_rules(), 1);
    assert_eq!(collect.rules()[0].mode(), 0);
    assert_eq!(collect.rules()[0].item(), 0);
    assert_eq!(collect.rules()[0].value(), b"%");

    let filtering = RedbCollectionRepository::decode_native_collection_rules(true, b"0:");
    assert_eq!(filtering.num_rules(), 0);
    assert!(filtering.rules().is_empty());
}

#[test]
fn malformed_rule_stream_keeps_only_the_successful_prefix() {
    let decoded = RedbCollectionRepository::decode_native_collection_rules(
        true,
        b"3:0:1:0:0:first$broken$1:2:0:0:third$",
    );
    assert_eq!(decoded.num_rules(), 1);
    assert_eq!(decoded.rules()[0].value(), b"first");
}

#[test]
fn more_than_ten_records_are_preserved_but_query_input_is_bounded() {
    let rules = NativeCollectionRules::collect(
        (0..11)
            .map(|item| NativeCollectionRule::collect(0, item, b"value".to_vec()))
            .collect(),
    )
    .expect("eleven rules");
    let bytes = RedbCollectionRepository::encode_native_collection_rules(&rules);
    let decoded = RedbCollectionRepository::decode_native_collection_rules(false, &bytes);
    assert_eq!(decoded.num_rules(), 11);
    assert_eq!(decoded.rules().len(), 11);
    assert_eq!(decoded.query_rules().len(), 10);
}

#[test]
fn sort_records_match_native_order_and_malformed_prefix_behavior() {
    let sorts = NativeCollectionSorts::new(vec![
        NativeCollectionSortRule::new(7, 1),
        NativeCollectionSortRule::new(0, 0),
    ])
    .expect("sorts");
    assert_eq!(
        RedbCollectionRepository::encode_native_collection_sorts(&sorts),
        b"2:7:1$0:0$"
    );

    let decoded = RedbCollectionRepository::decode_native_collection_sorts(b"3:7:1$bad");
    assert_eq!(decoded.num_sort(), 1);
    assert_eq!(decoded.rules()[0].sort_id(), 7);
}

#[test]
fn checksum_uses_native_endian_ints_and_raw_string_bytes() {
    let rules = NativeCollectionRules::filtering(vec![NativeCollectionRule::filtering(
        -1,
        7,
        2,
        9,
        vec![0, b'$', 255],
    )])
    .expect("rules");
    let mut input = Vec::new();
    input.extend_from_slice(&1_i32.to_ne_bytes());
    input.extend_from_slice(&(-1_i32).to_ne_bytes());
    input.extend_from_slice(&7_i32.to_ne_bytes());
    input.extend_from_slice(&2_i32.to_ne_bytes());
    input.extend_from_slice(&9_i32.to_ne_bytes());
    input.extend_from_slice(&[0, b'$', 255]);
    let expected = md5::compute(input).0;
    assert_eq!(
        RedbCollectionRepository::native_collection_checksum(&rules),
        expected
    );
}
