use std::io::Cursor;
use std::panic::{AssertUnwindSafe, catch_unwind};

use jxl_oxide::color::{
    ColourEncoding, ColourSpace, Primaries, RenderingIntent, TransferFunction, WhitePoint,
};
use jxl_oxide::frame::Encoding;
use jxl_oxide::image::{BitDepth, ExtraChannelType};
use jxl_oxide::{
    AllocTracker, InitializeResult, JpegReconstructionStatus, JxlImage, JxlThreadPool,
};
use rusttable_image::{AlphaMode, ImageDimensions, Orientation, Roi};
use sha2::{Digest, Sha256};

use super::types::{
    JXL_PROBE_BUDGET_BYTES, JxlAnimation, JxlBitDepth, JxlCodingMode, JxlColorEncoding,
    JxlColorSpace, JxlDecodeError, JxlDecodeLimits, JxlDecodeMode, JxlExtraChannel,
    JxlExtraChannelType, JxlFrameDescriptor, JxlHeader, JxlIccProfile, JxlJpegReconstruction,
    JxlPixelData, JxlPreviewDescriptor, JxlPrimaries, JxlRenderingIntent, JxlRoiBehavior,
    JxlStructuredColor, JxlToneMapping, JxlTransferFunction, JxlWhitePoint,
};

pub(crate) struct BackendResult {
    pub header: JxlHeader,
    pub pixels: Option<JxlPixelData>,
    pub coding: JxlCodingMode,
    pub roi_behavior: JxlRoiBehavior,
}

pub(crate) fn probe(
    bytes: &[u8],
    limits: JxlDecodeLimits,
) -> Result<ImageDimensions, JxlDecodeError> {
    let bounded = bytes
        .get(..bytes.len().min(JXL_PROBE_BUDGET_BYTES))
        .ok_or(JxlDecodeError::ArithmeticOverflow)?;
    let tracker = allocation_tracker(limits)?;
    let mut decoder = JxlImage::builder()
        .pool(JxlThreadPool::none())
        .alloc_tracker(tracker)
        .build_uninit();
    catch_backend("probe feed", || decoder.feed_bytes(bounded))?;
    let image = match catch_backend("probe header", || decoder.try_init())? {
        InitializeResult::Initialized(image) => image,
        InitializeResult::NeedMoreData(_) => {
            return Err(JxlDecodeError::Limit {
                kind: "probe bytes",
                actual: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                limit: u64::try_from(JXL_PROBE_BUDGET_BYTES).unwrap_or(u64::MAX),
            });
        }
    };
    dimensions(
        image.image_header().size.width,
        image.image_header().size.height,
        limits,
    )
}

pub(crate) fn decode(
    bytes: &[u8],
    mode: JxlDecodeMode,
    limits: JxlDecodeLimits,
) -> Result<BackendResult, JxlDecodeError> {
    let tracker = allocation_tracker(limits)?;
    let mut image = catch_backend("source decode", || {
        JxlImage::builder()
            .pool(JxlThreadPool::none())
            .alloc_tracker(tracker)
            .read(Cursor::new(bytes))
    })?;
    let header = normalize_header(&image, limits)?;
    let coding = header
        .frames
        .iter()
        .find(|frame| frame.displayed)
        .or_else(|| header.frames.first())
        .map_or(JxlCodingMode::VarDct, |frame| frame.coding);
    if header.animation.displayed_frames > 1 {
        return Err(JxlDecodeError::UnsupportedAnimation {
            displayed_frames: header.animation.displayed_frames,
        });
    }
    if image.pixel_format().has_black() {
        return Err(JxlDecodeError::UnsupportedBlackChannel);
    }
    if let JxlColorEncoding::Icc(profile) = &header.color {
        catch_backend("ICC render request", || image.request_icc(&profile.bytes))?;
    }
    image.set_render_spot_color(false);

    let (pixels, roi_behavior) = match mode {
        JxlDecodeMode::Header => (None, JxlRoiBehavior::NotRequested),
        JxlDecodeMode::Full => (
            Some(render(&image, header.output_dimensions(), &header, limits)?),
            JxlRoiBehavior::NotRequested,
        ),
        JxlDecodeMode::Region(region) => {
            let full_dimensions = header.output_dimensions();
            validate_region(region, full_dimensions)?;
            let full = render(&image, full_dimensions, &header, limits)?;
            let output = crop(&full, region, limits)?;
            (
                Some(output),
                JxlRoiBehavior::FullDecodeThenCrop {
                    source: full_dimensions,
                    region,
                },
            )
        }
        JxlDecodeMode::Thumbnail {
            max_width,
            max_height,
        } => {
            if max_width == 0 || max_height == 0 {
                return Err(JxlDecodeError::InvalidThumbnail);
            }
            let full_dimensions = header.output_dimensions();
            let full = render(&image, full_dimensions, &header, limits)?;
            let output_dimensions = thumbnail_dimensions(full_dimensions, max_width, max_height)?;
            let output = scale_nearest(&full, output_dimensions, limits)?;
            (
                Some(output),
                JxlRoiBehavior::FullDecodeThenScale {
                    source: full_dimensions,
                    output: output_dimensions,
                    preview_declared: header.preview.is_some(),
                },
            )
        }
    };
    Ok(BackendResult {
        header,
        pixels,
        coding,
        roi_behavior,
    })
}

