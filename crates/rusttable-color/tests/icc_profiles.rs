use rusttable_color::{
    IccColorSpace, IccCurve, IccLut, IccParseError, IccParseErrorKind, IccProfileLimits,
    IccSignature, IccTagValue, ProfileClass, ProfileModel, parse_icc_profile,
};

const D50: [f32; 3] = [0.964_2, 1.0, 0.824_9];

#[test]
fn v2_rgb_matrix_profile_parses_all_supported_metadata_and_identities() {
    let profile = rgb_matrix_profile(2);
    let parsed = parse(&profile);

    assert_eq!(parsed.header().version().major(), 2);
    assert_eq!(parsed.header().class(), ProfileClass::Display);
    assert_eq!(parsed.header().data_color_space(), IccColorSpace::Rgb);
    assert_eq!(parsed.model(), ProfileModel::Matrix);
    assert!(parsed.header().embedded_profile_id().is_some());
    assert_eq!(parsed.identity().byte().size() as usize, profile.len());
    assert_ne!(parsed.identity().byte().sha256(), [0; 32]);
    assert_ne!(parsed.identity().semantic().sha256(), [0; 32]);
    assert_eq!(
        parsed.profile_id().sha256(),
        parsed.identity().byte().sha256()
    );
    assert!(matches!(
        parsed.tag(sig(*b"desc")).expect("description").value(),
        IccTagValue::Description(description) if description.ascii() == "Synthetic RGB"
    ));
    assert!(matches!(
        parsed.tag(sig(*b"chad")).expect("adaptation").value(),
        IccTagValue::ChromaticAdaptation(_)
    ));
    assert!(matches!(
        parsed.tag(sig(*b"cicp")).expect("CICP").value(),
        IccTagValue::Cicp(cicp) if cicp.color_primaries == 1 && cicp.full_range
    ));
    assert!(matches!(
        parsed.tag(sig(*b"view")).expect("viewing condition").value(),
        IccTagValue::ViewingCondition(view) if view.illuminant_type == 1
    ));
    let opaque = parsed.tag(sig(*b"zzzz")).expect("unknown tag");
    assert!(matches!(
        opaque.value(),
        IccTagValue::Opaque(value)
            if value.type_signature() == sig(*b"data") && value.bytes() == opaque_data()
    ));
}

#[test]
fn v4_gray_matrix_and_rgb_multistage_display_profiles_are_supported() {
    let gray = ProfileBuilder::new(4, *b"mntr", *b"GRAY", *b"XYZ ")
        .tag(*b"wtpt", xyz_tag(D50))
        .tag(*b"kTRC", parametric_gamma(2.2))
        .tag(*b"desc", mluc("en", "US", "Synthetic gray"))
        .build(true);
    let parsed_gray = parse(&gray);
    assert_eq!(parsed_gray.header().version().major(), 4);
    assert_eq!(parsed_gray.header().data_color_space(), IccColorSpace::Gray);
    assert_eq!(parsed_gray.model(), ProfileModel::Matrix);
    assert!(matches!(
        parsed_gray.tag(sig(*b"kTRC")).expect("gray curve").value(),
        IccTagValue::Curve(IccCurve::Parametric(curve)) if curve.function() == 0
    ));
    assert!(matches!(
        parsed_gray.tag(sig(*b"desc")).expect("localized description").value(),
        IccTagValue::Localized(records)
            if records[0].language() == *b"en" && records[0].text() == "Synthetic gray"
    ));

    let display_lut = ProfileBuilder::new(4, *b"mntr", *b"RGB ", *b"XYZ ")
        .tag(*b"A2B0", multi_stage_b_curves())
        .build(false);
    let parsed_lut = parse(&display_lut);
    assert_eq!(parsed_lut.model(), ProfileModel::Lut);
    assert!(matches!(
        parsed_lut.tag(sig(*b"A2B0")).expect("A to B").value(),
        IccTagValue::Lut(IccLut::MultiStage(value))
            if value.b_curves().len() == 3 && value.a_curves().is_empty()
    ));
}

