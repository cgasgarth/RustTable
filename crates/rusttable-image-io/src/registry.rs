use std::io::Cursor;

use image::{ImageFormat, ImageReader};
use rusttable_image::{
    DecodeLimits, DecodedImage, DecoderDescriptor, DecoderIdentity, ImageDimensions,
    ImageInputError, ImageProbe, InputFormat, UnsupportedImageFeature,
};

use crate::input::{enforce_limits, malformed, probe_tiff, validate_tiff};
use crate::raw::{decode_raw, is_raw, probe_raw};

/// Maximum amount of a source that any format probe may inspect.
pub const PROBE_BUDGET_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    NoMatch,
    Match {
        decoder: DecoderDescriptor,
        probe: ImageProbe,
    },
    MalformedRecognized {
        decoder: DecoderDescriptor,
        error: ImageInputError,
    },
}

#[derive(Debug, Clone, Copy)]
struct DecoderEntry {
    descriptor: DecoderDescriptor,
    matches: fn(&[u8]) -> bool,
    full_source_probe: bool,
    probe: fn(&[u8], DecodeLimits) -> Result<ImageProbe, ImageInputError>,
    decode: fn(&[u8], DecodeLimits) -> Result<DecodedImage, ImageInputError>,
}

macro_rules! builtin_decoders {
    (
        $count:expr;
        $(
            $format:ident {
                id: $id:literal,
                implementation: $implementation:literal,
                matches: $matches:path,
                probe: $probe:path,
                decode: $decode:path,
                full_source_probe: $full_source_probe:expr
            }
        ),+ $(,)?
    ) => {
        static STANDARD_DESCRIPTORS: [DecoderDescriptor; $count] = [
            $(DecoderDescriptor::new(
                DecoderIdentity::new($id, 1, $implementation),
                InputFormat::$format,
            )),+
        ];

        static STANDARD_DECODERS: [DecoderEntry; $count] = [
            $(DecoderEntry {
                descriptor: DecoderDescriptor::new(
                    DecoderIdentity::new($id, 1, $implementation),
                    InputFormat::$format,
                ),
                matches: $matches,
                full_source_probe: $full_source_probe,
                probe: $probe,
                decode: $decode,
            }),+
        ];
    };
}

builtin_decoders! {
    4;
    Jpeg {
        id: "rusttable.decoder.jpeg.v1",
        implementation: "image-jpeg-0.25",
        matches: is_jpeg,
        probe: probe_jpeg,
        decode: decode_jpeg,
        full_source_probe: false
    },
    Png {
        id: "rusttable.decoder.png.v1",
        implementation: "image-png-0.25",
        matches: is_png,
        probe: probe_png,
        decode: decode_png,
        full_source_probe: false
    },
    Raw {
        id: "rusttable.decoder.raw.v1",
        implementation: "rawloader-0.37.1+rawler-0.7.2-raf",
        matches: is_raw,
        probe: probe_raw,
        decode: decode_raw,
        full_source_probe: true
    },
    Tiff {
        id: "rusttable.decoder.tiff.v1",
        implementation: "image-tiff-0.25",
        matches: is_tiff,
        probe: probe_classic_tiff,
        decode: decode_classic_tiff,
        full_source_probe: false
    }
}

/// Fixed, built-in image decoders selected solely from container signatures.
///
/// The registry is intentionally closed: `RustTable` does not load format
/// plugins, inspect extensions, or fall back to unrelated decoders.
#[derive(Debug, Clone, Copy)]
pub struct ImageDecoderRegistry {
    entries: &'static [DecoderEntry],
}

