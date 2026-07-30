use std::fmt;

const MAGIC: u32 = 0x0131_2f76;
const MAX_INSPECT_BYTES: usize = 4 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    pub width: u32,
    pub height: u32,
    pub channels: Vec<String>,
    pub sample_types: Vec<String>,
    pub compression: String,
    pub storage: String,
    pub tile_size: Option<(u32, u32)>,
    pub line_order: String,
    pub attribute_names: Vec<String>,
    pub chunk_count: u64,
    pub payload_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectionError {
    TooLarge,
    Truncated,
    Invalid(&'static str),
    Unsupported(&'static str),
    Overflow,
}

impl fmt::Display for InspectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => {
                formatter.write_str("OpenEXR inspection input exceeds the safe bound")
            }
            Self::Truncated => formatter.write_str("truncated OpenEXR data"),
            Self::Invalid(value) => write!(formatter, "invalid OpenEXR {value}"),
            Self::Unsupported(value) => write!(formatter, "unsupported OpenEXR {value}"),
            Self::Overflow => formatter.write_str("OpenEXR size arithmetic overflowed"),
        }
    }
}

impl std::error::Error for InspectionError {}

/// Independently validates the bounded single-part `OpenEXR` structure.
///
/// # Errors
///
/// Returns an error for truncated, malformed, oversized, multipart, deep, or
/// otherwise unsupported data.
pub fn inspect(bytes: &[u8]) -> Result<Inspection, InspectionError> {
    if bytes.len() > MAX_INSPECT_BYTES {
        return Err(InspectionError::TooLarge);
    }
    let mut cursor = Cursor::new(bytes);
    if cursor.u32()? != MAGIC {
        return Err(InspectionError::Invalid("magic"));
    }
    let version = cursor.u32()?;
    if version & 0x800 != 0 {
        return Err(InspectionError::Unsupported("deep data"));
    }
    if version & 0x1000 != 0 {
        return Err(InspectionError::Unsupported("multipart data"));
    }

    let mut channels = Vec::new();
    let mut sample_types = Vec::new();
    let mut compression = None;
    let mut storage = None;
    let mut tile_size = None;
    let mut line_order = None;
    let mut data_window = None;
    let mut display_window = None;
    let mut attributes = Vec::new();

    loop {
        let name = cursor.c_string(256)?;
        if name.is_empty() {
            break;
        }
        let kind = cursor.c_string(64)?;
        let size = usize::try_from(cursor.u32()?).map_err(|_| InspectionError::Overflow)?;
        let value = cursor.take(size)?;
        attributes.push(name.clone());
        match name.as_str() {
            "channels" => parse_channels(value, &mut channels, &mut sample_types)?,
            "compression" => {
                let value = *value.first().ok_or(InspectionError::Truncated)?;
                compression = Some(compression_name(value)?);
            }
            "dataWindow" => data_window = Some(parse_window(value)?),
            "displayWindow" => display_window = Some(parse_window(value)?),
            "lineOrder" => {
                line_order = Some(match value.first().copied() {
                    Some(0) => "increasing",
                    Some(1) => "decreasing",
                    Some(2) => "unspecified",
                    _ => return Err(InspectionError::Invalid("line order")),
                });
            }
            "tiles" => {
                if kind != "tiledesc" || value.len() != 9 {
                    return Err(InspectionError::Invalid("tile description"));
                }
                let width = u32::from_le_bytes(
                    value[0..4]
                        .try_into()
                        .map_err(|_| InspectionError::Truncated)?,
                );
                let height = u32::from_le_bytes(
                    value[4..8]
                        .try_into()
                        .map_err(|_| InspectionError::Truncated)?,
                );
                if width == 0 || height == 0 || value[8] & 0x0f != 0 {
                    return Err(InspectionError::Invalid("tile levels"));
                }
                tile_size = Some((width, height));
                storage = Some("tiles");
            }
            _ => {}
        }
    }

    let (min_x, min_y, max_x, max_y) =
        data_window.ok_or(InspectionError::Invalid("data window"))?;
    let display = display_window.ok_or(InspectionError::Invalid("display window"))?;
    if display != (min_x, min_y, max_x, max_y) {
        return Err(InspectionError::Invalid("display/data window mismatch"));
    }
    if min_x != 0 || min_y != 0 || max_x < min_x || max_y < min_y {
        return Err(InspectionError::Invalid("canonical window"));
    }
    let width = u32::try_from(i64::from(max_x) - i64::from(min_x) + 1)
        .map_err(|_| InspectionError::Overflow)?;
    let height = u32::try_from(i64::from(max_y) - i64::from(min_y) + 1)
        .map_err(|_| InspectionError::Overflow)?;
    let compression = compression.ok_or(InspectionError::Invalid("compression"))?;
    let storage = storage.unwrap_or("scanlines");
    let line_order = line_order.ok_or(InspectionError::Invalid("line order"))?;
    let chunks = chunk_count(width, height, compression, tile_size);
    let table_bytes = chunks.checked_mul(8).ok_or(InspectionError::Overflow)?;
    cursor.take(usize::try_from(table_bytes).map_err(|_| InspectionError::Overflow)?)?;
    let mut payload_bytes = 0_u64;
    for _ in 0..chunks {
        if tile_size.is_some() {
            cursor.take(16)?;
        } else {
            let _chunk_coordinate = cursor.i32()?;
        }
        let size = u64::from(cursor.u32()?);
        cursor.take(usize::try_from(size).map_err(|_| InspectionError::Overflow)?)?;
        payload_bytes = payload_bytes
            .checked_add(size)
            .ok_or(InspectionError::Overflow)?;
    }
    if cursor.position() != bytes.len() {
        return Err(InspectionError::Invalid("trailing bytes"));
    }
    Ok(Inspection {
        width,
        height,
        channels,
        sample_types,
        compression: compression.to_owned(),
        storage: storage.to_owned(),
        tile_size,
        line_order: line_order.to_owned(),
        attribute_names: attributes,
        chunk_count: chunks,
        payload_bytes,
    })
}