#[test]
fn selected_rgb_device_link_lut8_is_bounded_and_typed() {
    let profile = ProfileBuilder::new(2, *b"link", *b"RGB ", *b"RGB ")
        .tag(*b"A2B0", lut8())
        .build(false);
    let parsed = parse(&profile);

    assert_eq!(parsed.header().class(), ProfileClass::DeviceLink);
    assert_eq!(parsed.model(), ProfileModel::Lut);
    assert!(matches!(
        parsed.tag(sig(*b"A2B0")).expect("device link LUT").value(),
        IccTagValue::Lut(IccLut::Lut8 {
            input_channels: 3,
            output_channels: 3,
            grid_points: 2,
            clut,
            ..
        }) if clut.len() == 24
    ));

    let lut16_profile = ProfileBuilder::new(4, *b"link", *b"RGB ", *b"RGB ")
        .tag(*b"A2B0", lut16())
        .build(false);
    let parsed_lut16 = parse(&lut16_profile);
    assert!(matches!(
        parsed_lut16
            .tag(sig(*b"A2B0"))
            .expect("16-bit device link LUT")
            .value(),
        IccTagValue::Lut(IccLut::Lut16 { clut, .. }) if clut.len() == 24
    ));
}

#[test]
fn byte_identity_is_exact_while_semantic_identity_ignores_layout_and_order() {
    let first = rgb_matrix_builder(4).build(false);
    let mut reordered = rgb_matrix_builder(4);
    reordered.tags.reverse();
    reordered.extra_gap = 12;
    let second = reordered.build(false);
    let first = parse(&first);
    let second = parse(&second);

    assert_ne!(first.identity().byte(), second.identity().byte());
    assert_eq!(first.identity().semantic(), second.identity().semantic());
    assert_eq!(
        parse(&rgb_matrix_profile(2)).identity(),
        parse(&rgb_matrix_profile(2)).identity()
    );
}

#[test]
fn malformed_offsets_overlaps_and_profile_ids_are_rejected_explicitly() {
    let valid = rgb_matrix_builder(4).build(false);
    let mut before_table = valid.clone();
    put_u32(&mut before_table, 136, 128);
    assert_eq!(
        parse_icc_profile(&before_table, IccProfileLimits::default()),
        Err(IccParseError::InvalidTagRange(sig(*b"rXYZ")))
    );

    let mut overlap = valid.clone();
    let first_offset = get_u32(&overlap, 136);
    put_u32(&mut overlap, 148, first_offset + 4);
    assert!(matches!(
        parse_icc_profile(&overlap, IccProfileLimits::default()),
        Err(IccParseError::OverlappingTags(_, _))
    ));

    let mut bad_id = rgb_matrix_profile(4);
    *bad_id.last_mut().expect("profile byte") ^= 1;
    assert_eq!(
        parse_icc_profile(&bad_id, IccProfileLimits::default()),
        Err(IccParseError::InvalidProfileId)
    );
}

#[test]
fn cyclic_offsets_oversized_luts_and_invalid_curves_never_become_profiles() {
    let mut cyclic_lut = multi_stage_b_curves();
    put_u32(&mut cyclic_lut, 12, 16);
    let cyclic = ProfileBuilder::new(4, *b"mntr", *b"RGB ", *b"XYZ ")
        .tag(*b"A2B0", cyclic_lut)
        .build(false);
    assert_eq!(
        parse_icc_profile(&cyclic, IccProfileLimits::default()),
        Err(IccParseError::InvalidLut(sig(*b"A2B0")))
    );

    let device_link = ProfileBuilder::new(2, *b"link", *b"RGB ", *b"RGB ")
        .tag(*b"A2B0", lut8())
        .build(false);
    let limits = IccProfileLimits {
        max_lut_samples: 20,
        ..IccProfileLimits::default()
    };
    let oversized = parse_icc_profile(&device_link, limits).expect_err("bounded LUT");
    assert_eq!(oversized.kind(), IccParseErrorKind::ResourceLimit);
    assert!(matches!(
        oversized,
        IccParseError::LutTooLarge { count: 24, .. }
    ));

    let invalid_curve = ProfileBuilder::new(4, *b"mntr", *b"GRAY", *b"XYZ ")
        .tag(*b"wtpt", xyz_tag(D50))
        .tag(*b"kTRC", sampled_curve(&[0, 50_000, 1_000]))
        .build(false);
    assert_eq!(
        parse_icc_profile(&invalid_curve, IccProfileLimits::default()),
        Err(IccParseError::InvalidCurve(sig(*b"kTRC")))
    );
}

