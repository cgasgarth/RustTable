#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::chunks_exact_to_as_chunks,
    clippy::manual_is_multiple_of,
    clippy::missing_errors_doc,
    clippy::too_many_arguments,
    reason = "validated classic-TIFF fields are intentionally narrowed"
)]

use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use flate2::read::ZlibDecoder;
use rusttable_image::ImageDimensions;
use sha2::{Digest, Sha256};
use tiff::encoder::colortype::{Gray16, RGB16};
use tiff::encoder::{Compression, DeflateLevel, Predictor, TiffEncoder, TiffKind};
use tiff::tags::Tag;

use super::types::{
    DNG_SCHEMA_VERSION, DngCfaDescriptor, DngError, DngLinearDescriptor, DngOutputReceipt,
    DngOutputRequest, DngProbe, DngPublished, DngRawLayout, DngRawLayoutKind,
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const ROW_BYTES_TARGET: usize = 1 << 20;
const SOFTWARE: &str = "RustTable DNG writer 1";

/// Pure-Rust classic little-endian DNG publication and bounded round-trip probe.
#[derive(Debug, Clone, Copy, Default)]
pub struct DngOutput;

impl DngOutput {
    /// Publishes one checked request through a same-directory temporary file.
    pub fn publish<F: Fn() -> bool>(
        request: &DngOutputRequest,
        cancelled: F,
    ) -> Result<DngPublished, DngError> {
        request.validate()?;
        if cancelled() {
            return Err(DngError::Cancelled);
        }
        let identity = artifact_identity(request);
        let destination = resolve_destination(request, identity)?;
        if destination.exists() {
            let probe = Self::probe(&destination, request.limits.max_encoded_bytes)?;
            if probe.artifact_identity == identity {
                return receipt_for(&destination, request, identity);
            }
            return Err(DngError::DestinationExists);
        }
        let parent = destination.parent().unwrap_or_else(|| Path::new("."));
        if !parent.is_dir() {
            return Err(DngError::InvalidDestination);
        }
        let temporary = temporary_path(&destination);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| DngError::Io(error.to_string()))?;
        let mut writer = LimitedWriter {
            file,
            written: 0,
            limit: request.limits.max_encoded_bytes,
        };
        let result = encode(&mut writer, request, identity, &cancelled);
        if let Err(error) = result {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        writer
            .file
            .sync_all()
            .map_err(|error| cleanup(&temporary, DngError::Io(error.to_string())))?;
        let encoded_bytes = writer.written;
        drop(writer);
        let probe = Self::probe(&temporary, request.limits.max_encoded_bytes)
            .map_err(|error| cleanup(&temporary, error))?;
        if cancelled() {
            return Err(cleanup(&temporary, DngError::Cancelled));
        }
        if probe.artifact_identity != identity || probe.samples != compact_samples(request) {
            return Err(cleanup(&temporary, DngError::RoundTripMismatch));
        }
        fs::rename(&temporary, &destination)
            .map_err(|error| cleanup(&temporary, DngError::Io(error.to_string())))?;
        sync_parent(parent).map_err(|error| DngError::Io(error.to_string()))?;
        let rows_per_strip = rows_per_strip(request)?;
        let strip_count = request_dimensions(request)
            .height()
            .div_ceil(rows_per_strip);
        Ok(DngPublished {
            destination,
            receipt: DngOutputReceipt {
                schema_version: DNG_SCHEMA_VERSION,
                artifact_identity: identity,
                pixel_hash: pixel_hash(request),
                encoded_bytes,
                strip_count,
                rows_per_strip,
            },
        })
    }

    /// Reads only the generated classic TIFF/DNG subset and reconstructs exact u16 samples.
    pub fn probe(path: &Path, limit: u64) -> Result<DngProbe, DngError> {
        let length = fs::metadata(path)
            .map_err(|error| DngError::Io(error.to_string()))?
            .len();
        if length == 0 || length > limit {
            return Err(DngError::MemoryLimit);
        }
        let mut file = File::open(path).map_err(|error| DngError::Io(error.to_string()))?;
        let mut bytes =
            Vec::with_capacity(usize::try_from(length).map_err(|_| DngError::MemoryLimit)?);
        file.read_to_end(&mut bytes)
            .map_err(|error| DngError::Io(error.to_string()))?;
        parse(&bytes)
    }

    /// Removes a published artifact during cancellation or catalog rollback.
    pub fn discard(path: &Path) -> Result<(), DngError> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(DngError::Io(error.to_string())),
        }
    }
}

