use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PrivacyFindingKind {
    Exif,
    Iptc,
    JpegComment,
    Path,
    PngText,
    TiffString,
    Xmp,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PrivacyFinding {
    kind: PrivacyFindingKind,
    field: String,
}

impl PrivacyFinding {
    #[must_use]
    pub fn kind(&self) -> PrivacyFindingKind {
        self.kind
    }

    #[must_use]
    pub fn field(&self) -> &str {
        &self.field
    }
}

impl std::fmt::Display for PrivacyFinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.kind.label(), self.field)
    }
}

impl PrivacyFindingKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Exif => "EXIF",
            Self::Iptc => "IPTC",
            Self::JpegComment => "JPEG comment",
            Self::Path => "path",
            Self::PngText => "PNG text",
            Self::TiffString => "TIFF string",
            Self::Xmp => "XMP",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrivacyReport {
    findings: Vec<PrivacyFinding>,
}

impl PrivacyReport {
    #[must_use]
    pub fn findings(&self) -> &[PrivacyFinding] {
        &self.findings
    }

    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrivacyScannerLimits {
    pub max_segments: u32,
    pub max_chunks: u32,
    pub max_ifd_entries: u32,
    pub max_ifd_depth: u32,
}

impl Default for PrivacyScannerLimits {
    fn default() -> Self {
        Self {
            max_segments: 4096,
            max_chunks: 4096,
            max_ifd_entries: 4096,
            max_ifd_depth: 16,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PrivacyScanner {
    limits: PrivacyScannerLimits,
}

impl PrivacyScanner {
    #[must_use]
    pub const fn new(limits: PrivacyScannerLimits) -> Self {
        Self { limits }
    }

    /// Scans metadata containers and the supplied manifest-relative path.
    /// Findings contain field paths only; values are intentionally discarded.
    #[must_use]
    pub fn scan(&self, path: &Path, bytes: &[u8]) -> PrivacyReport {
        let mut findings = Vec::new();
        scan_path(path, &mut findings);
        if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            scan_jpeg(bytes, self.limits, &mut findings);
        } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            scan_png(bytes, self.limits, &mut findings);
        } else if is_tiff(bytes) {
            scan_tiff(bytes, "tiff", self.limits, &mut findings);
        }
        findings.sort();
        findings.dedup();
        PrivacyReport { findings }
    }

    #[must_use]
    pub fn scan_path(&self, path: &Path) -> PrivacyReport {
        let mut findings = Vec::new();
        scan_path(path, &mut findings);
        findings.sort();
        findings.dedup();
        PrivacyReport { findings }
    }
}

fn scan_path(path: &Path, findings: &mut Vec<PrivacyFinding>) {
    let value = path.to_string_lossy();
    let lower = value.to_ascii_lowercase();
    let suspicious = path.is_absolute()
        || lower.contains("/users/")
        || lower.contains("\\users\\")
        || lower.contains("/home/")
        || lower.starts_with("~/")
        || lower.contains('@')
        || lower.split(['/', '\\']).any(|part| {
            matches!(
                part,
                "private" | "personal" | "owner" | "desktop" | "documents"
            )
        });
    if suspicious {
        add(findings, PrivacyFindingKind::Path, "path");
    }
}

fn scan_jpeg(bytes: &[u8], limits: PrivacyScannerLimits, findings: &mut Vec<PrivacyFinding>) {
    let mut offset = 2usize;
    let mut segment = 0u32;
    while offset + 1 < bytes.len() && segment < limits.max_segments {
        if bytes[offset] != 0xff {
            offset += 1;
            continue;
        }
        while bytes.get(offset) == Some(&0xff) {
            offset += 1;
        }
        let Some(&marker) = bytes.get(offset) else {
            break;
        };
        offset += 1;
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        if marker == 0x00 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let Some(length_bytes) = bytes.get(offset..offset + 2) else {
            break;
        };
        let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
        if length < 2 {
            break;
        }
        let payload_start = offset + 2;
        let payload_end = match payload_start.checked_add(length - 2) {
            Some(end) if end <= bytes.len() => end,
            _ => break,
        };
        let payload = &bytes[payload_start..payload_end];
        if marker == 0xfe {
            add(
                findings,
                PrivacyFindingKind::JpegComment,
                &format!("jpeg.comment[{segment}]"),
            );
        } else if marker == 0xe1 {
            if payload.starts_with(b"Exif\0\0") {
                scan_tiff(&payload[6..], "exif", limits, findings);
            } else if payload.starts_with(b"http://ns.adobe.com/xap/1.0/\0") {
                scan_xmp(&payload[29..], findings);
            }
        } else if marker == 0xed {
            scan_iptc(payload, findings);
        }
        segment += 1;
        offset = payload_end;
    }
}

fn scan_png(bytes: &[u8], limits: PrivacyScannerLimits, findings: &mut Vec<PrivacyFinding>) {
    let mut offset = 8usize;
    let mut chunks = 0u32;
    while offset + 12 <= bytes.len() && chunks < limits.max_chunks {
        let length = u32::from_be_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]);
        let Ok(length) = usize::try_from(length) else {
            break;
        };
        let Some(data_start) = offset.checked_add(8) else {
            break;
        };
        let Some(data_end) = data_start.checked_add(length) else {
            break;
        };
        let Some(chunk_end) = data_end.checked_add(4) else {
            break;
        };
        if chunk_end > bytes.len() {
            break;
        }
        let kind = &bytes[offset + 4..offset + 8];
        let data = &bytes[data_start..data_end];
        if matches!(kind, b"tEXt" | b"zTXt" | b"iTXt") {
            let keyword_end = data
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(data.len());
            let keyword = String::from_utf8_lossy(&data[..keyword_end]);
            let field = format!("png.{}.{}", String::from_utf8_lossy(kind), keyword);
            add(findings, PrivacyFindingKind::PngText, &field);
        }
        chunks += 1;
        offset = chunk_end;
    }
}

