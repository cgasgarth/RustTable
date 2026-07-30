use super::codec::{
    RasterFileChannelMode, RasterMaskAssetError, RasterMaskFormat, RasterMaskLimits,
};
use rusttable_image::DecodeLimits;
use rusttable_masks::MaskRaster;

pub(crate) struct DecodedAsset {
    pub(crate) format: RasterMaskFormat,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) mask: MaskRaster,
}

pub(crate) fn decode_asset(
    bytes: &[u8],
    mode: RasterFileChannelMode,
    limits: RasterMaskLimits,
) -> Result<DecodedAsset, RasterMaskAssetError> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        decode_png(bytes, mode, limits)
    } else if bytes.starts_with(b"PF") {
        decode_pfm(bytes, mode, limits)
    } else {
        Err(RasterMaskAssetError::UnsupportedFormat)
    }
}

fn decode_png(
    bytes: &[u8],
    mode: RasterFileChannelMode,
    limits: RasterMaskLimits,
) -> Result<DecodedAsset, RasterMaskAssetError> {
    let decode_limits = DecodeLimits::new(
        limits.max_source_bytes,
        limits.max_width,
        limits.max_height,
        limits.max_pixel_count,
        limits.max_mask_bytes.min(
            limits
                .max_pixel_count
                .checked_mul(4)
                .ok_or(RasterMaskAssetError::ArithmeticOverflow)?,
        ),
    )
    .map_err(|_| RasterMaskAssetError::ArithmeticOverflow)?;
    let decoded =
        rusttable_image_io::decode_png_rgb_samples(bytes, decode_limits).map_err(|error| {
            match error {
                rusttable_image::ImageInputError::UnsupportedFeature {
                    reason: rusttable_image::UnsupportedImageFeature::ColorModel,
                    ..
                } => RasterMaskAssetError::UnsupportedChannels,
                rusttable_image::ImageInputError::UnsupportedFeature {
                    reason: rusttable_image::UnsupportedImageFeature::BitDepth,
                    ..
                } => RasterMaskAssetError::UnsupportedBitDepth,
                rusttable_image::ImageInputError::SourceTooLarge { actual, limit } => {
                    RasterMaskAssetError::SourceTooLarge { actual, limit }
                }
                rusttable_image::ImageInputError::WidthLimit { actual, .. }
                | rusttable_image::ImageInputError::HeightLimit { actual, .. } => {
                    RasterMaskAssetError::DimensionLimit {
                        width: actual,
                        height: actual,
                    }
                }
                other => RasterMaskAssetError::Malformed {
                    format: Some(RasterMaskFormat::Png),
                    reason: other.to_string(),
                },
            }
        })?;
    limits.validate(
        bytes.len(),
        decoded.dimensions().width(),
        decoded.dimensions().height(),
    )?;
    let values = select_channels(decoded.samples(), mode, decoded.bit_depth() == 8)?;
    let mask = MaskRaster::new(
        decoded.dimensions().width(),
        decoded.dimensions().height(),
        values,
    )
    .map_err(|error| RasterMaskAssetError::Malformed {
        format: Some(RasterMaskFormat::Png),
        reason: error.to_string(),
    })?;
    Ok(DecodedAsset {
        format: RasterMaskFormat::Png,
        width: decoded.dimensions().width(),
        height: decoded.dimensions().height(),
        mask,
    })
}