fn normalize_header(
    image: &JxlImage,
    limits: JxlDecodeLimits,
) -> Result<JxlHeader, JxlDecodeError> {
    let raw = image.image_header();
    let source_dimensions = dimensions(raw.size.width, raw.size.height, limits)?;
    let orientation = orientation(raw.metadata.orientation)?;
    let bit_depth = bit_depth(raw.metadata.bit_depth);
    let extra_channels = extra_channels(&raw.metadata.ec_info, limits)?;
    let alpha = extra_channels
        .iter()
        .find_map(|channel| match channel.channel_type {
            JxlExtraChannelType::Alpha { associated } => Some(if associated {
                AlphaMode::Premultiplied
            } else {
                AlphaMode::Straight
            }),
            _ => None,
        })
        .unwrap_or(AlphaMode::None);
    let color = color_encoding(image, limits)?;
    let preview = raw
        .metadata
        .preview
        .as_ref()
        .map(|preview| {
            if limits.max_previews == 0 {
                return Err(JxlDecodeError::Limit {
                    kind: "preview count",
                    actual: 1,
                    limit: 0,
                });
            }
            Ok(JxlPreviewDescriptor {
                dimensions: dimensions(preview.width, preview.height, limits)?,
            })
        })
        .transpose()?;
    let frames = frames(image, limits)?;
    let displayed_frames = frames.iter().filter(|frame| frame.displayed).count();
    let animation = raw.metadata.animation.as_ref();
    let animation = JxlAnimation {
        declared: animation.is_some(),
        displayed_frames,
        total_frames: frames.len(),
        ticks_per_second_numerator: animation.map(|value| value.tps_numerator),
        ticks_per_second_denominator: animation.map(|value| value.tps_denominator),
        loop_count: animation.map(|value| value.num_loops),
        has_timecodes: animation.is_some_and(|value| value.have_timecodes),
    };
    let reconstruction = match image.jpeg_reconstruction_status() {
        JpegReconstructionStatus::Available => JxlJpegReconstruction::Available,
        JpegReconstructionStatus::Invalid | JpegReconstructionStatus::NeedMoreData => {
            JxlJpegReconstruction::Invalid
        }
        JpegReconstructionStatus::Unavailable => JxlJpegReconstruction::Unavailable,
    };
    Ok(JxlHeader {
        dimensions: source_dimensions,
        orientation,
        bit_depth,
        color,
        xyb_encoded: raw.metadata.xyb_encoded,
        tone_mapping: JxlToneMapping {
            intensity_target: raw.metadata.tone_mapping.intensity_target,
            min_nits: raw.metadata.tone_mapping.min_nits,
            relative_to_max_display: raw.metadata.tone_mapping.relative_to_max_display,
            linear_below: raw.metadata.tone_mapping.linear_below,
        },
        alpha,
        extra_channels,
        preview,
        animation,
        frames,
        jpeg_reconstruction: reconstruction,
    })
}

