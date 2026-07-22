use sha2::{Digest, Sha256};

use super::{
    RAWLER_BACKEND_ID, RawCapabilityError, RawCapabilityKind, RawContainerKind, RawContainerProbe,
    RawDecodeError, RawDecodeLimits, RawDecodeReceipt, RawDecodeRequest, RawDecodeResult,
    RawDngReceipt, RawFrame, RawFrameParts, RawFrameValidationError, RawHeader, RawIlluminant,
    RawLevelPattern, RawOpcodeDescriptor, RawOpcodeStage, RawOrientation, RawPlane, RawPlaneLayout,
    RawPreviewDescriptor, RawPreviewFormat, RawPreviewKind, RawRect, RawSourceReceipt,
};
use crate::tiff::{
    TiffCompression, TiffDecodeError, TiffDecodeLimits, TiffDecodeRequest, TiffDecoder,
    TiffDngMetadata, TiffHeader, TiffPhotometric, TiffPixelData, TiffSampleData,
};

const MAX_OPCODE_COUNT: usize = 256;

pub(super) fn probe(
    bytes: &[u8],
    limits: RawDecodeLimits,
    probe: &RawContainerProbe,
) -> Result<RawHeader, RawDecodeError> {
    let (header, page_index, metadata) = inspect(bytes, limits, probe)?;
    let page = &header.pages[page_index];
    let active = active_area(page.dimensions.width(), page.dimensions.height(), &metadata)?;
    let crop = crop_area(active, &metadata)?;
    Ok(RawHeader {
        container: probe.container,
        dimensions: super::RawDimensions::new(page.dimensions.width(), page.dimensions.height())
            .map_err(invalid)?,
        active_area: active,
        crop_area: crop,
        orientation: orientation(page.orientation),
        camera: camera(&metadata),
        compression: probe.evidence.compression.clone(),
        bit_depth: page.bits_per_sample[0],
    })
}

pub(super) fn decode(
    bytes: &[u8],
    request: &RawDecodeRequest,
    probe: &RawContainerProbe,
) -> Result<RawDecodeResult, RawDecodeError> {
    if request.image_index != 0 {
        return Err(capability(
            RawCapabilityKind::ImageIndex,
            probe,
            "DNG contains one RAW frame",
        ));
    }
    let (header, page_index, metadata) = inspect(bytes, request.limits, probe)?;
    let page = header.pages[page_index].clone();
    reject_compression(page.compression, probe, &metadata)?;
    let tiff = TiffDecoder::new()
        .decode_bytes(
            bytes,
            &TiffDecodeRequest::new(tiff_limits(request.limits)).page(page_index),
        )
        .map_err(|error| map_tiff_error(error, probe))?;
    let pixels = tiff
        .pixels
        .ok_or_else(|| malformed("TIFF returned no RAW pixels", probe))?;
    let active = active_area(page.dimensions.width(), page.dimensions.height(), &metadata)?;
    let crop = crop_area(active, &metadata)?;
    let layout = layout(&page, &metadata, active, probe)?;
    let samples = u16_samples(pixels, page.bits_per_sample[0], probe)?;
    let channels = page.samples_per_pixel;
    let black = levels(
        &metadata,
        true,
        channels,
        page.bits_per_sample[0],
        request.limits,
    )?;
    let white = levels(
        &metadata,
        false,
        channels,
        page.bits_per_sample[0],
        request.limits,
    )?
    .values;
    let frame = build_frame(
        &page,
        &header,
        page_index,
        &metadata,
        active,
        crop,
        layout,
        samples,
        black,
        white,
        request.limits,
        probe,
    )?;
    let sample_count = frame
        .parts()
        .planes
        .iter()
        .try_fold(0_u64, |total, plane| {
            total.checked_add(u64::try_from(plane.samples.len()).ok()?)
        })
        .ok_or_else(|| invalid(RawFrameValidationError::ArithmeticOverflow))?;
    let output_bytes = sample_count
        .checked_mul(2)
        .ok_or_else(|| invalid(RawFrameValidationError::ArithmeticOverflow))?;
    let receipt = build_receipt(&DngReceiptInput {
        bytes,
        probe,
        page: &page,
        metadata: &metadata,
        frame: &frame,
        active,
        crop,
        sample_count,
        output_bytes,
    })?;
    Ok(RawDecodeResult { frame, receipt })
}