impl ImageDecoderRegistry {
    /// Returns the deterministic built-in PNG/JPEG/RAW/classic-TIFF registry.
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            entries: &STANDARD_DECODERS,
        }
    }

    /// Returns the stable identities registered by the standard registry.
    #[must_use]
    pub const fn descriptors(&self) -> &'static [DecoderDescriptor] {
        &STANDARD_DESCRIPTORS
    }

    /// Performs bounded signature/container probing without decoding pixels.
    #[must_use]
    pub fn probe(&self, bytes: &[u8], limits: DecodeLimits) -> ProbeOutcome {
        let window = &bytes[..bytes.len().min(PROBE_BUDGET_BYTES)];
        let Some(decoder) = self
            .entries
            .iter()
            .find(|decoder| (decoder.matches)(window))
        else {
            return ProbeOutcome::NoMatch;
        };
        let source = if decoder.full_source_probe {
            bytes
        } else {
            window
        };
        match (decoder.probe)(source, limits) {
            Ok(probe) => ProbeOutcome::Match {
                decoder: decoder.descriptor,
                probe,
            },
            Err(error) => ProbeOutcome::MalformedRecognized {
                decoder: decoder.descriptor,
                error,
            },
        }
    }

    /// Returns the selected decoder identity after signature selection.
    ///
    /// # Errors
    ///
    /// Returns `UnsupportedSignature` when no built-in signature matches.
    pub fn select(&self, bytes: &[u8]) -> Result<DecoderDescriptor, ImageInputError> {
        match self.probe(
            bytes,
            DecodeLimits::new(u64::MAX, u32::MAX, u32::MAX, u64::MAX / 4, u64::MAX / 4)
                .unwrap_or_else(|_| unreachable!("selection limits are representable")),
        ) {
            ProbeOutcome::NoMatch => Err(unsupported_signature(bytes)),
            ProbeOutcome::Match { decoder, .. }
            | ProbeOutcome::MalformedRecognized { decoder, .. } => Ok(decoder),
        }
    }

    /// Probes one immutable byte source through signature-first dispatch.
    ///
    /// # Errors
    ///
    /// Returns typed unsupported-signature, malformed-container, feature, or
    /// decode-limit failures without trying another decoder after a match.
    pub fn probe_bytes(
        &self,
        bytes: &[u8],
        limits: DecodeLimits,
    ) -> Result<ImageProbe, ImageInputError> {
        match self.probe(bytes, limits) {
            ProbeOutcome::NoMatch => Err(unsupported_signature(bytes)),
            ProbeOutcome::Match { probe, .. } => Ok(probe),
            ProbeOutcome::MalformedRecognized { error, .. } => Err(error),
        }
    }

    /// Decodes one immutable byte source through signature-first dispatch.
    ///
    /// The bounded structural probe runs before the backend and a backend
    /// failure never causes another decoder to be tried.
    ///
    /// # Errors
    ///
    /// Returns typed unsupported-signature, malformed-container, feature, or
    /// decode-limit failures without trying another decoder after a match.
    pub fn decode_bytes(
        &self,
        bytes: &[u8],
        limits: DecodeLimits,
    ) -> Result<DecodedImage, ImageInputError> {
        let decoder = match self.probe(bytes, limits) {
            ProbeOutcome::NoMatch => return Err(unsupported_signature(bytes)),
            ProbeOutcome::MalformedRecognized { error, .. } => return Err(error),
            ProbeOutcome::Match { decoder, .. } => decoder,
        };
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.descriptor == decoder)
            .ok_or_else(|| ImageInputError::MalformedInput {
                format: decoder.format(),
                message: "selected decoder disappeared from the registry".to_owned(),
            })?;
        (entry.decode)(bytes, limits)
    }
}

impl Default for ImageDecoderRegistry {
    fn default() -> Self {
        Self::standard()
    }
}

fn unsupported_signature(bytes: &[u8]) -> ImageInputError {
    ImageInputError::UnsupportedSignature {
        signature: bytes.iter().copied().take(8).collect(),
    }
}

fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xff, 0xd8])
}

fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'])
}

fn is_tiff(bytes: &[u8]) -> bool {
    bytes.starts_with(&[b'I', b'I', 0x2a, 0x00])
        || bytes.starts_with(&[b'M', b'M', 0x00, 0x2a])
        || bytes.starts_with(&[b'I', b'I', 0x2b, 0x00])
        || bytes.starts_with(&[b'M', b'M', 0x00, 0x2b])
}