fn frames(
    image: &JxlImage,
    limits: JxlDecodeLimits,
) -> Result<Vec<JxlFrameDescriptor>, JxlDecodeError> {
    let count = image.num_loaded_frames();
    limit("frame count", count, limits.max_frames)?;
    let mut frames = Vec::new();
    frames
        .try_reserve_exact(count)
        .map_err(|_| JxlDecodeError::AllocationFailure)?;
    for index in 0..count {
        let frame = image
            .frame(index)
            .ok_or_else(|| JxlDecodeError::Malformed("missing loaded frame".to_owned()))?;
        let header = frame.header();
        if header.name.len() > usize::try_from(limits.max_name_bytes).unwrap_or(usize::MAX) {
            return Err(JxlDecodeError::Limit {
                kind: "frame name bytes",
                actual: u64::try_from(header.name.len()).unwrap_or(u64::MAX),
                limit: u64::from(limits.max_name_bytes),
            });
        }
        frames.push(JxlFrameDescriptor {
            index,
            displayed: header.is_keyframe(),
            coding: coding(header.encoding),
            duration_ticks: header.duration,
            is_last: header.is_last,
            name: header.name.to_string(),
        });
    }
    Ok(frames)
}

fn extra_channels(
    channels: &[jxl_oxide::image::ExtraChannelInfo],
    limits: JxlDecodeLimits,
) -> Result<Vec<JxlExtraChannel>, JxlDecodeError> {
    limit(
        "extra channel count",
        channels.len(),
        limits.max_extra_channels,
    )?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(channels.len())
        .map_err(|_| JxlDecodeError::AllocationFailure)?;
    for (index, channel) in channels.iter().enumerate() {
        if channel.name.len() > usize::try_from(limits.max_name_bytes).unwrap_or(usize::MAX) {
            return Err(JxlDecodeError::Limit {
                kind: "extra channel name bytes",
                actual: u64::try_from(channel.name.len()).unwrap_or(u64::MAX),
                limit: u64::from(limits.max_name_bytes),
            });
        }
        let channel_type = match channel.ty {
            ExtraChannelType::Alpha { alpha_associated } => JxlExtraChannelType::Alpha {
                associated: alpha_associated,
            },
            ExtraChannelType::Depth => JxlExtraChannelType::Depth,
            ExtraChannelType::SpotColour {
                red,
                green,
                blue,
                solidity,
            } => JxlExtraChannelType::SpotColor {
                red,
                green,
                blue,
                solidity,
            },
            ExtraChannelType::SelectionMask => JxlExtraChannelType::SelectionMask,
            ExtraChannelType::Black => JxlExtraChannelType::Black,
            ExtraChannelType::Cfa { cfa_channel } => JxlExtraChannelType::Cfa {
                channel: cfa_channel,
            },
            ExtraChannelType::Thermal => JxlExtraChannelType::Thermal,
            ExtraChannelType::NonOptional => JxlExtraChannelType::NonOptional,
            ExtraChannelType::Optional => JxlExtraChannelType::Optional,
        };
        output.push(JxlExtraChannel {
            index,
            name: channel.name.to_string(),
            channel_type,
            bit_depth: bit_depth(channel.bit_depth),
            dimension_shift: channel.dim_shift,
        });
    }
    Ok(output)
}

