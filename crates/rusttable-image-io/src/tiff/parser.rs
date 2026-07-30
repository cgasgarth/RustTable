use std::collections::{HashSet, VecDeque};

use rusttable_image::{ImageDimensions, Orientation};

use super::types::{
    TiffAlphaSample, TiffByteOrder, TiffChunkKind, TiffChunkLayout, TiffCompression, TiffContainer,
    TiffDataLocation, TiffDecodeError, TiffDecodeLimits, TiffHeader, TiffMetadataInventory,
    TiffPage, TiffPhotometric, TiffPredictor, TiffSampleFormat, TiffStorageLayout,
};

mod dng;

const TAG_SUBFILE_TYPE: u16 = 254;
const TAG_WIDTH: u16 = 256;
const TAG_HEIGHT: u16 = 257;
const TAG_BITS: u16 = 258;
const TAG_COMPRESSION: u16 = 259;
const TAG_PHOTOMETRIC: u16 = 262;
const TAG_ORIENTATION: u16 = 274;
const TAG_STRIP_OFFSETS: u16 = 273;
const TAG_SAMPLES: u16 = 277;
const TAG_ROWS_PER_STRIP: u16 = 278;
const TAG_STRIP_COUNTS: u16 = 279;
const TAG_PLANAR: u16 = 284;
const TAG_PREDICTOR: u16 = 317;
const TAG_COLOR_MAP: u16 = 320;
const TAG_TILE_WIDTH: u16 = 322;
const TAG_TILE_HEIGHT: u16 = 323;
const TAG_TILE_OFFSETS: u16 = 324;
const TAG_TILE_COUNTS: u16 = 325;
const TAG_SUB_IFDS: u16 = 330;
const TAG_EXTRA_SAMPLES: u16 = 338;
const TAG_SAMPLE_FORMAT: u16 = 339;
const TAG_IPTC: u16 = 33_723;
const TAG_PHOTOSHOP: u16 = 34_377;
const TAG_ICC: u16 = 34_675;
const TAG_EXIF_IFD: u16 = 34_665;
const TAG_GPS_IFD: u16 = 34_853;
const TAG_XMP: u16 = 700;
const TAG_YCBCR_SUBSAMPLING: u16 = 530;
#[derive(Debug)]
pub(crate) struct ParsedTiff {
    pub header: TiffHeader,
}

#[derive(Debug, Clone)]
struct Entry {
    tag: u16,
    field_type: u16,
    count: u64,
    data: TiffDataLocation,
}

struct Parser<'a> {
    bytes: &'a [u8],
    limits: TiffDecodeLimits,
    order: TiffByteOrder,
    container: TiffContainer,
    total_tags: u64,
    total_metadata: u64,
    structural_ranges: Vec<(u64, u64)>,
    chunk_ranges: Vec<(u64, u64)>,
}

pub(crate) fn parse(bytes: &[u8], limits: TiffDecodeLimits) -> Result<ParsedTiff, TiffDecodeError> {
    let source_len = u64::try_from(bytes.len()).map_err(|_| TiffDecodeError::ArithmeticOverflow)?;
    check_limit("source bytes", source_len, limits.max_source_bytes)?;
    let (order, container, first_ifd) = parse_header(bytes)?;
    if first_ifd == 0 {
        return Err(malformed("first IFD offset is zero"));
    }
    let mut parser = Parser {
        bytes,
        limits,
        order,
        container,
        total_tags: 0,
        total_metadata: 0,
        structural_ranges: Vec::new(),
        chunk_ranges: Vec::new(),
    };
    let mut queue = VecDeque::from([(first_ifd, None)]);
    let mut scheduled = HashSet::from([first_ifd]);
    let mut pages = Vec::new();
    while let Some((offset, parent)) = queue.pop_front() {
        check_limit(
            "page count",
            u64::try_from(pages.len() + 1).map_err(|_| TiffDecodeError::ArithmeticOverflow)?,
            u64::from(limits.max_pages),
        )?;
        let (entries, next) = parser.read_ifd(offset)?;
        let sub_ifds = parser.unsigned_values_optional(&entries, TAG_SUB_IFDS)?;
        let page = parser.page(pages.len(), offset, parent, &entries)?;
        pages.push(page);

        if next != 0 {
            schedule(&mut queue, &mut scheduled, next, parent)?;
        }
        for sub_ifd in sub_ifds.unwrap_or_default() {
            if sub_ifd == 0 {
                return Err(malformed("SubIFD offset is zero"));
            }
            schedule(&mut queue, &mut scheduled, sub_ifd, Some(offset))?;
        }
    }
    parser.validate_ranges()?;
    if pages.is_empty() {
        return Err(malformed("TIFF contains no image pages"));
    }
    let default_page = pages
        .iter()
        .enumerate()
        .filter(|(_, page)| !page.reduced_image)
        .max_by(|(_, left), (_, right)| {
            let left_pixels = left.dimensions.pixel_count().unwrap_or(0);
            let right_pixels = right.dimensions.pixel_count().unwrap_or(0);
            left_pixels
                .cmp(&right_pixels)
                .then_with(|| right.ifd_offset.cmp(&left.ifd_offset))
        })
        .map_or_else(
            || {
                pages
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, page)| page.ifd_offset)
                    .map(|(index, _)| index)
                    .expect("pages are non-empty")
            },
            |(index, _)| index,
        );
    Ok(ParsedTiff {
        header: TiffHeader {
            container,
            byte_order: order,
            pages,
            default_page,
        },
    })
}