struct DngReceiptInput<'a> {
    bytes: &'a [u8],
    probe: &'a RawContainerProbe,
    page: &'a crate::tiff::TiffPage,
    metadata: &'a TiffDngMetadata,
    frame: &'a RawFrame,
    active: RawRect,
    crop: RawRect,
    sample_count: u64,
    output_bytes: u64,
}

fn build_receipt(input: &DngReceiptInput<'_>) -> Result<RawDecodeReceipt, RawDecodeError> {
    let frame_parts = input.frame.parts();
    let opcodes = frame_parts.opcodes.len();
    let previews = frame_parts.previews.len();
    Ok(RawDecodeReceipt {
        backend: RAWLER_BACKEND_ID.to_owned(),
        backend_format: "DNG".to_owned(),
        container: input.probe.container,
        family: super::RawVendorFamily::Standardized,
        camera_profile_id: "rusttable.dng".to_owned(),
        capability_manifest_sha256: super::manifest::rawler_capability_manifest().sha256,
        camera: camera(input.metadata),
        compression: input.probe.evidence.compression.clone(),
        bit_depth: input.page.bits_per_sample[0],
        sensor_area: RawRect::new(
            0,
            0,
            input.page.dimensions.width(),
            input.page.dimensions.height(),
        )
        .map_err(invalid)?,
        active_area: input.active,
        crop_area: input.crop,
        masked_areas: frame_parts.masked_areas.clone(),
        black_levels: frame_parts.black_levels.clone(),
        white_levels: frame_parts.white_levels.clone(),
        white_balance: frame_parts.white_balance.clone(),
        color_matrices: frame_parts.color_matrices.clone(),
        previews: frame_parts.previews.clone(),
        quirk_ids: Vec::new(),
        source: RawSourceReceipt {
            source_bytes: u64::try_from(input.bytes.len()).unwrap_or(u64::MAX),
            source_sha256: Sha256::digest(input.bytes).into(),
            stable_copy_used: true,
        },
        plane_count: 1,
        sample_count: input.sample_count,
        dng: Some(RawDngReceipt {
            version: input.metadata.version,
            backward_version: input.metadata.backward_version,
            selected_ifd: input.page.ifd_offset,
            preview_count: u8::try_from(previews).unwrap_or(u8::MAX),
            opcode_count: u8::try_from(opcodes).unwrap_or(u8::MAX),
            output_bytes: input.output_bytes,
        }),
    })
}

#[allow(clippy::too_many_arguments)]
fn build_frame(
    page: &crate::tiff::TiffPage,
    header: &TiffHeader,
    page_index: usize,
    metadata: &TiffDngMetadata,
    active: RawRect,
    crop: RawRect,
    layout: RawPlaneLayout,
    samples: Vec<u16>,
    black: RawLevelPattern,
    white: Vec<f32>,
    limits: RawDecodeLimits,
    probe: &RawContainerProbe,
) -> Result<RawFrame, RawDecodeError> {
    RawFrame::try_new(
        RawFrameParts {
            planes: vec![RawPlane {
                dimensions: super::RawDimensions::new(
                    page.dimensions.width(),
                    page.dimensions.height(),
                )
                .map_err(invalid)?,
                channels_per_pixel: u8::try_from(page.samples_per_pixel).map_err(|_| {
                    capability(RawCapabilityKind::Layout, probe, "channel count exceeds u8")
                })?,
                layout,
                samples,
            }],
            active_area: active,
            crop_area: crop,
            masked_areas: masked_areas(metadata)?,
            black_levels: black,
            white_levels: white,
            orientation: orientation(page.orientation),
            camera: camera(metadata),
            color_matrices: matrices(metadata, limits)?,
            white_balance: white_balance(metadata)?,
            compression: probe.evidence.compression.clone(),
            bit_depth: page.bits_per_sample[0],
            opcodes: opcodes(metadata, limits)?,
            previews: previews(header, page_index),
        },
        limits,
    )
    .map_err(invalid)
}