fn color_encoding(
    image: &JxlImage,
    limits: JxlDecodeLimits,
) -> Result<JxlColorEncoding, JxlDecodeError> {
    let encoding = &image.image_header().metadata.colour_encoding;
    let color_space = color_space(encoding.colour_space())?;
    match encoding {
        ColourEncoding::Enum(value) => {
            let transfer = match value.tf {
                TransferFunction::Gamma { g, inverted } => {
                    let gamma = scaled_gamma(g);
                    JxlTransferFunction::Gamma(if inverted { gamma.recip() } else { gamma })
                }
                TransferFunction::Bt709 => JxlTransferFunction::Bt709,
                TransferFunction::Linear => JxlTransferFunction::Linear,
                TransferFunction::Srgb => JxlTransferFunction::Srgb,
                TransferFunction::Pq => JxlTransferFunction::Pq,
                TransferFunction::Dci => JxlTransferFunction::Dci,
                TransferFunction::Hlg => JxlTransferFunction::Hlg,
                TransferFunction::Unknown => return Err(JxlDecodeError::UnsupportedColorSpace),
            };
            Ok(JxlColorEncoding::Structured(JxlStructuredColor {
                color_space,
                white_point: match value.white_point {
                    WhitePoint::D65 => JxlWhitePoint::D65,
                    WhitePoint::E => JxlWhitePoint::EqualEnergy,
                    WhitePoint::Dci => JxlWhitePoint::Dci,
                    WhitePoint::Custom(value) => JxlWhitePoint::Custom(value.as_float()),
                },
                primaries: match value.primaries {
                    Primaries::Srgb => JxlPrimaries::Srgb,
                    Primaries::Bt2100 => JxlPrimaries::Bt2100,
                    Primaries::P3 => JxlPrimaries::P3,
                    Primaries::Custom { red, green, blue } => JxlPrimaries::Custom {
                        red: red.as_float(),
                        green: green.as_float(),
                        blue: blue.as_float(),
                    },
                },
                transfer,
                rendering_intent: match value.rendering_intent {
                    RenderingIntent::Perceptual => JxlRenderingIntent::Perceptual,
                    RenderingIntent::Relative => JxlRenderingIntent::Relative,
                    RenderingIntent::Saturation => JxlRenderingIntent::Saturation,
                    RenderingIntent::Absolute => JxlRenderingIntent::Absolute,
                },
            }))
        }
        ColourEncoding::IccProfile(_) => {
            let bytes = image
                .original_icc()
                .ok_or_else(|| JxlDecodeError::InvalidIcc("profile data is absent".to_owned()))?;
            let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            if length > limits.max_metadata_bytes {
                return Err(JxlDecodeError::Limit {
                    kind: "ICC bytes",
                    actual: length,
                    limit: limits.max_metadata_bytes,
                });
            }
            validate_icc(bytes, color_space)?;
            Ok(JxlColorEncoding::Icc(JxlIccProfile {
                color_space,
                bytes: bytes.to_vec(),
                sha256: Sha256::digest(bytes).into(),
            }))
        }
    }
}

fn validate_icc(bytes: &[u8], expected: JxlColorSpace) -> Result<(), JxlDecodeError> {
    if bytes.len() < 132 {
        return Err(JxlDecodeError::InvalidIcc(
            "profile is truncated".to_owned(),
        ));
    }
    let declared = be_u32(bytes, 0)? as usize;
    if declared != bytes.len() || bytes.get(36..40) != Some(b"acsp") {
        return Err(JxlDecodeError::InvalidIcc(
            "size or ICC signature is invalid".to_owned(),
        ));
    }
    let signature = bytes
        .get(16..20)
        .ok_or_else(|| JxlDecodeError::InvalidIcc("color signature is absent".to_owned()))?;
    if !matches!(
        (expected, signature),
        (JxlColorSpace::Rgb, b"RGB ") | (JxlColorSpace::Gray, b"GRAY")
    ) {
        return Err(JxlDecodeError::InvalidIcc(
            "profile color space conflicts with the image header".to_owned(),
        ));
    }
    let count = be_u32(bytes, 128)? as usize;
    let table_end = count
        .checked_mul(12)
        .and_then(|value| value.checked_add(132))
        .ok_or(JxlDecodeError::ArithmeticOverflow)?;
    if table_end > bytes.len() {
        return Err(JxlDecodeError::InvalidIcc(
            "tag table is truncated".to_owned(),
        ));
    }
    for index in 0..count {
        let entry = 132 + index * 12;
        let offset = be_u32(bytes, entry + 4)? as usize;
        let length = be_u32(bytes, entry + 8)? as usize;
        let end = offset
            .checked_add(length)
            .ok_or(JxlDecodeError::ArithmeticOverflow)?;
        if offset < 128 || end > bytes.len() {
            return Err(JxlDecodeError::InvalidIcc(
                "tag payload is outside the profile".to_owned(),
            ));
        }
    }
    Ok(())
}

