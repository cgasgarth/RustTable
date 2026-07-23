pub const BARE_SIGNATURE: [u8; 2] = [0xff, 0x0a];
pub const CONTAINER_SIGNATURE: [u8; 12] =
    [0, 0, 0, 12, b'J', b'X', b'L', b' ', 0x0d, 0x0a, 0x87, 0x0a];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerKind {
    Bare,
    Isobmff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BoxRecord {
    pub box_type: [u8; 4],
    pub offset: u64,
    pub total_bytes: u64,
    pub payload_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContainerInspection {
    pub kind: ContainerKind,
    pub boxes: Vec<BoxRecord>,
    pub codestream_bytes: u64,
    pub codestream_parts: u32,
    pub jpeg_reconstruction_box: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ContainerLimits {
    pub max_boxes: u32,
    pub max_metadata_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerError {
    NotJpegXl,
    Truncated(&'static str),
    Invalid(&'static str),
    UnsupportedEssential([u8; 4]),
    UnsupportedCompression([u8; 4]),
    Limit {
        kind: &'static str,
        actual: u64,
        limit: u64,
    },
    Overflow,
}

pub(crate) fn matches_signature(bytes: &[u8]) -> bool {
    bytes.starts_with(&BARE_SIGNATURE) || bytes.starts_with(&CONTAINER_SIGNATURE)
}

pub(crate) fn signature_kind(bytes: &[u8]) -> Result<ContainerKind, ContainerError> {
    if bytes.starts_with(&BARE_SIGNATURE) {
        Ok(ContainerKind::Bare)
    } else if bytes.starts_with(&CONTAINER_SIGNATURE) {
        Ok(ContainerKind::Isobmff)
    } else {
        Err(ContainerError::NotJpegXl)
    }
}

pub(crate) fn inspect(
    bytes: &[u8],
    limits: ContainerLimits,
) -> Result<ContainerInspection, ContainerError> {
    match signature_kind(bytes)? {
        ContainerKind::Bare => {
            if bytes.len() <= BARE_SIGNATURE.len() {
                return Err(ContainerError::Truncated("bare codestream"));
            }
            Ok(ContainerInspection {
                kind: ContainerKind::Bare,
                boxes: Vec::new(),
                codestream_bytes: to_u64(bytes.len())?,
                codestream_parts: 1,
                jpeg_reconstruction_box: false,
            })
        }
        ContainerKind::Isobmff => inspect_container(bytes, limits),
    }
}

#[allow(clippy::too_many_lines)]
fn inspect_container(
    bytes: &[u8],
    limits: ContainerLimits,
) -> Result<ContainerInspection, ContainerError> {
    let mut cursor = CONTAINER_SIGNATURE.len();
    let mut boxes = Vec::new();
    let mut metadata_bytes = 0_u64;
    let mut saw_ftyp = false;
    let mut saw_complete_codestream = false;
    let mut saw_level_box = false;
    let mut saw_frame_index_box = false;
    let mut saw_reconstruction_box = false;
    let mut jxlp_next = None;
    let mut jxlp_finished = false;
    let mut codestream_bytes = 0_u64;
    let mut codestream_parts = 0_u32;

    while cursor < bytes.len() {
        if boxes.len() >= usize::try_from(limits.max_boxes).map_err(|_| ContainerError::Overflow)? {
            return Err(ContainerError::Limit {
                kind: "box count",
                actual: to_u64(boxes.len())?.saturating_add(1),
                limit: u64::from(limits.max_boxes),
            });
        }
        let header = parse_box_header(bytes, cursor)?;
        let payload = bytes
            .get(header.payload_start..header.end)
            .ok_or(ContainerError::Truncated("box payload"))?;
        let box_type = header.box_type;

        if boxes.is_empty() && box_type != *b"ftyp" {
            return Err(ContainerError::Invalid(
                "file type box must follow the signature box",
            ));
        }

        match &box_type {
            b"ftyp" => {
                if saw_ftyp || saw_complete_codestream || jxlp_next.is_some() {
                    return Err(ContainerError::Invalid(
                        "duplicate or misplaced file type box",
                    ));
                }
                validate_ftyp(payload)?;
                saw_ftyp = true;
            }
            b"jxlc" => {
                if !saw_ftyp || saw_complete_codestream || jxlp_next.is_some() {
                    return Err(ContainerError::Invalid(
                        "duplicate or conflicting codestream box",
                    ));
                }
                if payload.is_empty() {
                    return Err(ContainerError::Truncated("codestream box"));
                }
                saw_complete_codestream = true;
                codestream_parts = 1;
                codestream_bytes = to_u64(payload.len())?;
            }
            b"jxlp" => {
                if !saw_ftyp || saw_complete_codestream || jxlp_finished || payload.len() < 4 {
                    return Err(ContainerError::Invalid("invalid partial codestream box"));
                }
                let raw_index = u32::from_be_bytes(
                    payload[..4]
                        .try_into()
                        .map_err(|_| ContainerError::Overflow)?,
                );
                let index = raw_index & 0x7fff_ffff;
                let expected = jxlp_next.unwrap_or(0);
                if index != expected {
                    return Err(ContainerError::Invalid(
                        "partial codestream boxes are out of order",
                    ));
                }
                jxlp_finished = raw_index & 0x8000_0000 != 0;
                jxlp_next = Some(expected.checked_add(1).ok_or(ContainerError::Overflow)?);
                codestream_parts = codestream_parts
                    .checked_add(1)
                    .ok_or(ContainerError::Overflow)?;
                codestream_bytes = codestream_bytes
                    .checked_add(to_u64(payload.len() - 4)?)
                    .ok_or(ContainerError::Overflow)?;
            }
            b"brob" => return Err(ContainerError::UnsupportedCompression(box_type)),
            b"jxll" => {
                if saw_level_box || payload.len() != 1 {
                    return Err(ContainerError::Invalid("invalid JPEG XL level box"));
                }
                saw_level_box = true;
                add_metadata(&mut metadata_bytes, payload.len(), limits)?;
            }
            b"jxli" => {
                if saw_frame_index_box {
                    return Err(ContainerError::Invalid("duplicate frame index box"));
                }
                saw_frame_index_box = true;
                add_metadata(&mut metadata_bytes, payload.len(), limits)?;
            }
            b"jbrd" => {
                if saw_reconstruction_box {
                    return Err(ContainerError::Invalid("duplicate JPEG reconstruction box"));
                }
                saw_reconstruction_box = true;
                add_metadata(&mut metadata_bytes, payload.len(), limits)?;
            }
            b"JXL " => {
                return Err(ContainerError::Invalid("duplicate JPEG XL signature box"));
            }
            known if is_known_auxiliary(*known) => {
                add_metadata(&mut metadata_bytes, payload.len(), limits)?;
            }
            unknown if unknown.starts_with(b"jxl") => {
                return Err(ContainerError::UnsupportedEssential(box_type));
            }
            _ => add_metadata(&mut metadata_bytes, payload.len(), limits)?,
        }

        boxes.push(BoxRecord {
            box_type,
            offset: to_u64(cursor)?,
            total_bytes: to_u64(header.end - cursor)?,
            payload_bytes: to_u64(payload.len())?,
        });
        cursor = header.end;
    }

    if !saw_ftyp {
        return Err(ContainerError::Invalid("missing file type box"));
    }
    if saw_complete_codestream == jxlp_next.is_some() {
        return Err(ContainerError::Invalid(
            "container must carry exactly one codestream representation",
        ));
    }
    if jxlp_next.is_some() && !jxlp_finished {
        return Err(ContainerError::Invalid(
            "partial codestream sequence has no final part",
        ));
    }
    if codestream_bytes == 0 {
        return Err(ContainerError::Truncated("codestream payload"));
    }

    Ok(ContainerInspection {
        kind: ContainerKind::Isobmff,
        boxes,
        codestream_bytes,
        codestream_parts,
        jpeg_reconstruction_box: saw_reconstruction_box,
    })
}

fn validate_ftyp(payload: &[u8]) -> Result<(), ContainerError> {
    if payload.len() < 8 || !payload.len().is_multiple_of(4) {
        return Err(ContainerError::Invalid("invalid file type box"));
    }
    if payload.get(..4) != Some(b"jxl ") {
        return Err(ContainerError::Invalid(
            "file type box does not declare the JPEG XL brand",
        ));
    }
    Ok(())
}

fn is_known_auxiliary(box_type: [u8; 4]) -> bool {
    matches!(
        &box_type,
        b"Exif" | b"xml " | b"jumb" | b"jhgm" | b"free" | b"skip"
    )
}

fn add_metadata(
    total: &mut u64,
    bytes: usize,
    limits: ContainerLimits,
) -> Result<(), ContainerError> {
    *total = total
        .checked_add(to_u64(bytes)?)
        .ok_or(ContainerError::Overflow)?;
    if *total > limits.max_metadata_bytes {
        return Err(ContainerError::Limit {
            kind: "container metadata bytes",
            actual: *total,
            limit: limits.max_metadata_bytes,
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct BoxHeader {
    box_type: [u8; 4],
    payload_start: usize,
    end: usize,
}

fn parse_box_header(bytes: &[u8], offset: usize) -> Result<BoxHeader, ContainerError> {
    let prefix = bytes
        .get(offset..offset.checked_add(8).ok_or(ContainerError::Overflow)?)
        .ok_or(ContainerError::Truncated("box header"))?;
    let size32 = u32::from_be_bytes(
        prefix[..4]
            .try_into()
            .map_err(|_| ContainerError::Overflow)?,
    );
    let box_type = prefix[4..8]
        .try_into()
        .map_err(|_| ContainerError::Overflow)?;
    let (header_bytes, total_bytes) = match size32 {
        0 => (
            8_usize,
            bytes
                .len()
                .checked_sub(offset)
                .ok_or(ContainerError::Overflow)?,
        ),
        1 => {
            let extended = bytes
                .get(offset + 8..offset + 16)
                .ok_or(ContainerError::Truncated("extended box header"))?;
            let size =
                u64::from_be_bytes(extended.try_into().map_err(|_| ContainerError::Overflow)?);
            let size = usize::try_from(size).map_err(|_| ContainerError::Overflow)?;
            (16, size)
        }
        size => (
            8,
            usize::try_from(size).map_err(|_| ContainerError::Overflow)?,
        ),
    };
    if total_bytes < header_bytes {
        return Err(ContainerError::Invalid(
            "box size is smaller than its header",
        ));
    }
    let end = offset
        .checked_add(total_bytes)
        .ok_or(ContainerError::Overflow)?;
    if end > bytes.len() {
        return Err(ContainerError::Truncated("box payload"));
    }
    Ok(BoxHeader {
        box_type,
        payload_start: offset + header_bytes,
        end,
    })
}

fn to_u64(value: usize) -> Result<u64, ContainerError> {
    u64::try_from(value).map_err(|_| ContainerError::Overflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: ContainerLimits = ContainerLimits {
        max_boxes: 16,
        max_metadata_bytes: 1_024,
    };

    fn box32(box_type: [u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = u32::try_from(payload.len() + 8).unwrap();
        let mut bytes = Vec::from(size.to_be_bytes());
        bytes.extend_from_slice(&box_type);
        bytes.extend_from_slice(payload);
        bytes
    }

    fn container(boxes: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = Vec::from(CONTAINER_SIGNATURE);
        for item in boxes {
            bytes.extend_from_slice(item);
        }
        bytes
    }

    fn ftyp() -> Vec<u8> {
        box32(*b"ftyp", b"jxl \0\0\0\0jxl ")
    }

    #[test]
    fn recognizes_bare_codestream() {
        let result = inspect(&[0xff, 0x0a, 1], LIMITS).unwrap();
        assert_eq!(result.kind, ContainerKind::Bare);
        assert_eq!(result.codestream_parts, 1);
    }

    #[test]
    fn validates_single_codestream_container() {
        let bytes = container(&[ftyp(), box32(*b"jxlc", &[0xff, 0x0a, 1])]);
        let result = inspect(&bytes, LIMITS).unwrap();
        assert_eq!(result.kind, ContainerKind::Isobmff);
        assert_eq!(result.codestream_parts, 1);
        assert_eq!(result.codestream_bytes, 3);
    }

    #[test]
    fn validates_ordered_partial_codestream() {
        let mut first = Vec::from(0_u32.to_be_bytes());
        first.extend_from_slice(&[0xff, 0x0a]);
        let mut last = Vec::from(0x8000_0001_u32.to_be_bytes());
        last.push(1);
        let bytes = container(&[ftyp(), box32(*b"jxlp", &first), box32(*b"jxlp", &last)]);
        let result = inspect(&bytes, LIMITS).unwrap();
        assert_eq!(result.codestream_parts, 2);
        assert_eq!(result.codestream_bytes, 3);
    }

    #[test]
    fn rejects_out_of_order_partial_codestream() {
        let bytes = container(&[
            ftyp(),
            box32(*b"jxlp", &1_u32.to_be_bytes()),
            box32(*b"jxlp", &0x8000_0000_u32.to_be_bytes()),
        ]);
        assert!(matches!(
            inspect(&bytes, LIMITS),
            Err(ContainerError::Invalid(
                "partial codestream boxes are out of order"
            ))
        ));
    }

    #[test]
    fn rejects_unknown_reserved_box() {
        let bytes = container(&[
            ftyp(),
            box32(*b"jxlz", &[1]),
            box32(*b"jxlc", &[0xff, 0x0a, 1]),
        ]);
        assert_eq!(
            inspect(&bytes, LIMITS),
            Err(ContainerError::UnsupportedEssential(*b"jxlz"))
        );
    }

    #[test]
    fn rejects_brotli_box_before_backend_expansion() {
        let bytes = container(&[
            ftyp(),
            box32(*b"brob", b"Exif compressed"),
            box32(*b"jxlc", &[0xff, 0x0a, 1]),
        ]);
        assert_eq!(
            inspect(&bytes, LIMITS),
            Err(ContainerError::UnsupportedCompression(*b"brob"))
        );
    }

    #[test]
    fn rejects_duplicate_codestream() {
        let bytes = container(&[ftyp(), box32(*b"jxlc", &[1]), box32(*b"jxlc", &[2])]);
        assert!(matches!(
            inspect(&bytes, LIMITS),
            Err(ContainerError::Invalid(
                "duplicate or conflicting codestream box"
            ))
        ));
    }

    #[test]
    fn rejects_box_extending_past_source() {
        let mut bytes = container(&[ftyp()]);
        bytes.extend_from_slice(&100_u32.to_be_bytes());
        bytes.extend_from_slice(b"Exif");
        assert_eq!(
            inspect(&bytes, LIMITS),
            Err(ContainerError::Truncated("box payload"))
        );
    }
}
