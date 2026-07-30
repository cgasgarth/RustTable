use super::{
    Entry, Parser, groups4, malformed, read_i16, read_i32, read_u16, read_u32, read_u64, to_usize,
    type_width,
};
use crate::tiff::types::{TiffDngMatrix, TiffDngMetadata};

const TAG_MAKE: u16 = 271;
const TAG_MODEL: u16 = 272;
const TAG_DNG_VERSION: u16 = 50706;
const TAG_DNG_BACKWARD_VERSION: u16 = 50707;
const TAG_ACTIVE_AREA: u16 = 50829;
const TAG_DEFAULT_CROP_ORIGIN: u16 = 50719;
const TAG_DEFAULT_CROP_SIZE: u16 = 50720;
const TAG_MASKED_AREAS: u16 = 50830;
const TAG_CFA_REPEAT: u16 = 33421;
const TAG_CFA_PATTERN: u16 = 33422;
const TAG_BLACK_REPEAT: u16 = 50713;
const TAG_BLACK_LEVEL: u16 = 50714;
const TAG_WHITE_LEVEL: u16 = 50717;
const TAG_AS_SHOT_NEUTRAL: u16 = 50728;
const TAG_COLOR_MATRIX_1: u16 = 50721;
const TAG_COLOR_MATRIX_2: u16 = 50722;
const TAG_ILLUMINANT_1: u16 = 50778;
const TAG_ILLUMINANT_2: u16 = 50779;
const TAG_OPCODE_1: u16 = 51008;
const TAG_OPCODE_2: u16 = 51009;
const TAG_OPCODE_3: u16 = 51022;

