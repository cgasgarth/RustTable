use super::lut::{fixed_values, parse_lut};
use super::parser::{IccParseError, IccProfileLimits, be_u16, be_u32, fixed, signature, xyz};
use super::{
    IccCicp, IccCurve, IccDescription, IccLocalizedString, IccOpaqueTag, IccParametricCurve,
    IccSignature, IccTagValue, IccViewingCondition,
};
use crate::{FiniteF32, Matrix3};
use rusttable_core::numerics::ordered::finite_divide_f32;

pub(crate) fn parse_tag(
    tag: IccSignature,
    data: &[u8],
    limits: IccProfileLimits,
) -> Result<IccTagValue, IccParseError> {
    if data.len() < 8 || data[4..8] != [0, 0, 0, 0] {
        return Err(IccParseError::InvalidTagData(tag));
    }
    let type_signature = signature(data, 0)?;
    match &tag.0 {
        b"rXYZ" | b"gXYZ" | b"bXYZ" | b"wtpt" | b"bkpt" | b"lumi" => {
            require_type(tag, type_signature, *b"XYZ ")?;
            parse_xyz_values(tag, data)
        }
        b"rTRC" | b"gTRC" | b"bTRC" | b"kTRC" => {
            if !matches!(&type_signature.0, b"curv" | b"para") {
                return Err(IccParseError::InvalidTagType {
                    tag,
                    actual: type_signature,
                });
            }
            let (curve, consumed) = parse_curve_at(tag, data, limits)?;
            if consumed != data.len() {
                return Err(IccParseError::InvalidCurve(tag));
            }
            Ok(IccTagValue::Curve(curve))
        }
        b"A2B0" | b"A2B1" | b"A2B2" | b"A2B3" | b"B2A0" | b"B2A1" | b"B2A2" | b"B2A3" | b"gamt"
        | b"pre0" | b"pre1" | b"pre2" => {
            validate_lut_type_for_tag(tag, type_signature)?;
            Ok(IccTagValue::Lut(parse_lut(
                tag,
                data,
                type_signature,
                limits,
            )?))
        }
        b"chad" => {
            require_type(tag, type_signature, *b"sf32")?;
            parse_chromatic_adaptation(tag, data)
        }
        b"desc" | b"dmnd" | b"dmdd" => match &type_signature.0 {
            b"desc" => Ok(IccTagValue::Description(parse_description(
                tag, data, limits,
            )?)),
            b"mluc" => Ok(IccTagValue::Localized(parse_localized(tag, data, limits)?)),
            _ => Err(IccParseError::InvalidTagType {
                tag,
                actual: type_signature,
            }),
        },
        b"vued" => {
            require_type(tag, type_signature, *b"mluc")?;
            Ok(IccTagValue::Localized(parse_localized(tag, data, limits)?))
        }
        b"cicp" => {
            require_type(tag, type_signature, *b"cicp")?;
            parse_cicp(tag, data)
        }
        b"view" => {
            require_type(tag, type_signature, *b"view")?;
            parse_viewing_condition(tag, data)
        }
        _ => {
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(data.len())
                .map_err(|_| IccParseError::AllocationFailed)?;
            bytes.extend_from_slice(data);
            Ok(IccTagValue::Opaque(IccOpaqueTag {
                type_signature,
                bytes: bytes.into_boxed_slice(),
            }))
        }
    }
}

fn parse_xyz_values(tag: IccSignature, data: &[u8]) -> Result<IccTagValue, IccParseError> {
    let payload = data
        .len()
        .checked_sub(8)
        .ok_or(IccParseError::InvalidTagData(tag))?;
    if payload == 0 || !payload.is_multiple_of(12) {
        return Err(IccParseError::InvalidTagData(tag));
    }
    let count = payload / 12;
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| IccParseError::AllocationFailed)?;
    for index in 0..count {
        values.push(xyz(data, 8 + index * 12)?);
    }
    Ok(IccTagValue::Xyz(values))
}

pub(crate) fn parse_curve_sequence(
    tag: IccSignature,
    data: &[u8],
    count: usize,
    limits: IccProfileLimits,
) -> Result<Vec<IccCurve>, IccParseError> {
    let mut curves = Vec::new();
    curves
        .try_reserve_exact(count)
        .map_err(|_| IccParseError::AllocationFailed)?;
    let mut cursor = 0_usize;
    for _ in 0..count {
        let (curve, consumed) = parse_curve_at(
            tag,
            data.get(cursor..).ok_or(IccParseError::InvalidCurve(tag))?,
            limits,
        )?;
        curves.push(curve);
        cursor = cursor
            .checked_add(consumed)
            .and_then(align4)
            .ok_or(IccParseError::InvalidCurve(tag))?;
        if cursor > data.len() {
            return Err(IccParseError::InvalidCurve(tag));
        }
    }
    if data[cursor..].iter().any(|byte| *byte != 0) {
        return Err(IccParseError::InvalidCurve(tag));
    }
    Ok(curves)
}