fn parse_channels(
    value: &[u8],
    channels: &mut Vec<String>,
    samples: &mut Vec<String>,
) -> Result<(), InspectionError> {
    let mut cursor = Cursor::new(value);
    while cursor.position() < value.len() {
        let name = cursor.c_string(256)?;
        if name.is_empty() {
            break;
        }
        let sample = match cursor.i32()? {
            0 => "uint",
            1 => "half",
            2 => "float",
            _ => return Err(InspectionError::Unsupported("channel sample type")),
        };
        let _p_linear = cursor.u8()?;
        cursor.take(3)?;
        if cursor.i32()? != 1 || cursor.i32()? != 1 {
            return Err(InspectionError::Unsupported("channel subsampling"));
        }
        channels.push(name);
        samples.push(sample.to_owned());
    }
    if channels.is_empty() || cursor.position() != value.len() {
        return Err(InspectionError::Invalid("channel list"));
    }
    if channels.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(InspectionError::Invalid("channel order"));
    }
    Ok(())
}

fn parse_window(value: &[u8]) -> Result<(i32, i32, i32, i32), InspectionError> {
    if value.len() != 16 {
        return Err(InspectionError::Invalid("window"));
    }
    Ok((
        i32::from_le_bytes(
            value[0..4]
                .try_into()
                .map_err(|_| InspectionError::Truncated)?,
        ),
        i32::from_le_bytes(
            value[4..8]
                .try_into()
                .map_err(|_| InspectionError::Truncated)?,
        ),
        i32::from_le_bytes(
            value[8..12]
                .try_into()
                .map_err(|_| InspectionError::Truncated)?,
        ),
        i32::from_le_bytes(
            value[12..16]
                .try_into()
                .map_err(|_| InspectionError::Truncated)?,
        ),
    ))
}

fn compression_name(value: u8) -> Result<&'static str, InspectionError> {
    Ok(match value {
        0 => "none",
        1 => "rle",
        2 => "zip1",
        3 => "zip16",
        4 => "piz",
        5 => "pxr24",
        6 => "b44",
        7 => "b44a",
        _ => return Err(InspectionError::Unsupported("compression")),
    })
}

fn chunk_count(width: u32, height: u32, compression: &str, tiles: Option<(u32, u32)>) -> u64 {
    if let Some((tile_width, tile_height)) = tiles {
        return u64::from(width.div_ceil(tile_width)) * u64::from(height.div_ceil(tile_height));
    }
    let lines = if matches!(compression, "none" | "rle" | "zip1") {
        1
    } else if matches!(compression, "zip16" | "pxr24") {
        16
    } else {
        32
    };
    u64::from(height.div_ceil(lines))
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
    fn position(&self) -> usize {
        self.offset
    }
    fn take(&mut self, count: usize) -> Result<&'a [u8], InspectionError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or(InspectionError::Overflow)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(InspectionError::Truncated)?;
        self.offset = end;
        Ok(value)
    }
    fn u8(&mut self) -> Result<u8, InspectionError> {
        Ok(*self.take(1)?.first().ok_or(InspectionError::Truncated)?)
    }
    fn u32(&mut self) -> Result<u32, InspectionError> {
        Ok(u32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| InspectionError::Truncated)?,
        ))
    }
    fn i32(&mut self) -> Result<i32, InspectionError> {
        Ok(i32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| InspectionError::Truncated)?,
        ))
    }
    fn c_string(&mut self, max: usize) -> Result<String, InspectionError> {
        let remaining = self
            .bytes
            .get(self.offset..)
            .ok_or(InspectionError::Truncated)?;
        let end = remaining
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(InspectionError::Truncated)?;
        if end > max {
            return Err(InspectionError::Invalid("string length"));
        }
        let value = std::str::from_utf8(&remaining[..end])
            .map_err(|_| InspectionError::Invalid("attribute name"))?
            .to_owned();
        self.offset = self
            .offset
            .checked_add(end + 1)
            .ok_or(InspectionError::Overflow)?;
        Ok(value)
    }
}
