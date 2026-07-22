use super::{
    RawCameraEvidence, RawCompression, RawCompressionEvidence, RawContainerKind, RawContainerProbe,
    RawProbeEvidence, RawProbeOutcome,
};

/// Shared upper bound for camera-RAW signature and container probing.
pub const RAW_PROBE_BUDGET_BYTES: usize = 64 * 1024;
const MAX_IFDS: usize = 8;
const MAX_IFD_ENTRIES: usize = 1_024;
const MAX_EVIDENCE_TAGS: usize = 32;

#[derive(Debug, Clone, Copy, Default)]
pub struct RawContainerRegistry;

impl RawContainerRegistry {
    #[must_use]
    pub const fn standard() -> Self {
        Self
    }

    /// Probes bounded bytes with fixed RAW-before-generic precedence.
    #[must_use]
    pub fn probe_bytes(self, bytes: &[u8]) -> RawProbeOutcome {
        let bytes = &bytes[..bytes.len().min(RAW_PROBE_BUDGET_BYTES)];
        if bytes
            .windows(b"RustTable-DNG-v1".len())
            .any(|value| value == b"RustTable-DNG-v1")
        {
            return RawProbeOutcome::NoMatch;
        }
        if let Some(container) = direct_signature(bytes) {
            return direct_probe(bytes, container);
        }
        if is_classic_tiff(bytes) {
            return probe_tiff_raw(bytes);
        }
        RawProbeOutcome::NoMatch
    }
}

fn direct_signature(bytes: &[u8]) -> Option<RawContainerKind> {
    if bytes.starts_with(b"FUJIFILMCCD-RAW") || bytes.starts_with(b"Fujifilm") {
        Some(RawContainerKind::Raf)
    } else if bytes.starts_with(b"\0MRM") {
        Some(RawContainerKind::Mrw)
    } else if bytes.starts_with(b"FOVb") {
        Some(RawContainerKind::X3f)
    } else if bytes.starts_with(b"II\x1a\0\0\0HEAPCCDR") {
        Some(RawContainerKind::Crw)
    } else if bytes.get(8..10) == Some(b"CR") && bytes.get(10) == Some(&2) {
        Some(RawContainerKind::Cr2)
    } else if bytes.starts_with(b"IIRO") || bytes.starts_with(b"IIRS") || bytes.starts_with(b"MMOR")
    {
        Some(RawContainerKind::Orf)
    } else if bytes.starts_with(b"IIU\0\x08\0\0\0") {
        Some(RawContainerKind::Rw2)
    } else if bytes.get(8..12) == Some(b"IIII") {
        Some(RawContainerKind::Iiq)
    } else if is_cr3(bytes) {
        Some(RawContainerKind::Cr3)
    } else {
        None
    }
}

fn direct_probe(bytes: &[u8], container: RawContainerKind) -> RawProbeOutcome {
    let minimum = match container {
        RawContainerKind::Raf => 104,
        RawContainerKind::Cr3 => 24,
        RawContainerKind::Crw => 14,
        RawContainerKind::Cr2 => 16,
        RawContainerKind::Rw2 => 8,
        _ => 4,
    };
    if bytes.len() < minimum {
        return malformed(container, "recognized RAW signature is truncated");
    }
    let compression = match container {
        RawContainerKind::Raf => raf_compression(bytes),
        RawContainerKind::Cr3 => RawCompression::CanonCrx,
        _ => RawCompression::Unknown,
    };
    let mut camera = RawCameraEvidence {
        maker: None,
        model: None,
    };
    let mut bit_depth = None;
    let mut raw_tags = Vec::new();
    if container == RawContainerKind::Raf {
        camera.maker = Some("FUJIFILM".to_owned());
        bit_depth = raf_bit_depth(bytes);
        if let Some(tiff_offset) = read_u32(bytes, 84, Endian::Big)
            && let Ok(summary) = inspect_tiff_at(
                bytes,
                usize::try_from(tiff_offset)
                    .unwrap_or(usize::MAX)
                    .saturating_add(12),
            )
        {
            camera.model = summary.model;
            raw_tags = summary.raw_tags;
            bit_depth = summary.bit_depth.or(bit_depth);
        }
    }
    RawProbeOutcome::Match(RawContainerProbe {
        container,
        evidence: RawProbeEvidence {
            signature: bytes.iter().copied().take(24).collect(),
            raw_tags,
            camera,
            compression: RawCompressionEvidence {
                compression,
                container_code: None,
            },
            bit_depth,
        },
    })
}