fn scan_xmp(bytes: &[u8], findings: &mut Vec<PrivacyFinding>) {
    let text = String::from_utf8_lossy(bytes);
    let mut cursor = 0usize;
    while let Some(relative) = text[cursor..].find('<') {
        let start = cursor + relative + 1;
        let Some(end_relative) = text[start..].find('>') else {
            break;
        };
        let end = start + end_relative;
        let tag = text[start..end].trim_start_matches('/').trim();
        if !tag.is_empty() && !tag.starts_with(['!', '?']) {
            let name_end = tag.find(char::is_whitespace).unwrap_or(tag.len());
            let name = &tag[..name_end];
            if !name.is_empty() {
                add(findings, PrivacyFindingKind::Xmp, &format!("xmp.{name}"));
            }
        }
        cursor = end + 1;
    }
}

fn scan_iptc(bytes: &[u8], findings: &mut Vec<PrivacyFinding>) {
    let mut cursor = 0usize;
    while let Some(relative) = bytes[cursor..]
        .windows(4)
        .position(|window| window == b"8BIM")
    {
        let start = cursor + relative;
        if start + 12 > bytes.len() {
            break;
        }
        let resource = u16::from_be_bytes([bytes[start + 4], bytes[start + 5]]);
        let size = u32::from_be_bytes([
            bytes[start + 8],
            bytes[start + 9],
            bytes[start + 10],
            bytes[start + 11],
        ]);
        if resource == 0x0404 {
            add(
                findings,
                PrivacyFindingKind::Iptc,
                "iptc.photoshop.8BIM.0404",
            );
        }
        cursor = start + 4 + usize::try_from(size).unwrap_or(bytes.len());
        if cursor >= bytes.len() {
            break;
        }
    }
}