fn inspect(
    bytes: &[u8],
    limits: RawDecodeLimits,
    probe: &RawContainerProbe,
) -> Result<(TiffHeader, usize, TiffDngMetadata), RawDecodeError> {
    let header = TiffDecoder::new()
        .inspect_bytes(bytes, tiff_limits(limits))
        .map_err(|error| map_tiff_error(error, probe))?;
    let selected = header
        .pages
        .iter()
        .enumerate()
        .filter(|(_, page)| {
            !page.reduced_image
                && matches!(
                    page.photometric,
                    TiffPhotometric::Cfa | TiffPhotometric::LinearRaw
                )
                && matches!(page.samples_per_pixel, 1 | 3 | 4)
                && page
                    .sample_formats
                    .iter()
                    .all(|format| matches!(format, crate::tiff::TiffSampleFormat::Unsigned))
                && matches!(page.bits_per_sample.first(), Some(8..=16))
        })
        .max_by(|(left_index, left), (right_index, right)| {
            left.dimensions
                .pixel_count()
                .unwrap_or_default()
                .cmp(&right.dimensions.pixel_count().unwrap_or_default())
                .then_with(|| right.ifd_offset.cmp(&left.ifd_offset))
                .then_with(|| right_index.cmp(left_index))
        })
        .map(|(index, _)| index)
        .ok_or_else(|| capability(RawCapabilityKind::Layout, probe, "no supported RAW IFD"))?;
    let page = &header.pages[selected];
    let metadata = merge_metadata(
        page.dng.as_ref(),
        header.pages.first().and_then(|page| page.dng.as_ref()),
    )
    .unwrap_or_default();
    if probe.container == RawContainerKind::Dng {
        let version = metadata
            .version
            .ok_or_else(|| malformed("DNGVersion is missing", probe))?;
        if version[0] != 1 || !(1..=7).contains(&version[1]) || version[2..] != [0, 0] {
            return Err(capability(
                RawCapabilityKind::Container,
                probe,
                "DNG version is outside 1.1 through 1.7",
            ));
        }
    }
    Ok((header, selected, metadata))
}

fn merge_metadata(
    selected: Option<&TiffDngMetadata>,
    root: Option<&TiffDngMetadata>,
) -> Option<TiffDngMetadata> {
    let selected = selected.or(root)?;
    let root = root.unwrap_or(selected);
    Some(TiffDngMetadata {
        version: selected.version.or(root.version),
        backward_version: selected.backward_version.or(root.backward_version),
        make: selected.make.clone().or_else(|| root.make.clone()),
        model: selected.model.clone().or_else(|| root.model.clone()),
        active_area: selected.active_area.or(root.active_area),
        default_crop_origin: selected.default_crop_origin.or(root.default_crop_origin),
        default_crop_size: selected.default_crop_size.or(root.default_crop_size),
        masked_areas: if selected.masked_areas.is_empty() {
            root.masked_areas.clone()
        } else {
            selected.masked_areas.clone()
        },
        cfa_repeat: selected.cfa_repeat.or(root.cfa_repeat),
        cfa_pattern: selected
            .cfa_pattern
            .clone()
            .or_else(|| root.cfa_pattern.clone()),
        black_repeat: selected.black_repeat.or(root.black_repeat),
        black_levels: if selected.black_levels.is_empty() {
            root.black_levels.clone()
        } else {
            selected.black_levels.clone()
        },
        white_levels: if selected.white_levels.is_empty() {
            root.white_levels.clone()
        } else {
            selected.white_levels.clone()
        },
        as_shot_neutral: if selected.as_shot_neutral.is_empty() {
            root.as_shot_neutral.clone()
        } else {
            selected.as_shot_neutral.clone()
        },
        matrices: if selected.matrices.is_empty() {
            root.matrices.clone()
        } else {
            selected.matrices.clone()
        },
        opcodes: if selected.opcodes.is_empty() {
            root.opcodes.clone()
        } else {
            selected.opcodes.clone()
        },
    })
}