fn raf_bit_depth(bytes: &[u8]) -> Option<u8> {
    bytes.windows(12).find_map(|entry| {
        if entry[..8] == [0xf0, 0x03, 0, 3, 0, 0, 0, 1] {
            u8::try_from(u16::from_be_bytes([entry[8], entry[9]])).ok()
        } else {
            None
        }
    })
}

fn raf_compression(bytes: &[u8]) -> RawCompression {
    let width = raf_u32_tag(bytes, 0xf001);
    let height = raf_u32_tag(bytes, 0xf002);
    let bit_depth = raf_u16_tag(bytes, 0xf003).map(u64::from);
    let stored_bytes = raf_u32_tag(bytes, 0xf008).map(u64::from);
    match (width, height, bit_depth, stored_bytes) {
        (Some(width), Some(height), Some(bit_depth), Some(stored_bytes)) => {
            let unpacked = u64::from(width)
                .checked_mul(u64::from(height))
                .and_then(|pixels| pixels.checked_mul(bit_depth))
                .map(|bits| bits.div_ceil(8));
            if unpacked.is_some_and(|unpacked| stored_bytes < unpacked) {
                RawCompression::FujiCompressed
            } else {
                RawCompression::Uncompressed
            }
        }
        _ => RawCompression::Unknown,
    }
}

fn raf_u32_tag(bytes: &[u8], tag: u16) -> Option<u32> {
    let tag = tag.to_be_bytes();
    bytes.windows(12).find_map(|entry| {
        if entry[..2] == tag && entry[2..8] == [0, 4, 0, 0, 0, 1] {
            Some(u32::from_be_bytes(entry[8..12].try_into().ok()?))
        } else {
            None
        }
    })
}

fn raf_u16_tag(bytes: &[u8], tag: u16) -> Option<u16> {
    let tag = tag.to_be_bytes();
    bytes.windows(12).find_map(|entry| {
        if entry[..2] == tag && entry[2..8] == [0, 3, 0, 0, 0, 1] {
            Some(u16::from_be_bytes([entry[8], entry[9]]))
        } else {
            None
        }
    })
}

fn is_cr3(bytes: &[u8]) -> bool {
    if bytes.get(4..8) != Some(b"ftyp") || bytes.len() < 16 {
        return false;
    }
    bytes.get(8..12) == Some(b"crx ")
        || bytes[16..bytes.len().min(64)]
            .as_chunks::<4>()
            .0
            .iter()
            .any(|brand| brand == b"crx ")
}

fn is_classic_tiff(bytes: &[u8]) -> bool {
    bytes.starts_with(&[b'I', b'I', 42, 0]) || bytes.starts_with(&[b'M', b'M', 0, 42])
}

fn probe_tiff_raw(bytes: &[u8]) -> RawProbeOutcome {
    let Ok(summary) = inspect_tiff_at(bytes, 0) else {
        return RawProbeOutcome::NoMatch;
    };
    if !summary.proves_raw() {
        return RawProbeOutcome::NoMatch;
    }
    let container = summary.container();
    RawProbeOutcome::Match(RawContainerProbe {
        container,
        evidence: RawProbeEvidence {
            signature: bytes.iter().copied().take(16).collect(),
            raw_tags: summary.raw_tags,
            camera: RawCameraEvidence {
                maker: summary.maker,
                model: summary.model,
            },
            compression: RawCompressionEvidence {
                compression: compression_from_code(summary.compression),
                container_code: summary.compression,
            },
            bit_depth: summary.bit_depth,
        },
    })
}

fn malformed(container: RawContainerKind, message: &str) -> RawProbeOutcome {
    RawProbeOutcome::MalformedRecognized {
        container,
        message: message.to_owned(),
    }
}

#[derive(Debug, Clone, Copy)]
enum Endian {
    Little,
    Big,
}

#[derive(Debug, Default)]
struct TiffRawSummary {
    maker: Option<String>,
    model: Option<String>,
    raw_tags: Vec<u16>,
    compression: Option<u32>,
    bit_depth: Option<u8>,
}