fn encode<F: Fn() -> bool>(
    writer: &mut LimitedWriter,
    request: &DngOutputRequest,
    identity: [u8; 32],
    cancelled: &F,
) -> Result<(), DngError> {
    let rows = rows_per_strip(request)?;
    let mut encoder = TiffEncoder::new(writer)
        .map_err(encode_error)?
        .with_compression(Compression::Deflate(DeflateLevel::Balanced))
        .with_predictor(Predictor::Horizontal);
    match &request.layout {
        DngRawLayout::CfaBayerU16(value) => {
            let mut image = encoder
                .new_image::<Gray16>(value.dimensions().width(), value.dimensions().height())
                .map_err(encode_error)?;
            image.rows_per_strip(rows).map_err(encode_error)?;
            common_tags(
                image.encoder(),
                value.dimensions(),
                32803,
                1,
                value.orientation(),
                identity,
                value.source_identity(),
                value.output_identity(),
                value.active_area(),
                value.default_crop(),
                value.masked_areas(),
            )?;
            cfa_tags(image.encoder(), value)?;
            write_cfa_strips(image, value, rows, cancelled)?;
        }
        DngRawLayout::LinearRawRgbU16(value) => {
            let mut image = encoder
                .new_image::<RGB16>(value.dimensions().width(), value.dimensions().height())
                .map_err(encode_error)?;
            image.rows_per_strip(rows).map_err(encode_error)?;
            common_tags(
                image.encoder(),
                value.dimensions(),
                34892,
                3,
                value.orientation(),
                identity,
                value.source_identity(),
                value.output_identity(),
                value.active_area(),
                value.default_crop(),
                value.masked_areas(),
            )?;
            linear_tags(image.encoder(), value)?;
            write_linear_strips(image, value, rows, cancelled)?;
        }
    }
    Ok(())
}

fn common_tags<W: Write + Seek, K: TiffKind>(
    encoder: &mut tiff::encoder::DirectoryEncoder<'_, W, K>,
    dimensions: ImageDimensions,
    photometric: u16,
    samples: u16,
    orientation: rusttable_image::Orientation,
    artifact: [u8; 32],
    source: [u8; 32],
    output: [u8; 32],
    active: Option<rusttable_image::Roi>,
    crop: Option<rusttable_image::Roi>,
    masked: &[rusttable_image::Roi],
) -> Result<(), DngError> {
    let xmp = format!(
        "RustTable-DNG-v1;source={};output={};artifact={}",
        hex(source),
        hex(output),
        hex(artifact)
    );
    encoder
        .write_tag(Tag::PhotometricInterpretation, photometric)
        .map_err(encode_error)?;
    encoder
        .write_tag(Tag::PlanarConfiguration, 1u16)
        .map_err(encode_error)?;
    encoder
        .write_tag(Tag::Orientation, u16::from(orientation as u8))
        .map_err(encode_error)?;
    encoder
        .write_tag(Tag::Software, SOFTWARE)
        .map_err(encode_error)?;
    let description = format!("RustTable DNG v1 artifact={}", hex(artifact));
    encoder
        .write_tag(Tag::ImageDescription, description.as_str())
        .map_err(encode_error)?;
    encoder
        .write_tag(Tag::Unknown(50706), &[1u8, 4, 0, 0][..])
        .map_err(encode_error)?;
    encoder
        .write_tag(Tag::Unknown(50707), &[1u8, 4, 0, 0][..])
        .map_err(encode_error)?;
    encoder
        .write_tag(Tag::Unknown(50708), "RustTable Camera Linear")
        .map_err(encode_error)?;
    encoder
        .write_tag(Tag::Unknown(700), xmp.as_bytes())
        .map_err(encode_error)?;
    if let Some(area) = active {
        encoder
            .write_tag(
                Tag::Unknown(50829),
                &[area.y(), area.x(), area.bottom(), area.right()][..],
            )
            .map_err(encode_error)?;
    }
    if let Some(crop) = crop {
        encoder
            .write_tag(
                Tag::Unknown(50719),
                &[rational(crop.x()), rational(crop.y())][..],
            )
            .map_err(encode_error)?;
        encoder
            .write_tag(
                Tag::Unknown(50720),
                &[rational(crop.width()), rational(crop.height())][..],
            )
            .map_err(encode_error)?;
    }
    if !masked.is_empty() {
        let values: Vec<u32> = masked
            .iter()
            .flat_map(|roi| [roi.y(), roi.x(), roi.bottom(), roi.right()])
            .collect();
        encoder
            .write_tag(Tag::Unknown(50830), values.as_slice())
            .map_err(encode_error)?;
    }
    let _ = (dimensions, samples);
    Ok(())
}

