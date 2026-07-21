use crate::{LinearRgb, RasterDimensions};
use rusttable_image::{
    ByteOrder, ChannelLayout, ImageDescriptor, ImageView, ImageViewError, SampleType, StorageLayout,
};
use std::fmt;

use super::CanvasFill;
use super::geometry::EnlargeCanvasPlan;

impl EnlargeCanvasPlan {
    pub fn execute(
        &self,
        input: &[LinearRgb],
    ) -> Result<EnlargeCanvasExecution, EnlargeCanvasExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<EnlargeCanvasExecution, EnlargeCanvasExecutionError> {
        let expected = usize::try_from(self.source_dimensions().pixel_count()).map_err(|_| {
            EnlargeCanvasExecutionError::DimensionsMismatch {
                expected: usize::MAX,
                actual: input.len(),
            }
        })?;
        if input.len() != expected {
            return Err(EnlargeCanvasExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        let output_count = usize::try_from(self.output_dimensions().pixel_count())
            .map_err(|_| EnlargeCanvasExecutionError::ArithmeticOverflow)?;
        let mut pixels = vec![self.fill().rgb_pixel(); output_count];
        let source_width = usize::try_from(self.source_dimensions().width())
            .map_err(|_| EnlargeCanvasExecutionError::ArithmeticOverflow)?;
        let output_width = usize::try_from(self.output_dimensions().width())
            .map_err(|_| EnlargeCanvasExecutionError::ArithmeticOverflow)?;
        let left = usize::try_from(self.geometry().left())
            .map_err(|_| EnlargeCanvasExecutionError::ArithmeticOverflow)?;
        let top = usize::try_from(self.geometry().top())
            .map_err(|_| EnlargeCanvasExecutionError::ArithmeticOverflow)?;
        let source_height = usize::try_from(self.source_dimensions().height())
            .map_err(|_| EnlargeCanvasExecutionError::ArithmeticOverflow)?;
        for row in 0..source_height {
            if cancelled() {
                return Err(EnlargeCanvasExecutionError::Cancelled);
            }
            let source_start = row
                .checked_mul(source_width)
                .ok_or(EnlargeCanvasExecutionError::ArithmeticOverflow)?;
            let output_start = (top + row)
                .checked_mul(output_width)
                .and_then(|value| value.checked_add(left))
                .ok_or(EnlargeCanvasExecutionError::ArithmeticOverflow)?;
            pixels[output_start..output_start + source_width]
                .copy_from_slice(&input[source_start..source_start + source_width]);
        }
        Ok(EnlargeCanvasExecution {
            pixels,
            dimensions: self.output_dimensions(),
            identity: self.identity(),
        })
    }

    /// Copies a single-plane mask using the same placement and fills new
    /// canvas pixels with zero.  A zero-sized source intersection is valid.
    pub fn execute_mask(&self, input: &[f32]) -> Result<Vec<f32>, EnlargeCanvasExecutionError> {
        let expected = usize::try_from(self.source_dimensions().pixel_count())
            .map_err(|_| EnlargeCanvasExecutionError::ArithmeticOverflow)?;
        if input.len() != expected {
            return Err(EnlargeCanvasExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        let output_count = usize::try_from(self.output_dimensions().pixel_count())
            .map_err(|_| EnlargeCanvasExecutionError::ArithmeticOverflow)?;
        let mut output = vec![0.0; output_count];
        let source_width = usize::try_from(self.source_dimensions().width())
            .map_err(|_| EnlargeCanvasExecutionError::ArithmeticOverflow)?;
        let output_width = usize::try_from(self.output_dimensions().width())
            .map_err(|_| EnlargeCanvasExecutionError::ArithmeticOverflow)?;
        let left = usize::try_from(self.geometry().left())
            .map_err(|_| EnlargeCanvasExecutionError::ArithmeticOverflow)?;
        let top = usize::try_from(self.geometry().top())
            .map_err(|_| EnlargeCanvasExecutionError::ArithmeticOverflow)?;
        for row in 0..usize::try_from(self.source_dimensions().height()).expect("u32 fits usize") {
            let source_start = row * source_width;
            let output_start = (top + row) * output_width + left;
            output[output_start..output_start + source_width]
                .copy_from_slice(&input[source_start..source_start + source_width]);
        }
        Ok(output)
    }

    /// Executes a validated native-F32 image while preserving its image
    /// descriptor's color encoding, profile, orientation, and channel layout.
    pub fn execute_image(
        &self,
        descriptor: &ImageDescriptor,
        input: &[u8],
    ) -> Result<EnlargeCanvasImageExecution, EnlargeCanvasImageError> {
        let view = ImageView::new(descriptor, input).map_err(EnlargeCanvasImageError::View)?;
        validate_image_format(descriptor)?;
        let actual = descriptor.dimensions();
        if actual.width() != self.source_dimensions().width()
            || actual.height() != self.source_dimensions().height()
        {
            return Err(EnlargeCanvasImageError::DimensionsMismatch {
                expected: self.source_dimensions(),
                actual,
            });
        }
        let format = descriptor.format();
        let row_stride = output_row_stride(self.output_dimensions(), format)?;
        let strides = vec![row_stride; format.plane_count()];
        let output_descriptor = ImageDescriptor::with_strides(
            rusttable_image::ImageDimensions::new(
                self.output_dimensions().width(),
                self.output_dimensions().height(),
            )
            .map_err(|_| EnlargeCanvasImageError::ArithmeticOverflow)?,
            format,
            descriptor.color_encoding(),
            descriptor.profile().copied(),
            descriptor.orientation(),
            &strides,
        )
        .map_err(|_| EnlargeCanvasImageError::ArithmeticOverflow)?;
        let mut output = vec![0_u8; output_descriptor.byte_length()];
        fill_image(&output_descriptor, &mut output, self.fill())?;
        copy_image(
            descriptor,
            view,
            &output_descriptor,
            &mut output,
            self.geometry().left(),
            self.geometry().top(),
        )?;
        Ok(EnlargeCanvasImageExecution {
            descriptor: output_descriptor,
            bytes: output,
            identity: self.identity(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnlargeCanvasExecution {
    pixels: Vec<LinearRgb>,
    dimensions: RasterDimensions,
    identity: [u8; 32],
}

impl EnlargeCanvasExecution {
    #[must_use]
    pub fn pixels(&self) -> &[LinearRgb] {
        &self.pixels
    }
    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnlargeCanvasExecutionError {
    Cancelled,
    DimensionsMismatch { expected: usize, actual: usize },
    ArithmeticOverflow,
}

impl fmt::Display for EnlargeCanvasExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Cancelled => "enlargecanvas execution was cancelled",
            Self::DimensionsMismatch { .. } => {
                "enlargecanvas input dimensions do not match the plan"
            }
            Self::ArithmeticOverflow => "enlargecanvas execution arithmetic overflowed",
        })
    }
}

impl std::error::Error for EnlargeCanvasExecutionError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnlargeCanvasImageExecution {
    descriptor: ImageDescriptor,
    bytes: Vec<u8>,
    identity: [u8; 32],
}

impl EnlargeCanvasImageExecution {
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
pub enum EnlargeCanvasImageError {
    View(ImageViewError),
    UnsupportedSampleType,
    UnsupportedByteOrder,
    UnsupportedMosaic,
    DimensionsMismatch {
        expected: RasterDimensions,
        actual: rusttable_image::ImageDimensions,
    },
    ArithmeticOverflow,
}

impl fmt::Display for EnlargeCanvasImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::View(_) => "enlargecanvas image view is invalid",
            Self::UnsupportedSampleType => "enlargecanvas requires native F32 samples",
            Self::UnsupportedByteOrder => "enlargecanvas requires native byte order",
            Self::UnsupportedMosaic => "enlargecanvas does not fill mosaic images",
            Self::DimensionsMismatch { .. } => {
                "enlargecanvas image dimensions do not match the plan"
            }
            Self::ArithmeticOverflow => "enlargecanvas image arithmetic overflowed",
        })
    }
}

impl std::error::Error for EnlargeCanvasImageError {}

fn validate_image_format(descriptor: &ImageDescriptor) -> Result<(), EnlargeCanvasImageError> {
    let format = descriptor.format();
    if format.sample_type() != SampleType::F32 {
        return Err(EnlargeCanvasImageError::UnsupportedSampleType);
    }
    if format.byte_order() != ByteOrder::Native {
        return Err(EnlargeCanvasImageError::UnsupportedByteOrder);
    }
    if format.channels().is_mosaic() {
        return Err(EnlargeCanvasImageError::UnsupportedMosaic);
    }
    Ok(())
}

fn output_row_stride(
    dimensions: RasterDimensions,
    format: rusttable_image::PixelFormat,
) -> Result<usize, EnlargeCanvasImageError> {
    usize::try_from(dimensions.width())
        .ok()
        .and_then(|width| {
            width.checked_mul(if format.storage() == StorageLayout::Interleaved {
                format.bytes_per_pixel()
            } else {
                format.bytes_per_sample()
            })
        })
        .ok_or(EnlargeCanvasImageError::ArithmeticOverflow)
}

fn fill_image(
    descriptor: &ImageDescriptor,
    output: &mut [u8],
    fill: CanvasFill,
) -> Result<(), EnlargeCanvasImageError> {
    let format = descriptor.format();
    let values = channel_values(format.channels(), fill);
    for (plane_index, plane) in descriptor.planes().iter().enumerate() {
        for y in 0..plane.height() {
            let row_start = plane
                .byte_offset()
                .checked_add(
                    usize::try_from(y).map_err(|_| EnlargeCanvasImageError::ArithmeticOverflow)?
                        * plane.row_stride(),
                )
                .ok_or(EnlargeCanvasImageError::ArithmeticOverflow)?;
            let width = usize::try_from(plane.width())
                .map_err(|_| EnlargeCanvasImageError::ArithmeticOverflow)?;
            for x in 0..width {
                let channel = if format.storage() == StorageLayout::Planar {
                    plane_index
                } else {
                    0
                };
                let channel_count = if format.storage() == StorageLayout::Interleaved {
                    format.channels().channels()
                } else {
                    1
                };
                for component in 0..channel_count {
                    let logical = if format.storage() == StorageLayout::Planar {
                        channel
                    } else {
                        component
                    };
                    let offset = row_start
                        .checked_add(
                            x.checked_mul(format.bytes_per_pixel())
                                .ok_or(EnlargeCanvasImageError::ArithmeticOverflow)?,
                        )
                        .and_then(|value| value.checked_add(component * format.bytes_per_sample()))
                        .ok_or(EnlargeCanvasImageError::ArithmeticOverflow)?;
                    output[offset..offset + 4].copy_from_slice(&values[logical].to_ne_bytes());
                }
            }
        }
    }
    Ok(())
}

fn copy_image(
    source_descriptor: &ImageDescriptor,
    source: ImageView<'_>,
    output_descriptor: &ImageDescriptor,
    output: &mut [u8],
    left: u32,
    top: u32,
) -> Result<(), EnlargeCanvasImageError> {
    let format = source_descriptor.format();
    for (plane_index, source_plane) in source_descriptor.planes().iter().enumerate() {
        let output_plane = output_descriptor
            .planes()
            .get(plane_index)
            .ok_or(EnlargeCanvasImageError::ArithmeticOverflow)?;
        let row_bytes = usize::try_from(source_plane.width())
            .ok()
            .and_then(|width| {
                width.checked_mul(if format.storage() == StorageLayout::Interleaved {
                    format.bytes_per_pixel()
                } else {
                    format.bytes_per_sample()
                })
            })
            .ok_or(EnlargeCanvasImageError::ArithmeticOverflow)?;
        let left_bytes = usize::try_from(left)
            .ok()
            .and_then(|value| {
                value.checked_mul(if format.storage() == StorageLayout::Interleaved {
                    format.bytes_per_pixel()
                } else {
                    format.bytes_per_sample()
                })
            })
            .ok_or(EnlargeCanvasImageError::ArithmeticOverflow)?;
        for row in 0..source_plane.height() {
            let source_row = source
                .row(plane_index, row)
                .map_err(EnlargeCanvasImageError::View)?;
            let output_row = output_plane
                .byte_offset()
                .checked_add(
                    usize::try_from(top + row)
                        .map_err(|_| EnlargeCanvasImageError::ArithmeticOverflow)?
                        * output_plane.row_stride(),
                )
                .and_then(|value| value.checked_add(left_bytes))
                .ok_or(EnlargeCanvasImageError::ArithmeticOverflow)?;
            output[output_row..output_row + row_bytes].copy_from_slice(&source_row[..row_bytes]);
        }
    }
    Ok(())
}

fn channel_values(layout: ChannelLayout, fill: CanvasFill) -> [f32; 4] {
    let alpha = match fill.alpha() {
        value if value.get().is_finite() => value.get(),
        _ => 1.0,
    };
    let luma = 0.2126 * fill.red().get() + 0.7152 * fill.green().get() + 0.0722 * fill.blue().get();
    match layout {
        ChannelLayout::Gray => [luma, 0.0, 0.0, 0.0],
        ChannelLayout::GrayA => [luma, alpha, 0.0, 0.0],
        ChannelLayout::Rgb => [fill.red().get(), fill.green().get(), fill.blue().get(), 0.0],
        ChannelLayout::Rgba => [
            fill.red().get(),
            fill.green().get(),
            fill.blue().get(),
            alpha,
        ],
        ChannelLayout::Bayer | ChannelLayout::XTrans => [0.0, 0.0, 0.0, 0.0],
    }
}