fn render(
    image: &JxlImage,
    expected_dimensions: ImageDimensions,
    header: &JxlHeader,
    limits: JxlDecodeLimits,
) -> Result<JxlPixelData, JxlDecodeError> {
    let expected_channels = header.layout().channels();
    let samples = output_sample_count(expected_dimensions, expected_channels, limits)?;
    let render = catch_backend("frame render", || image.render_frame(0))?;
    let mut stream = render.stream();
    let dimensions = dimensions(stream.width(), stream.height(), limits)?;
    if dimensions != expected_dimensions {
        return Err(JxlDecodeError::Malformed(
            "backend output dimensions differ from the requested dimensions".to_owned(),
        ));
    }
    let channels =
        usize::try_from(stream.channels()).map_err(|_| JxlDecodeError::ArithmeticOverflow)?;
    if channels != expected_channels {
        return Err(JxlDecodeError::Malformed(
            "backend output channel layout differs from the JPEG XL header".to_owned(),
        ));
    }
    let mut output = Vec::new();
    output
        .try_reserve_exact(samples)
        .map_err(|_| JxlDecodeError::AllocationFailure)?;
    output.resize(samples, 0.0_f32);
    let written = catch_unwind(AssertUnwindSafe(|| stream.write_to_buffer(&mut output)))
        .map_err(|_| JxlDecodeError::Backend("backend panicked during sample copy".to_owned()))?;
    if written != samples {
        return Err(JxlDecodeError::Malformed(
            "backend produced a short sample buffer".to_owned(),
        ));
    }
    if output.iter().any(|sample| !sample.is_finite()) {
        return Err(JxlDecodeError::NonFiniteSamples);
    }
    Ok(JxlPixelData {
        dimensions,
        layout: header.layout(),
        alpha: header.alpha,
        samples: output,
    })
}

fn scale_nearest(
    source: &JxlPixelData,
    output_dimensions: ImageDimensions,
    limits: JxlDecodeLimits,
) -> Result<JxlPixelData, JxlDecodeError> {
    let channels = source.layout.channels();
    let sample_count = output_sample_count(output_dimensions, channels, limits)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(sample_count)
        .map_err(|_| JxlDecodeError::AllocationFailure)?;
    output.resize(sample_count, 0.0);
    let source_width = usize::try_from(source.dimensions.width())
        .map_err(|_| JxlDecodeError::ArithmeticOverflow)?;
    let output_width = usize::try_from(output_dimensions.width())
        .map_err(|_| JxlDecodeError::ArithmeticOverflow)?;
    for y in 0..usize::try_from(output_dimensions.height())
        .map_err(|_| JxlDecodeError::ArithmeticOverflow)?
    {
        let source_y = y * usize::try_from(source.dimensions.height()).unwrap_or(0)
            / usize::try_from(output_dimensions.height()).unwrap_or(1);
        for x in 0..output_width {
            let source_x = x * source_width / output_width;
            let source_offset = (source_y * source_width + source_x) * channels;
            let output_offset = (y * output_width + x) * channels;
            output[output_offset..output_offset + channels]
                .copy_from_slice(&source.samples[source_offset..source_offset + channels]);
        }
    }
    Ok(JxlPixelData {
        dimensions: output_dimensions,
        layout: source.layout,
        alpha: source.alpha,
        samples: output,
    })
}

fn crop(
    source: &JxlPixelData,
    region: Roi,
    limits: JxlDecodeLimits,
) -> Result<JxlPixelData, JxlDecodeError> {
    validate_region(region, source.dimensions)?;
    let output_dimensions = dimensions(region.width(), region.height(), limits)?;
    let channels = source.layout.channels();
    let sample_count = output_sample_count(output_dimensions, channels, limits)?;
    let source_width = usize::try_from(source.dimensions.width())
        .map_err(|_| JxlDecodeError::ArithmeticOverflow)?;
    let output_width =
        usize::try_from(region.width()).map_err(|_| JxlDecodeError::ArithmeticOverflow)?;
    let output_height =
        usize::try_from(region.height()).map_err(|_| JxlDecodeError::ArithmeticOverflow)?;
    let region_x = usize::try_from(region.x()).map_err(|_| JxlDecodeError::ArithmeticOverflow)?;
    let region_y = usize::try_from(region.y()).map_err(|_| JxlDecodeError::ArithmeticOverflow)?;
    let row_samples = output_width
        .checked_mul(channels)
        .ok_or(JxlDecodeError::ArithmeticOverflow)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(sample_count)
        .map_err(|_| JxlDecodeError::AllocationFailure)?;
    output.resize(sample_count, 0.0);
    for output_y in 0..output_height {
        let source_y = region_y
            .checked_add(output_y)
            .ok_or(JxlDecodeError::ArithmeticOverflow)?;
        let source_start = source_y
            .checked_mul(source_width)
            .and_then(|value| value.checked_add(region_x))
            .and_then(|value| value.checked_mul(channels))
            .ok_or(JxlDecodeError::ArithmeticOverflow)?;
        let source_end = source_start
            .checked_add(row_samples)
            .ok_or(JxlDecodeError::ArithmeticOverflow)?;
        let output_start = output_y
            .checked_mul(row_samples)
            .ok_or(JxlDecodeError::ArithmeticOverflow)?;
        let output_end = output_start
            .checked_add(row_samples)
            .ok_or(JxlDecodeError::ArithmeticOverflow)?;
        let source_row = source
            .samples
            .get(source_start..source_end)
            .ok_or_else(|| {
                JxlDecodeError::Malformed(
                    "full-frame sample buffer is shorter than its dimensions".to_owned(),
                )
            })?;
        output[output_start..output_end].copy_from_slice(source_row);
    }
    Ok(JxlPixelData {
        dimensions: output_dimensions,
        layout: source.layout,
        alpha: source.alpha,
        samples: output,
    })
}