#[test]
fn header_date_intent_illuminant_signatures_and_declared_size_are_validated() {
    let valid = rgb_matrix_builder(4).build(false);
    for (offset, mutation, expected) in [
        (28, [0_u8, 0], IccParseError::InvalidDate),
        (66, [0, 9], IccParseError::InvalidRenderingIntent(9)),
    ] {
        let mut bytes = valid.clone();
        bytes[offset..offset + 2].copy_from_slice(&mutation);
        assert_eq!(
            parse_icc_profile(&bytes, IccProfileLimits::default()),
            Err(expected)
        );
    }
    let mut illuminant = valid.clone();
    put_fixed(&mut illuminant, 68, 0.5);
    assert_eq!(
        parse_icc_profile(&illuminant, IccProfileLimits::default()),
        Err(IccParseError::InvalidIlluminant)
    );
    let mut magic = valid.clone();
    magic[36..40].copy_from_slice(b"nope");
    assert_eq!(
        parse_icc_profile(&magic, IccProfileLimits::default()),
        Err(IccParseError::InvalidHeaderSignature)
    );
    let mut size = valid;
    let declared = u32::try_from(size.len() + 4).expect("fixture size");
    put_u32(&mut size, 0, declared);
    assert!(matches!(
        parse_icc_profile(&size, IccProfileLimits::default()),
        Err(IccParseError::DeclaredSizeMismatch { .. })
    ));
}

#[test]
fn parser_is_deterministic_for_all_truncations_and_mutation_corpus() {
    let valid = rgb_matrix_builder(4).build(false);
    for end in 0..valid.len() {
        let first = parse_icc_profile(&valid[..end], IccProfileLimits::default());
        let second = parse_icc_profile(&valid[..end], IccProfileLimits::default());
        assert_eq!(first, second, "truncation at {end}");
    }
    for offset in (0..valid.len()).step_by(7) {
        let mut mutated = valid.clone();
        mutated[offset] ^= 0x5a;
        let first = parse_icc_profile(&mutated, IccProfileLimits::strict_for_embedded_images());
        let second = parse_icc_profile(&mutated, IccProfileLimits::strict_for_embedded_images());
        assert_eq!(first, second, "mutation at {offset}");
    }
}

#[test]
fn arbitrary_byte_corpus_is_panic_free_and_cannot_drive_unbounded_allocation() {
    let mut state = 0x259a_11ce_u64;
    for length in 0..512_usize {
        let mut bytes = Vec::with_capacity(length);
        for _ in 0..length {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            bytes.push(state.to_le_bytes()[0]);
        }
        let result = parse_icc_profile(&bytes, IccProfileLimits::strict_for_embedded_images());
        assert_eq!(
            result,
            parse_icc_profile(&bytes, IccProfileLimits::strict_for_embedded_images())
        );
    }
    let tiny_limit = IccProfileLimits {
        max_profile_bytes: 132,
        ..IccProfileLimits::default()
    };
    assert!(matches!(
        parse_icc_profile(&[0; 133], tiny_limit),
        Err(IccParseError::ProfileTooLarge { .. })
    ));
}