fn scan_tiff(
    bytes: &[u8],
    prefix: &str,
    limits: PrivacyScannerLimits,
    findings: &mut Vec<PrivacyFinding>,
) {
    let little = match bytes.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return,
    };
    if read_u16(bytes, 2, little) != Some(42) {
        return;
    }
    let Some(first_ifd) = read_u32(bytes, 4, little) else {
        return;
    };
    let mut pending = vec![(first_ifd, 0u32)];
    let mut visited = Vec::new();
    let mut seen_entries = 0u32;
    while let Some((offset, depth)) = pending.pop() {
        if offset == 0 || depth > limits.max_ifd_depth || visited.contains(&offset) {
            continue;
        }
        visited.push(offset);
        let Some(offset) = usize::try_from(offset).ok() else {
            break;
        };
        let Some(count) = read_u16(bytes, offset, little) else {
            break;
        };
        let count_u32 = u32::from(count);
        seen_entries = seen_entries.saturating_add(count_u32);
        if seen_entries > limits.max_ifd_entries {
            break;
        }
        let Some(entries_start) = offset.checked_add(2) else {
            break;
        };
        for index in 0..count {
            let Some(entry) = entries_start.checked_add(usize::from(index).saturating_mul(12))
            else {
                break;
            };
            let Some(tag) = read_u16(bytes, entry, little) else {
                break;
            };
            let Some(ifd_kind) = read_u16(bytes, entry + 2, little) else {
                break;
            };
            let Some(value_count) = read_u32(bytes, entry + 4, little) else {
                break;
            };
            let string_type = ifd_kind == 2 || ifd_kind == 7;
            if string_type || is_sensitive_tag(tag) {
                let kind = if prefix == "exif" {
                    PrivacyFindingKind::Exif
                } else if string_type {
                    PrivacyFindingKind::TiffString
                } else {
                    PrivacyFindingKind::Exif
                };
                add(
                    findings,
                    kind,
                    &format!("{prefix}.ifd{depth}.{}", tiff_tag_name(tag)),
                );
            }
            if matches!(tag, 0x8769 | 0x8825 | 0xa005)
                && ifd_kind == 4
                && value_count == 1
                && let Some(child) = read_u32(bytes, entry + 8, little)
            {
                pending.push((child, depth.saturating_add(1)));
            }
        }
        let next_offset = entries_start
            .checked_add(usize::from(count).saturating_mul(12))
            .and_then(|value| read_u32(bytes, value, little));
        if let Some(next) = next_offset {
            pending.push((next, depth));
        }
    }
}

fn is_tiff(bytes: &[u8]) -> bool {
    matches!(bytes.get(..4), Some(b"II*\0" | b"MM\0*"))
}

fn is_sensitive_tag(tag: u16) -> bool {
    matches!(
        tag,
        0x013b | 0x8298 | 0x8825 | 0x9286 | 0x9c9b | 0x9c9c | 0x9c9d | 0x9c9e | 0x9c9f
    )
}

fn tiff_tag_name(tag: u16) -> &'static str {
    match tag {
        0x010e => "ImageDescription",
        0x0131 => "Software",
        0x013b => "Artist",
        0x8298 => "Copyright",
        0x8825 => "GPSInfo",
        0x9003 => "DateTimeOriginal",
        0x927c => "MakerNote",
        0x9286 => "UserComment",
        0xa430 => "OwnerName",
        0xa431 => "SerialNumber",
        _ => "tag",
    }
}

fn read_u16(bytes: &[u8], offset: usize, little: bool) -> Option<u16> {
    let value = bytes.get(offset..offset.checked_add(2)?)?;
    Some(if little {
        u16::from_le_bytes([value[0], value[1]])
    } else {
        u16::from_be_bytes([value[0], value[1]])
    })
}

fn read_u32(bytes: &[u8], offset: usize, little: bool) -> Option<u32> {
    let value = bytes.get(offset..offset.checked_add(4)?)?;
    Some(if little {
        u32::from_le_bytes([value[0], value[1], value[2], value[3]])
    } else {
        u32::from_be_bytes([value[0], value[1], value[2], value[3]])
    })
}

fn add(findings: &mut Vec<PrivacyFinding>, kind: PrivacyFindingKind, field: &str) {
    findings.push(PrivacyFinding {
        kind,
        field: field.to_owned(),
    });
}