fn thumbnail_dimensions(
    source: ImageDimensions,
    max_width: u32,
    max_height: u32,
) -> Result<ImageDimensions, JxlDecodeError> {
    if source.width() <= max_width && source.height() <= max_height {
        return Ok(source);
    }
    let width_scale = u64::from(max_width) * u64::from(source.height());
    let height_scale = u64::from(max_height) * u64::from(source.width());
    let (width, height) = if width_scale <= height_scale {
        let height = u64::from(source.height())
            .checked_mul(u64::from(max_width))
            .ok_or(JxlDecodeError::ArithmeticOverflow)?
            .div_ceil(u64::from(source.width()));
        (
            max_width.min(source.width()),
            u32::try_from(height.max(1)).map_err(|_| JxlDecodeError::ArithmeticOverflow)?,
        )
    } else {
        let width = u64::from(source.width())
            .checked_mul(u64::from(max_height))
            .ok_or(JxlDecodeError::ArithmeticOverflow)?
            .div_ceil(u64::from(source.height()));
        (
            u32::try_from(width.max(1)).map_err(|_| JxlDecodeError::ArithmeticOverflow)?,
            max_height.min(source.height()),
        )
    };
    ImageDimensions::new(width, height).map_err(|_| JxlDecodeError::ArithmeticOverflow)
}

fn dimensions(
    width: u32,
    height: u32,
    limits: JxlDecodeLimits,
) -> Result<ImageDimensions, JxlDecodeError> {
    if width > limits.max_width {
        return Err(limit_error("width", width, limits.max_width));
    }
    if height > limits.max_height {
        return Err(limit_error("height", height, limits.max_height));
    }
    let dimensions =
        ImageDimensions::new(width, height).map_err(|_| JxlDecodeError::ArithmeticOverflow)?;
    let pixels = dimensions
        .pixel_count()
        .map_err(|_| JxlDecodeError::ArithmeticOverflow)?;
    if pixels > limits.max_pixels {
        return Err(JxlDecodeError::Limit {
            kind: "pixels",
            actual: pixels,
            limit: limits.max_pixels,
        });
    }
    Ok(dimensions)
}

fn output_sample_count(
    dimensions: ImageDimensions,
    channels: usize,
    limits: JxlDecodeLimits,
) -> Result<usize, JxlDecodeError> {
    let samples = dimensions
        .pixel_count()
        .map_err(|_| JxlDecodeError::ArithmeticOverflow)?
        .checked_mul(u64::try_from(channels).map_err(|_| JxlDecodeError::ArithmeticOverflow)?)
        .ok_or(JxlDecodeError::ArithmeticOverflow)?;
    let bytes = samples
        .checked_mul(4)
        .ok_or(JxlDecodeError::ArithmeticOverflow)?;
    if bytes > limits.max_decoded_bytes {
        return Err(JxlDecodeError::Limit {
            kind: "decoded bytes",
            actual: bytes,
            limit: limits.max_decoded_bytes,
        });
    }
    if bytes > limits.max_backend_alloc_bytes {
        return Err(JxlDecodeError::Limit {
            kind: "backend allocation bytes",
            actual: bytes,
            limit: limits.max_backend_alloc_bytes,
        });
    }
    usize::try_from(samples).map_err(|_| JxlDecodeError::ArithmeticOverflow)
}