fn cfa_tags<W: Write + Seek, K: TiffKind>(
    encoder: &mut tiff::encoder::DirectoryEncoder<'_, W, K>,
    value: &DngCfaDescriptor,
) -> Result<(), DngError> {
    let pattern = value.pattern();
    let colors = pattern.colors();
    let phase = pattern.phase();
    let bytes: Vec<u8> = (0..2)
        .flat_map(|y| {
            (0..2).map(move |x| {
                colors[(y + usize::from(phase.1)) % 2][(x + usize::from(phase.0)) % 2].plane()
            })
        })
        .collect();
    encoder
        .write_tag(Tag::Unknown(33421), &[2u16, 2][..])
        .map_err(encode_error)?;
    encoder
        .write_tag(Tag::Unknown(33422), bytes.as_slice())
        .map_err(encode_error)?;
    encoder
        .write_tag(Tag::Unknown(50710), &[0u8, 1, 2][..])
        .map_err(encode_error)?;
    encoder
        .write_tag(Tag::Unknown(50713), &[2u16, 2][..])
        .map_err(encode_error)?;
    encoder
        .write_tag(
            Tag::Unknown(50714),
            &value.black().map(|value| rational(u32::from(value)))[..],
        )
        .map_err(encode_error)?;
    encoder
        .write_tag(Tag::Unknown(50717), &value.white().map(u32::from)[..])
        .map_err(encode_error)?;
    encoder
        .write_tag(
            Tag::Unknown(50721),
            &value.camera_to_xyz().rows().map(srational)[..],
        )
        .map_err(encode_error)?;
    encoder
        .write_tag(
            Tag::Unknown(50728),
            &value.white_balance().map(|gain| rational_f32(1.0 / gain))[..],
        )
        .map_err(encode_error)?;
    encoder
        .write_tag(Tag::Unknown(50778), 21u16)
        .map_err(encode_error)?;
    encoder
        .write_tag(Tag::Unknown(50730), srational(0.0))
        .map_err(encode_error)?;
    Ok(())
}

fn linear_tags<W: Write + Seek, K: TiffKind>(
    encoder: &mut tiff::encoder::DirectoryEncoder<'_, W, K>,
    value: &DngLinearDescriptor,
) -> Result<(), DngError> {
    encoder
        .write_tag(Tag::Unknown(50713), &[1u16, 3][..])
        .map_err(encode_error)?;
    encoder
        .write_tag(
            Tag::Unknown(50714),
            &value.black().map(|value| rational(u32::from(value)))[..],
        )
        .map_err(encode_error)?;
    encoder
        .write_tag(Tag::Unknown(50717), &value.white().map(u32::from)[..])
        .map_err(encode_error)?;
    if let Some(matrix) = value.camera_to_xyz() {
        encoder
            .write_tag(Tag::Unknown(50721), &matrix.rows().map(srational)[..])
            .map_err(encode_error)?;
    }
    encoder
        .write_tag(Tag::Unknown(50778), 21u16)
        .map_err(encode_error)?;
    Ok(())
}