fn parse_curve_at(
    tag: IccSignature,
    data: &[u8],
    limits: IccProfileLimits,
) -> Result<(IccCurve, usize), IccParseError> {
    if data.len() < 12 || data[4..8] != [0, 0, 0, 0] {
        return Err(IccParseError::InvalidCurve(tag));
    }
    match data.get(..4) {
        Some(b"curv") => parse_sampled_curve(tag, data, limits),
        Some(b"para") => parse_parametric_curve(tag, data),
        _ => Err(IccParseError::InvalidCurve(tag)),
    }
}

fn parse_sampled_curve(
    tag: IccSignature,
    data: &[u8],
    limits: IccProfileLimits,
) -> Result<(IccCurve, usize), IccParseError> {
    let count = usize::try_from(be_u32(data, 8)?).map_err(|_| IccParseError::InvalidCurve(tag))?;
    if count > limits.max_curve_samples {
        return Err(IccParseError::CurveTooLarge {
            count,
            limit: limits.max_curve_samples,
        });
    }
    let consumed = count
        .checked_mul(2)
        .and_then(|value| value.checked_add(12))
        .ok_or(IccParseError::InvalidCurve(tag))?;
    if data.len() < consumed {
        return Err(IccParseError::InvalidCurve(tag));
    }
    match count {
        0 => Ok((IccCurve::Identity, consumed)),
        1 => {
            let gamma = f32::from(be_u16(data, 12)?) / 256.0;
            let gamma = FiniteF32::new(gamma).map_err(|_| IccParseError::InvalidCurve(tag))?;
            if gamma.get() <= 0.0 {
                return Err(IccParseError::InvalidCurve(tag));
            }
            Ok((IccCurve::Gamma(gamma), consumed))
        }
        _ => {
            let mut values = Vec::new();
            values
                .try_reserve_exact(count)
                .map_err(|_| IccParseError::AllocationFailed)?;
            for index in 0..count {
                values.push(be_u16(data, 12 + index * 2)?);
            }
            if values.windows(2).any(|pair| pair[0] > pair[1]) {
                return Err(IccParseError::InvalidCurve(tag));
            }
            Ok((IccCurve::Table(values), consumed))
        }
    }
}

fn parse_parametric_curve(
    tag: IccSignature,
    data: &[u8],
) -> Result<(IccCurve, usize), IccParseError> {
    if be_u16(data, 10)? != 0 {
        return Err(IccParseError::InvalidCurve(tag));
    }
    let function = be_u16(data, 8)?;
    let parameter_count = match function {
        0 => 1,
        1 => 3,
        2 => 4,
        3 => 5,
        4 => 7,
        _ => return Err(IccParseError::InvalidCurve(tag)),
    };
    let consumed = 12 + parameter_count * 4;
    if data.len() < consumed {
        return Err(IccParseError::InvalidCurve(tag));
    }
    let parameters = fixed_values(data, 12, parameter_count)?;
    validate_parametric(tag, function, &parameters)?;
    Ok((
        IccCurve::Parametric(IccParametricCurve::new(function, parameters)),
        consumed,
    ))
}

fn validate_parametric(
    tag: IccSignature,
    function: u16,
    parameters: &[FiniteF32],
) -> Result<(), IccParseError> {
    let value = |index: usize| parameters[index].get();
    if value(0) <= 0.0 {
        return Err(IccParseError::InvalidCurve(tag));
    }
    match function {
        0 => {}
        1 | 2 => {
            let a = value(1);
            let b = value(2);
            if a <= 0.0 {
                return Err(IccParseError::InvalidCurve(tag));
            }
            let boundary =
                finite_divide_f32(-b, a).map_err(|_| IccParseError::InvalidCurve(tag))?;
            if !(0.0..=1.0).contains(&boundary) {
                return Err(IccParseError::InvalidCurve(tag));
            }
        }
        3 | 4 => {
            let a = value(1);
            let c = value(3);
            let d = value(4);
            if a <= 0.0 || c < 0.0 || !(0.0..=1.0).contains(&d) {
                return Err(IccParseError::InvalidCurve(tag));
            }
        }
        _ => return Err(IccParseError::InvalidCurve(tag)),
    }
    Ok(())
}