fn schedule(
    queue: &mut VecDeque<(u64, Option<u64>)>,
    scheduled: &mut HashSet<u64>,
    offset: u64,
    parent: Option<u64>,
) -> Result<(), TiffDecodeError> {
    if !scheduled.insert(offset) {
        return Err(malformed(
            "IFD graph contains a cycle or repeated directory",
        ));
    }
    queue.push_back((offset, parent));
    Ok(())
}

fn parse_header(bytes: &[u8]) -> Result<(TiffByteOrder, TiffContainer, u64), TiffDecodeError> {
    let order = match bytes.get(..2) {
        Some(b"II") => TiffByteOrder::Little,
        Some(b"MM") => TiffByteOrder::Big,
        _ => return Err(malformed("missing TIFF byte-order marker")),
    };
    let magic = read_u16(bytes, 2, order)?;
    match magic {
        42 => Ok((
            order,
            TiffContainer::Classic,
            u64::from(read_u32(bytes, 4, order)?),
        )),
        43 => {
            if read_u16(bytes, 4, order)? != 8 || read_u16(bytes, 6, order)? != 0 {
                return Err(malformed("invalid BigTIFF offset-size header"));
            }
            Ok((order, TiffContainer::BigTiff, read_u64(bytes, 8, order)?))
        }
        _ => Err(malformed("invalid TIFF magic")),
    }
}