#[test]
fn unsupported_versions_classes_spaces_and_processing_types_are_distinct() {
    let version_five = ProfileBuilder::new(5, *b"mntr", *b"GRAY", *b"XYZ ")
        .tag(*b"wtpt", xyz_tag(D50))
        .tag(*b"kTRC", gamma_curve(2.2))
        .build(false);
    assert_eq!(
        parse_icc_profile(&version_five, IccProfileLimits::default())
            .expect_err("v5 is iccMAX")
            .kind(),
        IccParseErrorKind::Unsupported
    );

    let abstract_profile = ProfileBuilder::new(4, *b"abst", *b"RGB ", *b"Lab ").build(false);
    assert!(matches!(
        parse_icc_profile(&abstract_profile, IccProfileLimits::default()),
        Err(IccParseError::UnsupportedProfileClass(_))
    ));

    let cmyk = ProfileBuilder::new(4, *b"mntr", *b"CMYK", *b"XYZ ").build(false);
    assert!(matches!(
        parse_icc_profile(&cmyk, IccProfileLimits::default()),
        Err(IccParseError::UnsupportedColorSpace(_))
    ));

    let mut mpet = vec![0; 8];
    mpet[..4].copy_from_slice(b"mpet");
    let unsupported_processing = ProfileBuilder::new(4, *b"mntr", *b"RGB ", *b"XYZ ")
        .tag(*b"A2B0", mpet)
        .build(false);
    let error = parse_icc_profile(&unsupported_processing, IccProfileLimits::default())
        .expect_err("mpet execution is out of scope");
    assert_eq!(error.kind(), IccParseErrorKind::Unsupported);
    assert!(matches!(error, IccParseError::UnsupportedTagType { .. }));
}

fn parse(bytes: &[u8]) -> rusttable_color::IccProfile {
    parse_icc_profile(bytes, IccProfileLimits::default()).expect("valid synthetic ICC profile")
}

fn rgb_matrix_profile(version: u8) -> Vec<u8> {
    rgb_matrix_builder(version).build(true)
}

fn rgb_matrix_builder(version: u8) -> ProfileBuilder {
    ProfileBuilder::new(version, *b"mntr", *b"RGB ", *b"XYZ ")
        .tag(*b"rXYZ", xyz_tag([0.436_08, 0.222_5, 0.013_93]))
        .tag(*b"gXYZ", xyz_tag([0.385_07, 0.716_87, 0.097_11]))
        .tag(*b"bXYZ", xyz_tag([0.143_07, 0.060_61, 0.714_1]))
        .tag(*b"wtpt", xyz_tag(D50))
        .tag(*b"rTRC", gamma_curve(2.2))
        .tag(*b"gTRC", gamma_curve(2.2))
        .tag(*b"bTRC", gamma_curve(2.2))
        .tag(
            *b"chad",
            fixed_array_tag(*b"sf32", &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]),
        )
        .tag(*b"desc", description("Synthetic RGB"))
        .tag(*b"cicp", cicp())
        .tag(*b"view", viewing_condition())
        .tag(*b"zzzz", opaque_data())
}

#[derive(Clone)]
struct ProfileBuilder {
    version: u8,
    class: [u8; 4],
    data_space: [u8; 4],
    connection_space: [u8; 4],
    tags: Vec<([u8; 4], Vec<u8>)>,
    extra_gap: usize,
}

impl ProfileBuilder {
    fn new(version: u8, class: [u8; 4], data_space: [u8; 4], connection_space: [u8; 4]) -> Self {
        Self {
            version,
            class,
            data_space,
            connection_space,
            tags: Vec::new(),
            extra_gap: 0,
        }
    }

    fn tag(mut self, signature: [u8; 4], data: Vec<u8>) -> Self {
        self.tags.push((signature, data));
        self
    }