fn write_cfa_strips<C: Fn() -> bool, W: Write + Seek, K: TiffKind>(
    image: tiff::encoder::ImageEncoder<'_, W, Gray16, K>,
    value: &DngCfaDescriptor,
    rows: u32,
    cancelled: &C,
) -> Result<(), DngError> {
    if cancelled() {
        return Err(DngError::Cancelled);
    }
    let width =
        usize::try_from(value.dimensions().width()).map_err(|_| DngError::ArithmeticOverflow)?;
    let stride = value.row_stride_samples();
    let mut compact = Vec::with_capacity(
        width
            * usize::try_from(value.dimensions().height())
                .map_err(|_| DngError::ArithmeticOverflow)?,
    );
    for row in value.samples().chunks(stride) {
        compact.extend_from_slice(&row[..width]);
    }
    let _ = rows;
    image.write_data(&compact).map_err(encode_error)?;
    Ok(())
}

fn write_linear_strips<C: Fn() -> bool, W: Write + Seek, K: TiffKind>(
    image: tiff::encoder::ImageEncoder<'_, W, RGB16, K>,
    value: &DngLinearDescriptor,
    rows: u32,
    cancelled: &C,
) -> Result<(), DngError> {
    if cancelled() {
        return Err(DngError::Cancelled);
    }
    let width =
        usize::try_from(value.dimensions().width()).map_err(|_| DngError::ArithmeticOverflow)?;
    let _ = (width, rows);
    image.write_data(value.samples()).map_err(encode_error)?;
    Ok(())
}

fn parse(bytes: &[u8]) -> Result<DngProbe, DngError> {
    if bytes.len() < 8 || &bytes[..2] != b"II" || u16::from_le_bytes([bytes[2], bytes[3]]) != 42 {
        return Err(DngError::Probe(
            "not a classic little-endian TIFF".to_owned(),
        ));
    }
    let ifd = usize::try_from(u32::from_le_bytes(
        bytes[4..8]
            .try_into()
            .map_err(|_| DngError::Probe("short header".to_owned()))?,
    ))
    .map_err(|_| DngError::Probe("IFD offset overflow".to_owned()))?;
    let entries = read_ifd(bytes, ifd)?;
    let width = u32_tag(bytes, &entries, 256)?;
    let height = u32_tag(bytes, &entries, 257)?;
    let bits = u16_vec(bytes, &entries, 258)?;
    if bits.iter().any(|bit| *bit != 16) {
        return Err(DngError::Probe("DNG is not u16".to_owned()));
    }
    let compression = u16_tag(bytes, &entries, 259)?;
    let predictor = u16_tag(bytes, &entries, 317)?;
    if compression != 8 || predictor != 2 {
        return Err(DngError::Probe(
            "DNG compression or predictor policy differs".to_owned(),
        ));
    }
    let samples_per_pixel = u16_tag(bytes, &entries, 277)?;
    let photometric = u16_tag(bytes, &entries, 262)?;
    let rows = u32_tag(bytes, &entries, 278)?;
    let offsets = u32_vec(bytes, &entries, 273)?;
    let counts = u32_vec(bytes, &entries, 279)?;
    if offsets.len() != counts.len() || rows == 0 {
        return Err(DngError::Probe("invalid strips".to_owned()));
    }
    let mut samples = Vec::new();
    for (offset, count) in offsets.iter().zip(&counts) {
        let start = usize::try_from(*offset)
            .map_err(|_| DngError::Probe("strip offset overflow".to_owned()))?;
        let end = start
            .checked_add(
                usize::try_from(*count)
                    .map_err(|_| DngError::Probe("strip size overflow".to_owned()))?,
            )
            .ok_or_else(|| DngError::Probe("strip end overflow".to_owned()))?;
        if end > bytes.len() {
            return Err(DngError::Probe("strip outside file".to_owned()));
        }
        let mut decoded = ZlibDecoder::new(&bytes[start..end]);
        let mut raw = Vec::new();
        decoded.read_to_end(&mut raw).map_err(|error| {
            DngError::Probe(format!(
                "corrupt deflate stream at {start}+{count}: {error}; bytes={:02x?}",
                &bytes[start..end.min(start + 8)]
            ))
        })?;
        let mut strip = decode_predictor(&raw, usize::from(samples_per_pixel), width)?;
        samples.append(&mut strip);
    }
    let expected = usize::try_from(width)
        .ok()
        .and_then(|w| usize::try_from(height).ok()?.checked_mul(w))
        .and_then(|pixels| pixels.checked_mul(usize::from(samples_per_pixel)))
        .ok_or(DngError::ArithmeticOverflow)?;
    if samples.len() != expected {
        return Err(DngError::Probe("decoded sample count differs".to_owned()));
    }
    let xmp = bytes_tag(bytes, &entries, 700)?;
    let xmp = String::from_utf8(xmp).map_err(|_| DngError::Probe("XMP is not UTF-8".to_owned()))?;
    let artifact_identity = hex_field(&xmp, "artifact=")?;
    let source_identity = hex_field(&xmp, "source=")?;
    let pixel_hash = hash_samples(&samples);
    let layout = match photometric {
        32803 if samples_per_pixel == 1 => DngRawLayoutKind::CfaBayerU16,
        34892 if samples_per_pixel == 3 => DngRawLayoutKind::LinearRawRgbU16,
        _ => return Err(DngError::Probe("unsupported DNG layout".to_owned())),
    };
    let dimensions = ImageDimensions::new(width, height)
        .map_err(|_| DngError::Probe("invalid dimensions".to_owned()))?;
    Ok(DngProbe {
        layout,
        dimensions,
        samples,
        artifact_identity,
        source_identity,
        pixel_hash,
    })
}

