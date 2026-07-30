use rusttable_compat::{
    ColorLabel, DarktableRating, DecodeOptions, FindingCode, OrganizationDecoder,
    OrganizationLimits, Severity, SourceRowKey,
};
use rusttable_sqlite_native::{
    DarktableSchema, OrganizationRows, RawColorLabelRow, RawImageRow, RawTagAssignmentRow,
    RawTagRow,
};

fn rows() -> OrganizationRows {
    OrganizationRows {
        tags: vec![
            RawTagRow {
                source_row: 2,
                id: 8,
                name: b"parent|leaf".to_vec(),
                synonyms: Some(b"alias, autre".to_vec()),
                flags: 1 | (1_i64 << 40),
            },
            RawTagRow {
                source_row: 1,
                id: 7,
                name: b"parent".to_vec(),
                synonyms: None,
                flags: 2,
            },
        ],
        assignments: vec![
            RawTagAssignmentRow {
                source_row: 3,
                image_id: 11,
                tag_id: 8,
                position: Some(4),
            },
            RawTagAssignmentRow {
                source_row: 2,
                image_id: 11,
                tag_id: 8,
                position: Some(3),
            },
            RawTagAssignmentRow {
                source_row: 1,
                image_id: 99,
                tag_id: 404,
                position: None,
            },
        ],
        labels: vec![
            RawColorLabelRow {
                source_row: 5,
                image_id: 11,
                color: 4,
            },
            RawColorLabelRow {
                source_row: 4,
                image_id: 11,
                color: 99,
            },
            RawColorLabelRow {
                source_row: 3,
                image_id: 11,
                color: 99,
            },
        ],
        images: vec![
            RawImageRow {
                source_row: 2,
                image_id: 11,
                group_id: Some(12),
                flags: 0x08 | 5 | (1_i64 << 24),
            },
            RawImageRow {
                source_row: 1,
                image_id: 12,
                group_id: Some(11),
                flags: 6,
            },
        ],
    }
}

#[test]
fn organization_preserves_raw_values_and_reports_cross_record_failures() {
    let snapshot = OrganizationDecoder::new(DecodeOptions::default())
        .decode(DarktableSchema::new(57, 13), rows());

    assert_eq!(snapshot.tags[0].id, 7);
    assert_eq!(snapshot.tags[1].literal_name, b"parent|leaf");
    assert_eq!(snapshot.tags[1].components[1].raw, b"leaf");
    assert_eq!(snapshot.tags[1].synonyms[0].raw, b"alias");
    assert_eq!(snapshot.images[1].raw_flags, 6);
    assert_eq!(snapshot.images[1].rating, None);
    assert_eq!(snapshot.images[0].unknown_flag_bits, 1 << 24);
    assert!(
        snapshot
            .labels
            .iter()
            .any(|label| label.label == Some(ColorLabel::Purple))
    );
    assert!(
        snapshot
            .labels
            .iter()
            .any(|label| label.unknown_value && label.raw_color == 99)
    );
    assert!(
        snapshot
            .findings
            .iter()
            .any(|finding| finding.code == FindingCode::GroupCycle)
    );
    assert!(
        snapshot
            .findings
            .iter()
            .any(|finding| finding.code == FindingCode::OrphanAssignmentTag)
    );
    assert!(
        snapshot
            .findings
            .iter()
            .any(|finding| finding.code == FindingCode::DuplicateColorLabel)
    );
}

#[test]
fn physical_input_order_does_not_change_snapshot() {
    let mut shuffled = rows();
    shuffled.tags.reverse();
    shuffled.assignments.reverse();
    shuffled.labels.reverse();
    shuffled.images.reverse();
    let decoder = OrganizationDecoder::new(DecodeOptions::default());
    assert_eq!(
        decoder.decode(DarktableSchema::new(57, 13), rows()),
        decoder.decode(DarktableSchema::new(57, 13), shuffled)
    );
}

#[test]
fn bounds_emit_blocking_truncation_and_keep_no_partial_projection() {
    let options = DecodeOptions {
        limits: OrganizationLimits {
            max_tags: 1,
            ..OrganizationLimits::default()
        },
    };
    let snapshot = OrganizationDecoder::new(options).decode(DarktableSchema::new(57, 13), rows());
    assert_eq!(snapshot.tags.len(), 1);
    assert!(
        snapshot
            .findings
            .iter()
            .any(|finding| finding.code == FindingCode::LimitTruncated
                && finding.severity == Severity::Blocking)
    );
}

#[test]
fn source_row_keys_are_typed_and_stable() {
    let key = SourceRowKey::new("main.images", 12);
    assert_eq!(key, SourceRowKey::new("main.images", 12));
    assert_eq!(key.table(), "main.images");
    assert_eq!(key.row(), 12);
    assert_eq!(DarktableRating::from_bits(5), Some(DarktableRating::Five));
}
