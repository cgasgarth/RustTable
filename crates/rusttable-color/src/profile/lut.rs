use super::parser::{IccParseError, IccProfileLimits, be_u16, be_u32, fixed};
use super::tags::parse_curve_sequence;
use super::{IccClut, IccLut, IccLutDirection, IccMultiStageLut, IccSignature};
use crate::{FiniteF32, Matrix3};

pub(crate) fn parse_lut(
    tag: IccSignature,
    data: &[u8],
    type_signature: IccSignature,
    limits: IccProfileLimits,
) -> Result<IccLut, IccParseError> {
    match &type_signature.0 {
        b"mft1" => parse_lut8(tag, data, limits),
        b"mft2" => parse_lut16(tag, data, limits),
        b"mAB " => parse_multi_stage(tag, data, IccLutDirection::AToB, limits),
        b"mBA " => parse_multi_stage(tag, data, IccLutDirection::BToA, limits),
        _ => Err(IccParseError::UnsupportedTagType {
            tag,
            actual: type_signature,
        }),
    }
}

fn parse_lut8(
    tag: IccSignature,
    data: &[u8],
    limits: IccProfileLimits,
) -> Result<IccLut, IccParseError> {
    if data.len() < 48 || data[11] != 0 {
        return Err(IccParseError::InvalidLut(tag));
    }
    let input_channels = data[8];
    let output_channels = data[9];
    let grid_points = data[10];
    validate_channels(tag, input_channels, output_channels)?;
    let clut_count = clut_sample_count(tag, input_channels, output_channels, grid_points, limits)?;
    let input_count = usize::from(input_channels) * 256;
    let output_count = usize::from(output_channels) * 256;
    let expected = 48_usize
        .checked_add(input_count)
        .and_then(|value| value.checked_add(clut_count))
        .and_then(|value| value.checked_add(output_count))
        .ok_or(IccParseError::InvalidLut(tag))?;
    if data.len() != expected {
        return Err(IccParseError::InvalidLut(tag));
    }
    let matrix = parse_matrix(tag, data, 12)?;
    let mut cursor = 48;
    let mut input_tables = Vec::new();
    input_tables
        .try_reserve_exact(usize::from(input_channels))
        .map_err(|_| IccParseError::AllocationFailed)?;
    for _ in 0..input_channels {
        input_tables.push(copy_u8(&data[cursor..cursor + 256])?);
        cursor += 256;
    }
    let clut = copy_u8(&data[cursor..cursor + clut_count])?;
    cursor += clut_count;
    let mut output_tables = Vec::new();
    output_tables
        .try_reserve_exact(usize::from(output_channels))
        .map_err(|_| IccParseError::AllocationFailed)?;
    for _ in 0..output_channels {
        output_tables.push(copy_u8(&data[cursor..cursor + 256])?);
        cursor += 256;
    }
    Ok(IccLut::Lut8 {
        input_channels,
        output_channels,
        grid_points,
        matrix,
        input_tables,
        clut,
        output_tables,
    })
}

fn parse_lut16(
    tag: IccSignature,
    data: &[u8],
    limits: IccProfileLimits,
) -> Result<IccLut, IccParseError> {
    if data.len() < 52 || data[11] != 0 {
        return Err(IccParseError::InvalidLut(tag));
    }
    let input_channels = data[8];
    let output_channels = data[9];
    let grid_points = data[10];
    validate_channels(tag, input_channels, output_channels)?;
    let input_entries = usize::from(be_u16(data, 48)?);
    let output_entries = usize::from(be_u16(data, 50)?);
    if !(2..=limits.max_curve_samples).contains(&input_entries)
        || !(2..=limits.max_curve_samples).contains(&output_entries)
    {
        return Err(IccParseError::InvalidLut(tag));
    }
    let clut_count = clut_sample_count(tag, input_channels, output_channels, grid_points, limits)?;
    let input_count = usize::from(input_channels)
        .checked_mul(input_entries)
        .ok_or(IccParseError::InvalidLut(tag))?;
    let output_count = usize::from(output_channels)
        .checked_mul(output_entries)
        .ok_or(IccParseError::InvalidLut(tag))?;
    let value_count = input_count
        .checked_add(clut_count)
        .and_then(|value| value.checked_add(output_count))
        .ok_or(IccParseError::InvalidLut(tag))?;
    let expected = value_count
        .checked_mul(2)
        .and_then(|value| value.checked_add(52))
        .ok_or(IccParseError::InvalidLut(tag))?;
    if data.len() != expected {
        return Err(IccParseError::InvalidLut(tag));
    }
    let matrix = parse_matrix(tag, data, 12)?;
    let mut cursor = 52;
    let mut input_tables = Vec::new();
    input_tables
        .try_reserve_exact(usize::from(input_channels))
        .map_err(|_| IccParseError::AllocationFailed)?;
    for _ in 0..input_channels {
        input_tables.push(read_u16s(data, cursor, input_entries)?);
        cursor += input_entries * 2;
    }
    let clut = read_u16s(data, cursor, clut_count)?;
    cursor += clut_count * 2;
    let mut output_tables = Vec::new();
    output_tables
        .try_reserve_exact(usize::from(output_channels))
        .map_err(|_| IccParseError::AllocationFailed)?;
    for _ in 0..output_channels {
        output_tables.push(read_u16s(data, cursor, output_entries)?);
        cursor += output_entries * 2;
    }
    Ok(IccLut::Lut16 {
        input_channels,
        output_channels,
        grid_points,
        matrix,
        input_tables,
        clut,
        output_tables,
    })
}

