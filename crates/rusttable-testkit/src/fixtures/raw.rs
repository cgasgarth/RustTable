//! Deterministic, privacy-safe camera-RAW fixtures for application smoke paths.

/// A synthetic compressed Fujifilm RAF and the stable model facts it represents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawImportFixture {
    bytes: Vec<u8>,
}

impl RawImportFixture {
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub const fn source_name(&self) -> &'static str {
        "deterministic-xpro2.raf"
    }

    #[must_use]
    pub const fn expected_width(&self) -> u32 {
        768
    }

    #[must_use]
    pub const fn expected_height(&self) -> u32 {
        6
    }

    #[must_use]
    pub const fn expected_decoder_id(&self) -> &'static str {
        "rusttable.decoder.raw.v1"
    }

    #[must_use]
    pub const fn expected_decoder_version(&self) -> u32 {
        1
    }

    #[must_use]
    pub const fn expected_decoder_implementation(&self) -> &'static str {
        "rawler-0.7.2"
    }
}

/// Builds the same bounded synthetic RAF on every invocation; no camera photo or metadata is
/// stored in the repository.
#[must_use]
pub fn deterministic_compressed_raf() -> RawImportFixture {
    let mut bytes = vec![0; 70_000];
    bytes[..16].copy_from_slice(b"FUJIFILMCCD-RAW ");
    bytes[28..36].copy_from_slice(b"FUJIFILM");
    put_u32(&mut bytes, 84, 200);
    put_u32(&mut bytes, 92, 300);
    put_u32(&mut bytes, 100, 400);

    bytes[212..220].copy_from_slice(&[b'M', b'M', 0, 42, 0, 0, 0, 8]);
    put_u16(&mut bytes, 220, 3);
    put_entry(&mut bytes, 222, 0x010f, 2, 9, 338);
    put_entry(&mut bytes, 234, 0x0110, 2, 7, 348);
    put_entry(&mut bytes, 246, 0x8769, 4, 1, 388);
    put_u32(&mut bytes, 258, 0);
    bytes[550..559].copy_from_slice(b"FUJIFILM\0");
    bytes[560..567].copy_from_slice(b"X-Pro2\0");

    put_u16(&mut bytes, 600, 1);
    put_entry(&mut bytes, 602, 0x927c, 7, 16, 488);
    bytes[700..708].copy_from_slice(b"FUJIFILM");
    put_u32(&mut bytes, 708, 0);
    put_u16(&mut bytes, 712, 0);

    put_u32(&mut bytes, 300, 1);
    put_u16(&mut bytes, 304, 0x0131);
    put_u16(&mut bytes, 306, 36);
    for (index, color) in b"RBGBRGGGRGGBGGBGGRBRGRBGGGBGGRGGRGGB"
        .iter()
        .rev()
        .enumerate()
    {
        bytes[308 + index] = match color {
            b'R' => 0,
            b'G' => 1,
            b'B' => 2,
            _ => unreachable!("synthetic CFA uses RGB only"),
        };
    }

    bytes[400..408].copy_from_slice(&[b'M', b'M', 0, 42, 0, 0, 0, 8]);
    put_u16(&mut bytes, 408, 1);
    put_entry(&mut bytes, 410, 0xf000, 4, 1, 42);
    put_u32(&mut bytes, 422, 0);

    put_u16(&mut bytes, 442, 6);
    put_entry(&mut bytes, 444, 0xf001, 4, 1, 768);
    put_entry(&mut bytes, 456, 0xf002, 4, 1, 6);
    put_entry(&mut bytes, 468, 0xf003, 3, 1, 14 << 16);
    put_entry(&mut bytes, 480, 0xf007, 4, 1, 600);
    put_entry(&mut bytes, 492, 0xf008, 4, 1, 65_568);
    put_entry(&mut bytes, 504, 0xf00e, 3, 4, 320);
    put_u32(&mut bytes, 516, 0);
    put_u16(&mut bytes, 720, 1_024);
    put_u16(&mut bytes, 722, 1_024);
    put_u16(&mut bytes, 724, 1_024);
    put_u16(&mut bytes, 726, 1_024);

    bytes[1_000..1_002].copy_from_slice(b"IS");
    bytes[1_002..1_005].copy_from_slice(&[1, 16, 14]);
    put_u16(&mut bytes, 1_005, 6);
    put_u16(&mut bytes, 1_007, 768);
    put_u16(&mut bytes, 1_009, 768);
    put_u16(&mut bytes, 1_011, 768);
    bytes[1_013] = 1;
    put_u16(&mut bytes, 1_014, 1);
    put_u32(&mut bytes, 1_016, 65_536);
    bytes[1_032..66_568].fill(0xff);

    RawImportFixture { bytes }
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn put_entry(bytes: &mut [u8], offset: usize, tag: u16, ty: u16, count: u32, value: u32) {
    put_u16(bytes, offset, tag);
    put_u16(bytes, offset + 2, ty);
    put_u32(bytes, offset + 4, count);
    put_u32(bytes, offset + 8, value);
}

#[cfg(test)]
mod tests {
    use super::deterministic_compressed_raf;

    #[test]
    fn compressed_raf_fixture_is_repeatable_and_privacy_safe() {
        let first = deterministic_compressed_raf();
        let second = deterministic_compressed_raf();

        assert_eq!(first, second);
        assert_eq!(first.bytes().len(), 70_000);
        assert_eq!(first.source_name(), "deterministic-xpro2.raf");
        assert!(!first.source_name().contains('/'));
        assert!(!first.source_name().contains('\\'));
    }
}