fn layout(
    page: &crate::tiff::TiffPage,
    metadata: &TiffDngMetadata,
    active: RawRect,
    probe: &RawContainerProbe,
) -> Result<RawPlaneLayout, RawDecodeError> {
    match page.photometric {
        TiffPhotometric::Cfa => {
            let (rows, columns) = metadata.cfa_repeat.ok_or_else(|| {
                capability(
                    RawCapabilityKind::Layout,
                    probe,
                    "CFARepeatPatternDim is missing",
                )
            })?;
            let pattern = metadata.cfa_pattern.as_ref().ok_or_else(|| {
                capability(RawCapabilityKind::Layout, probe, "CFAPattern is missing")
            })?;
            let expected = usize::from(rows)
                .checked_mul(usize::from(columns))
                .ok_or_else(|| invalid(RawFrameValidationError::ArithmeticOverflow))?;
            if pattern.len() != expected {
                return Err(invalid(RawFrameValidationError::InvalidCfa));
            }
            Ok(RawPlaneLayout::Mosaic(super::RawCfa {
                width: columns,
                height: rows,
                phase_x: u8::try_from(active.x % u32::from(columns)).unwrap_or_default(),
                phase_y: u8::try_from(active.y % u32::from(rows)).unwrap_or_default(),
                pattern: pattern.iter().copied().map(channel).collect(),
            }))
        }
        TiffPhotometric::LinearRaw => {
            let channels = page.samples_per_pixel;
            let values = metadata.cfa_pattern.as_ref().map_or_else(
                || default_channels(channels),
                |pattern| pattern.iter().copied().map(channel).collect::<Vec<_>>(),
            );
            if values.len() != usize::from(channels) {
                return Err(capability(
                    RawCapabilityKind::Layout,
                    probe,
                    "linear channel mapping is unavailable",
                ));
            }
            Ok(RawPlaneLayout::Linear { channels: values })
        }
        _ => Err(capability(
            RawCapabilityKind::Layout,
            probe,
            "photometric layout is not RAW",
        )),
    }
}

fn levels(
    metadata: &TiffDngMetadata,
    black: bool,
    channels: u16,
    bits: u8,
    limits: RawDecodeLimits,
) -> Result<RawLevelPattern, RawDecodeError> {
    let (repeat_width, repeat_height) = if black {
        metadata
            .black_repeat
            .map_or((1, 1), |(rows, columns)| (columns, rows))
    } else {
        (1, 1)
    };
    let source = if black {
        &metadata.black_levels
    } else {
        &metadata.white_levels
    };
    let expected = usize::from(repeat_width)
        .checked_mul(usize::from(repeat_height))
        .and_then(|value| value.checked_mul(usize::from(channels)))
        .ok_or_else(|| invalid(RawFrameValidationError::ArithmeticOverflow))?;
    let default = if black {
        0.0
    } else {
        f32::from(u16::MAX >> (16 - bits))
    };
    let values = if source.is_empty() {
        vec![default; expected]
    } else if source.len() == 1 && expected > 1 {
        vec![source[0]; expected]
    } else if source.len() == expected {
        source.clone()
    } else {
        return Err(invalid(RawFrameValidationError::InvalidLevels));
    };
    if values
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
        || (!black && values.iter().any(|value| *value <= 0.0))
        || values.len() > usize::from(limits.max_level_values)
    {
        return Err(invalid(RawFrameValidationError::InvalidLevels));
    }
    Ok(RawLevelPattern {
        repeat_width,
        repeat_height,
        channels: u8::try_from(channels)
            .map_err(|_| invalid(RawFrameValidationError::PlaneLayout))?,
        values,
    })
}

fn masked_areas(metadata: &TiffDngMetadata) -> Result<Vec<RawRect>, RawDecodeError> {
    metadata
        .masked_areas
        .iter()
        .map(|area| rect_from_borders(*area))
        .collect()
}

fn matrices(
    metadata: &TiffDngMetadata,
    limits: RawDecodeLimits,
) -> Result<Vec<super::RawColorMatrix>, RawDecodeError> {
    metadata
        .matrices
        .iter()
        .map(|matrix| {
            let (rows, columns) = match matrix.coefficients.len() {
                9 => (3, 3),
                12 => (4, 3),
                _ => return Err(invalid(RawFrameValidationError::InvalidMatrix)),
            };
            if matrix.coefficients.iter().any(|value| !value.is_finite())
                || matrix.coefficients.len() > usize::from(limits.max_level_values)
            {
                return Err(invalid(RawFrameValidationError::InvalidMatrix));
            }
            Ok(super::RawColorMatrix {
                illuminant: illuminant(matrix.illuminant),
                rows,
                columns,
                coefficients: matrix.coefficients.clone(),
            })
        })
        .collect()
}

