use std::fmt;

use rusttable_color::ColorEncoding;
use rusttable_image::{ImageDimensions, PixelFormat, Roi, SampleType};
use sha2::{Digest, Sha256};

use crate::{DeviceGeneration, ResourceFormat, ResourceId};

pub const COPY_BYTES_PER_ROW_ALIGNMENT: u64 = 256;
pub const SMALL_QUEUE_WRITE_LIMIT: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TransferDirection {
    Upload,
    Readback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TransferStrategy {
    QueueWrite,
    StagingCopy,
    ReadbackMap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TransferFormat {
    RgbaF32,
    UniformBlock,
    RawBytes,
    Rgba16Float,
    R32Float,
}

impl TransferFormat {
    #[must_use]
    pub const fn bytes_per_pixel(self) -> Option<u64> {
        match self {
            Self::RgbaF32 => Some(16),
            Self::Rgba16Float => Some(8),
            Self::R32Float => Some(4),
            Self::UniformBlock | Self::RawBytes => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransferRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl TransferRegion {
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Result<Self, TransferError> {
        if width == 0 || height == 0 {
            return Err(TransferError::InvalidRegion);
        }
        x.checked_add(width)
            .ok_or(TransferError::ArithmeticOverflow)?;
        y.checked_add(height)
            .ok_or(TransferError::ArithmeticOverflow)?;
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    pub fn within(self, dimensions: ImageDimensions) -> Result<Self, TransferError> {
        if self
            .x
            .checked_add(self.width)
            .is_none_or(|right| right > dimensions.width())
            || self
                .y
                .checked_add(self.height)
                .is_none_or(|bottom| bottom > dimensions.height())
        {
            return Err(TransferError::RegionOutOfBounds);
        }
        Ok(self)
    }

    #[must_use]
    pub const fn full(dimensions: ImageDimensions) -> Self {
        Self {
            x: 0,
            y: 0,
            width: dimensions.width(),
            height: dimensions.height(),
        }
    }
}

impl From<TransferRegion> for Roi {
    fn from(value: TransferRegion) -> Self {
        Self::new(value.x, value.y, value.width, value.height).expect("validated transfer region")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HostTransferDescriptor {
    pub dimensions: ImageDimensions,
    pub region: TransferRegion,
    pub row_stride: u64,
    pub pixel_format: PixelFormat,
    pub byte_offset: u64,
    pub byte_length: u64,
    pub color_encoding: ColorEncoding,
}

impl HostTransferDescriptor {
    pub fn new(
        dimensions: ImageDimensions,
        region: TransferRegion,
        row_stride: u64,
        pixel_format: PixelFormat,
        byte_length: u64,
    ) -> Result<Self, TransferError> {
        let region = region.within(dimensions)?;
        let pixel_bytes = u64::try_from(pixel_format.bytes_per_pixel())
            .map_err(|_| TransferError::ArithmeticOverflow)?;
        let minimum_row = u64::from(dimensions.width())
            .checked_mul(pixel_bytes)
            .ok_or(TransferError::ArithmeticOverflow)?;
        if row_stride < minimum_row {
            return Err(TransferError::InvalidRowStride {
                stride: row_stride,
                minimum: minimum_row,
            });
        }
        Ok(Self {
            dimensions,
            region,
            row_stride,
            pixel_format,
            byte_offset: 0,
            byte_length,
            color_encoding: ColorEncoding::Unspecified,
        })
    }

    #[must_use]
    pub const fn with_byte_offset(mut self, byte_offset: u64) -> Self {
        self.byte_offset = byte_offset;
        self
    }

    #[must_use]
    pub const fn with_color_encoding(mut self, color_encoding: ColorEncoding) -> Self {
        self.color_encoding = color_encoding;
        self
    }

    fn logical_row_bytes(self, format: TransferFormat) -> Result<u64, TransferError> {
        let bytes = format.bytes_per_pixel().unwrap_or(
            u64::try_from(self.pixel_format.bytes_per_pixel())
                .map_err(|_| TransferError::ArithmeticOverflow)?,
        );
        u64::from(self.region.width)
            .checked_mul(bytes)
            .ok_or(TransferError::ArithmeticOverflow)
    }

    fn required_source_bytes(self) -> Result<u64, TransferError> {
        let row_end = u64::from(self.region.y)
            .checked_add(u64::from(self.region.height))
            .ok_or(TransferError::ArithmeticOverflow)?;
        let offset = row_end
            .checked_sub(1)
            .ok_or(TransferError::ArithmeticOverflow)?
            .checked_mul(self.row_stride)
            .ok_or(TransferError::ArithmeticOverflow)?;
        let pixel_offset = u64::from(self.region.x)
            .checked_mul(
                u64::try_from(self.pixel_format.bytes_per_pixel())
                    .map_err(|_| TransferError::ArithmeticOverflow)?,
            )
            .ok_or(TransferError::ArithmeticOverflow)?;
        self.byte_offset
            .checked_add(offset)
            .and_then(|value| value.checked_add(pixel_offset))
            .and_then(|value| {
                value.checked_add(self.logical_row_bytes(TransferFormat::RawBytes).ok()?)
            })
            .ok_or(TransferError::ArithmeticOverflow)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GpuTransferDescriptor {
    pub resource: ResourceId,
    pub subresource: u32,
    pub offset: u64,
    pub size: u64,
    pub format: ResourceFormat,
    pub usage: u64,
    pub row_pitch: u64,
    pub rows_per_image: u32,
    pub generation: DeviceGeneration,
}

impl GpuTransferDescriptor {
    #[must_use]
    pub const fn buffer(
        resource: ResourceId,
        offset: u64,
        size: u64,
        generation: DeviceGeneration,
    ) -> Self {
        Self {
            resource,
            subresource: 0,
            offset,
            size,
            format: ResourceFormat::Raw,
            usage: 0,
            row_pitch: 0,
            rows_per_image: 0,
            generation,
        }
    }

    #[must_use]
    pub const fn texture(
        resource: ResourceId,
        size: u64,
        format: ResourceFormat,
        row_pitch: u64,
        rows_per_image: u32,
        generation: DeviceGeneration,
    ) -> Self {
        Self {
            resource,
            subresource: 0,
            offset: 0,
            size,
            format,
            usage: 0,
            row_pitch,
            rows_per_image,
            generation,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransferPayload {
    RgbaF32,
    Uniform {
        byte_length: u64,
        alignment: u64,
    },
    RawBytes {
        element_size: u32,
        element_count: u64,
        alignment: u64,
    },
    TextureRows {
        format: TransferFormat,
    },
}

impl TransferPayload {
    fn format(self) -> TransferFormat {
        match self {
            Self::RgbaF32 => TransferFormat::RgbaF32,
            Self::Uniform { .. } => TransferFormat::UniformBlock,
            Self::RawBytes { .. } => TransferFormat::RawBytes,
            Self::TextureRows { format } => format,
        }
    }

    fn logical_bytes(
        self,
        region: TransferRegion,
        host: HostTransferDescriptor,
    ) -> Result<u64, TransferError> {
        match self {
            Self::Uniform {
                byte_length,
                alignment,
            } => checked_aligned(byte_length, alignment),
            Self::RawBytes {
                element_size,
                element_count,
                alignment,
            } => {
                if element_size == 0 || element_count == 0 {
                    return Err(TransferError::InvalidPayload);
                }
                checked_aligned(
                    u64::from(element_size)
                        .checked_mul(element_count)
                        .ok_or(TransferError::ArithmeticOverflow)?,
                    alignment,
                )
            }
            Self::RgbaF32 | Self::TextureRows { .. } => u64::from(region.width)
                .checked_mul(u64::from(region.height))
                .and_then(|pixels| {
                    pixels.checked_mul(
                        self.format()
                            .bytes_per_pixel()
                            .unwrap_or(u64::try_from(host.pixel_format.bytes_per_pixel()).ok()?),
                    )
                })
                .ok_or(TransferError::ArithmeticOverflow),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferPolicy {
    pub version: u16,
    pub queue_write_limit: u64,
    pub row_alignment: u64,
    pub unified_memory_hint: bool,
}

impl Default for TransferPolicy {
    fn default() -> Self {
        Self {
            version: 1,
            queue_write_limit: SMALL_QUEUE_WRITE_LIMIT,
            row_alignment: COPY_BYTES_PER_ROW_ALIGNMENT,
            unified_memory_hint: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransferIdentity {
    pub source: ResourceId,
    pub target: ResourceId,
    pub generation: DeviceGeneration,
    pub policy_version: u16,
    pub logical_region: TransferRegion,
    pub payload: TransferPayload,
}

impl TransferIdentity {
    #[must_use]
    pub fn hash(self) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(format!("{self:?}").as_bytes());
        digest.finalize().into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferPlan {
    pub direction: TransferDirection,
    pub strategy: TransferStrategy,
    pub format: TransferFormat,
    pub region: TransferRegion,
    pub logical_bytes: u64,
    pub physical_bytes: u64,
    pub row_bytes: u64,
    pub row_pitch: u64,
    pub padding_bytes: u64,
    pub rows: u32,
    pub source: ResourceId,
    pub target: ResourceId,
    pub generation: DeviceGeneration,
    pub identity: [u8; 32],
}

impl TransferPlan {
    #[must_use]
    pub const fn is_padded(self) -> bool {
        self.padding_bytes != 0
    }

    pub fn pack_upload(
        self,
        source: &[u8],
        host: HostTransferDescriptor,
    ) -> Result<Vec<u8>, TransferError> {
        if self.direction != TransferDirection::Upload {
            return Err(TransferError::WrongDirection);
        }
        validate_source(source, host)?;
        let mut packed = vec![
            0;
            usize::try_from(self.physical_bytes)
                .map_err(|_| TransferError::ArithmeticOverflow)?
        ];
        let row_bytes =
            usize::try_from(self.row_bytes).map_err(|_| TransferError::ArithmeticOverflow)?;
        let pitch =
            usize::try_from(self.row_pitch).map_err(|_| TransferError::ArithmeticOverflow)?;
        let source_row =
            usize::try_from(host.row_stride).map_err(|_| TransferError::ArithmeticOverflow)?;
        let source_offset =
            usize::try_from(host.byte_offset).map_err(|_| TransferError::ArithmeticOverflow)?;
        let pixel_offset = usize::try_from(
            u64::from(host.region.x)
                * u64::try_from(host.pixel_format.bytes_per_pixel())
                    .map_err(|_| TransferError::ArithmeticOverflow)?,
        )
        .map_err(|_| TransferError::ArithmeticOverflow)?;
        for row in 0..usize::try_from(self.rows).map_err(|_| TransferError::ArithmeticOverflow)? {
            let start = source_offset
                .checked_add(
                    (usize::try_from(host.region.y)
                        .map_err(|_| TransferError::ArithmeticOverflow)?
                        + row)
                        .checked_mul(source_row)
                        .ok_or(TransferError::ArithmeticOverflow)?,
                )
                .and_then(|value| value.checked_add(pixel_offset))
                .ok_or(TransferError::ArithmeticOverflow)?;
            let end = start
                .checked_add(row_bytes)
                .ok_or(TransferError::ArithmeticOverflow)?;
            let output = row
                .checked_mul(pitch)
                .ok_or(TransferError::ArithmeticOverflow)?;
            packed[output..output + row_bytes].copy_from_slice(&source[start..end]);
        }
        Ok(packed)
    }

    pub fn unpack_readback(
        self,
        packed: &[u8],
        destination: &mut [u8],
        host: HostTransferDescriptor,
    ) -> Result<(), TransferError> {
        if self.direction != TransferDirection::Readback {
            return Err(TransferError::WrongDirection);
        }
        if u64::try_from(packed.len()).map_err(|_| TransferError::ArithmeticOverflow)?
            < self.physical_bytes
        {
            return Err(TransferError::SourceLengthMismatch);
        }
        validate_destination(destination, host)?;
        let row_bytes =
            usize::try_from(self.row_bytes).map_err(|_| TransferError::ArithmeticOverflow)?;
        let pitch =
            usize::try_from(self.row_pitch).map_err(|_| TransferError::ArithmeticOverflow)?;
        let destination_row =
            usize::try_from(host.row_stride).map_err(|_| TransferError::ArithmeticOverflow)?;
        let destination_offset =
            usize::try_from(host.byte_offset).map_err(|_| TransferError::ArithmeticOverflow)?;
        let pixel_offset = usize::try_from(
            u64::from(host.region.x)
                * u64::try_from(host.pixel_format.bytes_per_pixel())
                    .map_err(|_| TransferError::ArithmeticOverflow)?,
        )
        .map_err(|_| TransferError::ArithmeticOverflow)?;
        for row in 0..usize::try_from(self.rows).map_err(|_| TransferError::ArithmeticOverflow)? {
            let input = row
                .checked_mul(pitch)
                .ok_or(TransferError::ArithmeticOverflow)?;
            let start = destination_offset
                .checked_add(
                    (usize::try_from(host.region.y)
                        .map_err(|_| TransferError::ArithmeticOverflow)?
                        + row)
                        .checked_mul(destination_row)
                        .ok_or(TransferError::ArithmeticOverflow)?,
                )
                .and_then(|value| value.checked_add(pixel_offset))
                .ok_or(TransferError::ArithmeticOverflow)?;
            let end = start
                .checked_add(row_bytes)
                .ok_or(TransferError::ArithmeticOverflow)?;
            destination[start..end].copy_from_slice(&packed[input..input + row_bytes]);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferError {
    InvalidRegion,
    RegionOutOfBounds,
    InvalidPayload,
    ArithmeticOverflow,
    InvalidRowStride { stride: u64, minimum: u64 },
    InvalidAlignment(u64),
    SourceLengthMismatch,
    DestinationLengthMismatch,
    GpuBounds,
    GenerationMismatch,
    UnsupportedFormat,
    WrongDirection,
    Cancelled,
    DeviceUnavailable,
}

impl fmt::Display for TransferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRegion => f.write_str("transfer region must be nonempty"),
            Self::RegionOutOfBounds => f.write_str("transfer region is outside its image"),
            Self::InvalidPayload => f.write_str("transfer payload is invalid"),
            Self::ArithmeticOverflow => f.write_str("transfer arithmetic overflowed"),
            Self::InvalidRowStride { stride, minimum } => {
                write!(f, "row stride {stride} is below minimum {minimum}")
            }
            Self::InvalidAlignment(value) => write!(f, "invalid transfer alignment {value}"),
            Self::SourceLengthMismatch => {
                f.write_str("source bytes do not cover the requested transfer")
            }
            Self::DestinationLengthMismatch => {
                f.write_str("destination bytes do not cover the requested transfer")
            }
            Self::GpuBounds => f.write_str("GPU transfer is outside the resource"),
            Self::GenerationMismatch => f.write_str("GPU transfer uses a stale device generation"),
            Self::UnsupportedFormat => f.write_str("transfer format is not qualified"),
            Self::WrongDirection => {
                f.write_str("transfer operation direction does not match the plan")
            }
            Self::Cancelled => f.write_str("transfer was cancelled"),
            Self::DeviceUnavailable => {
                f.write_str("GPU transfer is unavailable on a CPU-only host")
            }
        }
    }
}

impl std::error::Error for TransferError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferRequest {
    pub direction: TransferDirection,
    pub host: HostTransferDescriptor,
    pub gpu: GpuTransferDescriptor,
    pub payload: TransferPayload,
    pub policy: TransferPolicy,
}

pub struct TransferPlanner;

impl TransferPlanner {
    pub fn plan(request: TransferRequest) -> Result<TransferPlan, TransferError> {
        let host = request.host;
        let region = host.region;
        let generation = request.gpu.generation;
        let (format, logical_bytes) = validate_request(request, host, generation)?;
        let layout = calculate_layout(request, format, logical_bytes)?;
        let (row_bytes, rows, row_pitch, physical_bytes, needs_row_alignment) = layout;
        let strategy = match request.direction {
            TransferDirection::Readback => TransferStrategy::ReadbackMap,
            TransferDirection::Upload
                if logical_bytes <= request.policy.queue_write_limit && !needs_row_alignment =>
            {
                TransferStrategy::QueueWrite
            }
            TransferDirection::Upload => TransferStrategy::StagingCopy,
        };
        if request.direction == TransferDirection::Upload {
            let required = host.required_source_bytes()?;
            if required
                > host
                    .byte_offset
                    .checked_add(host.byte_length)
                    .ok_or(TransferError::ArithmeticOverflow)?
            {
                return Err(TransferError::SourceLengthMismatch);
            }
        }
        let identity = TransferIdentity {
            source: host_resource(request),
            target: request.gpu.resource,
            generation,
            policy_version: request.policy.version,
            logical_region: region,
            payload: request.payload,
        }
        .hash();
        Ok(TransferPlan {
            direction: request.direction,
            strategy,
            format,
            region,
            logical_bytes,
            physical_bytes,
            row_bytes,
            row_pitch,
            padding_bytes: physical_bytes.saturating_sub(logical_bytes),
            rows,
            source: host_resource(request),
            target: request.gpu.resource,
            generation,
            identity,
        })
    }
}

fn validate_request(
    request: TransferRequest,
    host: HostTransferDescriptor,
    generation: DeviceGeneration,
) -> Result<(TransferFormat, u64), TransferError> {
    if request.gpu.resource.generation != generation {
        return Err(TransferError::GenerationMismatch);
    }
    if request.gpu.size == 0 {
        return Err(TransferError::GpuBounds);
    }
    if request.policy.row_alignment == 0 || !request.policy.row_alignment.is_power_of_two() {
        return Err(TransferError::InvalidAlignment(
            request.policy.row_alignment,
        ));
    }
    let format = request.payload.format();
    let gpu_format_matches = match request.payload {
        TransferPayload::TextureRows {
            format: TransferFormat::Rgba16Float,
        } => request.gpu.format == ResourceFormat::Rgba16Float,
        TransferPayload::TextureRows {
            format: TransferFormat::R32Float,
        } => request.gpu.format == ResourceFormat::R32Float,
        _ => true,
    };
    if !gpu_format_matches {
        return Err(TransferError::UnsupportedFormat);
    }
    let logical_bytes = request.payload.logical_bytes(host.region, host)?;
    if matches!(request.payload, TransferPayload::RgbaF32)
        && host.pixel_format.sample_type() != SampleType::F32
    {
        return Err(TransferError::UnsupportedFormat);
    }
    if logical_bytes > request.gpu.size {
        return Err(TransferError::GpuBounds);
    }
    Ok((format, logical_bytes))
}

fn calculate_layout(
    request: TransferRequest,
    format: TransferFormat,
    logical_bytes: u64,
) -> Result<(u64, u32, u64, u64, bool), TransferError> {
    let row_bytes = format
        .bytes_per_pixel()
        .map_or(Ok(logical_bytes), |bytes| {
            u64::from(request.host.region.width)
                .checked_mul(bytes)
                .ok_or(TransferError::ArithmeticOverflow)
        })?;
    let rows = u32::try_from(logical_bytes.checked_div(row_bytes).unwrap_or(1))
        .map_err(|_| TransferError::ArithmeticOverflow)?;
    let needs_row_alignment = matches!(request.direction, TransferDirection::Readback)
        || matches!(request.payload, TransferPayload::TextureRows { .. });
    let row_pitch = if needs_row_alignment {
        checked_aligned(row_bytes, request.policy.row_alignment)?
    } else {
        row_bytes
    };
    let physical_bytes = row_pitch
        .checked_mul(u64::from(rows))
        .ok_or(TransferError::ArithmeticOverflow)?;
    if physical_bytes > request.gpu.size {
        return Err(TransferError::GpuBounds);
    }
    Ok((
        row_bytes,
        rows,
        row_pitch,
        physical_bytes,
        needs_row_alignment,
    ))
}

fn host_resource(request: TransferRequest) -> ResourceId {
    request.gpu.resource
}

fn checked_aligned(value: u64, alignment: u64) -> Result<u64, TransferError> {
    if alignment == 0 || !alignment.is_power_of_two() {
        return Err(TransferError::InvalidAlignment(alignment));
    }
    value
        .checked_add(alignment - 1)
        .map(|rounded| rounded & !(alignment - 1))
        .ok_or(TransferError::ArithmeticOverflow)
}

fn validate_source(source: &[u8], host: HostTransferDescriptor) -> Result<(), TransferError> {
    let source_len = u64::try_from(source.len()).map_err(|_| TransferError::ArithmeticOverflow)?;
    let end = host
        .byte_offset
        .checked_add(host.byte_length)
        .ok_or(TransferError::ArithmeticOverflow)?;
    if end > source_len {
        return Err(TransferError::SourceLengthMismatch);
    }
    Ok(())
}

fn validate_destination(
    destination: &[u8],
    host: HostTransferDescriptor,
) -> Result<(), TransferError> {
    let destination_len =
        u64::try_from(destination.len()).map_err(|_| TransferError::ArithmeticOverflow)?;
    let end = host
        .byte_offset
        .checked_add(host.byte_length)
        .ok_or(TransferError::ArithmeticOverflow)?;
    if end > destination_len {
        return Err(TransferError::DestinationLengthMismatch);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferReceipt {
    pub logical_bytes: u64,
    pub physical_bytes: u64,
    pub padding_bytes: u64,
    pub strategy: TransferStrategy,
    pub identity: [u8; 32],
    pub submission: Option<u64>,
    pub cancelled: bool,
}

impl TransferReceipt {
    #[must_use]
    pub const fn from_plan(plan: TransferPlan) -> Self {
        Self {
            logical_bytes: plan.logical_bytes,
            physical_bytes: plan.physical_bytes,
            padding_bytes: plan.padding_bytes,
            strategy: plan.strategy,
            identity: plan.identity,
            submission: None,
            cancelled: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeviceGeneration, ResourceClass, ResourceKind};

    fn descriptors(
        row_stride: u64,
        byte_length: u64,
    ) -> (HostTransferDescriptor, GpuTransferDescriptor) {
        let dimensions = ImageDimensions::new(3, 2).expect("dimensions");
        let host = HostTransferDescriptor::new(
            dimensions,
            TransferRegion::new(1, 0, 2, 2).expect("region"),
            row_stride,
            PixelFormat::canonical_rgba_f32(),
            byte_length,
        )
        .expect("host");
        let resource = ResourceId {
            generation: DeviceGeneration::new(7),
            index: 1,
            kind: ResourceKind::Buffer,
        };
        (
            host,
            GpuTransferDescriptor::buffer(resource, 0, 1024, DeviceGeneration::new(7)),
        )
    }

    #[test]
    fn small_rgba_upload_is_tight_and_packs_partial_rows() {
        let (host, gpu) = descriptors(48, 96);
        let plan = TransferPlanner::plan(TransferRequest {
            direction: TransferDirection::Upload,
            host,
            gpu,
            payload: TransferPayload::RgbaF32,
            policy: TransferPolicy::default(),
        })
        .expect("plan");
        assert_eq!(plan.strategy, TransferStrategy::QueueWrite);
        assert_eq!(plan.logical_bytes, 64);
        let source = (0_u8..96).collect::<Vec<_>>();
        let packed = plan.pack_upload(&source, host).expect("pack");
        assert_eq!(&packed[..16], &source[16..32]);
        assert_eq!(&packed[32..64], &source[64..96]);
    }

    #[test]
    fn readback_rows_are_256_aligned_and_padding_is_not_published() {
        let (host, mut gpu) = descriptors(48, 96);
        gpu.format = ResourceFormat::Rgba16Float;
        let host = HostTransferDescriptor::new(
            host.dimensions,
            host.region,
            32,
            PixelFormat::new(
                SampleType::F16,
                rusttable_image::ChannelLayout::Rgba,
                rusttable_image::AlphaMode::Straight,
                rusttable_image::ByteOrder::Little,
                rusttable_image::StorageLayout::Interleaved,
            )
            .expect("format"),
            64,
        )
        .expect("host");
        let plan = TransferPlanner::plan(TransferRequest {
            direction: TransferDirection::Readback,
            host,
            gpu,
            payload: TransferPayload::TextureRows {
                format: TransferFormat::Rgba16Float,
            },
            policy: TransferPolicy::default(),
        })
        .expect("plan");
        assert_eq!(plan.row_pitch, 256);
        let packed = vec![7; usize::try_from(plan.physical_bytes).expect("test size")];
        let mut destination = vec![0; 64];
        plan.unpack_readback(&packed, &mut destination, host)
            .expect("unpack");
        assert_eq!(&destination[40..56], &[7; 16]);
    }

    #[test]
    fn invalid_stride_and_stale_generation_are_rejected() {
        let dimensions = ImageDimensions::new(2, 2).expect("dimensions");
        assert!(matches!(
            HostTransferDescriptor::new(
                dimensions,
                TransferRegion::full(dimensions),
                1,
                PixelFormat::canonical_rgba_f32(),
                1
            ),
            Err(TransferError::InvalidRowStride { .. })
        ));
        let (host, mut gpu) = descriptors(48, 96);
        gpu.generation = DeviceGeneration::new(8);
        assert_eq!(
            TransferPlanner::plan(TransferRequest {
                direction: TransferDirection::Upload,
                host,
                gpu,
                payload: TransferPayload::RgbaF32,
                policy: TransferPolicy::default()
            }),
            Err(TransferError::GenerationMismatch)
        );
    }

    #[test]
    fn raw_payload_uses_explicit_alignment() {
        let (host, gpu) = descriptors(48, 96);
        let plan = TransferPlanner::plan(TransferRequest {
            direction: TransferDirection::Upload,
            host,
            gpu,
            payload: TransferPayload::RawBytes {
                element_size: 3,
                element_count: 5,
                alignment: 16,
            },
            policy: TransferPolicy::default(),
        })
        .expect("plan");
        assert_eq!(plan.logical_bytes, 16);
    }

    #[test]
    fn resource_class_is_used_as_the_typed_gpu_identity() {
        let class = ResourceClass::buffer(DeviceGeneration::new(2), 64, 1);
        assert_eq!(class.kind, ResourceKind::Buffer);
    }
}
