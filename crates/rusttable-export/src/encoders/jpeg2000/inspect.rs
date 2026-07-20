use std::fmt;

use crate::encoders::jpeg2000::boxes::{self, BoxError, ColorSpace, Jp2Info};
use crate::encoders::jpeg2000::settings::{Container, ProgressionOrder, Transform};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    pub container: Container,
    pub width: u32,
    pub height: u32,
    pub components: u8,
    pub bit_depth: u8,
    pub progression: ProgressionOrder,
    pub transform: Transform,
    pub resolution_levels: u8,
    pub code_block_width: u16,
    pub code_block_height: u16,
    pub alpha: bool,
    pub has_icc: bool,
    pub markers: Vec<String>,
    pub boxes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectionError {
    Malformed(&'static str),
    MalformedMarker(u16),
    Box(BoxError),
    Mismatch(&'static str),
}
impl fmt::Display for InspectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "JPEG 2000 validation failed: {self:?}")
    }
}
impl std::error::Error for InspectionError {}
impl From<BoxError> for InspectionError {
    fn from(value: BoxError) -> Self {
        Self::Box(value)
    }
}

/// # Errors
///
/// Returns a typed error when the codestream or JP2 boxes are malformed or contradictory.
pub fn inspect(data: &[u8], container: Container) -> Result<Inspection, InspectionError> {
    let (codestream, jp2) = match container {
        Container::J2k => (data, None),
        Container::Jp2 => (boxes::codestream(data, container)?, Some(parse_jp2(data)?)),
    };
    let stream = inspect_codestream(codestream)?;
    if let Some(jp2) = jp2 {
        if jp2.width != stream.width
            || jp2.height != stream.height
            || u8::try_from(jp2.components).ok() != Some(stream.components)
            || jp2.bit_depth != stream.bit_depth
        {
            return Err(InspectionError::Mismatch("JP2 header differs from SIZ"));
        }
        if jp2.alpha != (stream.components == 4) {
            return Err(InspectionError::Mismatch("JP2 alpha channel definition"));
        }
        return Ok(Inspection {
            alpha: jp2.alpha,
            has_icc: jp2.has_icc,
            boxes: vec!["jP  ".into(), "ftyp".into(), "jp2h".into(), "jp2c".into()],
            ..stream
        });
    }
    Ok(stream)
}

fn inspect_codestream(data: &[u8]) -> Result<Inspection, InspectionError> {
    if data.get(..2) != Some(b"\xff\x4f")
        || data.len() < 4
        || data[data.len() - 2..] != [0xff, 0xd9]
    {
        return Err(InspectionError::Malformed("SOC/EOC"));
    }
    let mut offset = 2usize;
    let mut markers = vec!["SOC".to_owned()];
    let mut siz = None;
    let mut cod = None;
    while offset + 2 <= data.len() {
        let marker = u16::from_be_bytes([data[offset], data[offset + 1]]);
        offset += 2;
        if marker == 0xffd9 {
            markers.push("EOC".to_owned());
            break;
        }
        if marker == 0xff93 || marker == 0xffd3 {
            markers.push("SOD".to_owned());
            offset = data.len().saturating_sub(2);
            continue;
        }
        if marker == 0xff90 {
            markers.push("SOT".to_owned());
        } else if marker == 0xff51 {
            markers.push("SIZ".to_owned());
        } else if marker == 0xff52 {
            markers.push("COD".to_owned());
        } else if marker == 0xff5c {
            markers.push("QCD".to_owned());
        } else {
            markers.push(format!("FF{marker:02X}"));
        }
        let length = usize::from(u16::from_be_bytes(
            data.get(offset..offset + 2)
                .ok_or(InspectionError::Malformed("segment length"))?
                .try_into()
                .map_err(|_| InspectionError::Malformed("segment length"))?,
        ));
        if length < 2
            || offset
                .checked_add(length)
                .is_none_or(|end| end > data.len())
        {
            return Err(InspectionError::MalformedMarker(marker));
        }
        let body = &data[offset + 2..offset + length];
        match marker {
            0xff51 => siz = Some(parse_siz(body)?),
            0xff52 => cod = Some(parse_cod(body)?),
            _ => {}
        }
        offset += if marker == 0xff52 { 12 } else { length };
    }
    let siz = siz.ok_or(InspectionError::Malformed("missing SIZ"))?;
    let cod = cod.ok_or(InspectionError::Malformed("missing COD"))?;
    if !markers.iter().any(|value| value == "SOD") {
        return Err(InspectionError::Malformed("missing SOD"));
    }
    Ok(Inspection {
        container: Container::J2k,
        width: siz.0,
        height: siz.1,
        components: siz.2,
        bit_depth: siz.3,
        progression: cod.0,
        transform: cod.1,
        resolution_levels: cod.2,
        code_block_width: 1u16 << (u32::from(cod.3) + 2),
        code_block_height: 1u16 << (u32::from(cod.4) + 2),
        alpha: siz.2 == 4,
        has_icc: false,
        markers,
        boxes: Vec::new(),
    })
}