#[derive(Clone, Copy)]
struct Entry {
    tag: u16,
    kind: u16,
    count: u32,
    offset: u32,
    inline: [u8; 4],
}
fn read_ifd(bytes: &[u8], at: usize) -> Result<Vec<Entry>, DngError> {
    let count = usize::from(read_u16(bytes, at)?);
    let end = at
        .checked_add(2)
        .and_then(|v| v.checked_add(count.checked_mul(12)?))
        .ok_or_else(|| DngError::Probe("IFD overflow".to_owned()))?;
    if end + 4 > bytes.len() {
        return Err(DngError::Probe("IFD outside file".to_owned()));
    }
    let mut out = Vec::with_capacity(count);
    for index in 0..count {
        let p = at + 2 + index * 12;
        out.push(Entry {
            tag: read_u16(bytes, p)?,
            kind: read_u16(bytes, p + 2)?,
            count: read_u32(bytes, p + 4)?,
            offset: read_u32(bytes, p + 8)?,
            inline: bytes[p + 8..p + 12]
                .try_into()
                .map_err(|_| DngError::Probe("short TIFF value".to_owned()))?,
        });
    }
    Ok(out)
}
fn read_u16(bytes: &[u8], at: usize) -> Result<u16, DngError> {
    bytes
        .get(at..at + 2)
        .and_then(|v| v.try_into().ok())
        .map(u16::from_le_bytes)
        .ok_or_else(|| DngError::Probe("short TIFF value".to_owned()))
}
fn read_u32(bytes: &[u8], at: usize) -> Result<u32, DngError> {
    bytes
        .get(at..at + 4)
        .and_then(|v| v.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| DngError::Probe("short TIFF value".to_owned()))
}
fn find(entries: &[Entry], tag: u16) -> Result<Entry, DngError> {
    entries
        .iter()
        .find(|entry| entry.tag == tag)
        .copied()
        .ok_or_else(|| DngError::Probe(format!("missing TIFF tag {tag}")))
}
fn entry_bytes(bytes: &[u8], entry: &Entry) -> Result<Vec<u8>, DngError> {
    let size = match entry.kind {
        1 | 2 | 7 => 1,
        3 => 2,
        4 | 9 => 4,
        5 | 10 => 8,
        _ => return Err(DngError::Probe("unsupported TIFF field type".to_owned())),
    };
    let total = usize::try_from(entry.count)
        .ok()
        .and_then(|n| n.checked_mul(size))
        .ok_or(DngError::ArithmeticOverflow)?;
    if total <= 4 {
        return Ok(entry.inline[..total].to_vec());
    }
    let start = usize::try_from(entry.offset)
        .map_err(|_| DngError::Probe("value offset overflow".to_owned()))?;
    bytes
        .get(
            start
                ..start
                    .checked_add(total)
                    .ok_or(DngError::ArithmeticOverflow)?,
        )
        .map(ToOwned::to_owned)
        .ok_or_else(|| DngError::Probe("tag value outside TIFF".to_owned()))
}
fn u16_tag(bytes: &[u8], entries: &[Entry], tag: u16) -> Result<u16, DngError> {
    let entry = find(entries, tag)?;
    if entry.kind != 3 || entry.count != 1 {
        return Err(DngError::Probe("unexpected SHORT tag".to_owned()));
    }
    let value = entry_bytes(bytes, &entry)?;
    value
        .try_into()
        .map(u16::from_le_bytes)
        .map_err(|_| DngError::Probe("invalid SHORT tag".to_owned()))
}
fn u32_tag(bytes: &[u8], entries: &[Entry], tag: u16) -> Result<u32, DngError> {
    let entry = find(entries, tag)?;
    if entry.kind != 4 || entry.count != 1 {
        return Err(DngError::Probe("unexpected LONG tag".to_owned()));
    }
    let value = entry_bytes(bytes, &entry)?;
    value
        .try_into()
        .map(u32::from_le_bytes)
        .map_err(|_| DngError::Probe("invalid LONG tag".to_owned()))
}
fn u16_vec(bytes: &[u8], entries: &[Entry], tag: u16) -> Result<Vec<u16>, DngError> {
    let entry = find(entries, tag)?;
    if entry.kind != 3 {
        return Err(DngError::Probe("unexpected SHORT vector".to_owned()));
    }
    let value = entry_bytes(bytes, &entry)?;
    value
        .chunks_exact(2)
        .map(|v| {
            Ok(u16::from_le_bytes(v.try_into().map_err(|_| {
                DngError::Probe("invalid SHORT vector".to_owned())
            })?))
        })
        .collect()
}
fn u32_vec(bytes: &[u8], entries: &[Entry], tag: u16) -> Result<Vec<u32>, DngError> {
    let entry = find(entries, tag)?;
    if entry.kind != 4 {
        return Err(DngError::Probe("unexpected LONG vector".to_owned()));
    }
    let value = entry_bytes(bytes, &entry)?;
    value
        .chunks_exact(4)
        .map(|v| {
            Ok(u32::from_le_bytes(v.try_into().map_err(|_| {
                DngError::Probe("invalid LONG vector".to_owned())
            })?))
        })
        .collect()
}
fn bytes_tag(bytes: &[u8], entries: &[Entry], tag: u16) -> Result<Vec<u8>, DngError> {
    let entry = find(entries, tag)?;
    if entry.kind != 1 && entry.kind != 2 && entry.kind != 7 {
        return Err(DngError::Probe("unexpected byte tag".to_owned()));
    }
    entry_bytes(bytes, &entry)
}
fn decode_predictor(raw: &[u8], channels: usize, width: u32) -> Result<Vec<u16>, DngError> {
    if raw.len() % 2 != 0 {
        return Err(DngError::Probe("odd decoded strip".to_owned()));
    }
    let mut values: Vec<u16> = raw
        .chunks_exact(2)
        .map(|v| u16::from_le_bytes([v[0], v[1]]))
        .collect();
    let width = usize::try_from(width).map_err(|_| DngError::ArithmeticOverflow)?;
    let row = width
        .checked_mul(channels)
        .ok_or(DngError::ArithmeticOverflow)?;
    if row == 0 || values.len() % row != 0 {
        return Err(DngError::Probe("strip row mismatch".to_owned()));
    }
    for line in values.chunks_mut(row) {
        for index in channels..line.len() {
            line[index] = line[index].wrapping_add(line[index - channels]);
        }
    }
    Ok(values)
}
fn hex_field(value: &str, key: &str) -> Result<[u8; 32], DngError> {
    let start = value
        .find(key)
        .ok_or_else(|| DngError::Probe(format!("missing XMP field {key}")))?
        + key.len();
    let text = value[start..]
        .split(';')
        .next()
        .ok_or_else(|| DngError::Probe("malformed XMP field".to_owned()))?;
    if text.len() != 64 {
        return Err(DngError::Probe("invalid identity length".to_owned()));
    }
    let mut out = [0u8; 32];
    for (index, pair) in text.as_bytes().chunks_exact(2).enumerate() {
        out[index] = u8::from_str_radix(
            std::str::from_utf8(pair)
                .map_err(|_| DngError::Probe("invalid identity".to_owned()))?,
            16,
        )
        .map_err(|_| DngError::Probe("invalid identity".to_owned()))?;
    }
    Ok(out)
}
fn artifact_identity(request: &DngOutputRequest) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"rusttable.dng-output-v1");
    match &request.layout {
        DngRawLayout::CfaBayerU16(v) => {
            h.update([0]);
            h.update(v.output_identity());
            h.update(v.source_identity());
            update_samples(&mut h, v.samples());
        }
        DngRawLayout::LinearRawRgbU16(v) => {
            h.update([1]);
            h.update(v.output_identity());
            h.update(v.source_identity());
            update_samples(&mut h, v.samples());
        }
    }
    h.finalize().into()
}
fn compact_samples(request: &DngOutputRequest) -> Vec<u16> {
    match &request.layout {
        DngRawLayout::CfaBayerU16(v) => {
            let width = usize::try_from(v.dimensions().width()).unwrap_or(0);
            v.samples()
                .chunks(v.row_stride_samples())
                .flat_map(|row| row[..width].iter().copied())
                .collect()
        }
        DngRawLayout::LinearRawRgbU16(v) => v.samples().to_vec(),
    }
}
fn pixel_hash(request: &DngOutputRequest) -> [u8; 32] {
    hash_samples(&compact_samples(request))
}
fn hash_samples(samples: &[u16]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"rusttable.dng-pixels-u16-le-v1");
    for value in samples {
        h.update(value.to_le_bytes());
    }
    h.finalize().into()
}
fn update_samples(hasher: &mut Sha256, samples: &[u16]) {
    for value in samples {
        hasher.update(value.to_le_bytes());
    }
}
fn request_dimensions(request: &DngOutputRequest) -> ImageDimensions {
    match &request.layout {
        DngRawLayout::CfaBayerU16(v) => v.dimensions(),
        DngRawLayout::LinearRawRgbU16(v) => v.dimensions(),
    }
}
fn rows_per_strip(request: &DngOutputRequest) -> Result<u32, DngError> {
    let width = usize::try_from(request_dimensions(request).width())
        .map_err(|_| DngError::ArithmeticOverflow)?;
    let channels = match &request.layout {
        DngRawLayout::CfaBayerU16(_) => 1,
        DngRawLayout::LinearRawRgbU16(_) => 3,
    };
    u32::try_from(
        (ROW_BYTES_TARGET
            / width
                .checked_mul(channels)
                .and_then(|v| v.checked_mul(2))
                .ok_or(DngError::ArithmeticOverflow)?)
        .max(1),
    )
    .map_err(|_| DngError::ArithmeticOverflow)
}
fn resolve_destination(
    request: &DngOutputRequest,
    identity: [u8; 32],
) -> Result<PathBuf, DngError> {
    let path = &request.destination;
    if !path.exists() {
        return Ok(path.clone());
    }
    if matches!(
        request.collision,
        super::types::DngCollisionPolicy::ReuseIdentical
    ) && SelfProbe::same(path, identity, request.limits.max_encoded_bytes)
    {
        return Ok(path.clone());
    }
    if matches!(
        request.collision,
        super::types::DngCollisionPolicy::Fail | super::types::DngCollisionPolicy::ReuseIdentical
    ) {
        return Err(DngError::DestinationExists);
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .ok_or(DngError::InvalidDestination)?
        .to_string_lossy();
    let ext = path
        .extension()
        .map(|v| format!(".{}", v.to_string_lossy()))
        .unwrap_or_default();
    for suffix in 1..10_000_u32 {
        let candidate = parent.join(format!("{stem}-{suffix}{ext}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(DngError::DestinationExists)
}
struct SelfProbe;
impl SelfProbe {
    fn same(path: &Path, identity: [u8; 32], limit: u64) -> bool {
        DngOutput::probe(path, limit).is_ok_and(|p| p.artifact_identity == identity)
    }
}
fn receipt_for(
    path: &Path,
    request: &DngOutputRequest,
    identity: [u8; 32],
) -> Result<DngPublished, DngError> {
    let probe = DngOutput::probe(path, request.limits.max_encoded_bytes)?;
    let metadata = fs::metadata(path).map_err(|error| DngError::Io(error.to_string()))?;
    let rows = rows_per_strip(request)?;
    let strip_count = request_dimensions(request).height().div_ceil(rows);
    Ok(DngPublished {
        destination: path.to_owned(),
        receipt: DngOutputReceipt {
            schema_version: DNG_SCHEMA_VERSION,
            artifact_identity: identity,
            pixel_hash: probe.pixel_hash,
            encoded_bytes: metadata.len(),
            strip_count,
            rows_per_strip: rows,
        },
    })
}
fn cleanup(path: &Path, error: DngError) -> DngError {
    let _ = fs::remove_file(path);
    error
}
fn temporary_path(destination: &Path) -> PathBuf {
    let seq = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = destination.file_name().map_or_else(
        || "output.dng".to_owned(),
        |v| v.to_string_lossy().into_owned(),
    );
    destination
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".{name}.rusttable-{seq}.tmp"))
}
fn sync_parent(parent: &Path) -> io::Result<()> {
    File::open(parent)?.sync_all()
}
fn encode_error(error: impl std::fmt::Display) -> DngError {
    DngError::Encode(error.to_string())
}
fn hex(value: [u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in value {
        let _ = write!(out, "{byte:02x}");
    }
    out
}
fn rational(value: u32) -> tiff::encoder::Rational {
    tiff::encoder::Rational { n: value, d: 1 }
}
fn rational_f32(value: f32) -> tiff::encoder::Rational {
    let scaled = (value * 1_000_000.0).round().clamp(1.0, u32::MAX as f32) as u32;
    tiff::encoder::Rational {
        n: scaled,
        d: 1_000_000,
    }
}
fn srational(value: f32) -> tiff::encoder::SRational {
    let scaled = (value * 1_000_000.0)
        .round()
        .clamp(i32::MIN as f32, i32::MAX as f32) as i32;
    tiff::encoder::SRational {
        n: scaled,
        d: 1_000_000,
    }
}

struct LimitedWriter {
    file: File,
    written: u64,
    limit: u64,
}
impl Write for LimitedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let next = self
            .written
            .checked_add(
                u64::try_from(bytes.len()).map_err(|_| io::Error::other("write overflow"))?,
            )
            .ok_or_else(|| io::Error::other("write overflow"))?;
        if next > self.limit {
            return Err(io::Error::other("DNG exceeds byte limit"));
        }
        let count = self.file.write(bytes)?;
        self.written = self
            .written
            .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
impl Seek for LimitedWriter {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}