    fn build(self, include_id: bool) -> Vec<u8> {
        let table_end = 132 + self.tags.len() * 12;
        let mut cursor = align4(table_end + self.extra_gap);
        let mut entries = Vec::with_capacity(self.tags.len());
        for (signature, data) in &self.tags {
            entries.push((*signature, cursor, data.len()));
            cursor = align4(cursor + data.len());
        }
        let mut profile = vec![0_u8; cursor];
        let size = u32::try_from(profile.len()).expect("bounded fixture");
        put_u32(&mut profile, 0, size);
        profile[8] = self.version;
        profile[9] = 0x40;
        profile[12..16].copy_from_slice(&self.class);
        profile[16..20].copy_from_slice(&self.data_space);
        profile[20..24].copy_from_slice(&self.connection_space);
        for (offset, value) in [(24, 2026), (26, 7), (28, 22), (30, 12), (32, 34), (34, 56)] {
            put_u16(&mut profile, offset, value);
        }
        profile[36..40].copy_from_slice(b"acsp");
        profile[40..44].copy_from_slice(b"APPL");
        profile[48..52].copy_from_slice(b"TEST");
        profile[52..56].copy_from_slice(b"MODL");
        for (index, value) in D50.into_iter().enumerate() {
            put_fixed(&mut profile, 68 + index * 4, value);
        }
        profile[80..84].copy_from_slice(b"RUST");
        put_u32(
            &mut profile,
            128,
            u32::try_from(entries.len()).expect("tag count"),
        );
        for (index, ((signature, data), (_, offset, size))) in
            self.tags.into_iter().zip(entries).enumerate()
        {
            let entry = 132 + index * 12;
            profile[entry..entry + 4].copy_from_slice(&signature);
            put_u32(
                &mut profile,
                entry + 4,
                u32::try_from(offset).expect("offset"),
            );
            put_u32(
                &mut profile,
                entry + 8,
                u32::try_from(size).expect("tag size"),
            );
            profile[offset..offset + size].copy_from_slice(&data);
        }
        if include_id {
            let id = profile_md5(&profile);
            profile[84..100].copy_from_slice(&id);
        }
        profile
    }
}

fn xyz_tag(values: [f32; 3]) -> Vec<u8> {
    fixed_array_tag(*b"XYZ ", &values)
}

fn fixed_array_tag(type_signature: [u8; 4], values: &[f32]) -> Vec<u8> {
    let mut data = vec![0; 8 + values.len() * 4];
    data[..4].copy_from_slice(&type_signature);
    for (index, value) in values.iter().enumerate() {
        put_fixed(&mut data, 8 + index * 4, *value);
    }
    data
}

fn gamma_curve(gamma: f32) -> Vec<u8> {
    let mut data = vec![0; 14];
    data[..4].copy_from_slice(b"curv");
    put_u32(&mut data, 8, 1);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    put_u16(&mut data, 12, (gamma * 256.0).round() as u16);
    data
}

fn sampled_curve(samples: &[u16]) -> Vec<u8> {
    let mut data = vec![0; 12 + samples.len() * 2];
    data[..4].copy_from_slice(b"curv");
    put_u32(
        &mut data,
        8,
        u32::try_from(samples.len()).expect("sample count"),
    );
    for (index, value) in samples.iter().enumerate() {
        put_u16(&mut data, 12 + index * 2, *value);
    }
    data
}

fn parametric_gamma(gamma: f32) -> Vec<u8> {
    let mut data = vec![0; 16];
    data[..4].copy_from_slice(b"para");
    put_fixed(&mut data, 12, gamma);
    data
}

fn description(text: &str) -> Vec<u8> {
    let mut data = vec![0; 12 + text.len() + 1];
    data[..4].copy_from_slice(b"desc");
    put_u32(
        &mut data,
        8,
        u32::try_from(text.len() + 1).expect("text length"),
    );
    data[12..12 + text.len()].copy_from_slice(text.as_bytes());
    data
}

fn mluc(language: &str, country: &str, text: &str) -> Vec<u8> {
    let encoded: Vec<u16> = text.encode_utf16().collect();
    let mut data = vec![0; 28 + encoded.len() * 2];
    data[..4].copy_from_slice(b"mluc");
    put_u32(&mut data, 8, 1);
    put_u32(&mut data, 12, 12);
    data[16..18].copy_from_slice(language.as_bytes());
    data[18..20].copy_from_slice(country.as_bytes());
    put_u32(
        &mut data,
        20,
        u32::try_from(encoded.len() * 2).expect("localized length"),
    );
    put_u32(&mut data, 24, 28);
    for (index, value) in encoded.into_iter().enumerate() {
        put_u16(&mut data, 28 + index * 2, value);
    }
    data
}