#[allow(clippy::too_many_lines)]
fn decode_pfm(
    bytes: &[u8],
    mode: RasterFileChannelMode,
    limits: RasterMaskLimits,
) -> Result<DecodedAsset, RasterMaskAssetError> {
    let mut cursor = 0;
    let magic = next_token(bytes, &mut cursor).ok_or_else(|| malformed("missing PFM magic"))?;
    if magic != b"PF" {
        return Err(malformed("PFM must be RGB (PF)"));
    }
    let width = parse_dimension(next_token(bytes, &mut cursor), "width")?;
    let height = parse_dimension(next_token(bytes, &mut cursor), "height")?;
    let scale_token =
        next_token(bytes, &mut cursor).ok_or_else(|| malformed("missing PFM scale"))?;
    let scale = std::str::from_utf8(scale_token)
        .ok()
        .and_then(|text| text.parse::<f32>().ok())
        .ok_or_else(|| malformed("invalid PFM scale"))?;
    if !scale.is_finite() || scale == 0.0 {
        return Err(RasterMaskAssetError::Malformed {
            format: Some(RasterMaskFormat::Pfm),
            reason: "PFM scale must be finite and nonzero".to_owned(),
        });
    }
    limits.validate(bytes.len(), width, height)?;
    let pixels = usize::try_from(
        u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or(RasterMaskAssetError::ArithmeticOverflow)?,
    )
    .map_err(|_| RasterMaskAssetError::ArithmeticOverflow)?;
    let sample_bytes = pixels
        .checked_mul(12)
        .ok_or(RasterMaskAssetError::ArithmeticOverflow)?;
    consume_binary_separator(bytes, &mut cursor)?;
    let data_start = cursor;
    let data_end = data_start
        .checked_add(sample_bytes)
        .ok_or(RasterMaskAssetError::ArithmeticOverflow)?;
    if data_end != bytes.len() {
        return Err(malformed("PFM sample count does not match the source"));
    }
    let little_endian = scale.is_sign_negative();
    let magnitude = scale.abs();
    let mut values = vec![0.0; pixels];
    for file_row in
        0..usize::try_from(height).map_err(|_| RasterMaskAssetError::ArithmeticOverflow)?
    {
        let output_row = usize::try_from(height)
            .map_err(|_| RasterMaskAssetError::ArithmeticOverflow)?
            - 1
            - file_row;
        for x in 0..usize::try_from(width).map_err(|_| RasterMaskAssetError::ArithmeticOverflow)? {
            let source_pixel = file_row
                .checked_mul(
                    usize::try_from(width).map_err(|_| RasterMaskAssetError::ArithmeticOverflow)?,
                )
                .and_then(|row| row.checked_add(x))
                .ok_or(RasterMaskAssetError::ArithmeticOverflow)?;
            let destination_pixel = output_row
                .checked_mul(
                    usize::try_from(width).map_err(|_| RasterMaskAssetError::ArithmeticOverflow)?,
                )
                .and_then(|row| row.checked_add(x))
                .ok_or(RasterMaskAssetError::ArithmeticOverflow)?;
            let source_offset = data_start
                .checked_add(
                    source_pixel
                        .checked_mul(12)
                        .ok_or(RasterMaskAssetError::ArithmeticOverflow)?,
                )
                .ok_or(RasterMaskAssetError::ArithmeticOverflow)?;
            let mut selected = [0.0; 3];
            for (channel, selected_sample) in selected.iter_mut().enumerate() {
                let offset = source_offset
                    .checked_add(
                        channel
                            .checked_mul(4)
                            .ok_or(RasterMaskAssetError::ArithmeticOverflow)?,
                    )
                    .ok_or(RasterMaskAssetError::ArithmeticOverflow)?;
                let raw: [u8; 4] = bytes[offset..offset + 4]
                    .try_into()
                    .expect("PFM sample bounds checked");
                let sample = if little_endian {
                    f32::from_le_bytes(raw)
                } else {
                    f32::from_be_bytes(raw)
                } * magnitude;
                if !sample.is_finite() {
                    return Err(RasterMaskAssetError::NonFiniteSample);
                }
                *selected_sample = sample;
            }
            values[destination_pixel] = select_pixel(selected, mode);
        }
    }
    let mask = MaskRaster::new(width, height, values).map_err(|error| {
        RasterMaskAssetError::Malformed {
            format: Some(RasterMaskFormat::Pfm),
            reason: error.to_string(),
        }
    })?;
    Ok(DecodedAsset {
        format: RasterMaskFormat::Pfm,
        width,
        height,
        mask,
    })
}

fn select_channels(
    samples: &[u16],
    mode: RasterFileChannelMode,
    eight_bit: bool,
) -> Result<Vec<f32>, RasterMaskAssetError> {
    if !samples.len().is_multiple_of(3) {
        return Err(RasterMaskAssetError::ArithmeticOverflow);
    }
    let (samples, remainder) = samples.as_chunks::<3>();
    debug_assert!(remainder.is_empty());
    Ok(samples
        .iter()
        .map(|sample| {
            let scale = if eight_bit { 255.0 } else { 65_535.0 };
            select_pixel(
                [
                    f32::from(sample[0]) / scale,
                    f32::from(sample[1]) / scale,
                    f32::from(sample[2]) / scale,
                ],
                mode,
            )
        })
        .collect())
}

fn select_pixel(sample: [f32; 3], mode: RasterFileChannelMode) -> f32 {
    let mut value: f32 = 0.0;
    for (channel, value_sample) in sample.into_iter().enumerate() {
        if mode.selects(channel) {
            value = value.max(value_sample);
        }
    }
    value.clamp(0.0, 1.0)
}

fn next_token<'a>(bytes: &'a [u8], cursor: &mut usize) -> Option<&'a [u8]> {
    loop {
        while bytes.get(*cursor).is_some_and(u8::is_ascii_whitespace) {
            *cursor += 1;
        }
        if bytes.get(*cursor) == Some(&b'#') {
            while bytes.get(*cursor).is_some_and(|byte| *byte != b'\n') {
                *cursor += 1;
            }
            continue;
        }
        break;
    }
    let start = *cursor;
    while bytes
        .get(*cursor)
        .is_some_and(|byte| !byte.is_ascii_whitespace())
    {
        *cursor += 1;
    }
    (start != *cursor).then(|| &bytes[start..*cursor])
}