fn parse_multi_stage(
    tag: IccSignature,
    data: &[u8],
    direction: IccLutDirection,
    limits: IccProfileLimits,
) -> Result<IccLut, IccParseError> {
    if data.len() < 32 || data[10..12] != [0, 0] {
        return Err(IccParseError::InvalidLut(tag));
    }
    let input_channels = data[8];
    let output_channels = data[9];
    validate_channels(tag, input_channels, output_channels)?;
    let offsets = [
        usize::try_from(be_u32(data, 12)?).map_err(|_| IccParseError::InvalidLut(tag))?,
        usize::try_from(be_u32(data, 16)?).map_err(|_| IccParseError::InvalidLut(tag))?,
        usize::try_from(be_u32(data, 20)?).map_err(|_| IccParseError::InvalidLut(tag))?,
        usize::try_from(be_u32(data, 24)?).map_err(|_| IccParseError::InvalidLut(tag))?,
        usize::try_from(be_u32(data, 28)?).map_err(|_| IccParseError::InvalidLut(tag))?,
    ];
    let [b_offset, matrix_offset, m_offset, clut_offset, a_offset] = offsets;
    if b_offset == 0 {
        return Err(IccParseError::InvalidLut(tag));
    }
    let mut present = Vec::new();
    present
        .try_reserve_exact(offsets.len())
        .map_err(|_| IccParseError::AllocationFailed)?;
    present.extend(offsets.into_iter().filter(|offset| *offset != 0));
    if present
        .iter()
        .any(|offset| *offset < 32 || *offset >= data.len() || *offset % 4 != 0)
    {
        return Err(IccParseError::InvalidLut(tag));
    }
    present.sort_unstable();
    if present.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(IccParseError::InvalidLut(tag));
    }
    let region = |offset: usize| -> Result<&[u8], IccParseError> {
        if offset == 0 {
            return Ok(&[]);
        }
        let end = present
            .iter()
            .copied()
            .find(|candidate| *candidate > offset)
            .unwrap_or(data.len());
        data.get(offset..end).ok_or(IccParseError::InvalidLut(tag))
    };
    let (a_count, b_count) = match direction {
        IccLutDirection::AToB => (input_channels, output_channels),
        IccLutDirection::BToA => (output_channels, input_channels),
    };
    let b_curves = parse_curve_sequence(tag, region(b_offset)?, usize::from(b_count), limits)?;
    let a_curves = if a_offset == 0 {
        Vec::new()
    } else {
        parse_curve_sequence(tag, region(a_offset)?, usize::from(a_count), limits)?
    };
    let m_curves = if m_offset == 0 {
        Vec::new()
    } else {
        parse_curve_sequence(tag, region(m_offset)?, 3, limits)?
    };
    let (matrix, matrix_vector) = if matrix_offset == 0 {
        (None, None)
    } else {
        let region = region(matrix_offset)?;
        if region.len() < 48 || region[48..].iter().any(|byte| *byte != 0) {
            return Err(IccParseError::InvalidLut(tag));
        }
        (
            Some(parse_matrix(tag, region, 0)?),
            Some([fixed(region, 36)?, fixed(region, 40)?, fixed(region, 44)?]),
        )
    };
    let (clut_grid_points, clut) = if clut_offset == 0 {
        (Vec::new(), None)
    } else {
        parse_multi_clut(
            tag,
            region(clut_offset)?,
            input_channels,
            output_channels,
            limits,
        )?
    };
    if clut.is_some() == a_curves.is_empty()
        || matrix.is_some() == m_curves.is_empty()
        || (matrix.is_some() && (input_channels != 3 || output_channels != 3))
    {
        return Err(IccParseError::InvalidLut(tag));
    }
    Ok(IccLut::MultiStage(IccMultiStageLut {
        direction,
        input_channels,
        output_channels,
        a_curves,
        clut_grid_points,
        clut,
        m_curves,
        matrix,
        matrix_offset: matrix_vector,
        b_curves,
    }))
}

