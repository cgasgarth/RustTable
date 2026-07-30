use sha2::{Digest, Sha256};

use super::backend;
use super::container::{
    BoxRecord, ContainerError, ContainerInspection, ContainerKind, ContainerLimits, inspect,
};
use super::types::{
    JXL_BACKEND_ID, JxlBoxDescriptor, JxlContainerInventory, JxlContainerKind, JxlDecodeError,
    JxlDecodeLimits, JxlDecodeReceipt, JxlDecodeRequest, JxlDecodeResult, JxlHeader,
};
use crate::raw::{RawByteSource, RawSourceError, SliceRawSource};

const COPY_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, Default)]
pub struct JxlDecoder;

impl JxlDecoder {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Probes a bounded prefix and returns dimensions without rendering pixels.
    ///
    /// # Errors
    ///
    /// Returns typed signature, malformed-header, probe-budget, or resource-limit failures.
    pub fn probe_bytes(
        &self,
        bytes: &[u8],
        limits: JxlDecodeLimits,
    ) -> Result<rusttable_image::ImageDimensions, JxlDecodeError> {
        let source_bytes =
            u64::try_from(bytes.len()).map_err(|_| JxlDecodeError::ArithmeticOverflow)?;
        if source_bytes > limits.max_source_bytes {
            return Err(JxlDecodeError::Limit {
                kind: "source bytes",
                actual: source_bytes,
                limit: limits.max_source_bytes,
            });
        }
        if !super::container::matches_signature(bytes) {
            return Err(JxlDecodeError::NotJpegXl);
        }
        backend::probe(bytes, limits)
    }

    /// Inventories and validates the complete container and JPEG XL header without pixels.
    ///
    /// # Errors
    ///
    /// Returns typed malformed, unsupported-animation, color, profile, or limit failures.
    pub fn inspect_bytes(
        &self,
        bytes: &[u8],
        limits: JxlDecodeLimits,
    ) -> Result<JxlHeader, JxlDecodeError> {
        let request = JxlDecodeRequest::new(limits).header();
        Ok(self.decode_bytes(bytes, &request)?.header)
    }

    /// Decodes an immutable byte slice without publishing partial output.
    ///
    /// # Errors
    ///
    /// Returns typed source, cancellation, malformed, unsupported, resource, profile, or backend
    /// failures.
    pub fn decode_bytes(
        &self,
        bytes: &[u8],
        request: &JxlDecodeRequest,
    ) -> Result<JxlDecodeResult, JxlDecodeError> {
        self.decode_source(&SliceRawSource::new(bytes), request)
    }

    /// Copies a bounded source, verifies its revision, then performs strict still-image decode.
    ///
    /// # Errors
    ///
    /// Returns typed source-mutation in addition to the byte-slice decode failures.
    pub fn decode_source<S: RawByteSource + ?Sized>(
        &self,
        source: &S,
        request: &JxlDecodeRequest,
    ) -> Result<JxlDecodeResult, JxlDecodeError> {
        check_cancel(request)?;
        let snapshot = Snapshot::read(source, request)?;
        let container = inspect(
            &snapshot.bytes,
            ContainerLimits {
                max_boxes: request.limits.max_boxes,
                max_metadata_bytes: request.limits.max_metadata_bytes,
            },
        )
        .map_err(map_container)?;
        check_cancel(request)?;
        let decoded = backend::decode(&snapshot.bytes, request.mode, request.limits)?;
        check_cancel(request)?;
        let output_bytes = decoded.pixels.as_ref().map_or(Ok(0), |pixels| {
            u64::try_from(pixels.byte_len()).map_err(|_| JxlDecodeError::ArithmeticOverflow)
        })?;
        let inventory = inventory(container)?;
        let receipt = JxlDecodeReceipt {
            backend: JXL_BACKEND_ID.to_owned(),
            source_bytes: snapshot.source_bytes,
            source_sha256: snapshot.sha256,
            bytes_read: snapshot.source_bytes,
            output_bytes,
            container: inventory,
            coding: decoded.coding,
            color: decoded.header.color.clone(),
            orientation: decoded.header.orientation,
            orientation_applied: decoded.pixels.is_some(),
            alpha: decoded.header.alpha,
            extra_channels: decoded.header.extra_channels.clone(),
            frame_count: decoded.header.animation.total_frames,
            displayed_frame_count: decoded.header.animation.displayed_frames,
            single_frame_animation: decoded.header.animation.declared
                && decoded.header.animation.displayed_frames == 1,
            roi_behavior: decoded.roi_behavior,
            header_only: decoded.pixels.is_none(),
        };
        Ok(JxlDecodeResult {
            header: decoded.header,
            pixels: decoded.pixels,
            receipt,
        })
    }
}