impl Parser<'_> {
    fn read_ifd(&mut self, offset: u64) -> Result<(Vec<Entry>, u64), TiffDecodeError> {
        let offset_usize = to_usize(offset, "IFD offset")?;
        let (count, count_bytes, entry_bytes, inline_bytes, next_bytes) = match self.container {
            TiffContainer::Classic => (
                u64::from(read_u16(self.bytes, offset_usize, self.order)?),
                2_usize,
                12_usize,
                4_u64,
                4_usize,
            ),
            TiffContainer::BigTiff => {
                (read_u64(self.bytes, offset_usize, self.order)?, 8, 20, 8, 8)
            }
        };
        self.total_tags = self
            .total_tags
            .checked_add(count)
            .ok_or(TiffDecodeError::ArithmeticOverflow)?;
        check_limit(
            "tag count",
            self.total_tags,
            u64::from(self.limits.max_tags),
        )?;
        let count_usize = to_usize(count, "IFD entry count")?;
        let entries_len = count_usize
            .checked_mul(entry_bytes)
            .ok_or(TiffDecodeError::ArithmeticOverflow)?;
        let entries_start = offset_usize
            .checked_add(count_bytes)
            .ok_or(TiffDecodeError::ArithmeticOverflow)?;
        let entries_end = entries_start
            .checked_add(entries_len)
            .ok_or(TiffDecodeError::ArithmeticOverflow)?;
        let ifd_end = entries_end
            .checked_add(next_bytes)
            .ok_or(TiffDecodeError::ArithmeticOverflow)?;
        self.bytes
            .get(offset_usize..ifd_end)
            .ok_or_else(|| malformed("IFD entries exceed source"))?;
        self.add_structural_range(offset, usize_to_u64(ifd_end - offset_usize)?)?;

        let mut entries = Vec::new();
        entries
            .try_reserve_exact(count_usize)
            .map_err(|_| TiffDecodeError::AllocationFailure)?;
        let mut previous = None;
        for index in 0..count_usize {
            let start = entries_start + index * entry_bytes;
            let tag = read_u16(self.bytes, start, self.order)?;
            if previous.is_some_and(|value| value >= tag) {
                return Err(malformed("IFD tags are duplicated or not strictly ordered"));
            }
            previous = Some(tag);
            let field_type = read_u16(self.bytes, start + 2, self.order)?;
            let width = type_width(field_type)?;
            let (count, inline_start) = match self.container {
                TiffContainer::Classic => (
                    u64::from(read_u32(self.bytes, start + 4, self.order)?),
                    start + 8,
                ),
                TiffContainer::BigTiff => {
                    (read_u64(self.bytes, start + 4, self.order)?, start + 12)
                }
            };
            let length = count
                .checked_mul(width)
                .ok_or(TiffDecodeError::ArithmeticOverflow)?;
            check_limit("IFD value bytes", length, self.limits.max_ifd_value_bytes)?;
            let data_offset = if length <= inline_bytes {
                usize_to_u64(inline_start)?
            } else {
                match self.container {
                    TiffContainer::Classic => {
                        u64::from(read_u32(self.bytes, inline_start, self.order)?)
                    }
                    TiffContainer::BigTiff => read_u64(self.bytes, inline_start, self.order)?,
                }
            };
            self.slice(data_offset, length, "IFD value")?;
            entries.push(Entry {
                tag,
                field_type,
                count,
                data: TiffDataLocation {
                    offset: data_offset,
                    length,
                },
            });
        }
        let next = match self.container {
            TiffContainer::Classic => u64::from(read_u32(self.bytes, entries_end, self.order)?),
            TiffContainer::BigTiff => read_u64(self.bytes, entries_end, self.order)?,
        };
        Ok((entries, next))
    }

    #[allow(clippy::too_many_lines)]
    fn page(
        &mut self,
        index: usize,
        ifd_offset: u64,
        parent_ifd_offset: Option<u64>,
        entries: &[Entry],
    ) -> Result<TiffPage, TiffDecodeError> {
        let width = self.single_u32(entries, TAG_WIDTH, "ImageWidth")?;
        let height = self.single_u32(entries, TAG_HEIGHT, "ImageLength")?;
        let dimensions = ImageDimensions::new(width, height)
            .map_err(|_| malformed("image dimensions must be nonzero"))?;
        check_limit("width", u64::from(width), u64::from(self.limits.max_width))?;
        check_limit(
            "height",
            u64::from(height),
            u64::from(self.limits.max_height),
        )?;
        let pixels = dimensions
            .pixel_count()
            .map_err(|_| TiffDecodeError::ArithmeticOverflow)?;
        check_limit("pixel count", pixels, self.limits.max_pixels)?;

        let samples_per_pixel = self.single_u16_default(entries, TAG_SAMPLES, 1)?;
        if samples_per_pixel == 0 || samples_per_pixel > 32 {
            return Err(unsupported(
                "samples per pixel",
                u64::from(samples_per_pixel),
            ));
        }
        let mut bits = self
            .unsigned_values_optional(entries, TAG_BITS)?
            .unwrap_or_else(|| vec![1]);
        expand_per_sample(&mut bits, samples_per_pixel, "BitsPerSample")?;
        let bits_per_sample: Vec<u8> = bits
            .into_iter()
            .map(|value| u8::try_from(value).map_err(|_| unsupported("bits per sample", value)))
            .collect::<Result<_, _>>()?;
        if bits_per_sample.windows(2).any(|pair| pair[0] != pair[1]) {
            return Err(unsupported("mixed bits per sample", 0));
        }

        let mut formats = self
            .unsigned_values_optional(entries, TAG_SAMPLE_FORMAT)?
            .unwrap_or_else(|| vec![1]);
        expand_per_sample(&mut formats, samples_per_pixel, "SampleFormat")?;
        let sample_formats: Vec<TiffSampleFormat> = formats
            .into_iter()
            .map(sample_format)
            .collect::<Result<_, _>>()?;
        if sample_formats.windows(2).any(|pair| pair[0] != pair[1]) {
            return Err(unsupported("mixed sample formats", 0));
        }
        validate_sample(bits_per_sample[0], sample_formats[0])?;

        let photometric = photometric(self.single_u16(entries, TAG_PHOTOMETRIC, "Photometric")?)?;
        if samples_per_pixel < photometric.color_samples() {
            return Err(malformed(
                "SamplesPerPixel is smaller than photometric channels",
            ));
        }
        let compression = compression(self.single_u16_default(entries, TAG_COMPRESSION, 1)?)?;
        let predictor = predictor(self.single_u16_default(entries, TAG_PREDICTOR, 1)?)?;
        validate_predictor(predictor, sample_formats[0], bits_per_sample[0])?;
        let storage = match self.single_u16_default(entries, TAG_PLANAR, 1)? {
            1 => TiffStorageLayout::Chunky,
            2 => TiffStorageLayout::Planar,
            value => return Err(unsupported("planar configuration", u64::from(value))),
        };
        let orientation = orientation(self.single_u16_default(entries, TAG_ORIENTATION, 1)?)?;
        let alpha = self.alpha(entries, samples_per_pixel - photometric.color_samples())?;
        let chunks = self.chunks(entries, dimensions, samples_per_pixel, storage)?;

        let bytes_per_sample = u64::from(bits_per_sample[0].div_ceil(8).max(1));
        let decoded_bytes = pixels
            .checked_mul(u64::from(samples_per_pixel))
            .and_then(|value| value.checked_mul(bytes_per_sample))
            .ok_or(TiffDecodeError::ArithmeticOverflow)?;
        check_limit(
            "decoded bytes",
            decoded_bytes,
            self.limits.max_decoded_bytes,
        )?;
        check_limit(
            "decompressed bytes",
            decoded_bytes,
            self.limits.max_decompressed_bytes,
        )?;

        let color_map = self
            .unsigned_values_optional(entries, TAG_COLOR_MAP)?
            .map(|values| {
                values
                    .into_iter()
                    .map(|value| {
                        u16::try_from(value).map_err(|_| malformed("ColorMap value exceeds u16"))
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?;
        if photometric == TiffPhotometric::Palette {
            let expected = 3_usize
                .checked_mul(
                    1_usize
                        .checked_shl(u32::from(bits_per_sample[0]))
                        .ok_or(TiffDecodeError::ArithmeticOverflow)?,
                )
                .ok_or(TiffDecodeError::ArithmeticOverflow)?;
            if color_map.as_ref().is_none_or(|map| map.len() != expected) {
                return Err(malformed("palette TIFF has an invalid ColorMap"));
            }
        }
        let ycbcr_subsampling = self
            .unsigned_values_optional(entries, TAG_YCBCR_SUBSAMPLING)?
            .map(|values| {
                if values.len() != 2 {
                    return Err(malformed("YCbCrSubSampling must contain two values"));
                }
                let horizontal = u16::try_from(values[0])
                    .map_err(|_| malformed("YCbCrSubSampling exceeds u16"))?;
                let vertical = u16::try_from(values[1])
                    .map_err(|_| malformed("YCbCrSubSampling exceeds u16"))?;
                if !matches!(horizontal, 1 | 2 | 4) || !matches!(vertical, 1 | 2 | 4) {
                    return Err(unsupported("YCbCr subsampling", values[0]));
                }
                Ok((horizontal, vertical))
            })
            .transpose()?;
        let metadata = self.metadata(entries)?;
        let dng = self.dng_metadata(entries)?;
        let reduced_image = self
            .unsigned_values_optional(entries, TAG_SUBFILE_TYPE)?
            .and_then(|values| values.first().copied())
            .is_some_and(|value| value & 1 != 0);
        Ok(TiffPage {
            index,
            ifd_offset,
            parent_ifd_offset,
            reduced_image,
            dimensions,
            bits_per_sample,
            sample_formats,
            samples_per_pixel,
            photometric,
            compression,
            predictor,
            storage,
            orientation,
            alpha,
            chunks,
            dng,
            color_map,
            ycbcr_subsampling,
            metadata,
        })
    }

    fn alpha(
        &self,
        entries: &[Entry],
        extra_count: u16,
    ) -> Result<Vec<TiffAlphaSample>, TiffDecodeError> {
        let values = self
            .unsigned_values_optional(entries, TAG_EXTRA_SAMPLES)?
            .unwrap_or_else(|| vec![0; usize::from(extra_count)]);
        if values.len() != usize::from(extra_count) {
            return Err(malformed("ExtraSamples count contradicts SamplesPerPixel"));
        }
        values
            .into_iter()
            .map(|value| match value {
                0 => Ok(TiffAlphaSample::Unspecified),
                1 => Ok(TiffAlphaSample::Premultiplied),
                2 => Ok(TiffAlphaSample::Straight),
                _ => Err(unsupported("extra sample type", value)),
            })
            .collect()
    }

    #[allow(clippy::too_many_lines)]
    fn chunks(
        &mut self,
        entries: &[Entry],
        dimensions: ImageDimensions,
        samples: u16,
        storage: TiffStorageLayout,
    ) -> Result<TiffChunkLayout, TiffDecodeError> {
        let strips = (
            self.unsigned_values_optional(entries, TAG_STRIP_OFFSETS)?,
            self.unsigned_values_optional(entries, TAG_STRIP_COUNTS)?,
        );
        let tiles = (
            self.unsigned_values_optional(entries, TAG_TILE_OFFSETS)?,
            self.unsigned_values_optional(entries, TAG_TILE_COUNTS)?,
        );
        let (kind, width, height, offsets, counts, expected) = match (strips, tiles) {
            ((Some(offsets), Some(counts)), (None, None)) => {
                let rows = self.single_u32_default(entries, TAG_ROWS_PER_STRIP, u32::MAX)?;
                if rows == 0 {
                    return Err(malformed("RowsPerStrip is zero"));
                }
                let per_plane = dimensions.height().div_ceil(rows);
                let planes = if storage == TiffStorageLayout::Planar {
                    u32::from(samples)
                } else {
                    1
                };
                (
                    TiffChunkKind::Strips,
                    dimensions.width(),
                    rows.min(dimensions.height()),
                    offsets,
                    counts,
                    per_plane
                        .checked_mul(planes)
                        .ok_or(TiffDecodeError::ArithmeticOverflow)?,
                )
            }
            ((None, None), (Some(offsets), Some(counts))) => {
                let width = self.single_u32(entries, TAG_TILE_WIDTH, "TileWidth")?;
                let height = self.single_u32(entries, TAG_TILE_HEIGHT, "TileLength")?;
                if width == 0 || height == 0 {
                    return Err(malformed("tile dimensions are zero"));
                }
                let per_plane = dimensions
                    .width()
                    .div_ceil(width)
                    .checked_mul(dimensions.height().div_ceil(height))
                    .ok_or(TiffDecodeError::ArithmeticOverflow)?;
                let planes = if storage == TiffStorageLayout::Planar {
                    u32::from(samples)
                } else {
                    1
                };
                (
                    TiffChunkKind::Tiles,
                    width,
                    height,
                    offsets,
                    counts,
                    per_plane
                        .checked_mul(planes)
                        .ok_or(TiffDecodeError::ArithmeticOverflow)?,
                )
            }
            _ => {
                return Err(malformed(
                    "TIFF strip/tile tables are incomplete or conflicting",
                ));
            }
        };
        if offsets.len() != counts.len()
            || offsets.len() != usize::try_from(expected).unwrap_or(usize::MAX)
        {
            return Err(malformed("TIFF chunk table count contradicts geometry"));
        }
        check_limit(
            "chunk count",
            u64::from(expected),
            u64::from(self.limits.max_chunks),
        )?;
        let mut compressed_bytes = 0_u64;
        let locations = offsets
            .iter()
            .zip(&counts)
            .map(|(&offset, &length)| TiffDataLocation { offset, length })
            .collect();
        for (&offset, &count) in offsets.iter().zip(&counts) {
            if count == 0 {
                return Err(malformed("TIFF chunk has zero byte count"));
            }
            self.slice(offset, count, "chunk data")?;
            let end = offset
                .checked_add(count)
                .ok_or(TiffDecodeError::ArithmeticOverflow)?;
            self.chunk_ranges.push((offset, end));
            compressed_bytes = compressed_bytes
                .checked_add(count)
                .ok_or(TiffDecodeError::ArithmeticOverflow)?;
        }
        Ok(TiffChunkLayout {
            kind,
            width,
            height,
            count: expected,
            compressed_bytes,
            locations,
        })
    }

    fn metadata(&mut self, entries: &[Entry]) -> Result<TiffMetadataInventory, TiffDecodeError> {
        let mut inventory = TiffMetadataInventory {
            icc: Self::location(entries, TAG_ICC),
            exif_ifd: self.single_optional(entries, TAG_EXIF_IFD)?,
            gps_ifd: self.single_optional(entries, TAG_GPS_IFD)?,
            xmp: Self::location(entries, TAG_XMP),
            iptc: Self::location(entries, TAG_IPTC),
            photoshop: Self::location(entries, TAG_PHOTOSHOP),
            metadata_bytes: 0,
        };
        inventory.metadata_bytes = [
            inventory.icc.as_ref(),
            inventory.xmp.as_ref(),
            inventory.iptc.as_ref(),
            inventory.photoshop.as_ref(),
        ]
        .into_iter()
        .flatten()
        .try_fold(0_u64, |sum, value| sum.checked_add(value.length))
        .ok_or(TiffDecodeError::ArithmeticOverflow)?;
        self.total_metadata = self
            .total_metadata
            .checked_add(inventory.metadata_bytes)
            .ok_or(TiffDecodeError::ArithmeticOverflow)?;
        check_limit(
            "metadata bytes",
            self.total_metadata,
            self.limits.max_metadata_bytes,
        )?;
        Ok(inventory)
    }

    fn location(entries: &[Entry], tag: u16) -> Option<TiffDataLocation> {
        entries
            .iter()
            .find(|entry| entry.tag == tag)
            .map(|entry| entry.data.clone())
    }

    fn unsigned_values_optional(
        &self,
        entries: &[Entry],
        tag: u16,
    ) -> Result<Option<Vec<u64>>, TiffDecodeError> {
        entries
            .iter()
            .find(|entry| entry.tag == tag)
            .map(|entry| self.unsigned_values(entry))
            .transpose()
    }

    fn unsigned_values(&self, entry: &Entry) -> Result<Vec<u64>, TiffDecodeError> {
        let width = type_width(entry.field_type)?;
        if !matches!(entry.field_type, 1 | 3 | 4 | 13 | 16 | 18) {
            return Err(malformed("tag uses a non-unsigned field type"));
        }
        let bytes = self.slice(entry.data.offset, entry.data.length, "tag data")?;
        let count = to_usize(entry.count, "tag count")?;
        let width_usize = to_usize(width, "tag type width")?;
        (0..count)
            .map(|index| {
                let offset = index
                    .checked_mul(width_usize)
                    .ok_or(TiffDecodeError::ArithmeticOverflow)?;
                match width {
                    1 => Ok(u64::from(bytes[offset])),
                    2 => Ok(u64::from(read_u16(bytes, offset, self.order)?)),
                    4 => Ok(u64::from(read_u32(bytes, offset, self.order)?)),
                    8 => read_u64(bytes, offset, self.order),
                    _ => Err(malformed("invalid unsigned field width")),
                }
            })
            .collect()
    }

    fn single_optional(&self, entries: &[Entry], tag: u16) -> Result<Option<u64>, TiffDecodeError> {
        self.unsigned_values_optional(entries, tag)?
            .map(|values| single(&values, "single-value tag"))
            .transpose()
    }

    fn single_u16(&self, entries: &[Entry], tag: u16, name: &str) -> Result<u16, TiffDecodeError> {
        let value = single(
            &self
                .unsigned_values_optional(entries, tag)?
                .ok_or_else(|| malformed(&format!("missing {name}")))?,
            name,
        )?;
        u16::try_from(value).map_err(|_| malformed(&format!("{name} exceeds u16")))
    }

    fn single_u16_default(
        &self,
        entries: &[Entry],
        tag: u16,
        default: u16,
    ) -> Result<u16, TiffDecodeError> {
        self.unsigned_values_optional(entries, tag)?
            .map_or(Ok(default), |values| {
                u16::try_from(single(&values, "single-value tag")?)
                    .map_err(|_| malformed("tag value exceeds u16"))
            })
    }

    fn single_u32(&self, entries: &[Entry], tag: u16, name: &str) -> Result<u32, TiffDecodeError> {
        let value = single(
            &self
                .unsigned_values_optional(entries, tag)?
                .ok_or_else(|| malformed(&format!("missing {name}")))?,
            name,
        )?;
        u32::try_from(value).map_err(|_| malformed(&format!("{name} exceeds u32")))
    }

    fn single_u32_default(
        &self,
        entries: &[Entry],
        tag: u16,
        default: u32,
    ) -> Result<u32, TiffDecodeError> {
        self.unsigned_values_optional(entries, tag)?
            .map_or(Ok(default), |values| {
                u32::try_from(single(&values, "single-value tag")?)
                    .map_err(|_| malformed("tag value exceeds u32"))
            })
    }

    fn slice(&self, offset: u64, length: u64, name: &str) -> Result<&[u8], TiffDecodeError> {
        let start = to_usize(offset, name)?;
        let length = to_usize(length, name)?;
        let end = start
            .checked_add(length)
            .ok_or(TiffDecodeError::ArithmeticOverflow)?;
        self.bytes
            .get(start..end)
            .ok_or_else(|| malformed(&format!("{name} exceeds source")))
    }

    fn add_structural_range(&mut self, offset: u64, length: u64) -> Result<(), TiffDecodeError> {
        let end = offset
            .checked_add(length)
            .ok_or(TiffDecodeError::ArithmeticOverflow)?;
        if self
            .structural_ranges
            .iter()
            .any(|&(start, stop)| offset < stop && start < end)
        {
            return Err(malformed("IFD structures overlap"));
        }
        self.structural_ranges.push((offset, end));
        Ok(())
    }

    fn validate_ranges(&mut self) -> Result<(), TiffDecodeError> {
        self.chunk_ranges.sort_unstable();
        if self
            .chunk_ranges
            .windows(2)
            .any(|pair| pair[1].0 < pair[0].1)
        {
            return Err(malformed("TIFF chunk ranges overlap"));
        }
        if self.chunk_ranges.iter().any(|&(start, end)| {
            self.structural_ranges
                .iter()
                .any(|&(ifd_start, ifd_end)| start < ifd_end && ifd_start < end)
        }) {
            return Err(malformed("TIFF chunk data overlaps an IFD"));
        }
        Ok(())
    }
}

fn expand_per_sample(
    values: &mut Vec<u64>,
    samples: u16,
    name: &str,
) -> Result<(), TiffDecodeError> {
    if values.len() == 1 {
        values.resize(usize::from(samples), values[0]);
    }
    if values.len() != usize::from(samples) {
        return Err(malformed(&format!(
            "{name} count contradicts SamplesPerPixel"
        )));
    }
    Ok(())
}

fn validate_sample(bits: u8, format: TiffSampleFormat) -> Result<(), TiffDecodeError> {
    let valid = match format {
        TiffSampleFormat::Unsigned => matches!(bits, 1 | 2 | 4 | 8 | 16 | 32),
        TiffSampleFormat::Signed => matches!(bits, 8 | 16 | 32),
        TiffSampleFormat::Float => matches!(bits, 16 | 32),
    };
    if valid {
        Ok(())
    } else {
        Err(unsupported("sample depth", u64::from(bits)))
    }
}

fn validate_predictor(
    predictor: TiffPredictor,
    format: TiffSampleFormat,
    bits: u8,
) -> Result<(), TiffDecodeError> {
    let valid = match predictor {
        TiffPredictor::None => true,
        TiffPredictor::Horizontal => format != TiffSampleFormat::Float && bits >= 8,
        TiffPredictor::FloatingPoint => format == TiffSampleFormat::Float,
    };
    if valid {
        Ok(())
    } else {
        Err(malformed("predictor is incompatible with sample format"))
    }
}

fn sample_format(value: u64) -> Result<TiffSampleFormat, TiffDecodeError> {
    match value {
        1 => Ok(TiffSampleFormat::Unsigned),
        2 => Ok(TiffSampleFormat::Signed),
        3 => Ok(TiffSampleFormat::Float),
        _ => Err(unsupported("sample format", value)),
    }
}

fn photometric(value: u16) -> Result<TiffPhotometric, TiffDecodeError> {
    match value {
        0 => Ok(TiffPhotometric::WhiteIsZero),
        1 => Ok(TiffPhotometric::BlackIsZero),
        2 => Ok(TiffPhotometric::Rgb),
        3 => Ok(TiffPhotometric::Palette),
        5 => Ok(TiffPhotometric::Cmyk),
        6 => Ok(TiffPhotometric::YCbCr),
        8 => Ok(TiffPhotometric::CieLab),
        9 => Ok(TiffPhotometric::IccLab),
        32_803 => Ok(TiffPhotometric::Cfa),
        34_892 => Ok(TiffPhotometric::LinearRaw),
        _ => Err(unsupported("photometric interpretation", u64::from(value))),
    }
}

fn compression(value: u16) -> Result<TiffCompression, TiffDecodeError> {
    match value {
        1 => Ok(TiffCompression::None),
        5 => Ok(TiffCompression::Lzw),
        7 => Ok(TiffCompression::Jpeg),
        8 => Ok(TiffCompression::Deflate),
        32_946 => Ok(TiffCompression::AdobeDeflate),
        32_773 => Ok(TiffCompression::PackBits),
        50_000 => Ok(TiffCompression::Zstd),
        52_546 => Ok(TiffCompression::JpegXl),
        2..=4 => Err(unsupported("CCITT fax compression", u64::from(value))),
        6 => Err(unsupported("old-style JPEG compression", u64::from(value))),
        34_761 => Err(unsupported("JBIG compression", u64::from(value))),
        _ => Err(unsupported("compression", u64::from(value))),
    }
}

fn predictor(value: u16) -> Result<TiffPredictor, TiffDecodeError> {
    match value {
        1 => Ok(TiffPredictor::None),
        2 => Ok(TiffPredictor::Horizontal),
        3 => Ok(TiffPredictor::FloatingPoint),
        _ => Err(unsupported("predictor", u64::from(value))),
    }
}

fn orientation(value: u16) -> Result<Orientation, TiffDecodeError> {
    match value {
        1 => Ok(Orientation::Normal),
        2 => Ok(Orientation::FlipHorizontal),
        3 => Ok(Orientation::Rotate180),
        4 => Ok(Orientation::FlipVertical),
        5 => Ok(Orientation::Transpose),
        6 => Ok(Orientation::Rotate90),
        7 => Ok(Orientation::Transverse),
        8 => Ok(Orientation::Rotate270),
        _ => Err(malformed("Orientation must be in 1..=8")),
    }
}

fn type_width(field_type: u16) -> Result<u64, TiffDecodeError> {
    match field_type {
        1 | 2 | 6 | 7 => Ok(1),
        3 | 8 => Ok(2),
        4 | 9 | 11 | 13 => Ok(4),
        5 | 10 | 12 | 16 | 17 | 18 => Ok(8),
        _ => Err(malformed("unknown TIFF field type")),
    }
}

fn single(values: &[u64], name: &str) -> Result<u64, TiffDecodeError> {
    if let [value] = values {
        Ok(*value)
    } else {
        Err(malformed(&format!("{name} must contain one value")))
    }
}

fn read_u16(bytes: &[u8], offset: usize, order: TiffByteOrder) -> Result<u16, TiffDecodeError> {
    let value: [u8; 2] = bytes
        .get(
            offset
                ..offset
                    .checked_add(2)
                    .ok_or(TiffDecodeError::ArithmeticOverflow)?,
        )
        .ok_or_else(|| malformed("TIFF value is truncated"))?
        .try_into()
        .map_err(|_| malformed("TIFF value is truncated"))?;
    Ok(match order {
        TiffByteOrder::Little => u16::from_le_bytes(value),
        TiffByteOrder::Big => u16::from_be_bytes(value),
    })
}

fn read_u32(bytes: &[u8], offset: usize, order: TiffByteOrder) -> Result<u32, TiffDecodeError> {
    let value: [u8; 4] = bytes
        .get(
            offset
                ..offset
                    .checked_add(4)
                    .ok_or(TiffDecodeError::ArithmeticOverflow)?,
        )
        .ok_or_else(|| malformed("TIFF value is truncated"))?
        .try_into()
        .map_err(|_| malformed("TIFF value is truncated"))?;
    Ok(match order {
        TiffByteOrder::Little => u32::from_le_bytes(value),
        TiffByteOrder::Big => u32::from_be_bytes(value),
    })
}

fn read_i16(bytes: &[u8], offset: usize, order: TiffByteOrder) -> Result<i16, TiffDecodeError> {
    Ok(match order {
        TiffByteOrder::Little => i16::from_le_bytes(read_u16(bytes, offset, order)?.to_le_bytes()),
        TiffByteOrder::Big => i16::from_be_bytes(read_u16(bytes, offset, order)?.to_be_bytes()),
    })
}

fn read_i32(bytes: &[u8], offset: usize, order: TiffByteOrder) -> Result<i32, TiffDecodeError> {
    Ok(match order {
        TiffByteOrder::Little => i32::from_le_bytes(read_u32(bytes, offset, order)?.to_le_bytes()),
        TiffByteOrder::Big => i32::from_be_bytes(read_u32(bytes, offset, order)?.to_be_bytes()),
    })
}

fn read_u64(bytes: &[u8], offset: usize, order: TiffByteOrder) -> Result<u64, TiffDecodeError> {
    let value: [u8; 8] = bytes
        .get(
            offset
                ..offset
                    .checked_add(8)
                    .ok_or(TiffDecodeError::ArithmeticOverflow)?,
        )
        .ok_or_else(|| malformed("TIFF value is truncated"))?
        .try_into()
        .map_err(|_| malformed("TIFF value is truncated"))?;
    Ok(match order {
        TiffByteOrder::Little => u64::from_le_bytes(value),
        TiffByteOrder::Big => u64::from_be_bytes(value),
    })
}

fn to_usize(value: u64, name: &str) -> Result<usize, TiffDecodeError> {
    usize::try_from(value).map_err(|_| malformed(&format!("{name} does not fit host size")))
}

fn groups4(values: &[u32], name: &str) -> Result<Vec<[u32; 4]>, TiffDecodeError> {
    if !values.len().is_multiple_of(4) {
        return Err(malformed(&format!("{name} count is not divisible by four")));
    }
    Ok(values
        .chunks(4)
        .map(|chunk| [chunk[0], chunk[1], chunk[2], chunk[3]])
        .collect())
}

fn usize_to_u64(value: usize) -> Result<u64, TiffDecodeError> {
    u64::try_from(value).map_err(|_| TiffDecodeError::ArithmeticOverflow)
}

fn check_limit(kind: &'static str, actual: u64, limit: u64) -> Result<(), TiffDecodeError> {
    if actual > limit {
        Err(TiffDecodeError::Limit {
            kind,
            actual,
            limit,
        })
    } else {
        Ok(())
    }
}

fn malformed(message: &str) -> TiffDecodeError {
    TiffDecodeError::Malformed(message.to_owned())
}

fn unsupported(feature: &'static str, value: u64) -> TiffDecodeError {
    TiffDecodeError::Unsupported { feature, value }
}