fn white_balance(metadata: &TiffDngMetadata) -> Result<Vec<Option<f32>>, RawDecodeError> {
    if metadata.as_shot_neutral.is_empty() {
        return Ok(vec![None, None, None, None]);
    }
    if metadata.as_shot_neutral.len() != 3
        || metadata
            .as_shot_neutral
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
    {
        return Err(invalid(RawFrameValidationError::NonFiniteMetadata));
    }
    Ok(metadata
        .as_shot_neutral
        .iter()
        .map(|value| Some(1.0 / value))
        .chain([None])
        .collect())
}

fn opcodes(
    metadata: &TiffDngMetadata,
    limits: RawDecodeLimits,
) -> Result<Vec<RawOpcodeDescriptor>, RawDecodeError> {
    metadata
        .opcodes
        .iter()
        .map(|(tag, bytes)| {
            validate_opcode_list(bytes, limits.max_metadata_bytes)?;
            Ok(RawOpcodeDescriptor {
                stage: match *tag {
                    51_008 => RawOpcodeStage::BeforeDemosaic,
                    51_009 => RawOpcodeStage::AfterDemosaic,
                    _ => RawOpcodeStage::AfterGainMap,
                },
                byte_length: u32::try_from(bytes.len())
                    .map_err(|_| invalid(RawFrameValidationError::MetadataByteLimit))?,
                sha256: Sha256::digest(bytes).into(),
            })
        })
        .collect()
}

fn validate_opcode_list(bytes: &[u8], limit: u64) -> Result<(), RawDecodeError> {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit || bytes.len() < 4 {
        return Err(invalid(RawFrameValidationError::MetadataByteLimit));
    }
    let count =
        usize::try_from(u32::from_be_bytes(bytes[..4].try_into().unwrap())).unwrap_or(usize::MAX);
    if count > MAX_OPCODE_COUNT {
        return Err(invalid(RawFrameValidationError::MetadataLimit));
    }
    let mut offset = 4_usize;
    for _ in 0..count {
        let header_end = offset
            .checked_add(16)
            .ok_or_else(|| invalid(RawFrameValidationError::ArithmeticOverflow))?;
        let header = bytes
            .get(offset..header_end)
            .ok_or_else(|| invalid(RawFrameValidationError::MetadataByteLimit))?;
        let length = usize::try_from(u32::from_be_bytes(header[12..16].try_into().unwrap()))
            .unwrap_or(usize::MAX);
        offset = header_end
            .checked_add(length)
            .ok_or_else(|| invalid(RawFrameValidationError::ArithmeticOverflow))?;
        if offset > bytes.len() {
            return Err(invalid(RawFrameValidationError::MetadataByteLimit));
        }
    }
    Ok(())
}

fn previews(header: &TiffHeader, selected: usize) -> Vec<RawPreviewDescriptor> {
    header
        .pages
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != selected)
        .filter_map(|(_, page)| {
            let first = page
                .chunks
                .locations
                .iter()
                .map(|location| location.offset)
                .min()?;
            let end = page
                .chunks
                .locations
                .iter()
                .filter_map(|location| location.offset.checked_add(location.length))
                .max()?;
            Some(RawPreviewDescriptor {
                kind: if page.reduced_image {
                    RawPreviewKind::Thumbnail
                } else {
                    RawPreviewKind::Preview
                },
                format: if matches!(page.compression, TiffCompression::Jpeg) {
                    RawPreviewFormat::Jpeg
                } else {
                    RawPreviewFormat::Tiff
                },
                offset: first,
                byte_length: end.saturating_sub(first),
                dimensions: Some(super::RawDimensions {
                    width: page.dimensions.width(),
                    height: page.dimensions.height(),
                }),
            })
        })
        .collect()
}

fn u16_samples(
    pixels: TiffPixelData,
    bits: u8,
    probe: &RawContainerProbe,
) -> Result<Vec<u16>, RawDecodeError> {
    match pixels.samples {
        TiffSampleData::U8(values) if bits <= 8 => Ok(values.into_iter().map(u16::from).collect()),
        TiffSampleData::U16(values) if bits <= 16 => Ok(values),
        _ => Err(capability(
            RawCapabilityKind::SampleType,
            probe,
            "RAW samples are not unsigned 8-16 bit values",
        )),
    }
}