fn inventory(value: ContainerInspection) -> Result<JxlContainerInventory, JxlDecodeError> {
    let mut boxes = Vec::new();
    boxes
        .try_reserve_exact(value.boxes.len())
        .map_err(|_| JxlDecodeError::AllocationFailure)?;
    boxes.extend(value.boxes.into_iter().map(box_descriptor));
    Ok(JxlContainerInventory {
        kind: match value.kind {
            ContainerKind::Bare => JxlContainerKind::BareCodestream,
            ContainerKind::Isobmff => JxlContainerKind::Isobmff,
        },
        boxes,
        codestream_bytes: value.codestream_bytes,
        codestream_parts: value.codestream_parts,
        jpeg_reconstruction_box: value.jpeg_reconstruction_box,
    })
}

const fn box_descriptor(value: BoxRecord) -> JxlBoxDescriptor {
    JxlBoxDescriptor {
        box_type: value.box_type,
        offset: value.offset,
        total_bytes: value.total_bytes,
        payload_bytes: value.payload_bytes,
    }
}

fn map_container(error: ContainerError) -> JxlDecodeError {
    match error {
        ContainerError::NotJpegXl => JxlDecodeError::NotJpegXl,
        ContainerError::Truncated(message) | ContainerError::Invalid(message) => {
            JxlDecodeError::Malformed(message.to_owned())
        }
        ContainerError::UnsupportedEssential(box_type) => {
            JxlDecodeError::UnsupportedEssentialBox(box_type)
        }
        ContainerError::UnsupportedCompression(box_type) => {
            JxlDecodeError::UnsupportedBoxCompression(box_type)
        }
        ContainerError::Limit {
            kind,
            actual,
            limit,
        } => JxlDecodeError::Limit {
            kind,
            actual,
            limit,
        },
        ContainerError::Overflow => JxlDecodeError::ArithmeticOverflow,
    }
}

fn check_cancel(request: &JxlDecodeRequest) -> Result<(), JxlDecodeError> {
    if request.cancellation.is_cancelled() {
        Err(JxlDecodeError::Cancelled)
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
        request: &JxlDecodeRequest,
    ) -> Result<Self, JxlDecodeError> {
        let length = source.len().map_err(JxlDecodeError::Source)?;
        if length == 0 {
            return Err(JxlDecodeError::Source(RawSourceError::Empty));
        }
        if length > request.limits.max_source_bytes {
            return Err(JxlDecodeError::Source(RawSourceError::TooLarge {
                actual: length,
                limit: request.limits.max_source_bytes,
            }));
        }
        let revision = source.revision().map_err(JxlDecodeError::Source)?;
        let length_usize = usize::try_from(length)
            .map_err(|_| JxlDecodeError::Source(RawSourceError::LengthConversion))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length_usize)
            .map_err(|_| JxlDecodeError::Source(RawSourceError::AllocationFailure))?;
        bytes.resize(length_usize, 0);
        for (index, chunk) in bytes.chunks_mut(COPY_CHUNK_BYTES).enumerate() {
            check_cancel(request)?;
            let offset = index
                .checked_mul(COPY_CHUNK_BYTES)
                .and_then(|value| u64::try_from(value).ok())
                .ok_or(JxlDecodeError::Source(RawSourceError::LengthConversion))?;
            source
                .read_exact_at(offset, chunk)
                .map_err(JxlDecodeError::Source)?;
        }
        if source.revision().map_err(JxlDecodeError::Source)? != revision {
            return Err(JxlDecodeError::Source(RawSourceError::Changed));
        }
        Ok(Self {
            source_bytes: length,
            sha256: Sha256::digest(&bytes).into(),
            bytes,
        })
    }
}