impl TiffRawSummary {
    fn proves_raw(&self) -> bool {
        self.raw_tags.iter().any(|tag| {
            matches!(
                tag,
                33421 | 33422 | 50706 | 50710 | 50711 | 50713 | 50714 | 50717 | 50829 | 50830
            )
        })
    }

    fn container(&self) -> RawContainerKind {
        if self.raw_tags.contains(&50706) {
            return RawContainerKind::Dng;
        }
        match self
            .maker
            .as_deref()
            .unwrap_or_default()
            .to_ascii_uppercase()
        {
            maker if maker.contains("CANON") => RawContainerKind::Cr2,
            maker if maker.contains("NIKON") => RawContainerKind::Nef,
            maker if maker.contains("SONY") => RawContainerKind::Arw,
            maker if maker.contains("OLYMPUS") || maker.contains("OM DIGITAL") => {
                RawContainerKind::Orf
            }
            maker if maker.contains("PANASONIC") || maker.contains("LEICA") => {
                RawContainerKind::Rw2
            }
            maker if maker.contains("PENTAX") || maker.contains("RICOH") => RawContainerKind::Pef,
            maker if maker.contains("SAMSUNG") => RawContainerKind::Srw,
            maker if maker.contains("EPSON") => RawContainerKind::Erf,
            maker if maker.contains("PHASE ONE") || maker.contains("LEAF") => RawContainerKind::Iiq,
            _ => RawContainerKind::TiffRaw,
        }
    }
}

fn inspect_tiff_at(bytes: &[u8], base: usize) -> Result<TiffRawSummary, ()> {
    let order = match bytes.get(base..base.saturating_add(2)) {
        Some([b'I', b'I']) => Endian::Little,
        Some([b'M', b'M']) => Endian::Big,
        _ => return Err(()),
    };
    if read_u16(bytes, base.saturating_add(2), order) != Some(42) {
        return Err(());
    }
    let first = read_u32(bytes, base.saturating_add(4), order).ok_or(())?;
    let mut pending = vec![usize::try_from(first).map_err(|_| ())?];
    let mut summary = TiffRawSummary::default();
    let mut visited = Vec::new();
    while let Some(relative) = pending.pop() {
        if relative == 0 || visited.contains(&relative) || visited.len() >= MAX_IFDS {
            continue;
        }
        visited.push(relative);
        let offset = base.checked_add(relative).ok_or(())?;
        let count = usize::from(read_u16(bytes, offset, order).ok_or(())?);
        if count > MAX_IFD_ENTRIES {
            return Err(());
        }
        let entries_start = offset.checked_add(2).ok_or(())?;
        let entries_bytes = count.checked_mul(12).ok_or(())?;
        let entries_end = entries_start.checked_add(entries_bytes).ok_or(())?;
        if entries_end
            .checked_add(4)
            .is_none_or(|end| end > bytes.len())
        {
            return Err(());
        }
        for index in 0..count {
            let entry = entries_start + index * 12;
            let tag = read_u16(bytes, entry, order).ok_or(())?;
            let ty = read_u16(bytes, entry + 2, order).ok_or(())?;
            let value_count = read_u32(bytes, entry + 4, order).ok_or(())?;
            if is_raw_evidence_tag(tag) && summary.raw_tags.len() < MAX_EVIDENCE_TAGS {
                summary.raw_tags.push(tag);
            }
            match tag {
                258 => {
                    summary.bit_depth =
                        read_first_unsigned(bytes, base, entry, ty, value_count, order)
                            .and_then(|value| u8::try_from(value).ok());
                }
                259 => {
                    summary.compression =
                        read_first_unsigned(bytes, base, entry, ty, value_count, order);
                }
                271 => summary.maker = read_ascii(bytes, base, entry, ty, value_count, order),
                272 => summary.model = read_ascii(bytes, base, entry, ty, value_count, order),
                330 => {
                    for value in read_unsigned_values(bytes, base, entry, ty, value_count, order)
                        .unwrap_or_default()
                    {
                        if let Ok(value) = usize::try_from(value) {
                            pending.push(value);
                        }
                    }
                }
                _ => {}
            }
        }
        let next = read_u32(bytes, entries_end, order).ok_or(())?;
        if let Ok(next) = usize::try_from(next) {
            pending.push(next);
        }
    }
    summary.raw_tags.sort_unstable();
    summary.raw_tags.dedup();
    Ok(summary)
}