fn reject_compression(
    compression: TiffCompression,
    probe: &RawContainerProbe,
    _metadata: &TiffDngMetadata,
) -> Result<(), RawDecodeError> {
    if matches!(compression, TiffCompression::JpegXl) {
        return Err(capability(
            RawCapabilityKind::Compression,
            probe,
            "JPEG XL RAW decoding is not provided by the pinned TIFF backend",
        ));
    }
    Ok(())
}

fn active_area(
    width: u32,
    height: u32,
    metadata: &TiffDngMetadata,
) -> Result<RawRect, RawDecodeError> {
    metadata
        .active_area
        .map(rect_from_borders)
        .transpose()?
        .map_or_else(|| RawRect::new(0, 0, width, height).map_err(invalid), Ok)
}

fn crop_area(active: RawRect, metadata: &TiffDngMetadata) -> Result<RawRect, RawDecodeError> {
    let Some(origin) = metadata.default_crop_origin else {
        return Ok(active);
    };
    let Some(size) = metadata.default_crop_size else {
        return Ok(active);
    };
    let values = [origin[0], origin[1], size[0], size[1]];
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let integers = values.map(|value| {
        if !value.is_finite() || value < 0.0 || value.fract() != 0.0 {
            None
        } else {
            u32::try_from(value as u64).ok()
        }
    });
    let [Some(x), Some(y), Some(width), Some(height)] = integers else {
        return Err(invalid(RawFrameValidationError::InvalidLevels));
    };
    RawRect::new(
        active
            .x
            .checked_add(x)
            .ok_or_else(|| invalid(RawFrameValidationError::ArithmeticOverflow))?,
        active
            .y
            .checked_add(y)
            .ok_or_else(|| invalid(RawFrameValidationError::ArithmeticOverflow))?,
        width,
        height,
    )
    .map_err(invalid)
}

fn rect_from_borders(area: [u32; 4]) -> Result<RawRect, RawDecodeError> {
    let [top, left, bottom, right] = area;
    if bottom < top || right < left {
        return Err(invalid(RawFrameValidationError::AreaOutOfBounds));
    }
    RawRect::new(left, top, right - left, bottom - top).map_err(invalid)
}

fn camera(metadata: &TiffDngMetadata) -> super::RawCameraIdentity {
    let maker = safe_text(metadata.make.as_deref().unwrap_or_default());
    let model = safe_text(metadata.model.as_deref().unwrap_or_default());
    super::RawCameraIdentity {
        normalized_maker: maker.clone(),
        normalized_model: model.clone(),
        maker,
        model,
        mode: "dng".to_owned(),
    }
}

fn default_channels(channels: u16) -> Vec<super::RawChannel> {
    match channels {
        1 => vec![super::RawChannel::Unknown],
        3 => vec![
            super::RawChannel::Red,
            super::RawChannel::Green,
            super::RawChannel::Blue,
        ],
        4 => vec![
            super::RawChannel::Red,
            super::RawChannel::Green,
            super::RawChannel::Blue,
            super::RawChannel::Green,
        ],
        _ => Vec::new(),
    }
}

fn channel(value: u8) -> super::RawChannel {
    match value {
        0 => super::RawChannel::Red,
        1 => super::RawChannel::Green,
        2 => super::RawChannel::Blue,
        3 => super::RawChannel::Cyan,
        4 => super::RawChannel::Magenta,
        5 => super::RawChannel::Yellow,
        6 => super::RawChannel::White,
        _ => super::RawChannel::Unknown,
    }
}

fn illuminant(value: u16) -> RawIlluminant {
    match value {
        1 => RawIlluminant::Daylight,
        2 => RawIlluminant::Fluorescent,
        3 => RawIlluminant::Tungsten,
        4 => RawIlluminant::Flash,
        9 => RawIlluminant::FineWeather,
        10 => RawIlluminant::CloudyWeather,
        11 => RawIlluminant::Shade,
        12 => RawIlluminant::DaylightFluorescent,
        13 => RawIlluminant::DaylightWhiteFluorescent,
        14 => RawIlluminant::CoolWhiteFluorescent,
        15 => RawIlluminant::WhiteFluorescent,
        17 => RawIlluminant::A,
        18 => RawIlluminant::B,
        19 => RawIlluminant::C,
        20 => RawIlluminant::D55,
        21 => RawIlluminant::D65,
        22 => RawIlluminant::D75,
        23 => RawIlluminant::D50,
        24 => RawIlluminant::IsoStudioTungsten,
        _ => RawIlluminant::Unknown,
    }
}

