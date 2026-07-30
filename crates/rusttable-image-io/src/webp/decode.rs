use std::io::{BufReader, Cursor};
use std::panic::{AssertUnwindSafe, catch_unwind};

use image_webp::WebPDecoder as BackendDecoder;
use sha2::{Digest, Sha256};

use super::container::parse;
use super::types::{
    WEBP_BACKEND_ID, WebPCodingMode, WebPDecodeError, WebPDecodeLimits, WebPDecodeMode,
    WebPDecodeReceipt, WebPDecodeRequest, WebPDecodeResult, WebPHeader, WebPPixelData,
};
use crate::raw::{RawByteSource, RawSourceError, SliceRawSource};

const COPY_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, Default)]
pub struct WebPDecoder;

impl WebPDecoder {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Probes and validates a complete WebP source without decoding pixels.
    ///
    /// # Errors
    ///
    /// Returns typed malformed, unsupported-animation, or resource-limit failures.
    pub fn probe_bytes(
        &self,
        bytes: &[u8],
        limits: WebPDecodeLimits,
    ) -> Result<WebPHeader, WebPDecodeError> {
        super::container::probe(bytes, limits)
    }

    /// Inventories and validates every WebP chunk without decoding pixels.
    ///
    /// # Errors
    ///
    /// Returns typed malformed, unsupported-animation, or resource-limit failures.
    pub fn inspect_bytes(
        &self,
        bytes: &[u8],
        limits: WebPDecodeLimits,
    ) -> Result<WebPHeader, WebPDecodeError> {
        super::container::inspect(bytes, limits)
    }

    /// Decodes an immutable byte slice without publishing partial output.
    ///
    /// # Errors
    ///
    /// Returns typed source, cancellation, malformed, unsupported, resource, or backend failures.
    pub fn decode_bytes(
        &self,
        bytes: &[u8],
        request: &WebPDecodeRequest,
    ) -> Result<WebPDecodeResult, WebPDecodeError> {
        self.decode_source(&SliceRawSource::new(bytes), request)
    }

    /// Copies a bounded source, verifies its revision, then performs strict still decode.
    ///
    /// # Errors
    ///
    /// Returns typed source, source-change, cancellation, malformed, unsupported, resource, or
    /// backend failures.
    pub fn decode_source<S: RawByteSource + ?Sized>(
        &self,
        source: &S,
        request: &WebPDecodeRequest,
    ) -> Result<WebPDecodeResult, WebPDecodeError> {
        check_cancel(request)?;
        if matches!(request.mode, WebPDecodeMode::Region { .. }) {
            return Err(WebPDecodeError::UnsupportedRegion);
        }
        let snapshot = Snapshot::read(source, request)?;
        check_cancel(request)?;
        let parsed = parse(&snapshot.bytes, request.limits)?;
        let header = parsed.header;
        let pixels = if matches!(request.mode, WebPDecodeMode::Header) {
            None
        } else {
            Some(decode_pixels(&snapshot.bytes, &header, request)?)
        };
        check_cancel(request)?;
        let output_bytes = pixels.as_ref().map_or(0, |pixels| pixels.samples().len());
        let receipt = WebPDecodeReceipt {
            backend: WEBP_BACKEND_ID.to_owned(),
            source_bytes: snapshot.source_bytes,
            source_sha256: snapshot.sha256,
            riff_declared_bytes: header.riff_declared_bytes,
            dimensions: header.dimensions,
            container: header.container,
            coding: header.coding,
            features: header.features,
            metadata: header.metadata.clone(),
            chunk_count: u32::try_from(header.chunks.chunks.len())
                .map_err(|_| WebPDecodeError::Malformed("chunk count overflows".to_owned()))?,
            compressed_bytes: header.chunks.compressed_bytes,
            output_bytes: u64::try_from(output_bytes).map_err(|_| {
                WebPDecodeError::Malformed("output byte count overflows".to_owned())
            })?,
            mode: request.mode,
        };
        Ok(WebPDecodeResult {
            header,
            pixels,
            receipt,
        })
    }
}