fn cicp() -> Vec<u8> {
    let mut data = vec![0; 12];
    data[..4].copy_from_slice(b"cicp");
    data[8..12].copy_from_slice(&[1, 13, 0, 1]);
    data
}

fn viewing_condition() -> Vec<u8> {
    let mut data = vec![0; 36];
    data[..4].copy_from_slice(b"view");
    for (index, value) in D50.into_iter().enumerate() {
        put_fixed(&mut data, 8 + index * 4, value);
    }
    for (index, value) in [0.2, 0.2, 0.2].into_iter().enumerate() {
        put_fixed(&mut data, 20 + index * 4, value);
    }
    put_u32(&mut data, 32, 1);
    data
}

fn opaque_data() -> Vec<u8> {
    let mut data = vec![0; 12];
    data[..4].copy_from_slice(b"data");
    data[8..].copy_from_slice(b"rust");
    data
}

fn lut8() -> Vec<u8> {
    let mut data = vec![0; 48 + 3 * 256 + 24 + 3 * 256];
    data[..4].copy_from_slice(b"mft1");
    data[8..11].copy_from_slice(&[3, 3, 2]);
    for index in 0..9 {
        put_fixed(
            &mut data,
            12 + index * 4,
            if index % 4 == 0 { 1.0 } else { 0.0 },
        );
    }
    let mut cursor = 48;
    for _ in 0..3 {
        for value in 0_u8..=u8::MAX {
            data[cursor] = value;
            cursor += 1;
        }
    }
    for value in 0..24_u8 {
        data[cursor] = value.saturating_mul(10);
        cursor += 1;
    }
    for _ in 0..3 {
        for value in 0_u8..=u8::MAX {
            data[cursor] = value;
            cursor += 1;
        }
    }
    data
}

fn lut16() -> Vec<u8> {
    const VALUE_COUNT: usize = 3 * 2 + 24 + 3 * 2;
    let mut data = vec![0; 52 + VALUE_COUNT * 2];
    data[..4].copy_from_slice(b"mft2");
    data[8..11].copy_from_slice(&[3, 3, 2]);
    for index in 0..9 {
        put_fixed(
            &mut data,
            12 + index * 4,
            if index % 4 == 0 { 1.0 } else { 0.0 },
        );
    }
    put_u16(&mut data, 48, 2);
    put_u16(&mut data, 50, 2);
    for index in 0..VALUE_COUNT {
        let value = if index.is_multiple_of(2) { 0 } else { u16::MAX };
        put_u16(&mut data, 52 + index * 2, value);
    }
    data
}

fn multi_stage_b_curves() -> Vec<u8> {
    let identity = sampled_curve(&[]);
    let mut data = vec![0; 32 + identity.len() * 3];
    data[..4].copy_from_slice(b"mAB ");
    data[8..10].copy_from_slice(&[3, 3]);
    put_u32(&mut data, 12, 32);
    for index in 0..3 {
        let start = 32 + index * identity.len();
        data[start..start + identity.len()].copy_from_slice(&identity);
    }
    data
}

fn profile_md5(bytes: &[u8]) -> [u8; 16] {
    let mut context = md5::Context::new();
    context.consume(&bytes[..44]);
    context.consume([0_u8; 4]);
    context.consume(&bytes[48..64]);
    context.consume([0_u8; 4]);
    context.consume(&bytes[68..84]);
    context.consume([0_u8; 16]);
    context.consume(&bytes[100..]);
    context.finalize().0
}

fn sig(value: [u8; 4]) -> IccSignature {
    IccSignature::from_bytes(value)
}

fn align4(value: usize) -> usize {
    (value + 3) & !3
}

fn get_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().expect("fixture u32"))
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

#[allow(clippy::cast_possible_truncation)]
fn put_fixed(bytes: &mut [u8], offset: usize, value: f32) {
    let fixed = (f64::from(value) * 65_536.0).round() as i32;
    bytes[offset..offset + 4].copy_from_slice(&fixed.to_be_bytes());
}
