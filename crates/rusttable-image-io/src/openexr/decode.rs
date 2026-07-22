use std::io::Cursor;
use std::panic::{AssertUnwindSafe, catch_unwind};

use exr::block::{BlockIndex, UncompressedBlock};
use exr::image::read::image::{LayersReader, ReadImage, ReadLayers};
use exr::image::read::layers::{ChannelsReader, ReadChannels};
use exr::image::read::samples::ReadFlatSamples;
use exr::image::{Blocks, Encoding, FlatSamples, Layer, Levels};
use exr::math::{RoundingMode, Vec2};
use exr::meta::BlockDescription;
use exr::meta::attribute::LevelMode;
use rusttable_image::{
    ChannelLayout, DecodeLimits, ImageDimensions, ImageInputError, ImageProbe, InputFormat, Roi,
};
use sha2::{Digest, Sha256};

use super::inspect::{Inspection, inspect};
use super::selection::{Selection, mapping_alpha, mapping_names, select};
use super::types::{
    EXR_BACKEND_ID, ExrAlphaAssociation, ExrChannelMapping, ExrDecodeError, ExrDecodeLimits,
    ExrDecodeMode, ExrDecodeReceipt, ExrDecodeRequest, ExrDecodeResult, ExrHeader, ExrLevelIndex,
    ExrMissingChannelFill, ExrPixelData, ExrSampleData, ExrSampleType, ExrWindow,
};
use crate::raw::{RawByteSource, RawSourceError, SliceRawSource};

const MAGIC: [u8; 4] = [0x76, 0x2f, 0x31, 0x01];
const COPY_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, Default)]
pub struct ExrDecoder;

impl ExrDecoder {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Parses all bounded `OpenEXR` headers, parts, channels, levels, and chunk tables.
    ///
    /// # Errors
    ///
    /// Returns typed limit, malformed-input, or unsupported-capability errors.
    pub fn inspect_bytes(
        &self,
        bytes: &[u8],
        limits: ExrDecodeLimits,
    ) -> Result<ExrHeader, ExrDecodeError> {
        Ok(inspect(bytes, limits)?.header)
    }

    /// Decodes an immutable byte snapshot without publishing partial output.
    ///
    /// # Errors
    ///
    /// Returns typed source, selection, limit, malformed-input, or backend errors.
    pub fn decode_bytes(
        &self,
        bytes: &[u8],
        request: &ExrDecodeRequest,
    ) -> Result<ExrDecodeResult, ExrDecodeError> {
        self.decode_source(&SliceRawSource::new(bytes), request)
    }

    /// Copies a bounded source and rejects cancellation or source mutation.
    ///
    /// # Errors
    ///
    /// Returns typed source, selection, limit, malformed-input, or backend errors.
    pub fn decode_source<S: RawByteSource + ?Sized>(
        &self,
        source: &S,
        request: &ExrDecodeRequest,
    ) -> Result<ExrDecodeResult, ExrDecodeError> {
        check_cancel(request)?;
        let snapshot = Snapshot::read(source, request)?;
        check_cancel(request)?;
        let inspection = inspect(&snapshot.bytes, request.limits)?;
        let selection = select(&inspection.header, request)?;
        let part = selection.part.clone();
        let header_only = request.mode == ExrDecodeMode::Header;
        let level_window = level_display_window(&inspection, part.index, request.level)?;
        let region = requested_region(request.mode, level_window)?;
        let pixels = if header_only {
            None
        } else {
            Some(decode_selected(
                &snapshot.bytes,
                &inspection,
                &selection,
                request.level,
                region,
                request,
            )?)
        };
        check_cancel(request)?;
        let channels = mapping_names(&selection.mapping);
        let output_origin = pixels
            .as_ref()
            .map_or([level_window.x, level_window.y], |pixels| pixels.origin);
        let output_size = pixels
            .as_ref()
            .map_or([level_window.width, level_window.height], |pixels| {
                [pixels.dimensions.width(), pixels.dimensions.height()]
            });
        let output_bytes = pixels.as_ref().map_or(0, |pixels| {
            pixels.samples.len() as u64 * pixels.sample_type.bytes()
        });
        let receipt = ExrDecodeReceipt {
            backend: EXR_BACKEND_ID.to_owned(),
            source_bytes: snapshot.source_bytes,
            source_sha256: snapshot.sha256,
            version: inspection.header.version,
            flags: inspection.header.flags,
            part_index: part.index,
            part_name: part.name.clone(),
            layer: selection.group.layer.clone(),
            view: selection.group.view.clone(),
            channels,
            sample_type: selection.sample_type,
            compression: part.compression,
            data_window: part.data_window,
            display_window: part.display_window,
            storage: part.storage,
            level: request.level,
            region,
            output_origin,
            output_size,
            output_bytes,
            decompressed_bytes: if header_only {
                0
            } else {
                logical_level_bytes(&selection, output_size)?
            },
            alpha: if mapping_alpha(&selection.mapping).is_some() {
                ExrAlphaAssociation::Associated
            } else {
                ExrAlphaAssociation::None
            },
            fill: ExrMissingChannelFill {
                color: 0.0,
                alpha: 1.0,
                outside_data: 0.0,
            },
            metadata: part.metadata.clone(),
            header_only,
        };
        Ok(ExrDecodeResult {
            header: inspection.header,
            part,
            pixels,
            receipt,
        })
    }
}