fn is_raw_evidence_tag(tag: u16) -> bool {
    matches!(
        tag,
        33421 | 33422 | 50706 | 50710 | 50711 | 50713 | 50714 | 50717 | 50829 | 50830
    )
}

fn read_ascii(
    bytes: &[u8],
    base: usize,
    entry: usize,
    ty: u16,
    count: u32,
    order: Endian,
) -> Option<String> {
    if ty != 2 || count == 0 || count > 256 {
        return None;
    }
    let count = usize::try_from(count).ok()?;
    let data = entry_data(bytes, base, entry, ty, count, order)?;
    let end = data
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(data.len());
    let text = String::from_utf8_lossy(&data[..end]).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn read_first_unsigned(
    bytes: &[u8],
    base: usize,
    entry: usize,
    ty: u16,
    count: u32,
    order: Endian,
) -> Option<u32> {
    read_unsigned_values(bytes, base, entry, ty, count, order)?
        .first()
        .copied()
}

fn read_unsigned_values(
    bytes: &[u8],
    base: usize,
    entry: usize,
    ty: u16,
    count: u32,
    order: Endian,
) -> Option<Vec<u32>> {
    if count == 0 || count > 32 || !matches!(ty, 1 | 3 | 4) {
        return None;
    }
    let count = usize::try_from(count).ok()?;
    let data = entry_data(bytes, base, entry, ty, count, order)?;
    match ty {
        1 => Some(data.iter().map(|value| u32::from(*value)).collect()),
        3 => data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|value| read_u16(value, 0, order).map(u32::from))
            .collect(),
        4 => data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|value| read_u32(value, 0, order))
            .collect(),
        _ => None,
    }
}

fn entry_data(
    bytes: &[u8],
    base: usize,
    entry: usize,
    ty: u16,
    count: usize,
    order: Endian,
) -> Option<&[u8]> {
    let item_size = match ty {
        1 | 2 => 1,
        3 => 2,
        4 => 4,
        _ => return None,
    };
    let length = count.checked_mul(item_size)?;
    if length <= 4 {
        return bytes.get(entry.checked_add(8)?..entry.checked_add(8 + length)?);
    }
    let relative = usize::try_from(read_u32(bytes, entry + 8, order)?).ok()?;
    let start = base.checked_add(relative)?;
    bytes.get(start..start.checked_add(length)?)
}

fn read_u16(bytes: &[u8], offset: usize, order: Endian) -> Option<u16> {
    let value = bytes.get(offset..offset.checked_add(2)?)?;
    Some(match order {
        Endian::Little => u16::from_le_bytes([value[0], value[1]]),
        Endian::Big => u16::from_be_bytes([value[0], value[1]]),
    })
}

fn read_u32(bytes: &[u8], offset: usize, order: Endian) -> Option<u32> {
    let value: [u8; 4] = bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?;
    Some(match order {
        Endian::Little => u32::from_le_bytes(value),
        Endian::Big => u32::from_be_bytes(value),
    })
}

fn compression_from_code(code: Option<u32>) -> RawCompression {
    match code {
        Some(1) => RawCompression::Uncompressed,
        Some(7 | 34_792) => RawCompression::LosslessJpeg,
        Some(34_813) => RawCompression::LossyJpeg,
        Some(8 | 32_913) => RawCompression::Deflate,
        Some(52_546) => RawCompression::JpegXl,
        Some(value) => RawCompression::Vendor(value),
        None => RawCompression::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_tiff_without_raw_tags_is_not_raw() {
        let bytes = [b'I', b'I', 42, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            RawContainerRegistry::standard().probe_bytes(&bytes),
            RawProbeOutcome::NoMatch
        );
    }

    #[test]
    fn direct_raw_signatures_have_precedence() {
        let mut bytes = vec![0; 104];
        bytes[..16].copy_from_slice(b"FUJIFILMCCD-RAW ");
        assert!(matches!(
            RawContainerRegistry::standard().probe_bytes(&bytes),
            RawProbeOutcome::Match(RawContainerProbe {
                container: RawContainerKind::Raf,
                ..
            })
        ));
    }
}
