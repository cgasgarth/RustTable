use super::{ScalePixelsKernel, ScalePixelsPlan};
use rusttable_image::{
    ByteOrder, ImageDescriptor, ImageView, ImageViewError, SampleType, StorageLayout,
};

impl ScalePixelsPlan {
    /// Executes a validated F32 image while preserving channels, alpha, color
    /// metadata, and allowing padded input rows. Output rows are tight.
    pub fn execute_image(
        &self,
        descriptor: &ImageDescriptor,
        input: &[u8],
    ) -> Result<ScalePixelsImageExecution, ScalePixelsImageError> {
        let view = ImageView::new(descriptor, input).map_err(ScalePixelsImageError::View)?;
        validate_format(descriptor)?;
        if descriptor.dimensions().width() != self.source_dimensions.width()
            || descriptor.dimensions().height() != self.source_dimensions.height()
        {
            return Err(ScalePixelsImageError::DimensionsMismatch {
                expected: self.source_dimensions,
                actual: descriptor.dimensions(),
            });
        }
        validate_finite(view)?;
        let format = descriptor.format();
        let stride = usize::try_from(self.output_dimensions.width())
            .ok()
            .and_then(|width| {
                width.checked_mul(if format.storage() == StorageLayout::Interleaved {
                    format.bytes_per_pixel()
                } else {
                    format.bytes_per_sample()
                })
            })
            .ok_or(ScalePixelsImageError::ArithmeticOverflow)?;
        let strides = vec![stride; format.plane_count()];
        let output_descriptor = ImageDescriptor::with_strides(
            rusttable_image::ImageDimensions::new(
                self.output_dimensions.width(),
                self.output_dimensions.height(),
            )
            .map_err(|_| ScalePixelsImageError::ArithmeticOverflow)?,
            format,
            descriptor.color_encoding(),
            descriptor.profile().copied(),
            descriptor.orientation(),
            &strides,
        )
        .map_err(|_| ScalePixelsImageError::ArithmeticOverflow)?;
        let mut output = vec![0_u8; output_descriptor.byte_length()];
        for y in 0..self.output_dimensions.height() {
            for x in 0..self.output_dimensions.width() {
                let source_x = x as f32 * self.x_scale;
                let source_y = y as f32 * self.y_scale;
                for channel in 0..format.channels().channels() {
                    let value =
                        sample_image(view, source_x, source_y, channel, self.preferences.image());
                    write_sample(&output_descriptor, &mut output, x, y, channel, value)?;
                }
            }
        }
        Ok(ScalePixelsImageExecution {
            descriptor: output_descriptor,
            bytes: output,
            identity: self.identity,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalePixelsImageExecution {
    descriptor: ImageDescriptor,
    bytes: Vec<u8>,
    identity: [u8; 32],
}

impl ScalePixelsImageExecution {
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
pub enum ScalePixelsImageError {
    View(ImageViewError),
    UnsupportedLayout,
    UnsupportedSampleType,
    UnsupportedByteOrder,
    DimensionsMismatch {
        expected: crate::RasterDimensions,
        actual: rusttable_image::ImageDimensions,
    },
    NonFiniteInput {
        byte_offset: usize,
    },
    ArithmeticOverflow,
}

impl std::fmt::Display for ScalePixelsImageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::View(_) => "image view is invalid",
            Self::UnsupportedLayout => "image layout is unsupported by scalepixels",
            Self::UnsupportedSampleType => "image sample type is unsupported by scalepixels",
            Self::UnsupportedByteOrder => "image byte order is unsupported by scalepixels",
            Self::DimensionsMismatch { .. } => "image dimensions do not match the plan",
            Self::NonFiniteInput { .. } => "image contains a non-finite sample",
            Self::ArithmeticOverflow => "image arithmetic overflowed",
        })
    }
}

impl std::error::Error for ScalePixelsImageError {}

fn validate_format(descriptor: &ImageDescriptor) -> Result<(), ScalePixelsImageError> {
    let format = descriptor.format();
    if format.sample_type() != SampleType::F32 {
        return Err(ScalePixelsImageError::UnsupportedSampleType);
    }
    if format.byte_order() != ByteOrder::Native {
        return Err(ScalePixelsImageError::UnsupportedByteOrder);
    }
    if format.channels().is_mosaic() {
        return Err(ScalePixelsImageError::UnsupportedLayout);
    }
    Ok(())
}

fn validate_finite(view: ImageView<'_>) -> Result<(), ScalePixelsImageError> {
    let format = view.descriptor().format();
    for (plane_index, plane) in view.descriptor().planes().iter().enumerate() {
        let samples_per_row = usize::try_from(plane.width())
            .ok()
            .and_then(|width| {
                width.checked_mul(if format.storage() == StorageLayout::Interleaved {
                    format.channels().channels()
                } else {
                    1
                })
            })
            .ok_or(ScalePixelsImageError::ArithmeticOverflow)?;
        for row_index in 0..plane.height() {
            let row = view
                .row(plane_index, row_index)
                .map_err(ScalePixelsImageError::View)?;
            for sample_index in 0..samples_per_row {
                let start = sample_index * format.bytes_per_sample();
                let value = f32::from_ne_bytes(
                    row[start..start + 4]
                        .try_into()
                        .expect("F32 sample has four bytes"),
                );
                if !value.is_finite() {
                    return Err(ScalePixelsImageError::NonFiniteInput {
                        byte_offset: plane.byte_offset()
                            + row_index as usize * plane.row_stride()
                            + start,
                    });
                }
            }
        }
    }
    Ok(())
}

fn sample_image(
    view: ImageView<'_>,
    x: f32,
    y: f32,
    channel: usize,
    kernel: ScalePixelsKernel,
) -> f32 {
    let format = view.descriptor().format();
    let width = view.descriptor().dimensions().width();
    let height = view.descriptor().dimensions().height();
    super::resample::sample_kernel(width, height, x, y, kernel, |source_x, source_y| {
        let (plane, channel_offset) = if format.storage() == StorageLayout::Planar {
            (channel, 0)
        } else {
            (0, channel)
        };
        let row = view
            .row(plane, source_y as u32)
            .expect("validated image row");
        let offset =
            source_x * format.bytes_per_pixel() + channel_offset * format.bytes_per_sample();
        f32::from_ne_bytes(row[offset..offset + 4].try_into().expect("F32 sample"))
    })
}

fn write_sample(
    descriptor: &ImageDescriptor,
    output: &mut [u8],
    x: u32,
    y: u32,
    channel: usize,
    value: f32,
) -> Result<(), ScalePixelsImageError> {
    let format = descriptor.format();
    let (plane_index, channel_offset) = if format.storage() == StorageLayout::Planar {
        (channel, 0)
    } else {
        (0, channel)
    };
    let plane = descriptor
        .planes()
        .get(plane_index)
        .ok_or(ScalePixelsImageError::ArithmeticOverflow)?;
    let row_offset = usize::try_from(y)
        .map_err(|_| ScalePixelsImageError::ArithmeticOverflow)?
        .checked_mul(plane.row_stride())
        .ok_or(ScalePixelsImageError::ArithmeticOverflow)?;
    let pixel_offset = usize::try_from(x)
        .ok()
        .and_then(|x| x.checked_mul(format.bytes_per_pixel()))
        .and_then(|offset| offset.checked_add(channel_offset * format.bytes_per_sample()))
        .ok_or(ScalePixelsImageError::ArithmeticOverflow)?;
    let offset = plane
        .byte_offset()
        .checked_add(row_offset)
        .and_then(|offset| offset.checked_add(pixel_offset))
        .ok_or(ScalePixelsImageError::ArithmeticOverflow)?;
    output[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
    Ok(())
}