fn orientation(value: rusttable_image::Orientation) -> RawOrientation {
    match value {
        rusttable_image::Orientation::Normal => RawOrientation::Normal,
        rusttable_image::Orientation::FlipHorizontal => RawOrientation::HorizontalFlip,
        rusttable_image::Orientation::Rotate180 => RawOrientation::Rotate180,
        rusttable_image::Orientation::FlipVertical => RawOrientation::VerticalFlip,
        rusttable_image::Orientation::Transpose => RawOrientation::Transpose,
        rusttable_image::Orientation::Rotate90 => RawOrientation::Rotate90,
        rusttable_image::Orientation::Transverse => RawOrientation::Transverse,
        rusttable_image::Orientation::Rotate270 => RawOrientation::Rotate270,
    }
}

fn tiff_limits(limits: RawDecodeLimits) -> TiffDecodeLimits {
    TiffDecodeLimits {
        max_source_bytes: limits.max_source_bytes,
        max_width: limits.max_width,
        max_height: limits.max_height,
        max_pixels: limits.max_pixels,
        max_decoded_bytes: limits.max_decoded_bytes,
        max_decompressed_bytes: limits.max_decoded_bytes,
        max_temporary_bytes: limits.max_decoded_bytes,
        max_metadata_bytes: limits.max_metadata_bytes,
        ..TiffDecodeLimits::standard()
    }
}

fn map_tiff_error(error: TiffDecodeError, probe: &RawContainerProbe) -> RawDecodeError {
    match error {
        TiffDecodeError::Source(error) => RawDecodeError::Source(error),
        TiffDecodeError::Cancelled => RawDecodeError::Cancelled,
        TiffDecodeError::ArithmeticOverflow => invalid(RawFrameValidationError::ArithmeticOverflow),
        TiffDecodeError::AllocationFailure => {
            RawDecodeError::Source(super::RawSourceError::AllocationFailure)
        }
        TiffDecodeError::Unsupported { feature, value } => capability(
            RawCapabilityKind::Layout,
            probe,
            &format!("unsupported TIFF {feature}: {value}"),
        ),
        TiffDecodeError::Limit {
            kind: "decoded bytes",
            actual,
            limit,
        } => invalid(RawFrameValidationError::DecodedByteLimit { actual, limit }),
        TiffDecodeError::Limit {
            kind: "pixel count",
            actual,
            limit,
        } => invalid(RawFrameValidationError::PixelLimit { actual, limit }),
        TiffDecodeError::Limit { .. } => capability(
            RawCapabilityKind::Container,
            probe,
            "TIFF/DNG limit rejected the source",
        ),
        TiffDecodeError::Backend(message) if message.to_ascii_lowercase().contains("jpeg xl") => {
            capability(RawCapabilityKind::Compression, probe, &message)
        }
        TiffDecodeError::InvalidPage(_)
        | TiffDecodeError::InvalidRegion
        | TiffDecodeError::Malformed(_)
        | TiffDecodeError::Backend(_) => malformed("TIFF/DNG pixel decode failed", probe),
    }
}

fn capability(kind: RawCapabilityKind, probe: &RawContainerProbe, detail: &str) -> RawDecodeError {
    RawDecodeError::Capability(RawCapabilityError {
        missing: kind,
        container: Some(probe.container),
        maker: safe_text(probe.evidence.camera.maker.as_deref().unwrap_or_default()),
        model: safe_text(probe.evidence.camera.model.as_deref().unwrap_or_default()),
        mode: "dng".to_owned(),
        detail: safe_text(detail),
        evidence: Box::new(super::RawCapabilityEvidence {
            signature: probe.evidence.signature.clone(),
            raw_tags: probe.evidence.raw_tags.clone(),
            backend_format: "DNG".to_owned(),
            compression: probe.evidence.compression.clone(),
            bit_depth: probe.evidence.bit_depth,
        }),
    })
}

fn malformed(message: &str, probe: &RawContainerProbe) -> RawDecodeError {
    RawDecodeError::Malformed {
        container: Some(probe.container),
        message: safe_text(message),
    }
}

fn invalid(error: RawFrameValidationError) -> RawDecodeError {
    RawDecodeError::InvalidFrame(error)
}

fn safe_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect()
}