fn decode_pixels(
    bytes: &[u8],
    header: &WebPHeader,
    request: &WebPDecodeRequest,
) -> Result<WebPPixelData, WebPDecodeError> {
    let cursor = Cursor::new(bytes);
    let mut decoder = catch_unwind(AssertUnwindSafe(|| {
        BackendDecoder::new(BufReader::new(cursor))
    }))
    .map_err(|_| WebPDecodeError::Backend("backend panicked during header decode".to_owned()))?
    .map_err(|error| map_backend(error, request.limits))?;
    let memory_limit = usize::try_from(request.limits.max_temporary_bytes.min(usize::MAX as u64))
        .map_err(|_| WebPDecodeError::Limit {
        kind: "backend memory bytes",
        actual: request.limits.max_temporary_bytes,
        limit: usize::MAX as u64,
    })?;
    decoder.set_memory_limit(memory_limit);
    if decoder.is_animated() {
        return Err(WebPDecodeError::UnsupportedAnimation);
    }
    if decoder.dimensions() != (header.dimensions.width(), header.dimensions.height()) {
        return Err(WebPDecodeError::Malformed(
            "backend dimensions differ from validated container".to_owned(),
        ));
    }
    if decoder.has_alpha() != header.features.alpha {
        return Err(WebPDecodeError::Malformed(
            "backend alpha layout differs from validated container".to_owned(),
        ));
    }
    let backend_lossy = decoder.is_lossy();
    if backend_lossy != (header.coding == WebPCodingMode::LossyVp8) {
        return Err(WebPDecodeError::Malformed(
            "backend coding mode differs from validated container".to_owned(),
        ));
    }
    let expected = expected_output_bytes(header)?;
    let backend_size = decoder.output_buffer_size().ok_or(WebPDecodeError::Limit {
        kind: "backend output bytes",
        actual: u64::MAX,
        limit: request.limits.max_decoded_bytes,
    })?;
    if backend_size != expected {
        return Err(WebPDecodeError::Malformed(
            "backend channel count differs from validated container".to_owned(),
        ));
    }
    let mut output = Vec::new();
    output
        .try_reserve_exact(expected)
        .map_err(|_| WebPDecodeError::AllocationFailure)?;
    output.resize(expected, 0);
    check_cancel(request)?;
    catch_unwind(AssertUnwindSafe(|| decoder.read_image(&mut output)))
        .map_err(|_| WebPDecodeError::Backend("backend panicked during pixel decode".to_owned()))?
        .map_err(|error| map_backend(error, request.limits))?;
    check_cancel(request)?;
    if header.features.alpha {
        Ok(WebPPixelData::RgbaU8 {
            dimensions: header.dimensions,
            samples: output,
        })
    } else {
        Ok(WebPPixelData::RgbU8 {
            dimensions: header.dimensions,
            samples: output,
        })
    }
}

fn expected_output_bytes(header: &WebPHeader) -> Result<usize, WebPDecodeError> {
    let channels = if header.features.alpha { 4_u64 } else { 3 };
    let bytes = header
        .dimensions
        .pixel_count()
        .map_err(|_| WebPDecodeError::Malformed("pixel count overflows".to_owned()))?
        .checked_mul(channels)
        .ok_or_else(|| WebPDecodeError::Malformed("output byte count overflows".to_owned()))?;
    usize::try_from(bytes).map_err(|_| WebPDecodeError::Limit {
        kind: "output bytes",
        actual: bytes,
        limit: usize::MAX as u64,
    })
}

fn map_backend(error: image_webp::DecodingError, limits: WebPDecodeLimits) -> WebPDecodeError {
    match error {
        image_webp::DecodingError::MemoryLimitExceeded => WebPDecodeError::Limit {
            kind: "backend memory bytes",
            actual: limits.max_temporary_bytes.saturating_add(1),
            limit: limits.max_temporary_bytes,
        },
        image_webp::DecodingError::ImageTooLarge => WebPDecodeError::Limit {
            kind: "backend output bytes",
            actual: limits.max_decoded_bytes.saturating_add(1),
            limit: limits.max_decoded_bytes,
        },
        image_webp::DecodingError::IoError(error)
            if error.kind() == std::io::ErrorKind::OutOfMemory =>
        {
            WebPDecodeError::AllocationFailure
        }
        other => WebPDecodeError::Backend(other.to_string()),
    }
}

fn check_cancel(request: &WebPDecodeRequest) -> Result<(), WebPDecodeError> {
    if request.cancellation.is_cancelled() {
        Err(WebPDecodeError::Cancelled)
    } else {
        Ok(())
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
        request: &WebPDecodeRequest,
    ) -> Result<Self, WebPDecodeError> {
        let length = source.len().map_err(WebPDecodeError::Source)?;
        if length == 0 {
            return Err(WebPDecodeError::Source(RawSourceError::Empty));
        }
        if length > request.limits.max_source_bytes {
            return Err(WebPDecodeError::Source(RawSourceError::TooLarge {
                actual: length,
                limit: request.limits.max_source_bytes,
            }));
        }
        let revision = source.revision().map_err(WebPDecodeError::Source)?;
        let length_usize = usize::try_from(length)
            .map_err(|_| WebPDecodeError::Source(RawSourceError::LengthConversion))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length_usize)
            .map_err(|_| WebPDecodeError::Source(RawSourceError::AllocationFailure))?;
        bytes.resize(length_usize, 0);
        for (index, chunk) in bytes.chunks_mut(COPY_CHUNK_BYTES).enumerate() {
            check_cancel(request)?;
            let offset = index
                .checked_mul(COPY_CHUNK_BYTES)
                .and_then(|value| u64::try_from(value).ok())
                .ok_or(WebPDecodeError::Source(RawSourceError::LengthConversion))?;
            source
                .read_exact_at(offset, chunk)
                .map_err(WebPDecodeError::Source)?;
        }
        if source.revision().map_err(WebPDecodeError::Source)? != revision {
            return Err(WebPDecodeError::Source(RawSourceError::Changed));
        }
        Ok(Self {
            source_bytes: length,
            sha256: Sha256::digest(&bytes).into(),
            bytes,
        })
    }
}
