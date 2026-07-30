//! RustTable-owned, deterministic RGB↔YUV conversion shared by AVIF and HEIF.
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::enum_variant_names)]
#![allow(clippy::similar_names)]

use rusttable_image::{ByteOrder, ChannelLayout, SampleType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitDepth {
    Eight,
    Ten,
    Twelve,
}

impl BitDepth {
    pub const fn max(self) -> f32 {
        match self {
            Self::Eight => 255.0,
            Self::Ten => 1023.0,
            Self::Twelve => 4095.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subsampling {
    FourFourFour,
    FourTwoTwo,
    FourTwoZero,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Matrix {
    Bt601,
    Bt709,
    Bt2020,
    Identity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    Full,
    Limited,
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub chroma_width: u32,
    pub chroma_height: u32,
    pub depth: BitDepth,
    pub y: Vec<u16>,
    pub cb: Vec<u16>,
    pub cr: Vec<u16>,
    pub alpha: Option<Vec<u16>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Input<'a> {
    pub bytes: &'a [u8],
    pub layout: ChannelLayout,
    pub sample: SampleType,
    pub byte_order: ByteOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    UnsupportedSample,
    UnsupportedLayout,
    InvalidDimensions,
    InvalidBuffer,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedSample => "unsupported YUV source sample",
            Self::UnsupportedLayout => "unsupported YUV source layout",
            Self::InvalidDimensions => "invalid YUV dimensions",
            Self::InvalidBuffer => "invalid YUV source buffer",
        })
    }
}

impl std::error::Error for Error {}

pub fn convert(
    input: Input<'_>,
    width: u32,
    height: u32,
    depth: BitDepth,
    matrix: Matrix,
    range: Range,
    subsampling: Subsampling,
) -> Result<Frame, Error> {
    if width == 0 || height == 0 {
        return Err(Error::InvalidDimensions);
    }
    if !matches!(
        input.layout,
        ChannelLayout::Gray | ChannelLayout::Rgb | ChannelLayout::Rgba
    ) {
        return Err(Error::UnsupportedLayout);
    }
    if !matches!(input.sample, SampleType::U8 | SampleType::U16) {
        return Err(Error::UnsupportedSample);
    }
    let channels = input.layout.channels();
    let sample_bytes = input.sample.bytes();
    let expected = usize::try_from(u64::from(width) * u64::from(height))
        .ok()
        .and_then(|pixels| pixels.checked_mul(channels))
        .and_then(|samples| samples.checked_mul(sample_bytes))
        .ok_or(Error::InvalidBuffer)?;
    if input.bytes.len() != expected {
        return Err(Error::InvalidBuffer);
    }
    let width_usize = usize::try_from(width).map_err(|_| Error::InvalidDimensions)?;
    let height_usize = usize::try_from(height).map_err(|_| Error::InvalidDimensions)?;
    let mut rgb = Vec::with_capacity(width_usize * height_usize);
    let mut alpha =
        (input.layout == ChannelLayout::Rgba).then(|| Vec::with_capacity(rgb.capacity()));
    let mut cursor = 0;
    for _ in 0..width_usize * height_usize {
        let mut values = [0.0; 4];
        for value in values.iter_mut().take(channels) {
            *value = read_sample(input.bytes, &mut cursor, input.sample, input.byte_order)?;
        }
        if input.layout == ChannelLayout::Gray {
            values[1] = values[0];
            values[2] = values[0];
        }
        rgb.push([values[0], values[1], values[2]]);
        if let Some(alpha) = alpha.as_mut() {
            alpha.push(scale(values[3], depth.max()));
        }
    }

    let (kr, kb) = coefficients(matrix);
    let kg = 1.0 - kr - kb;
    let mut y = Vec::with_capacity(rgb.len());
    let mut cb_full = Vec::with_capacity(rgb.len());
    let mut cr_full = Vec::with_capacity(rgb.len());
    for [red, green, blue] in rgb {
        let luminance = kr * red + kg * green + kb * blue;
        let cb = if matrix == Matrix::Identity {
            blue
        } else {
            (blue - luminance) / (2.0 * (1.0 - kb)) + 0.5
        };
        let cr = if matrix == Matrix::Identity {
            red
        } else {
            (red - luminance) / (2.0 * (1.0 - kr)) + 0.5
        };
        y.push(code(luminance, depth, range, true));
        cb_full.push(code(cb, depth, range, false));
        cr_full.push(code(cr, depth, range, false));
    }
    let (chroma_width, chroma_height) = chroma_dimensions(width, height, subsampling);
    let (cb, cr) = downsample(
        &cb_full,
        &cr_full,
        width_usize,
        height_usize,
        subsampling,
        chroma_width,
        chroma_height,
    );
    Ok(Frame {
        width,
        height,
        chroma_width,
        chroma_height,
        depth,
        y,
        cb,
        cr,
        alpha,
    })
}

pub fn roundtrip_rgb(frame: &Frame, matrix: Matrix, range: Range) -> Vec<[u16; 3]> {
    let (kr, kb) = coefficients(matrix);
    let kg = 1.0 - kr - kb;
    let max = frame.depth.max();
    let mut result = Vec::with_capacity(frame.y.len());
    for index in 0..frame.y.len() {
        let y = uncode(frame.y[index], frame.depth, range, true);
        let chroma_index = index.min(frame.cb.len().saturating_sub(1));
        let cb = uncode(frame.cb[chroma_index], frame.depth, range, false) - 0.5;
        let cr = uncode(frame.cr[chroma_index], frame.depth, range, false) - 0.5;
        let red = if matrix == Matrix::Identity {
            y
        } else {
            y + 2.0 * (1.0 - kr) * cr
        };
        let blue = if matrix == Matrix::Identity {
            cb + 0.5
        } else {
            y + 2.0 * (1.0 - kb) * cb
        };
        let green = (y - kr * red - kb * blue) / kg;
        result.push([scale(red, max), scale(green, max), scale(blue, max)]);
    }
    result
}

fn read_sample(
    bytes: &[u8],
    cursor: &mut usize,
    sample: SampleType,
    order: ByteOrder,
) -> Result<f32, Error> {
    let value = match sample {
        SampleType::U8 => {
            let value = *bytes.get(*cursor).ok_or(Error::InvalidBuffer)?;
            *cursor += 1;
            f32::from(value) / 255.0
        }
        SampleType::U16 => {
            let pair = bytes
                .get(*cursor..*cursor + 2)
                .ok_or(Error::InvalidBuffer)?;
            *cursor += 2;
            let bytes = [pair[0], pair[1]];
            let value = match order {
                ByteOrder::Big => u16::from_be_bytes(bytes),
                ByteOrder::Little => u16::from_le_bytes(bytes),
                ByteOrder::Native => u16::from_ne_bytes(bytes),
            };
            f32::from(value) / 65_535.0
        }
        SampleType::F16 | SampleType::F32 => return Err(Error::UnsupportedSample),
    };
    Ok(value)
}

fn code(value: f32, depth: BitDepth, range: Range, luma: bool) -> u16 {
    let max = depth.max();
    let scaled = match (range, luma) {
        (Range::Full, _) => value * max,
        (Range::Limited, true) => (16.0 / 255.0 + value * (219.0 / 255.0)) * max,
        (Range::Limited, false) => (128.0 / 255.0 + (value - 0.5) * (224.0 / 255.0)) * max,
    };
    scaled.clamp(0.0, max).round() as u16
}

fn uncode(value: u16, depth: BitDepth, range: Range, luma: bool) -> f32 {
    let max = depth.max();
    let value = f32::from(value) / max;
    match (range, luma) {
        (Range::Full, _) => value,
        (Range::Limited, true) => ((value * 255.0 - 16.0) / 219.0).clamp(0.0, 1.0),
        (Range::Limited, false) => 0.5 + (value * 255.0 - 128.0) / 224.0,
    }
}

fn scale(value: f32, max: f32) -> u16 {
    (value.clamp(0.0, 1.0) * max).round() as u16
}

const fn coefficients(matrix: Matrix) -> (f32, f32) {
    match matrix {
        Matrix::Bt601 => (0.299, 0.114),
        Matrix::Bt709 => (0.2126, 0.0722),
        Matrix::Bt2020 => (0.2627, 0.0593),
        Matrix::Identity => (0.0, 0.0),
    }
}

const fn chroma_dimensions(width: u32, height: u32, subsampling: Subsampling) -> (u32, u32) {
    match subsampling {
        Subsampling::FourFourFour => (width, height),
        Subsampling::FourTwoTwo => (width.saturating_add(1) / 2, height),
        Subsampling::FourTwoZero => (width.saturating_add(1) / 2, height.saturating_add(1) / 2),
    }
}

fn downsample(
    cb: &[u16],
    cr: &[u16],
    width: usize,
    height: usize,
    subsampling: Subsampling,
    chroma_width: u32,
    chroma_height: u32,
) -> (Vec<u16>, Vec<u16>) {
    if subsampling == Subsampling::FourFourFour {
        return (cb.to_vec(), cr.to_vec());
    }
    let cw = chroma_width as usize;
    let ch = chroma_height as usize;
    let block_y = if subsampling == Subsampling::FourTwoZero {
        2
    } else {
        1
    };
    let mut out_cb = Vec::with_capacity(cw * ch);
    let mut out_cr = Vec::with_capacity(cw * ch);
    for y in 0..ch {
        for x in 0..cw {
            let mut cb_sum = 0_u32;
            let mut cr_sum = 0_u32;
            let mut count = 0_u32;
            for dy in 0..block_y {
                for dx in 0..2 {
                    let sx = (x * 2 + dx).min(width.saturating_sub(1));
                    let sy = (y * block_y + dy).min(height.saturating_sub(1));
                    let index = sy * width + sx;
                    cb_sum += u32::from(cb[index]);
                    cr_sum += u32::from(cr[index]);
                    count += 1;
                }
            }
            out_cb.push(
                u16::try_from((cb_sum + count / 2) / count).expect("chroma average fits sample"),
            );
            out_cr.push(
                u16::try_from((cr_sum + count / 2) / count).expect("chroma average fits sample"),
            );
        }
    }
    (out_cb, out_cr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_odd_dimensions_with_edge_extended_chroma() {
        let input = Input {
            bytes: &[
                255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 32, 64, 96, 128, 200, 150,
            ],
            layout: ChannelLayout::Rgb,
            sample: SampleType::U8,
            byte_order: ByteOrder::Native,
        };
        let frame = convert(
            input,
            3,
            2,
            BitDepth::Ten,
            Matrix::Bt709,
            Range::Full,
            Subsampling::FourTwoZero,
        )
        .expect("YUV");
        assert_eq!((frame.chroma_width, frame.chroma_height), (2, 1));
        assert_eq!(frame.y.len(), 6);
        assert_eq!(frame.cb.len(), 2);
        assert!(
            roundtrip_rgb(&frame, Matrix::Bt709, Range::Full)
                .iter()
                .all(|pixel| pixel.iter().all(|value| *value <= 1023))
        );
    }

    #[test]
    fn limited_range_has_video_code_bounds() {
        let input = Input {
            bytes: &[0, 0, 0, 255, 255, 255],
            layout: ChannelLayout::Rgb,
            sample: SampleType::U8,
            byte_order: ByteOrder::Native,
        };
        let frame = convert(
            input,
            2,
            1,
            BitDepth::Eight,
            Matrix::Bt709,
            Range::Limited,
            Subsampling::FourFourFour,
        )
        .expect("YUV");
        assert_eq!(frame.y[0], 16);
        assert_eq!(frame.y[1], 235);
    }
}