fn parse_siz(body: &[u8]) -> Result<(u32, u32, u8, u8), InspectionError> {
    if body.len() < 36 {
        return Err(InspectionError::Malformed("short SIZ"));
    }
    let width = u32::from_be_bytes(
        body[2..6]
            .try_into()
            .map_err(|_| InspectionError::Malformed("SIZ width"))?,
    );
    let height = u32::from_be_bytes(
        body[6..10]
            .try_into()
            .map_err(|_| InspectionError::Malformed("SIZ height"))?,
    );
    let components = u16::from_be_bytes(
        body[34..36]
            .try_into()
            .map_err(|_| InspectionError::Malformed("SIZ components"))?,
    );
    let components =
        u8::try_from(components).map_err(|_| InspectionError::Malformed("too many components"))?;
    let depth = (body
        .get(36)
        .ok_or(InspectionError::Malformed("SIZ depth"))?
        & 0x7f)
        + 1;
    let expected = 36usize
        .checked_add(
            3usize
                .checked_mul(usize::from(components))
                .ok_or(InspectionError::Malformed("SIZ components"))?,
        )
        .ok_or(InspectionError::Malformed("SIZ length"))?;
    if body.len() < expected || width == 0 || height == 0 || depth == 0 {
        return Err(InspectionError::Malformed("SIZ values"));
    }
    Ok((width, height, components, depth))
}

fn parse_cod(body: &[u8]) -> Result<(ProgressionOrder, Transform, u8, u8, u8), InspectionError> {
    if body.len() < 10 {
        return Err(InspectionError::Malformed("short COD"));
    }
    let progression = match body[1] {
        0 => ProgressionOrder::Lrcp,
        1 => ProgressionOrder::Rlcp,
        2 => ProgressionOrder::Rpcl,
        3 => ProgressionOrder::Pcrl,
        4 => ProgressionOrder::Cprl,
        _ => return Err(InspectionError::Malformed("progression")),
    };
    let levels = body[5];
    let transform = match body[9] {
        0 => Transform::Irreversible97,
        1 => Transform::Reversible53,
        _ => return Err(InspectionError::Malformed("transform")),
    };
    Ok((progression, transform, levels, body[6], body[7]))
}

fn parse_jp2(data: &[u8]) -> Result<Jp2Info, InspectionError> {
    let mut position = 0usize;
    let mut info = None;
    let mut has_icc = false;
    let mut alpha = false;
    while position + 8 <= data.len() {
        let length = usize::try_from(u32::from_be_bytes(
            data[position..position + 4]
                .try_into()
                .map_err(|_| InspectionError::Malformed("box length"))?,
        ))
        .map_err(|_| InspectionError::Malformed("box length"))?;
        if length < 8 || position + length > data.len() {
            return Err(InspectionError::Malformed("box bounds"));
        }
        let kind = &data[position + 4..position + 8];
        let body = &data[position + 8..position + length];
        if kind == b"jp2h" {
            parse_jp2h(body, &mut info, &mut has_icc, &mut alpha)?;
        }
        position += length;
    }
    let mut info = info.ok_or(InspectionError::Malformed("missing jp2h"))?;
    info.alpha = alpha;
    info.has_icc = has_icc;
    Ok(info)
}

fn parse_jp2h(
    body: &[u8],
    info: &mut Option<Jp2Info>,
    has_icc: &mut bool,
    alpha: &mut bool,
) -> Result<(), InspectionError> {
    let mut position = 0usize;
    while position + 8 <= body.len() {
        let length = usize::try_from(u32::from_be_bytes(
            body[position..position + 4]
                .try_into()
                .map_err(|_| InspectionError::Malformed("box length"))?,
        ))
        .map_err(|_| InspectionError::Malformed("box length"))?;
        if length < 8 || position + length > body.len() {
            return Err(InspectionError::Malformed("nested box bounds"));
        }
        let kind = &body[position + 4..position + 8];
        let value = &body[position + 8..position + length];
        match kind {
            b"ihdr" if value.len() >= 14 => {
                let height = u32::from_be_bytes(
                    value[0..4]
                        .try_into()
                        .map_err(|_| InspectionError::Malformed("ihdr"))?,
                );
                let width = u32::from_be_bytes(
                    value[4..8]
                        .try_into()
                        .map_err(|_| InspectionError::Malformed("ihdr"))?,
                );
                let components = u16::from_be_bytes(
                    value[8..10]
                        .try_into()
                        .map_err(|_| InspectionError::Malformed("ihdr"))?,
                );
                let bit_depth = (value[10] & 0x7f) + 1;
                *info = Some(Jp2Info {
                    width,
                    height,
                    components,
                    bit_depth,
                    alpha: false,
                    color_space: if components == 1 {
                        ColorSpace::Gray
                    } else {
                        ColorSpace::Srgb
                    },
                    has_icc: false,
                });
            }
            b"cdef" if value.len() >= 2 => {
                let count = usize::from(u16::from_be_bytes(
                    value[0..2]
                        .try_into()
                        .map_err(|_| InspectionError::Malformed("cdef"))?,
                ));
                for index in 0..count {
                    let start = 2 + index * 6;
                    if value.get(start..start + 6).is_none() {
                        return Err(InspectionError::Malformed("cdef bounds"));
                    }
                    if u16::from_be_bytes(
                        value[start + 2..start + 4]
                            .try_into()
                            .map_err(|_| InspectionError::Malformed("cdef type"))?,
                    ) == 1
                    {
                        *alpha = true;
                    }
                }
            }
            b"colr" if value.first() == Some(&2) => *has_icc = true,
            b"colr" if value.first() == Some(&1) => {}
            _ => {}
        }
        position += length;
    }
    Ok(())
}
