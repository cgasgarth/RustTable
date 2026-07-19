use rusttable_testkit::fixtures::{
    ArtifactClass, Compression, FixtureDimensions, FixtureEntry, FixtureExpectation, PrivacyClass,
    QualificationError, qualify_binary,
};

fn entry(format: &str, expected: FixtureExpectation) -> FixtureEntry {
    FixtureEntry {
        id: format!("fixture.{format}"),
        path: format!("fixtures/test.{format}"),
        size: 1,
        sha256: "0000000000000000000000000000000000000000000000000000000000000000".to_owned(),
        media_type: "application/octet-stream".to_owned(),
        compression: Compression::default(),
        privacy: PrivacyClass::Synthetic,
        consumers: Vec::new(),
        consuming_issue_ranges: Vec::new(),
        artifact_class: ArtifactClass::ValidBinary,
        format: format.to_owned(),
        source: "darktable:test-fixture".to_owned(),
        generator: "rusttable-test".to_owned(),
        parser: "rusttable-testkit".to_owned(),
        seed_fixture: None,
        mutation: None,
        expected,
        allow_privacy_fields: Vec::new(),
    }
}

#[test]
fn qualifies_both_tiff_byte_orders_and_rejects_truncation() {
    for endian in [Endian::Little, Endian::Big] {
        let bytes = tiff(endian, 2, 1, 8);
        let mut fixture = entry(
            "tiff",
            FixtureExpectation {
                dimensions: Some(FixtureDimensions {
                    width: 2,
                    height: 1,
                }),
                ..Default::default()
            },
        );
        fixture.size = bytes.len() as u64;
        qualify_binary(&fixture, &bytes).expect("both TIFF byte orders should qualify");
        let truncated = &bytes[..bytes.len() - 1];
        assert!(matches!(
            qualify_binary(&fixture, truncated),
            Err(QualificationError::Truncated { .. })
        ));
    }
}

#[test]
fn rejects_malformed_icc_tag_table_and_accepts_minimal_matrix_profile() {
    let bytes = icc_profile();
    let fixture = entry(
        "icc",
        FixtureExpectation {
            metadata: vec!["profile=matrix-icc".to_owned()],
            ..Default::default()
        },
    );
    qualify_binary(&fixture, &bytes).expect("minimal ICC matrix profile should qualify");

    let mut malformed = bytes;
    malformed[136..140].copy_from_slice(&1u32.to_be_bytes());
    assert!(matches!(
        qualify_binary(&fixture, &malformed),
        Err(QualificationError::InvalidStructure { .. })
    ));
}

#[test]
fn qualifies_sqlite_xmp_and_png_semantics() {
    let mut sqlite = b"SQLite format 3\0"
        .iter()
        .copied()
        .chain([0; 4080])
        .collect::<Vec<_>>();
    sqlite[16..18].copy_from_slice(&4096u16.to_be_bytes());
    let sqlite_fixture = entry("sqlite", FixtureExpectation::default());
    qualify_binary(&sqlite_fixture, &sqlite).expect("SQLite header should qualify");

    let xmp = br#"<?xml version="1.0"?><x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF><rdf:Description xmlns:darktable="http://darktable.sf.net/" darktable:xmp_version="4"><darktable:history><rdf:Seq><rdf:li darktable:num="0"/></rdf:Seq></darktable:history></rdf:Description></rdf:RDF></x:xmpmeta>"#;
    let xmp_fixture = entry(
        "xmp",
        FixtureExpectation {
            metadata: vec!["history=simple".to_owned()],
            ..Default::default()
        },
    );
    qualify_binary(&xmp_fixture, xmp).expect("darktable XMP history should qualify");

    let png = png(2, 1, 8, 6);
    let png_fixture = entry(
        "png",
        FixtureExpectation {
            dimensions: Some(FixtureDimensions {
                width: 2,
                height: 1,
            }),
            metadata: vec!["alpha=true".to_owned(), "bits=8".to_owned()],
            ..Default::default()
        },
    );
    qualify_binary(&png_fixture, &png).expect("PNG dimensions and alpha should qualify");
}

#[derive(Clone, Copy)]
enum Endian {
    Little,
    Big,
}

fn tiff(endian: Endian, width: u16, height: u16, bits: u16) -> Vec<u8> {
    let mut bytes = Vec::new();
    let (byte_order, magic, offset) = match endian {
        Endian::Little => (b"II".as_slice(), 42u16.to_le_bytes(), 8u32.to_le_bytes()),
        Endian::Big => (b"MM".as_slice(), 42u16.to_be_bytes(), 8u32.to_be_bytes()),
    };
    bytes.extend_from_slice(byte_order);
    bytes.extend_from_slice(&magic);
    bytes.extend_from_slice(&offset);
    let count = 6u16;
    bytes.extend_from_slice(&match endian {
        Endian::Little => count.to_le_bytes(),
        Endian::Big => count.to_be_bytes(),
    });
    for (tag, value) in [
        (256u16, width),
        (257u16, height),
        (258u16, bits),
        (277u16, 1u16),
        (278u16, height),
        (279u16, 2u16),
    ] {
        bytes.extend_from_slice(&match endian {
            Endian::Little => tag.to_le_bytes(),
            Endian::Big => tag.to_be_bytes(),
        });
        bytes.extend_from_slice(&match endian {
            Endian::Little => 3u16.to_le_bytes(),
            Endian::Big => 3u16.to_be_bytes(),
        });
        bytes.extend_from_slice(&match endian {
            Endian::Little => 1u32.to_le_bytes(),
            Endian::Big => 1u32.to_be_bytes(),
        });
        bytes.extend_from_slice(&match endian {
            Endian::Little => value.to_le_bytes(),
            Endian::Big => value.to_be_bytes(),
        });
        bytes.extend_from_slice(&[0, 0]);
    }
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes.extend_from_slice(&[0x7f, 0x00]);
    bytes
}

fn icc_profile() -> Vec<u8> {
    let tag_count = 3u32;
    let table_size = 4 + tag_count as usize * 12;
    let profile_size = u32::try_from(128 + table_size + 12 * 3).expect("profile size fits");
    let mut bytes = vec![0; usize::try_from(profile_size).expect("profile size fits")];
    bytes[0..4].copy_from_slice(&profile_size.to_be_bytes());
    bytes[8..12].copy_from_slice(b"mntr");
    bytes[12..16].copy_from_slice(b"RGB ");
    bytes[16..20].copy_from_slice(b"XYZ ");
    bytes[36..40].copy_from_slice(b"acsp");
    bytes[128..132].copy_from_slice(&tag_count.to_be_bytes());
    for (index, signature) in [b"rXYZ", b"gXYZ", b"bXYZ"].iter().enumerate() {
        let start = 132 + index * 12;
        bytes[start..start + 4].copy_from_slice(*signature);
        let offset = u32::try_from(128 + table_size + index * 12).expect("tag offset fits");
        bytes[start + 4..start + 8].copy_from_slice(&offset.to_be_bytes());
        bytes[start + 8..start + 12].copy_from_slice(&12u32.to_be_bytes());
        bytes[offset as usize..offset as usize + 4].copy_from_slice(b"XYZ ");
    }
    bytes
}

fn png(width: u32, height: u32, bit_depth: u8, color_type: u8) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[bit_depth, color_type, 0, 0, 0]);
    bytes.extend_from_slice(&u32::try_from(ihdr.len()).expect("IHDR fits").to_be_bytes());
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&ihdr);
    bytes.extend_from_slice(&[0; 4]);
    bytes
}