fn parse_multi_clut(
    tag: IccSignature,
    data: &[u8],
    input_channels: u8,
    output_channels: u8,
    limits: IccProfileLimits,
) -> Result<(Vec<u8>, Option<IccClut>), IccParseError> {
    if data.len() < 20 || data[17..20] != [0, 0, 0] {
        return Err(IccParseError::InvalidLut(tag));
    }
    let used = usize::from(input_channels);
    if data[used..16].iter().any(|value| *value != 0) {
        return Err(IccParseError::InvalidLut(tag));
    }
    let mut grid_points = Vec::new();
    grid_points
        .try_reserve_exact(used)
        .map_err(|_| IccParseError::AllocationFailed)?;
    grid_points.extend_from_slice(&data[..used]);
    if grid_points.iter().any(|value| !(2..=65).contains(value)) {
        return Err(IccParseError::InvalidLut(tag));
    }
    let count = grid_points
        .iter()
        .try_fold(usize::from(output_channels), |count, edge| {
            count.checked_mul(usize::from(*edge))
        })
        .ok_or(IccParseError::InvalidLut(tag))?;
    if count > limits.max_lut_samples {
        return Err(IccParseError::LutTooLarge {
            count,
            limit: limits.max_lut_samples,
        });
    }
    let precision = data[16];
    let byte_count = count
        .checked_mul(usize::from(precision))
        .ok_or(IccParseError::InvalidLut(tag))?;
    let expected = 20_usize
        .checked_add(byte_count)
        .ok_or(IccParseError::InvalidLut(tag))?;
    if data.len() < expected || data[expected..].iter().any(|byte| *byte != 0) {
        return Err(IccParseError::InvalidLut(tag));
    }
    let clut = match precision {
        1 => IccClut::U8(copy_u8(&data[20..expected])?),
        2 => IccClut::U16(read_u16s(data, 20, count)?),
        _ => return Err(IccParseError::InvalidLut(tag)),
    };
    Ok((grid_points, Some(clut)))
}

fn validate_channels(
    tag: IccSignature,
    input_channels: u8,
    output_channels: u8,
) -> Result<(), IccParseError> {
    if matches!(input_channels, 1 | 3) && matches!(output_channels, 1 | 3) {
        Ok(())
    } else {
        Err(IccParseError::InvalidLut(tag))
    }
}

fn clut_sample_count(
    tag: IccSignature,
    input_channels: u8,
    output_channels: u8,
    grid_points: u8,
    limits: IccProfileLimits,
) -> Result<usize, IccParseError> {
    if grid_points < 2 || (input_channels == 3 && grid_points > 65) {
        return Err(IccParseError::InvalidLut(tag));
    }
    let count = (0..input_channels)
        .try_fold(usize::from(output_channels), |count, _| {
            count.checked_mul(usize::from(grid_points))
        })
        .ok_or(IccParseError::InvalidLut(tag))?;
    if count > limits.max_lut_samples {
        return Err(IccParseError::LutTooLarge {
            count,
            limit: limits.max_lut_samples,
        });
    }
    Ok(count)
}

fn parse_matrix(tag: IccSignature, data: &[u8], offset: usize) -> Result<Matrix3, IccParseError> {
    let mut values = [0.0; 9];
    for (index, value) in values.iter_mut().enumerate() {
        *value = fixed(data, offset + index * 4)?.get();
    }
    Matrix3::new(values).map_err(|_| IccParseError::InvalidLut(tag))
}

fn copy_u8(source: &[u8]) -> Result<Vec<u8>, IccParseError> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(source.len())
        .map_err(|_| IccParseError::AllocationFailed)?;
    output.extend_from_slice(source);
    Ok(output)
}

fn read_u16s(bytes: &[u8], offset: usize, count: usize) -> Result<Vec<u16>, IccParseError> {
    let byte_count = count.checked_mul(2).ok_or(IccParseError::Truncated)?;
    let data = bytes
        .get(offset..offset + byte_count)
        .ok_or(IccParseError::Truncated)?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| IccParseError::AllocationFailed)?;
    let (pairs, remainder) = data.as_chunks::<2>();
    debug_assert!(remainder.is_empty());
    for pair in pairs {
        values.push(u16::from_be_bytes([pair[0], pair[1]]));
    }
    Ok(values)
}

pub(crate) fn fixed_values(
    bytes: &[u8],
    offset: usize,
    count: usize,
) -> Result<Vec<FiniteF32>, IccParseError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| IccParseError::AllocationFailed)?;
    for index in 0..count {
        values.push(fixed(bytes, offset + index * 4)?);
    }
    Ok(values)
}