impl Parser<'_> {
    pub(super) fn dng_metadata(
        &self,
        entries: &[Entry],
    ) -> Result<Option<TiffDngMetadata>, super::TiffDecodeError> {
        let has_dng = entries.iter().any(|entry| {
            matches!(
                entry.tag,
                TAG_MAKE
                    | TAG_MODEL
                    | TAG_DNG_VERSION
                    | TAG_ACTIVE_AREA
                    | TAG_CFA_REPEAT
                    | TAG_CFA_PATTERN
                    | TAG_BLACK_LEVEL
                    | TAG_WHITE_LEVEL
            )
        });
        if !has_dng {
            return Ok(None);
        }
        let version = self.bytes_exact(entries, TAG_DNG_VERSION, 4)?;
        let backward_version = self.bytes_exact(entries, TAG_DNG_BACKWARD_VERSION, 4)?;
        let make = self.ascii_optional(entries, TAG_MAKE)?;
        let model = self.ascii_optional(entries, TAG_MODEL)?;
        let active_area = self.u32_array(entries, TAG_ACTIVE_AREA, 4)?;
        let default_crop_origin = self.f32_array(entries, TAG_DEFAULT_CROP_ORIGIN, 2)?;
        let default_crop_size = self.f32_array(entries, TAG_DEFAULT_CROP_SIZE, 2)?;
        let masked_values = self.u32_values_optional(entries, TAG_MASKED_AREAS)?;
        let masked_values = masked_values.unwrap_or_default();
        let masked_areas = groups4(&masked_values, "MaskedAreas")?;
        let cfa_repeat = self.u8_pair(entries, TAG_CFA_REPEAT)?;
        let cfa_pattern = self.bytes_optional(entries, TAG_CFA_PATTERN)?;
        let black_repeat = self.u8_pair(entries, TAG_BLACK_REPEAT)?;
        let black_levels = self
            .f32_values_optional(entries, TAG_BLACK_LEVEL)?
            .unwrap_or_default();
        let white_levels = self
            .f32_values_optional(entries, TAG_WHITE_LEVEL)?
            .unwrap_or_default();
        let as_shot_neutral = self
            .f32_values_optional(entries, TAG_AS_SHOT_NEUTRAL)?
            .unwrap_or_default();
        let mut matrices = Vec::new();
        for (illuminant_tag, matrix_tag) in [
            (TAG_ILLUMINANT_1, TAG_COLOR_MATRIX_1),
            (TAG_ILLUMINANT_2, TAG_COLOR_MATRIX_2),
        ] {
            if let Some(coefficients) = self.f32_values_optional(entries, matrix_tag)? {
                let illuminant = self
                    .single_unsigned_optional(entries, illuminant_tag)?
                    .and_then(|value| u16::try_from(value).ok())
                    .unwrap_or(21);
                matrices.push(TiffDngMatrix {
                    illuminant,
                    coefficients,
                });
            }
        }
        let mut opcodes = Vec::new();
        for tag in [TAG_OPCODE_1, TAG_OPCODE_2, TAG_OPCODE_3] {
            if let Some(bytes) = self.bytes_optional(entries, tag)? {
                opcodes.push((tag, bytes));
            }
        }
        Ok(Some(TiffDngMetadata {
            version: version
                .map(|value| {
                    value
                        .try_into()
                        .map_err(|_| malformed("DNGVersion must contain four bytes"))
                })
                .transpose()?,
            backward_version: backward_version
                .map(|value| {
                    value
                        .try_into()
                        .map_err(|_| malformed("DNGBackwardVersion must contain four bytes"))
                })
                .transpose()?,
            make,
            model,
            active_area,
            default_crop_origin,
            default_crop_size,
            masked_areas,
            cfa_repeat,
            cfa_pattern,
            black_repeat,
            black_levels,
            white_levels,
            as_shot_neutral,
            matrices,
            opcodes,
        }))
    }

    fn bytes_exact(
        &self,
        entries: &[Entry],
        tag: u16,
        expected: usize,
    ) -> Result<Option<Vec<u8>>, super::TiffDecodeError> {
        let Some(bytes) = self.bytes_optional(entries, tag)? else {
            return Ok(None);
        };
        if bytes.len() != expected {
            return Err(malformed("DNG byte tag has an invalid count"));
        }
        Ok(Some(bytes))
    }

    fn bytes_optional(
        &self,
        entries: &[Entry],
        tag: u16,
    ) -> Result<Option<Vec<u8>>, super::TiffDecodeError> {
        let Some(entry) = entries.iter().find(|entry| entry.tag == tag) else {
            return Ok(None);
        };
        if !matches!(entry.field_type, 1 | 7) {
            return Err(malformed("DNG byte tag has an invalid field type"));
        }
        Ok(Some(
            self.slice(entry.data.offset, entry.data.length, "DNG byte tag")?
                .to_vec(),
        ))
    }

    fn ascii_optional(
        &self,
        entries: &[Entry],
        tag: u16,
    ) -> Result<Option<String>, super::TiffDecodeError> {
        let Some(entry) = entries.iter().find(|entry| entry.tag == tag) else {
            return Ok(None);
        };
        if entry.field_type != 2 || entry.count == 0 || entry.count > 256 {
            return Err(malformed("TIFF ASCII tag is invalid"));
        }
        let bytes = self.slice(entry.data.offset, entry.data.length, "TIFF ASCII tag")?;
        let length = bytes
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(bytes.len());
        let value = String::from_utf8(bytes[..length].to_vec())
            .map_err(|_| malformed("TIFF ASCII tag is not UTF-8"))?;
        Ok((!value.is_empty()).then_some(value))
    }

    fn u32_values_optional(
        &self,
        entries: &[Entry],
        tag: u16,
    ) -> Result<Option<Vec<u32>>, super::TiffDecodeError> {
        self.unsigned_values_optional(entries, tag)?
            .map(|values| {
                values
                    .into_iter()
                    .map(|value| {
                        u32::try_from(value).map_err(|_| malformed("DNG integer exceeds u32"))
                    })
                    .collect()
            })
            .transpose()
    }

    fn single_unsigned_optional(
        &self,
        entries: &[Entry],
        tag: u16,
    ) -> Result<Option<u64>, super::TiffDecodeError> {
        self.unsigned_values_optional(entries, tag)?
            .map(|values| super::single(&values, "DNG single-value tag"))
            .transpose()
    }

    fn u32_array<const N: usize>(
        &self,
        entries: &[Entry],
        tag: u16,
        expected: usize,
    ) -> Result<Option<[u32; N]>, super::TiffDecodeError> {
        let Some(values) = self.u32_values_optional(entries, tag)? else {
            return Ok(None);
        };
        if values.len() != expected || expected != N {
            return Err(malformed("DNG integer array has an invalid count"));
        }
        values
            .try_into()
            .map(Some)
            .map_err(|_| malformed("DNG integer array has an invalid count"))
    }

    fn u8_pair(
        &self,
        entries: &[Entry],
        tag: u16,
    ) -> Result<Option<(u8, u8)>, super::TiffDecodeError> {
        let Some(values) = self.unsigned_values_optional(entries, tag)? else {
            return Ok(None);
        };
        if values.len() != 2 {
            return Err(malformed("DNG repeat dimension has an invalid count"));
        }
        let width =
            u8::try_from(values[0]).map_err(|_| malformed("DNG repeat dimension exceeds u8"))?;
        let height =
            u8::try_from(values[1]).map_err(|_| malformed("DNG repeat dimension exceeds u8"))?;
        if width == 0 || height == 0 {
            return Err(malformed("DNG repeat dimension contains zero"));
        }
        Ok(Some((width, height)))
    }

    fn f32_array<const N: usize>(
        &self,
        entries: &[Entry],
        tag: u16,
        expected: usize,
    ) -> Result<Option<[f32; N]>, super::TiffDecodeError> {
        let Some(values) = self.f32_values_optional(entries, tag)? else {
            return Ok(None);
        };
        if values.len() != expected || expected != N {
            return Err(malformed("DNG rational array has an invalid count"));
        }
        if values.iter().any(|value| !value.is_finite()) {
            return Err(malformed("DNG rational array contains a non-finite value"));
        }
        values
            .try_into()
            .map(Some)
            .map_err(|_| malformed("DNG rational array has an invalid count"))
    }

    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    fn f32_values_optional(
        &self,
        entries: &[Entry],
        tag: u16,
    ) -> Result<Option<Vec<f32>>, super::TiffDecodeError> {
        let Some(entry) = entries.iter().find(|entry| entry.tag == tag) else {
            return Ok(None);
        };
        let item_width = to_usize(type_width(entry.field_type)?, "DNG numeric tag width")?;
        let bytes = self.slice(entry.data.offset, entry.data.length, "DNG numeric tag")?;
        let count = to_usize(entry.count, "DNG numeric tag count")?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(count)
            .map_err(|_| super::TiffDecodeError::AllocationFailure)?;
        for index in 0..count {
            let offset = index
                .checked_mul(item_width)
                .ok_or(super::TiffDecodeError::ArithmeticOverflow)?;
            let value = match entry.field_type {
                3 => f32::from(read_u16(bytes, offset, self.order)?),
                4 | 13 => read_u32(bytes, offset, self.order)? as f32,
                5 => {
                    let denominator = read_u32(bytes, offset + 4, self.order)?;
                    if denominator == 0 {
                        return Err(malformed("DNG rational has a zero denominator"));
                    }
                    read_u32(bytes, offset, self.order)? as f32 / denominator as f32
                }
                8 => f32::from(read_i16(bytes, offset, self.order)?),
                9 => read_i32(bytes, offset, self.order)? as f32,
                10 => {
                    let denominator = read_i32(bytes, offset + 4, self.order)?;
                    if denominator == 0 {
                        return Err(malformed("DNG signed rational has a zero denominator"));
                    }
                    read_i32(bytes, offset, self.order)? as f32 / denominator as f32
                }
                11 => f32::from_bits(read_u32(bytes, offset, self.order)?),
                12 => f64::from_bits(read_u64(bytes, offset, self.order)?) as f32,
                16..=18 => read_u64(bytes, offset, self.order)? as f32,
                _ => return Err(malformed("DNG numeric tag has an unsupported field type")),
            };
            values.push(value);
        }
        Ok(Some(values))
    }
}