fn parse_dimension(token: Option<&[u8]>, name: &str) -> Result<u32, RasterMaskAssetError> {
    let value = token
        .and_then(|token| std::str::from_utf8(token).ok())
        .and_then(|text| text.parse::<u64>().ok())
        .ok_or_else(|| malformed(format!("invalid PFM {name}")))?;
    u32::try_from(value).map_err(|_| RasterMaskAssetError::ArithmeticOverflow)
}

fn consume_binary_separator(bytes: &[u8], cursor: &mut usize) -> Result<(), RasterMaskAssetError> {
    match bytes.get(*cursor) {
        Some(b'\r') if bytes.get(*cursor + 1) == Some(&b'\n') => *cursor += 2,
        Some(byte) if byte.is_ascii_whitespace() => *cursor += 1,
        _ => return Err(malformed("PFM header is not separated from samples")),
    }
    Ok(())
}

fn malformed(reason: impl Into<String>) -> RasterMaskAssetError {
    RasterMaskAssetError::Malformed {
        format: Some(RasterMaskFormat::Pfm),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pfm(width: u32, height: u32, scale: f32, samples: &[[f32; 3]]) -> Vec<u8> {
        let mut bytes = format!("PF\n{width} {height}\n{scale}\n").into_bytes();
        let little_endian = scale.is_sign_negative();
        for sample in samples {
            for channel in sample {
                bytes.extend(if little_endian {
                    channel.to_le_bytes()
                } else {
                    channel.to_be_bytes()
                });
            }
        }
        bytes
    }

    #[test]
    fn every_channel_mode_selects_the_maximum_requested_channel() {
        let bytes = pfm(1, 1, 1.0, &[[0.2, 0.7, 0.4]]);
        for (mode, expected) in [
            (RasterFileChannelMode::RED, 0.2),
            (RasterFileChannelMode::GREEN, 0.7),
            (RasterFileChannelMode::BLUE, 0.4),
            (RasterFileChannelMode::RED_GREEN, 0.7),
            (RasterFileChannelMode::RED_BLUE, 0.4),
            (RasterFileChannelMode::GREEN_BLUE, 0.7),
            (RasterFileChannelMode::ALL, 0.7),
        ] {
            let decoded = decode_asset(&bytes, mode, RasterMaskLimits::default()).expect("PFM");
            assert!((decoded.mask.values()[0] - expected).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn pfm_handles_little_endian_scale_and_flips_file_rows() {
        let bytes = pfm(1, 1, -2.0, &[[0.25, 0.0, 0.0]]);
        let decoded = decode_asset(
            &bytes,
            RasterFileChannelMode::RED,
            RasterMaskLimits::default(),
        )
        .expect("little-endian PFM");
        assert_eq!(decoded.mask.values(), &[0.5]);

        let bytes = pfm(1, 2, 1.0, &[[0.1, 0.0, 0.0], [0.9, 0.0, 0.0]]);
        let decoded = decode_asset(
            &bytes,
            RasterFileChannelMode::RED,
            RasterMaskLimits::default(),
        )
        .expect("row order");
        assert_eq!(decoded.mask.values(), &[0.9, 0.1]);
    }

    #[test]
    fn pfm_preserves_binary_whitespace_and_clamps_mask_values() {
        let leading_whitespace_sample = f32::from_bits(0x2000_0000);
        let bytes = pfm(1, 1, 1.0, &[[leading_whitespace_sample, 2.0, -1.0]]);
        let decoded = decode_asset(
            &bytes,
            RasterFileChannelMode::GREEN_BLUE,
            RasterMaskLimits::default(),
        )
        .expect("binary whitespace and clamp");
        assert_eq!(decoded.mask.values(), &[1.0]);
    }

    #[test]
    fn pfm_rejects_wrong_entry_count_nonfinite_and_limits() {
        let mut truncated = pfm(1, 1, 1.0, &[[0.2, 0.3, 0.4]]);
        truncated.pop();
        assert!(matches!(
            decode_asset(
                &truncated,
                RasterFileChannelMode::ALL,
                RasterMaskLimits::default()
            ),
            Err(RasterMaskAssetError::Malformed { .. })
        ));

        let nonfinite = pfm(1, 1, 1.0, &[[f32::NAN, 0.0, 0.0]]);
        assert!(matches!(
            decode_asset(
                &nonfinite,
                RasterFileChannelMode::RED,
                RasterMaskLimits::default()
            ),
            Err(RasterMaskAssetError::NonFiniteSample)
        ));

        let limits = RasterMaskLimits {
            max_pixel_count: 1,
            ..RasterMaskLimits::default()
        };
        assert!(matches!(
            decode_asset(
                &pfm(2, 1, 1.0, &[[0.1, 0.0, 0.0], [0.2, 0.0, 0.0]]),
                RasterFileChannelMode::ALL,
                limits
            ),
            Err(RasterMaskAssetError::PixelLimit { .. })
        ));
    }
}