fn parse_chromatic_adaptation(
    tag: IccSignature,
    data: &[u8],
) -> Result<IccTagValue, IccParseError> {
    if data.len() != 44 {
        return Err(IccParseError::InvalidTagData(tag));
    }
    let mut rows = [0.0; 9];
    for (index, value) in rows.iter_mut().enumerate() {
        *value = fixed(data, 8 + index * 4)?.get();
    }
    Matrix3::new(rows)
        .map(IccTagValue::ChromaticAdaptation)
        .map_err(|_| IccParseError::InvalidTagData(tag))
}

fn parse_description(
    tag: IccSignature,
    data: &[u8],
    limits: IccProfileLimits,
) -> Result<IccDescription, IccParseError> {
    let ascii_count =
        usize::try_from(be_u32(data, 8)?).map_err(|_| IccParseError::InvalidText(tag))?;
    if ascii_count == 0 || ascii_count > limits.max_text_bytes {
        return Err(IccParseError::InvalidText(tag));
    }
    let ascii_end = 12_usize
        .checked_add(ascii_count)
        .ok_or(IccParseError::InvalidText(tag))?;
    let ascii_bytes = data
        .get(12..ascii_end)
        .ok_or(IccParseError::InvalidText(tag))?;
    if ascii_bytes.last() != Some(&0) || !ascii_bytes[..ascii_count - 1].is_ascii() {
        return Err(IccParseError::InvalidText(tag));
    }
    let ascii_source = std::str::from_utf8(&ascii_bytes[..ascii_count - 1])
        .map_err(|_| IccParseError::InvalidText(tag))?;
    let mut ascii = String::new();
    ascii
        .try_reserve_exact(ascii_source.len())
        .map_err(|_| IccParseError::AllocationFailed)?;
    ascii.push_str(ascii_source);
    if ascii_end == data.len() {
        return Ok(IccDescription {
            ascii,
            unicode: None,
            script: None,
        });
    }
    if data.len() < ascii_end + 8 {
        return Err(IccParseError::InvalidText(tag));
    }
    let unicode_count = usize::try_from(be_u32(data, ascii_end + 4)?)
        .map_err(|_| IccParseError::InvalidText(tag))?;
    let unicode_bytes = unicode_count
        .checked_mul(2)
        .ok_or(IccParseError::InvalidText(tag))?;
    if unicode_bytes > limits.max_text_bytes {
        return Err(IccParseError::InvalidText(tag));
    }
    let unicode_start = ascii_end + 8;
    let unicode_end = unicode_start
        .checked_add(unicode_bytes)
        .ok_or(IccParseError::InvalidText(tag))?;
    let unicode = if unicode_count == 0 {
        None
    } else {
        Some(decode_utf16_be(
            data.get(unicode_start..unicode_end)
                .ok_or(IccParseError::InvalidText(tag))?,
            tag,
        )?)
    };
    let script = if unicode_end == data.len() {
        None
    } else {
        if data.len() != unicode_end + 70 {
            return Err(IccParseError::InvalidText(tag));
        }
        let count = usize::from(data[unicode_end + 2]);
        if count > 67 {
            return Err(IccParseError::InvalidText(tag));
        }
        let value = data
            .get(unicode_end + 3..unicode_end + 3 + count)
            .ok_or(IccParseError::InvalidText(tag))?;
        Some(
            std::str::from_utf8(value)
                .map_err(|_| IccParseError::InvalidText(tag))?
                .trim_end_matches('\0')
                .to_owned(),
        )
    };
    Ok(IccDescription {
        ascii,
        unicode,
        script,
    })
}