#[derive(Debug, Clone, Copy)]
struct PngHeader {
    dimensions: ImageDimensions,
    color_encoding: rusttable_image::ColorEncoding,
}

fn probe_png(bytes: &[u8], limits: DecodeLimits) -> Result<ImageProbe, ImageInputError> {
    let header = inspect_png(bytes)?;
    enforce_limits(limits, header.dimensions)?;
    Ok(ImageProbe::new(InputFormat::Png, header.dimensions))
}

fn inspect_png(bytes: &[u8]) -> Result<PngHeader, ImageInputError> {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if !bytes.starts_with(SIGNATURE) {
        return Err(malformed_input(InputFormat::Png, "missing PNG signature"));
    }
    let ihdr = bytes
        .get(8..33)
        .ok_or_else(|| malformed_input(InputFormat::Png, "truncated PNG IHDR"))?;
    if u32::from_be_bytes(ihdr[0..4].try_into().expect("fixed IHDR length")) != 13
        || &ihdr[4..8] != b"IHDR"
    {
        return Err(malformed_input(InputFormat::Png, "invalid PNG IHDR"));
    }
    let width = u32::from_be_bytes(ihdr[8..12].try_into().expect("fixed IHDR length"));
    let height = u32::from_be_bytes(ihdr[12..16].try_into().expect("fixed IHDR length"));
    let dimensions = ImageDimensions::new(width, height)
        .map_err(|_| malformed_input(InputFormat::Png, "PNG dimensions must be nonzero"))?;
    let bit_depth = ihdr[16];
    let color_type = ihdr[17];
    if !valid_png_bit_depth(color_type, bit_depth) {
        return Err(ImageInputError::UnsupportedFeature {
            format: InputFormat::Png,
            reason: UnsupportedImageFeature::BitDepth,
        });
    }
    if bit_depth != 8 {
        return Err(ImageInputError::UnsupportedFeature {
            format: InputFormat::Png,
            reason: UnsupportedImageFeature::BitDepth,
        });
    }
    if ihdr[18] != 0 || ihdr[19] != 0 || ihdr[20] > 1 {
        return Err(malformed_input(
            InputFormat::Png,
            "unsupported PNG compression, filter, or interlace method",
        ));
    }

    let mut offset = 33_usize;
    let mut color_encoding = rusttable_image::ColorEncoding::Unspecified;
    while let Some(chunk_header) = bytes.get(offset..offset.saturating_add(8)) {
        let length = usize::try_from(u32::from_be_bytes(
            chunk_header[0..4]
                .try_into()
                .expect("fixed chunk header length"),
        ))
        .map_err(|_| ImageInputError::ArithmeticOverflow)?;
        let chunk_type = &chunk_header[4..8];
        let end = offset
            .checked_add(12)
            .and_then(|end| end.checked_add(length))
            .ok_or(ImageInputError::ArithmeticOverflow)?;
        if end > bytes.len() {
            break;
        }
        if chunk_type == b"acTL" {
            return Err(ImageInputError::UnsupportedFeature {
                format: InputFormat::Png,
                reason: UnsupportedImageFeature::Animation,
            });
        }
        if chunk_type == b"sRGB" {
            if length != 1 {
                return Err(malformed_input(InputFormat::Png, "invalid PNG sRGB chunk"));
            }
            color_encoding = rusttable_image::ColorEncoding::Srgb;
        }
        if chunk_type == b"IDAT" {
            break;
        }
        if chunk_type == b"IEND" {
            return Err(malformed_input(InputFormat::Png, "PNG ended before IDAT"));
        }
        offset = end;
    }
    Ok(PngHeader {
        dimensions,
        color_encoding,
    })
}

fn valid_png_bit_depth(color_type: u8, bit_depth: u8) -> bool {
    match color_type {
        0 => matches!(bit_depth, 1 | 2 | 4 | 8 | 16),
        2 | 4 | 6 => matches!(bit_depth, 8 | 16),
        3 => matches!(bit_depth, 1 | 2 | 4 | 8),
        _ => false,
    }
}