fn validate_region(region: Roi, dimensions: ImageDimensions) -> Result<(), JxlDecodeError> {
    if region.is_empty() || region.within(dimensions).is_err() {
        Err(JxlDecodeError::InvalidRegion)
    } else {
        Ok(())
    }
}

fn color_space(value: ColourSpace) -> Result<JxlColorSpace, JxlDecodeError> {
    match value {
        ColourSpace::Rgb => Ok(JxlColorSpace::Rgb),
        ColourSpace::Grey => Ok(JxlColorSpace::Gray),
        ColourSpace::Xyb | ColourSpace::Unknown => Err(JxlDecodeError::UnsupportedColorSpace),
    }
}

#[allow(clippy::cast_precision_loss)]
fn scaled_gamma(value: u32) -> f32 {
    value as f32 / 10_000_000.0
}

fn orientation(value: u32) -> Result<Orientation, JxlDecodeError> {
    match value {
        1 => Ok(Orientation::Normal),
        2 => Ok(Orientation::FlipHorizontal),
        3 => Ok(Orientation::Rotate180),
        4 => Ok(Orientation::FlipVertical),
        5 => Ok(Orientation::Transpose),
        6 => Ok(Orientation::Rotate90),
        7 => Ok(Orientation::Transverse),
        8 => Ok(Orientation::Rotate270),
        _ => Err(JxlDecodeError::Malformed("invalid orientation".to_owned())),
    }
}

const fn bit_depth(value: BitDepth) -> JxlBitDepth {
    match value {
        BitDepth::IntegerSample { bits_per_sample } => JxlBitDepth::Integer { bits_per_sample },
        BitDepth::FloatSample {
            bits_per_sample,
            exp_bits,
        } => JxlBitDepth::Float {
            bits_per_sample,
            exponent_bits: exp_bits,
        },
    }
}

const fn coding(value: Encoding) -> JxlCodingMode {
    match value {
        Encoding::VarDct => JxlCodingMode::VarDct,
        Encoding::Modular => JxlCodingMode::Modular,
    }
}

fn allocation_tracker(limits: JxlDecodeLimits) -> Result<AllocTracker, JxlDecodeError> {
    let bytes =
        usize::try_from(limits.max_backend_alloc_bytes).map_err(|_| JxlDecodeError::Limit {
            kind: "backend allocation bytes",
            actual: limits.max_backend_alloc_bytes,
            limit: usize::MAX as u64,
        })?;
    Ok(AllocTracker::with_limit(bytes))
}

fn limit(kind: &'static str, actual: usize, limit: u32) -> Result<(), JxlDecodeError> {
    if actual > usize::try_from(limit).unwrap_or(usize::MAX) {
        Err(JxlDecodeError::Limit {
            kind,
            actual: u64::try_from(actual).unwrap_or(u64::MAX),
            limit: u64::from(limit),
        })
    } else {
        Ok(())
    }
}

fn limit_error(kind: &'static str, actual: u32, limit: u32) -> JxlDecodeError {
    JxlDecodeError::Limit {
        kind,
        actual: u64::from(actual),
        limit: u64::from(limit),
    }
}

fn be_u32(bytes: &[u8], offset: usize) -> Result<u32, JxlDecodeError> {
    let end = offset
        .checked_add(4)
        .ok_or(JxlDecodeError::ArithmeticOverflow)?;
    let value = bytes
        .get(offset..end)
        .ok_or_else(|| JxlDecodeError::InvalidIcc("field is truncated".to_owned()))?;
    Ok(u32::from_be_bytes(
        value
            .try_into()
            .map_err(|_| JxlDecodeError::ArithmeticOverflow)?,
    ))
}

fn catch_backend<T>(
    operation: &'static str,
    call: impl FnOnce() -> jxl_oxide::Result<T>,
) -> Result<T, JxlDecodeError> {
    catch_unwind(AssertUnwindSafe(call))
        .map_err(|_| JxlDecodeError::Backend(format!("backend panicked during {operation}")))?
        .map_err(|error| JxlDecodeError::Backend(format!("{operation}: {error}")))
}
