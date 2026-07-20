use std::fmt;

use crate::encoders::jpeg2000::settings::Container;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpace {
    Gray,
    Srgb,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jp2Info {
    pub width: u32,
    pub height: u32,
    pub components: u16,
    pub bit_depth: u8,
    pub alpha: bool,
    pub color_space: ColorSpace,
    pub has_icc: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoxError {
    Invalid(&'static str),
    TooLarge,
}

impl fmt::Display for BoxError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid JP2 container: {self:?}")
    }
}
impl std::error::Error for BoxError {}

pub fn wrap(
    codestream: &[u8],
    width: u32,
    height: u32,
    components: u8,
    bit_depth: u8,
    color_space: ColorSpace,
    icc: Option<&[u8]>,
) -> Result<Vec<u8>, BoxError> {
    let channels = usize::from(components);
    let mut jp2h = Vec::new();
    box_bytes(
        *b"ihdr",
        &{
            let mut body = Vec::with_capacity(14);
            body.extend_from_slice(&height.to_be_bytes());
            body.extend_from_slice(&width.to_be_bytes());
            body.extend_from_slice(&u16::from(components).to_be_bytes());
            body.push(bit_depth.saturating_sub(1));
            body.push(7);
            body.extend_from_slice(&[0, 0, 0, 0]);
            body
        },
        &mut jp2h,
    )?;
    if components == 4 {
        let mut body = Vec::with_capacity(2 + channels * 6);
        body.extend_from_slice(&u16::from(components).to_be_bytes());
        for channel in 0..components {
            body.extend_from_slice(&u16::from(channel).to_be_bytes());
            body.extend_from_slice(&u16::from(channel == 3).to_be_bytes());
            body.extend_from_slice(
                &(if channel == 3 {
                    0u16
                } else {
                    u16::from(channel + 1)
                })
                .to_be_bytes(),
            );
        }
        box_bytes(*b"cdef", &body, &mut jp2h)?;
    }
    let colr = if let Some(profile) = icc {
        if profile.is_empty() {
            return Err(BoxError::Invalid("empty ICC profile"));
        }
        let mut body = vec![2, 0, 0];
        body.extend_from_slice(profile);
        body
    } else {
        let enumerated = match color_space {
            ColorSpace::Gray => 17u32,
            ColorSpace::Srgb => 16u32,
        };
        let mut body = vec![1, 0, 0];
        body.extend_from_slice(&enumerated.to_be_bytes());
        body
    };
    box_bytes(*b"colr", &colr, &mut jp2h)?;

    let mut output = Vec::new();
    box_bytes(*b"jP  ", b"\x0d\x0a\x87\x0a", &mut output)?;
    box_bytes(*b"ftyp", b"jp2 \0\0\0\0jp2 ", &mut output)?;
    box_bytes(*b"jp2h", &jp2h, &mut output)?;
    box_bytes(*b"jp2c", codestream, &mut output)?;
    Ok(output)
}

fn box_bytes(kind: [u8; 4], body: &[u8], output: &mut Vec<u8>) -> Result<(), BoxError> {
    let length = body.len().checked_add(8).ok_or(BoxError::TooLarge)?;
    let length = u32::try_from(length).map_err(|_| BoxError::TooLarge)?;
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(&kind);
    output.extend_from_slice(body);
    Ok(())
}

pub fn codestream(data: &[u8], container: Container) -> Result<&[u8], BoxError> {
    match container {
        Container::J2k => {
            if data.starts_with(b"\xff\x4f") {
                Ok(data)
            } else {
                Err(BoxError::Invalid("missing SOC marker"))
            }
        }
        Container::Jp2 => {
            let mut position = 0usize;
            while position.checked_add(8).is_some_and(|end| end <= data.len()) {
                let length = usize::try_from(u32::from_be_bytes(
                    data[position..position + 4]
                        .try_into()
                        .map_err(|_| BoxError::Invalid("box length"))?,
                ))
                .map_err(|_| BoxError::TooLarge)?;
                if length < 8 || position + length > data.len() {
                    return Err(BoxError::Invalid("box bounds"));
                }
                if &data[position + 4..position + 8] == b"jp2c" {
                    return Ok(&data[position + 8..position + length]);
                }
                position += length;
            }
            Err(BoxError::Invalid("missing jp2c box"))
        }
    }
}