#[derive(Debug, Clone, Copy)]
struct JpegHeader {
    dimensions: ImageDimensions,
}

fn probe_jpeg(bytes: &[u8], limits: DecodeLimits) -> Result<ImageProbe, ImageInputError> {
    let header = inspect_jpeg(bytes)?;
    enforce_limits(limits, header.dimensions)?;
    Ok(ImageProbe::new(InputFormat::Jpeg, header.dimensions))
}

fn inspect_jpeg(bytes: &[u8]) -> Result<JpegHeader, ImageInputError> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return Err(malformed_input(InputFormat::Jpeg, "missing JPEG SOI"));
    }
    let mut offset = 2_usize;
    loop {
        if bytes.get(offset) != Some(&0xff) {
            return Err(malformed_input(
                InputFormat::Jpeg,
                &format!("invalid JPEG marker prefix at offset {offset}"),
            ));
        }
        while bytes.get(offset) == Some(&0xff) {
            offset += 1;
        }
        let marker = *bytes
            .get(offset)
            .ok_or_else(|| malformed_input(InputFormat::Jpeg, "truncated JPEG marker"))?;
        offset += 1;
        if marker == 0 || marker == 0xd8 || marker == 0xd9 || marker == 0xda {
            return Err(malformed_input(
                InputFormat::Jpeg,
                "JPEG frame header is missing",
            ));
        }
        if matches!(marker, 0xd0..=0xd7 | 0x01) {
            continue;
        }
        let length = usize::from(read_be_u16(
            bytes,
            offset,
            InputFormat::Jpeg,
            "truncated JPEG segment length",
        )?);
        if length < 2 {
            return Err(malformed_input(
                InputFormat::Jpeg,
                "invalid JPEG segment length",
            ));
        }
        let segment_start = offset + 2;
        let segment_end = segment_start
            .checked_add(length - 2)
            .ok_or(ImageInputError::ArithmeticOverflow)?;
        let segment = bytes.get(segment_start..segment_end).ok_or_else(|| {
            malformed_input(InputFormat::Jpeg, "JPEG segment exceeds probe budget")
        })?;
        if is_jpeg_sof(marker) {
            if segment.len() < 6 {
                return Err(malformed_input(
                    InputFormat::Jpeg,
                    "truncated JPEG frame header",
                ));
            }
            if segment[0] != 8 {
                return Err(ImageInputError::UnsupportedFeature {
                    format: InputFormat::Jpeg,
                    reason: UnsupportedImageFeature::BitDepth,
                });
            }
            let height = u32::from(read_be_u16_from(segment, 1));
            let width = u32::from(read_be_u16_from(segment, 3));
            let components = segment[5];
            if components != 1 && components != 3 {
                return Err(ImageInputError::UnsupportedFeature {
                    format: InputFormat::Jpeg,
                    reason: UnsupportedImageFeature::ColorModel,
                });
            }
            let dimensions = ImageDimensions::new(width, height).map_err(|_| {
                malformed_input(InputFormat::Jpeg, "JPEG dimensions must be nonzero")
            })?;
            return Ok(JpegHeader { dimensions });
        }
        offset = segment_end;
        while bytes.get(offset) != Some(&0xff) {
            if offset >= segment_end.saturating_add(8) {
                return Err(malformed_input(
                    InputFormat::Jpeg,
                    "JPEG marker is not within the bounded segment padding",
                ));
            }
            offset += 1;
        }
    }
}

fn is_jpeg_sof(marker: u8) -> bool {
    matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf)
}

fn read_be_u16(
    bytes: &[u8],
    offset: usize,
    format: InputFormat,
    message: &str,
) -> Result<u16, ImageInputError> {
    let bytes = bytes
        .get(offset..offset.saturating_add(2))
        .ok_or_else(|| malformed_input(format, message))?;
    Ok(read_be_u16_from(bytes, 0))
}

