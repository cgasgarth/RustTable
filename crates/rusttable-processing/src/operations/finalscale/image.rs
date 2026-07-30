use super::{FinalScaleExecutionError, FinalScalePlan};
use rusttable_image::{
    ByteOrder, ImageDescriptor, ImageDimensions, ImageView, ImageViewError, SampleType,
    StorageLayout,
};

impl FinalScalePlan {
    /// Resamples the storage-neutral F32 image contract while preserving its
    /// channel layout, alpha mode, profile, encoding, orientation, and plane model.
    pub fn execute_image(
        &self,
        descriptor: &ImageDescriptor,
        input: &[u8],
    ) -> Result<FinalScaleImageExecution, FinalScaleImageError> {
        let view = ImageView::new(descriptor, input).map_err(FinalScaleImageError::View)?;
        validate_format(descriptor)?;
        if descriptor.dimensions().width() != self.source_dimensions().width()
            || descriptor.dimensions().height() != self.source_dimensions().height()
        {
            return Err(FinalScaleImageError::DimensionsMismatch);
        }
        let channels = descriptor.format().channels().channels();
        let source = read_interleaved(view, channels)?;
        let stride = usize::try_from(descriptor.dimensions().width())
            .ok()
            .and_then(|width| width.checked_mul(channels))
            .ok_or(FinalScaleImageError::ArithmeticOverflow)?;
        let values = self
            .execute_interleaved(&source, channels, stride)
            .map_err(FinalScaleImageError::Execution)?;
        let format = descriptor.format();
        let output_dimensions = ImageDimensions::new(
            self.output_dimensions().width(),
            self.output_dimensions().height(),
        )
        .map_err(|_| FinalScaleImageError::ArithmeticOverflow)?;
        let output_stride = usize::try_from(self.output_dimensions().width())
            .ok()
            .and_then(|width| {
                width.checked_mul(if format.storage() == StorageLayout::Interleaved {
                    format.bytes_per_pixel()
                } else {
                    format.bytes_per_sample()
                })
            })
            .ok_or(FinalScaleImageError::ArithmeticOverflow)?;
        let strides = vec![output_stride; format.plane_count()];
        let output_descriptor = ImageDescriptor::with_strides(
            output_dimensions,
            format,
            descriptor.color_encoding(),
            descriptor.profile().copied(),
            descriptor.orientation(),
            &strides,
        )
        .map_err(|_| FinalScaleImageError::ArithmeticOverflow)?;
        let mut bytes = vec![0; output_descriptor.byte_length()];
        write_interleaved(&output_descriptor, &mut bytes, &values, channels)?;
        Ok(FinalScaleImageExecution {
            descriptor: output_descriptor,
            bytes,
            identity: self.identity(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalScaleImageExecution {
    descriptor: ImageDescriptor,
    bytes: Vec<u8>,
    identity: [u8; 32],
}

impl FinalScaleImageExecution {
    #[must_use]
    pub const fn descriptor(&self) -> &ImageDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalScaleImageError {
    View(ImageViewError),
    Execution(FinalScaleExecutionError),
    UnsupportedLayout,
    UnsupportedSampleType,
    UnsupportedByteOrder,
    DimensionsMismatch,
    NonFiniteInput { byte_offset: usize },
    ArithmeticOverflow,
}

impl std::fmt::Display for FinalScaleImageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::View(_) => "finalscale image view is invalid",
            Self::Execution(_) => "finalscale image execution failed",
            Self::UnsupportedLayout => "finalscale image layout is unsupported",
            Self::UnsupportedSampleType => "finalscale image sample type is unsupported",
            Self::UnsupportedByteOrder => "finalscale image byte order is unsupported",
            Self::DimensionsMismatch => "finalscale image dimensions do not match the plan",
            Self::NonFiniteInput { .. } => "finalscale image contains a non-finite sample",
            Self::ArithmeticOverflow => "finalscale image arithmetic overflowed",
        })
    }
}

impl std::error::Error for FinalScaleImageError {}

fn validate_format(descriptor: &ImageDescriptor) -> Result<(), FinalScaleImageError> {
    let format = descriptor.format();
    if format.sample_type() != SampleType::F32 {
        return Err(FinalScaleImageError::UnsupportedSampleType);
    }
    if format.byte_order() != ByteOrder::Native {
        return Err(FinalScaleImageError::UnsupportedByteOrder);
    }
    if format.channels().is_mosaic() {
        return Err(FinalScaleImageError::UnsupportedLayout);
    }
    Ok(())
}

fn read_interleaved(
    view: ImageView<'_>,
    channels: usize,
) -> Result<Vec<f32>, FinalScaleImageError> {
    let descriptor = view.descriptor();
    let format = descriptor.format();
    let width = usize::try_from(descriptor.dimensions().width())
        .map_err(|_| FinalScaleImageError::ArithmeticOverflow)?;
    let height = usize::try_from(descriptor.dimensions().height())
        .map_err(|_| FinalScaleImageError::ArithmeticOverflow)?;
    let capacity = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(channels))
        .ok_or(FinalScaleImageError::ArithmeticOverflow)?;
    let mut values = Vec::with_capacity(capacity);
    for y in 0..descriptor.dimensions().height() {
        for x in 0..descriptor.dimensions().width() {
            for channel in 0..channels {
                let (plane, channel_offset) = if format.storage() == StorageLayout::Planar {
                    (channel, 0)
                } else {
                    (0, channel)
                };
                let row = view.row(plane, y).map_err(FinalScaleImageError::View)?;
                let offset = usize::try_from(x)
                    .ok()
                    .and_then(|x| x.checked_mul(format.bytes_per_pixel()))
                    .and_then(|offset| offset.checked_add(channel_offset * 4))
                    .ok_or(FinalScaleImageError::ArithmeticOverflow)?;
                let sample = f32::from_ne_bytes(
                    row[offset..offset + 4]
                        .try_into()
                        .expect("validated F32 sample"),
                );
                if !sample.is_finite() {
                    return Err(FinalScaleImageError::NonFiniteInput {
                        byte_offset: offset,
                    });
                }
                values.push(sample);
            }
        }
    }
    Ok(values)
}

fn write_interleaved(
    descriptor: &ImageDescriptor,
    output: &mut [u8],
    values: &[f32],
    channels: usize,
) -> Result<(), FinalScaleImageError> {
    let format = descriptor.format();
    let width = usize::try_from(descriptor.dimensions().width())
        .map_err(|_| FinalScaleImageError::ArithmeticOverflow)?;
    for y in 0..descriptor.dimensions().height() {
        for x in 0..descriptor.dimensions().width() {
            let pixel = usize::try_from(y)
                .ok()
                .and_then(|y| y.checked_mul(width))
                .and_then(|offset| offset.checked_add(x as usize))
                .and_then(|offset| offset.checked_mul(channels))
                .ok_or(FinalScaleImageError::ArithmeticOverflow)?;
            for channel in 0..channels {
                let (plane, channel_offset) = if format.storage() == StorageLayout::Planar {
                    (channel, 0)
                } else {
                    (0, channel)
                };
                let plane_descriptor = descriptor
                    .planes()
                    .get(plane)
                    .ok_or(FinalScaleImageError::ArithmeticOverflow)?;
                let offset = plane_descriptor
                    .byte_offset()
                    .checked_add(y as usize * plane_descriptor.row_stride())
                    .and_then(|offset| offset.checked_add(x as usize * format.bytes_per_pixel()))
                    .and_then(|offset| offset.checked_add(channel_offset * 4))
                    .ok_or(FinalScaleImageError::ArithmeticOverflow)?;
                output[offset..offset + 4].copy_from_slice(&values[pixel + channel].to_ne_bytes());
            }
        }
    }
    Ok(())
}