fn parse_localized(
    tag: IccSignature,
    data: &[u8],
    limits: IccProfileLimits,
) -> Result<Vec<IccLocalizedString>, IccParseError> {
    if data.len() < 16 {
        return Err(IccParseError::InvalidText(tag));
    }
    let count = usize::try_from(be_u32(data, 8)?).map_err(|_| IccParseError::InvalidText(tag))?;
    if count == 0 || count > limits.max_localized_records || be_u32(data, 12)? != 12 {
        return Err(IccParseError::InvalidText(tag));
    }
    let table_end = count
        .checked_mul(12)
        .and_then(|value| value.checked_add(16))
        .ok_or(IccParseError::InvalidText(tag))?;
    if table_end > data.len() {
        return Err(IccParseError::InvalidText(tag));
    }
    let mut records = Vec::new();
    records
        .try_reserve_exact(count)
        .map_err(|_| IccParseError::AllocationFailed)?;
    for index in 0..count {
        let start = 16 + index * 12;
        let language: [u8; 2] = data[start..start + 2]
            .try_into()
            .map_err(|_| IccParseError::InvalidText(tag))?;
        let country: [u8; 2] = data[start + 2..start + 4]
            .try_into()
            .map_err(|_| IccParseError::InvalidText(tag))?;
        if !language.iter().all(u8::is_ascii_alphabetic)
            || !country.iter().all(u8::is_ascii_alphabetic)
        {
            return Err(IccParseError::InvalidText(tag));
        }
        let length = usize::try_from(be_u32(data, start + 4)?)
            .map_err(|_| IccParseError::InvalidText(tag))?;
        let offset = usize::try_from(be_u32(data, start + 8)?)
            .map_err(|_| IccParseError::InvalidText(tag))?;
        if length == 0
            || !length.is_multiple_of(2)
            || length > limits.max_text_bytes
            || offset < table_end
            || offset
                .checked_add(length)
                .is_none_or(|end| end > data.len())
        {
            return Err(IccParseError::InvalidText(tag));
        }
        records.push(IccLocalizedString {
            language,
            country,
            text: decode_utf16_be(&data[offset..offset + length], tag)?,
        });
    }
    Ok(records)
}

fn parse_cicp(tag: IccSignature, data: &[u8]) -> Result<IccTagValue, IccParseError> {
    if data.len() != 12 || data[11] > 1 {
        return Err(IccParseError::InvalidTagData(tag));
    }
    Ok(IccTagValue::Cicp(IccCicp {
        color_primaries: data[8],
        transfer_characteristics: data[9],
        matrix_coefficients: data[10],
        full_range: data[11] == 1,
    }))
}

fn parse_viewing_condition(tag: IccSignature, data: &[u8]) -> Result<IccTagValue, IccParseError> {
    if data.len() != 36 {
        return Err(IccParseError::InvalidTagData(tag));
    }
    let illuminant_type = be_u32(data, 32)?;
    if illuminant_type > 8 {
        return Err(IccParseError::InvalidTagData(tag));
    }
    Ok(IccTagValue::ViewingCondition(IccViewingCondition {
        illuminant: xyz(data, 8)?,
        surround: xyz(data, 20)?,
        illuminant_type,
    }))
}

fn decode_utf16_be(bytes: &[u8], tag: IccSignature) -> Result<String, IccParseError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(IccParseError::InvalidText(tag));
    }
    let (pairs, remainder) = bytes.as_chunks::<2>();
    debug_assert!(remainder.is_empty());
    let units = pairs.iter().copied().map(u16::from_be_bytes);
    let mut output = String::new();
    output
        .try_reserve(bytes.len() / 2)
        .map_err(|_| IccParseError::AllocationFailed)?;
    for character in char::decode_utf16(units) {
        output.push(character.map_err(|_| IccParseError::InvalidText(tag))?);
    }
    while output.ends_with('\0') {
        output.pop();
    }
    Ok(output)
}

fn require_type(
    tag: IccSignature,
    actual: IccSignature,
    expected: [u8; 4],
) -> Result<(), IccParseError> {
    (actual.0 == expected)
        .then_some(())
        .ok_or(IccParseError::InvalidTagType { tag, actual })
}

fn validate_lut_type_for_tag(
    tag: IccSignature,
    type_signature: IccSignature,
) -> Result<(), IccParseError> {
    let compatible = match &tag.0 {
        b"A2B0" | b"A2B1" | b"A2B2" | b"A2B3" | b"gamt" => {
            matches!(&type_signature.0, b"mft1" | b"mft2" | b"mAB ")
        }
        b"B2A0" | b"B2A1" | b"B2A2" | b"B2A3" => {
            matches!(&type_signature.0, b"mft1" | b"mft2" | b"mBA ")
        }
        b"pre0" | b"pre1" | b"pre2" => matches!(&type_signature.0, b"mft1" | b"mft2"),
        _ => false,
    };
    if compatible {
        return Ok(());
    }
    if matches!(&type_signature.0, b"mft1" | b"mft2" | b"mAB " | b"mBA ") {
        Err(IccParseError::InvalidTagType {
            tag,
            actual: type_signature,
        })
    } else {
        Err(IccParseError::UnsupportedTagType {
            tag,
            actual: type_signature,
        })
    }
}

fn align4(value: usize) -> Option<usize> {
    value.checked_add(3).map(|value| value & !3)
}