pub(crate) fn is_exr_signature(bytes: &[u8]) -> bool {
    bytes.starts_with(&MAGIC)
}

pub(crate) fn decode_exr_probe(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<ImageProbe, ImageInputError> {
    let header = ExrDecoder::new()
        .inspect_bytes(bytes, ExrDecodeLimits::from_common(limits))
        .map_err(map_input_error)?;
    let part = header
        .default_part
        .and_then(|index| header.parts.get(index))
        .ok_or_else(|| unsupported_input("no decodable RGB or Y OpenEXR part"))?;
    Ok(ImageProbe::new(
        InputFormat::OpenExr,
        part.display_window.dimensions().map_err(map_input_error)?,
    ))
}

pub(crate) fn decode_legacy_rgba8(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<(ImageDimensions, Vec<u8>), ImageInputError> {
    let request = ExrDecodeRequest::new(ExrDecodeLimits::from_common(limits));
    let result = ExrDecoder::new()
        .decode_bytes(bytes, &request)
        .map_err(map_input_error)?;
    let pixels = result
        .pixels
        .ok_or_else(|| malformed_input("full OpenEXR decode returned no pixels"))?;
    let rgba = to_rgba8(&pixels)?;
    Ok((pixels.dimensions, rgba))
}

fn decode_selected(
    bytes: &[u8],
    inspection: &Inspection,
    selection: &Selection<'_>,
    level: ExrLevelIndex,
    region: Option<Roi>,
    request: &ExrDecodeRequest,
) -> Result<ExrPixelData, ExrDecodeError> {
    let channels = ReadFlatSamples.all_resolution_levels().all_channels();
    let specification = SelectedPart {
        index: selection.part.index,
        channels,
    };
    let image = catch_unwind(AssertUnwindSafe(|| {
        ReadImage::new(specification, |_| {})
            .pedantic()
            .non_parallel()
            .from_buffered(Cursor::new(bytes))
    }))
    .map_err(|_| ExrDecodeError::Backend("decoder panicked".to_owned()))?
    .map_err(map_exr_error)?;
    check_cancel(request)?;
    let backend_header = inspection
        .meta
        .headers
        .get(selection.part.index)
        .ok_or(ExrDecodeError::InvalidPart(selection.part.index))?;
    let data_window = level_data_window(backend_header, level)?;
    let display_window = level_display_window(inspection, selection.part.index, level)?;
    let output = output_window(display_window, region)?;
    let layout = mapping_layout(&selection.mapping);
    let channel_count = layout.channels();
    let sample_count = u64::from(output.width)
        .checked_mul(u64::from(output.height))
        .and_then(|pixels| pixels.checked_mul(channel_count as u64))
        .ok_or(ExrDecodeError::ArithmeticOverflow)?;
    let output_bytes = sample_count
        .checked_mul(selection.sample_type.bytes())
        .ok_or(ExrDecodeError::ArithmeticOverflow)?;
    enforce_limit(
        "decoded bytes",
        output_bytes,
        request.limits.max_decoded_bytes,
    )?;
    let selected = mapping_names(&selection.mapping)
        .iter()
        .map(|name| {
            image
                .layer_data
                .channel_data
                .list
                .iter()
                .find(|channel| text(&channel.name) == *name)
                .ok_or_else(|| ExrDecodeError::Malformed(format!("backend omitted channel {name}")))
                .and_then(|channel| level_samples(&channel.sample_data, level))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let samples = match selection.sample_type {
        ExrSampleType::F16 => ExrSampleData::F16(interleave_f16(
            &selected,
            data_window,
            output,
            channel_count,
        )?),
        ExrSampleType::F32 => ExrSampleData::F32(interleave_f32(
            &selected,
            data_window,
            output,
            channel_count,
        )?),
        ExrSampleType::U32 => {
            return Err(ExrDecodeError::UnsupportedSampleType {
                channel: selection.channels[0].name.clone(),
            });
        }
    };
    Ok(ExrPixelData {
        dimensions: output.dimensions()?,
        origin: [output.x, output.y],
        layout,
        sample_type: selection.sample_type,
        samples,
    })
}

struct SelectedPart<C> {
    index: usize,
    channels: C,
}

struct SelectedPartReader<C> {
    index: usize,
    channels: C,
    attributes: exr::meta::header::LayerAttributes,
    size: Vec2<usize>,
    encoding: Encoding,
}

impl<'s, C> ReadLayers<'s> for SelectedPart<C>
where
    C: ReadChannels<'s>,
{
    type Layers = Layer<<C::Reader as ChannelsReader>::Channels>;
    type Reader = SelectedPartReader<C::Reader>;

    fn create_layers_reader(
        &'s self,
        headers: &[exr::meta::header::Header],
    ) -> exr::error::Result<Self::Reader> {
        let header = headers.get(self.index).ok_or_else(|| {
            exr::error::Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "selected part does not exist",
            ))
        })?;
        Ok(SelectedPartReader {
            index: self.index,
            channels: self.channels.create_channels_reader(header)?,
            attributes: header.own_attributes.clone(),
            size: header.layer_size,
            encoding: Encoding {
                compression: header.compression,
                line_order: header.line_order,
                blocks: match header.blocks {
                    BlockDescription::ScanLines => Blocks::ScanLines,
                    BlockDescription::Tiles(tile) => Blocks::Tiles(tile.tile_size),
                },
            },
        })
    }
}

impl<C: ChannelsReader> LayersReader for SelectedPartReader<C> {
    type Layers = Layer<C::Channels>;

    fn filter_block(
        &self,
        _meta: &exr::meta::MetaData,
        tile: exr::block::chunk::TileCoordinates,
        block: BlockIndex,
    ) -> bool {
        block.layer == self.index && self.channels.filter_block(tile)
    }

    fn read_block(
        &mut self,
        headers: &[exr::meta::header::Header],
        block: UncompressedBlock,
    ) -> exr::error::UnitResult {
        self.channels.read_block(&headers[self.index], block)
    }

    fn into_layers(self) -> Self::Layers {
        Layer {
            channel_data: self.channels.into_channels(),
            attributes: self.attributes,
            size: self.size,
            encoding: self.encoding,
        }
    }
}

fn level_samples(
    levels: &Levels<FlatSamples>,
    level: ExrLevelIndex,
) -> Result<&FlatSamples, ExrDecodeError> {
    match levels {
        Levels::Singular(samples) if level == ExrLevelIndex::default() => Ok(samples),
        Levels::Mip { level_data, .. } if level.x == level.y => level_data
            .get(level.x as usize)
            .ok_or(ExrDecodeError::InvalidLevel(level)),
        Levels::Singular(_) | Levels::Mip { .. } => Err(ExrDecodeError::InvalidLevel(level)),
        Levels::Rip { level_data, .. } => {
            let width = level_data.level_count.width();
            let index = (level.y as usize)
                .checked_mul(width)
                .and_then(|value| value.checked_add(level.x as usize))
                .ok_or(ExrDecodeError::ArithmeticOverflow)?;
            if level.x as usize >= width || level.y as usize >= level_data.level_count.height() {
                return Err(ExrDecodeError::InvalidLevel(level));
            }
            level_data
                .map_data
                .get(index)
                .ok_or(ExrDecodeError::InvalidLevel(level))
        }
    }
}

fn interleave_f16(
    channels: &[&FlatSamples],
    data: ExrWindow,
    output: ExrWindow,
    channel_count: usize,
) -> Result<Vec<u16>, ExrDecodeError> {
    let length = output_len(output, channel_count)?;
    let mut result = Vec::new();
    result
        .try_reserve_exact(length)
        .map_err(|_| ExrDecodeError::AllocationFailure)?;
    result.resize(length, 0);
    for_each_data_pixel(data, output, |source, destination| {
        for (channel_index, channel) in channels.iter().enumerate() {
            let value = match channel {
                FlatSamples::F16(values) => values[source].to_bits(),
                _ => {
                    return Err(ExrDecodeError::Malformed(
                        "F16 selection changed type".to_owned(),
                    ));
                }
            };
            result[destination * channel_count + channel_index] = value;
        }
        Ok(())
    })?;
    Ok(result)
}

fn interleave_f32(
    channels: &[&FlatSamples],
    data: ExrWindow,
    output: ExrWindow,
    channel_count: usize,
) -> Result<Vec<f32>, ExrDecodeError> {
    let length = output_len(output, channel_count)?;
    let mut result = Vec::new();
    result
        .try_reserve_exact(length)
        .map_err(|_| ExrDecodeError::AllocationFailure)?;
    result.resize(length, 0.0);
    for_each_data_pixel(data, output, |source, destination| {
        for (channel_index, channel) in channels.iter().enumerate() {
            let value = match channel {
                FlatSamples::F16(values) => values[source].to_f32(),
                FlatSamples::F32(values) => values[source],
                FlatSamples::U32(_) => {
                    return Err(ExrDecodeError::UnsupportedSampleType {
                        channel: channel_index.to_string(),
                    });
                }
            };
            result[destination * channel_count + channel_index] = value;
        }
        Ok(())
    })?;
    Ok(result)
}

fn for_each_data_pixel(
    data: ExrWindow,
    output: ExrWindow,
    mut insert: impl FnMut(usize, usize) -> Result<(), ExrDecodeError>,
) -> Result<(), ExrDecodeError> {
    let left = data.x.max(output.x);
    let top = data.y.max(output.y);
    let right = window_end_x(data)?.min(window_end_x(output)?);
    let bottom = window_end_y(data)?.min(window_end_y(output)?);
    if right <= left || bottom <= top {
        return Ok(());
    }
    for y in top..bottom {
        for x in left..right {
            let source = index_in(data, x, y)?;
            let destination = index_in(output, x, y)?;
            insert(source, destination)?;
        }
    }
    Ok(())
}

fn output_len(window: ExrWindow, channels: usize) -> Result<usize, ExrDecodeError> {
    usize::try_from(window.width)
        .ok()
        .and_then(|width| width.checked_mul(window.height as usize))
        .and_then(|pixels| pixels.checked_mul(channels))
        .ok_or(ExrDecodeError::ArithmeticOverflow)
}

fn index_in(window: ExrWindow, x: i32, y: i32) -> Result<usize, ExrDecodeError> {
    let local_x = usize::try_from(
        x.checked_sub(window.x)
            .ok_or(ExrDecodeError::ArithmeticOverflow)?,
    )
    .map_err(|_| ExrDecodeError::ArithmeticOverflow)?;
    let local_y = usize::try_from(
        y.checked_sub(window.y)
            .ok_or(ExrDecodeError::ArithmeticOverflow)?,
    )
    .map_err(|_| ExrDecodeError::ArithmeticOverflow)?;
    local_y
        .checked_mul(window.width as usize)
        .and_then(|row| row.checked_add(local_x))
        .ok_or(ExrDecodeError::ArithmeticOverflow)
}

fn requested_region(mode: ExrDecodeMode, window: ExrWindow) -> Result<Option<Roi>, ExrDecodeError> {
    match mode {
        ExrDecodeMode::Header | ExrDecodeMode::Full => Ok(None),
        ExrDecodeMode::Region(roi) => {
            let dimensions = window.dimensions()?;
            roi.within(dimensions)
                .map_err(|_| ExrDecodeError::InvalidRegion)?;
            if roi.is_empty() {
                return Err(ExrDecodeError::InvalidRegion);
            }
            Ok(Some(roi))
        }
    }
}

fn output_window(display: ExrWindow, region: Option<Roi>) -> Result<ExrWindow, ExrDecodeError> {
    let Some(region) = region else {
        return Ok(display);
    };
    Ok(ExrWindow {
        x: display
            .x
            .checked_add(i32::try_from(region.x()).map_err(|_| ExrDecodeError::ArithmeticOverflow)?)
            .ok_or(ExrDecodeError::ArithmeticOverflow)?,
        y: display
            .y
            .checked_add(i32::try_from(region.y()).map_err(|_| ExrDecodeError::ArithmeticOverflow)?)
            .ok_or(ExrDecodeError::ArithmeticOverflow)?,
        width: region.width(),
        height: region.height(),
    })
}

fn level_data_window(
    header: &exr::meta::header::Header,
    level: ExrLevelIndex,
) -> Result<ExrWindow, ExrDecodeError> {
    let (width, height) = level_dimensions(header, level)?;
    Ok(ExrWindow {
        x: header.own_attributes.layer_position.x(),
        y: header.own_attributes.layer_position.y(),
        width,
        height,
    })
}

fn level_display_window(
    inspection: &Inspection,
    part: usize,
    level: ExrLevelIndex,
) -> Result<ExrWindow, ExrDecodeError> {
    let header = inspection
        .meta
        .headers
        .get(part)
        .ok_or(ExrDecodeError::InvalidPart(part))?;
    let display = inspection.header.parts[part].display_window;
    if level == ExrLevelIndex::default() {
        return Ok(display);
    }
    let rounding = match header.blocks {
        BlockDescription::Tiles(tile) => tile.rounding_mode,
        BlockDescription::ScanLines => return Err(ExrDecodeError::InvalidLevel(level)),
    };
    Ok(ExrWindow {
        x: display.x,
        y: display.y,
        width: level_size(rounding, display.width, level.x)?,
        height: level_size(rounding, display.height, level.y)?,
    })
}

fn level_dimensions(
    header: &exr::meta::header::Header,
    level: ExrLevelIndex,
) -> Result<(u32, u32), ExrDecodeError> {
    match header.blocks {
        BlockDescription::ScanLines if level == ExrLevelIndex::default() => Ok((
            u32::try_from(header.layer_size.width())
                .map_err(|_| ExrDecodeError::ArithmeticOverflow)?,
            u32::try_from(header.layer_size.height())
                .map_err(|_| ExrDecodeError::ArithmeticOverflow)?,
        )),
        BlockDescription::ScanLines => Err(ExrDecodeError::InvalidLevel(level)),
        BlockDescription::Tiles(tile) => {
            match tile.level_mode {
                LevelMode::Singular if level != ExrLevelIndex::default() => {
                    return Err(ExrDecodeError::InvalidLevel(level));
                }
                LevelMode::MipMap if level.x != level.y => {
                    return Err(ExrDecodeError::InvalidLevel(level));
                }
                _ => {}
            }
            Ok((
                level_size(
                    tile.rounding_mode,
                    u32::try_from(header.layer_size.width())
                        .map_err(|_| ExrDecodeError::ArithmeticOverflow)?,
                    level.x,
                )?,
                level_size(
                    tile.rounding_mode,
                    u32::try_from(header.layer_size.height())
                        .map_err(|_| ExrDecodeError::ArithmeticOverflow)?,
                    level.y,
                )?,
            ))
        }
    }
}

fn level_size(rounding: RoundingMode, full: u32, level: u32) -> Result<u32, ExrDecodeError> {
    if level >= u32::BITS {
        return Err(ExrDecodeError::InvalidLevel(ExrLevelIndex::new(
            level, level,
        )));
    }
    let divisor = 1u32
        .checked_shl(level)
        .ok_or(ExrDecodeError::ArithmeticOverflow)?;
    Ok(match rounding {
        RoundingMode::Down => full / divisor,
        RoundingMode::Up => full.div_ceil(divisor),
    }
    .max(1))
}

fn mapping_layout(mapping: &ExrChannelMapping) -> ChannelLayout {
    match mapping {
        ExrChannelMapping::Gray { alpha: None, .. } => ChannelLayout::Gray,
        ExrChannelMapping::Gray { alpha: Some(_), .. } => ChannelLayout::GrayA,
        ExrChannelMapping::Rgb { alpha: None, .. } => ChannelLayout::Rgb,
        ExrChannelMapping::Rgb { alpha: Some(_), .. } => ChannelLayout::Rgba,
    }
}

fn logical_level_bytes(selection: &Selection<'_>, output: [u32; 2]) -> Result<u64, ExrDecodeError> {
    u64::from(output[0])
        .checked_mul(u64::from(output[1]))
        .and_then(|pixels| pixels.checked_mul(selection.channels.len() as u64))
        .and_then(|samples| samples.checked_mul(selection.sample_type.bytes()))
        .ok_or(ExrDecodeError::ArithmeticOverflow)
}

fn to_rgba8(pixels: &ExrPixelData) -> Result<Vec<u8>, ImageInputError> {
    let count = pixels
        .dimensions
        .pixel_count()
        .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let count = usize::try_from(count).map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let source_channels = pixels.layout.channels();
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(
        count
            .checked_mul(4)
            .ok_or(ImageInputError::ArithmeticOverflow)?,
    )
    .map_err(|_| ImageInputError::AllocationFailure)?;
    for index in 0..count {
        let sample = |channel: usize| -> f32 {
            match &pixels.samples {
                ExrSampleData::F16(values) => {
                    half::f16::from_bits(values[index * source_channels + channel]).to_f32()
                }
                ExrSampleData::F32(values) => values[index * source_channels + channel],
            }
        };
        let (red, green, blue, alpha) = match pixels.layout {
            ChannelLayout::Gray => (sample(0), sample(0), sample(0), 1.0),
            ChannelLayout::GrayA => (sample(0), sample(0), sample(0), sample(1)),
            ChannelLayout::Rgb => (sample(0), sample(1), sample(2), 1.0),
            ChannelLayout::Rgba => (sample(0), sample(1), sample(2), sample(3)),
            ChannelLayout::Bayer | ChannelLayout::XTrans => {
                return Err(unsupported_input("mosaic OpenEXR output"));
            }
        };
        rgba.extend([red, green, blue, alpha].map(float_to_u8));
    }
    Ok(rgba)
}

fn float_to_u8(value: f32) -> u8 {
    if value.is_nan() {
        0
    } else {
        let rounded = (value.clamp(0.0, 1.0) * 255.0).round();
        (0_u8..=u8::MAX)
            .take_while(|candidate| f32::from(*candidate) <= rounded)
            .last()
            .unwrap_or(0)
    }
}

fn window_end_x(window: ExrWindow) -> Result<i32, ExrDecodeError> {
    window
        .x
        .checked_add(i32::try_from(window.width).map_err(|_| ExrDecodeError::ArithmeticOverflow)?)
        .ok_or(ExrDecodeError::ArithmeticOverflow)
}

fn window_end_y(window: ExrWindow) -> Result<i32, ExrDecodeError> {
    window
        .y
        .checked_add(i32::try_from(window.height).map_err(|_| ExrDecodeError::ArithmeticOverflow)?)
        .ok_or(ExrDecodeError::ArithmeticOverflow)
}

fn text(value: &exr::meta::attribute::Text) -> String {
    value
        .as_slice()
        .iter()
        .map(|byte| char::from(*byte))
        .collect()
}

fn check_cancel(request: &ExrDecodeRequest) -> Result<(), ExrDecodeError> {
    if request.cancellation.is_cancelled() {
        Err(ExrDecodeError::Cancelled)
    } else {
        Ok(())
    }
}

fn enforce_limit(kind: &'static str, actual: u64, limit: u64) -> Result<(), ExrDecodeError> {
    if actual > limit {
        Err(ExrDecodeError::Limit {
            kind,
            actual,
            limit,
        })
    } else {
        Ok(())
    }
}

fn map_exr_error(error: exr::error::Error) -> ExrDecodeError {
    match error {
        exr::error::Error::NotSupported(message) => ExrDecodeError::Backend(message.into_owned()),
        exr::error::Error::Invalid(message) => ExrDecodeError::Malformed(message.into_owned()),
        exr::error::Error::Io(error) => ExrDecodeError::Malformed(error.to_string()),
        exr::error::Error::Aborted => ExrDecodeError::Cancelled,
    }
}

fn map_input_error(error: ExrDecodeError) -> ImageInputError {
    match error {
        ExrDecodeError::Source(RawSourceError::TooLarge { actual, limit })
        | ExrDecodeError::Limit {
            kind: "source bytes",
            actual,
            limit,
        } => ImageInputError::SourceTooLarge { actual, limit },
        ExrDecodeError::Limit {
            kind: "width",
            actual,
            limit,
        } => ImageInputError::WidthLimit {
            actual: u32::try_from(actual).unwrap_or(u32::MAX),
            limit: u32::try_from(limit).unwrap_or(u32::MAX),
        },
        ExrDecodeError::Limit {
            kind: "height",
            actual,
            limit,
        } => ImageInputError::HeightLimit {
            actual: u32::try_from(actual).unwrap_or(u32::MAX),
            limit: u32::try_from(limit).unwrap_or(u32::MAX),
        },
        ExrDecodeError::Limit {
            kind: "pixel count",
            actual,
            limit,
        } => ImageInputError::PixelLimit { actual, limit },
        ExrDecodeError::Limit {
            kind: "decoded bytes",
            actual,
            limit,
        } => ImageInputError::DecodedByteLimit { actual, limit },
        ExrDecodeError::ArithmeticOverflow => ImageInputError::ArithmeticOverflow,
        ExrDecodeError::AllocationFailure
        | ExrDecodeError::Source(RawSourceError::AllocationFailure) => {
            ImageInputError::AllocationFailure
        }
        ExrDecodeError::UnsupportedDeepData { .. }
        | ExrDecodeError::UnsupportedSampleType { .. }
        | ExrDecodeError::UnsupportedCompression { .. }
        | ExrDecodeError::UnsupportedFlags(_)
        | ExrDecodeError::UnsupportedVersion(_) => unsupported_input(&error.to_string()),
        other => malformed_input(&other.to_string()),
    }
}

fn malformed_input(message: &str) -> ImageInputError {
    ImageInputError::MalformedInput {
        format: InputFormat::OpenExr,
        message: message.to_owned(),
    }
}

fn unsupported_input(_message: &str) -> ImageInputError {
    ImageInputError::UnsupportedFeature {
        format: InputFormat::OpenExr,
        reason: rusttable_image::UnsupportedImageFeature::SampleFormat,
    }
}

struct Snapshot {
    bytes: Vec<u8>,
    source_bytes: u64,
    sha256: [u8; 32],
}

impl Snapshot {
    fn read<S: RawByteSource + ?Sized>(
        source: &S,
        request: &ExrDecodeRequest,
    ) -> Result<Self, ExrDecodeError> {
        let length = source.len().map_err(ExrDecodeError::Source)?;
        if length == 0 {
            return Err(ExrDecodeError::Source(RawSourceError::Empty));
        }
        if length > request.limits.max_source_bytes {
            return Err(ExrDecodeError::Source(RawSourceError::TooLarge {
                actual: length,
                limit: request.limits.max_source_bytes,
            }));
        }
        let revision = source.revision().map_err(ExrDecodeError::Source)?;
        let length = usize::try_from(length)
            .map_err(|_| ExrDecodeError::Source(RawSourceError::LengthConversion))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|_| ExrDecodeError::AllocationFailure)?;
        bytes.resize(length, 0);
        for (index, chunk) in bytes.chunks_mut(COPY_CHUNK_BYTES).enumerate() {
            check_cancel(request)?;
            source
                .read_exact_at((index * COPY_CHUNK_BYTES) as u64, chunk)
                .map_err(ExrDecodeError::Source)?;
        }
        if source.revision().map_err(ExrDecodeError::Source)? != revision {
            return Err(ExrDecodeError::Source(RawSourceError::Changed));
        }
        let sha256 = Sha256::digest(&bytes).into();
        Ok(Self {
            bytes,
            source_bytes: length as u64,
            sha256,
        })
    }
}