fn read_be_u16_from(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([bytes[offset], bytes[offset + 1]])
}

fn probe_classic_tiff(bytes: &[u8], limits: DecodeLimits) -> Result<ImageProbe, ImageInputError> {
    probe_tiff(bytes, limits)
}

fn decode_jpeg(bytes: &[u8], limits: DecodeLimits) -> Result<DecodedImage, ImageInputError> {
    decode_standard(bytes, limits, InputFormat::Jpeg)
}

fn decode_png(bytes: &[u8], limits: DecodeLimits) -> Result<DecodedImage, ImageInputError> {
    let header = inspect_png(bytes)?;
    enforce_limits(limits, header.dimensions)?;
    let probe = ImageProbe::new(InputFormat::Png, header.dimensions);
    decode_image_with_encoding(
        bytes,
        probe,
        limits,
        ImageFormat::Png,
        header.color_encoding,
    )
}

fn decode_classic_tiff(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<DecodedImage, ImageInputError> {
    validate_tiff(bytes)?;
    let probe = probe_tiff(bytes, limits)?;
    decode_image(bytes, probe, limits, ImageFormat::Tiff)
}

fn decode_standard(
    bytes: &[u8],
    limits: DecodeLimits,
    format: InputFormat,
) -> Result<DecodedImage, ImageInputError> {
    let probe = match format {
        InputFormat::Jpeg => {
            let header = inspect_jpeg(bytes)?;
            enforce_limits(limits, header.dimensions)?;
            ImageProbe::new(format, header.dimensions)
        }
        InputFormat::Png => {
            let header = inspect_png(bytes)?;
            enforce_limits(limits, header.dimensions)?;
            ImageProbe::new(format, header.dimensions)
        }
        InputFormat::Tiff => probe_tiff(bytes, limits)?,
        InputFormat::Raw => return decode_raw(bytes, limits),
    };
    decode_image(bytes, probe, limits, image_format(format))
}

fn decode_image(
    bytes: &[u8],
    probe: ImageProbe,
    limits: DecodeLimits,
    format: ImageFormat,
) -> Result<DecodedImage, ImageInputError> {
    decode_image_with_encoding(
        bytes,
        probe,
        limits,
        format,
        rusttable_image::ColorEncoding::Unspecified,
    )
}

fn decode_image_with_encoding(
    bytes: &[u8],
    probe: ImageProbe,
    limits: DecodeLimits,
    format: ImageFormat,
    color_encoding: rusttable_image::ColorEncoding,
) -> Result<DecodedImage, ImageInputError> {
    let decoded = ImageReader::with_format(Cursor::new(bytes), format)
        .decode()
        .map_err(|error| malformed(probe.format(), &error))?
        .to_rgba8();
    let dimensions = ImageDimensions::new(decoded.width(), decoded.height())
        .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    enforce_limits(limits, dimensions)?;
    if dimensions != probe.dimensions() {
        return Err(ImageInputError::MalformedInput {
            format: probe.format(),
            message: "decoded dimensions differ from the container probe".to_owned(),
        });
    }
    DecodedImage::new_with_color_encoding(dimensions, decoded.into_raw(), color_encoding).map_err(
        |error| match error {
            rusttable_image::DecodedImageError::ArithmeticOverflow => {
                ImageInputError::ArithmeticOverflow
            }
            rusttable_image::DecodedImageError::ByteLengthMismatch { expected, actual } => {
                ImageInputError::DecodedBufferInvariant { expected, actual }
            }
        },
    )
}

fn image_format(format: InputFormat) -> ImageFormat {
    match format {
        InputFormat::Jpeg => ImageFormat::Jpeg,
        InputFormat::Png => ImageFormat::Png,
        InputFormat::Tiff => ImageFormat::Tiff,
        InputFormat::Raw => unreachable!("RAW decoding uses the camera-RAW backend"),
    }
}

fn malformed_input(format: InputFormat, message: &str) -> ImageInputError {
    ImageInputError::MalformedInput {
        format,
        message: message.to_owned(),
    }
}
