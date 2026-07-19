use std::fmt;

use crate::{ImageDimensions, ImageRect, PixelFormat, PixelFormatError, StorageLayout};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaneError {
    InvalidFormat(PixelFormatError),
    ZeroRowStride,
    RowStrideTooSmall { required: usize, actual: usize },
    BufferTooShort { required: usize, actual: usize },
    OutOfBounds,
    ArithmeticOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaneDescriptor {
    dimensions: ImageDimensions,
    format: PixelFormat,
    storage: StorageLayout,
    row_stride: usize,
}

impl PlaneDescriptor {
    /// Creates a checked descriptor for one owned or borrowed plane.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid storage, zero stride, undersized stride,
    /// or arithmetic overflow.
    pub fn new(
        dimensions: ImageDimensions,
        format: PixelFormat,
        storage: StorageLayout,
        row_stride: usize,
    ) -> Result<Self, PlaneError> {
        format
            .validate_storage(storage)
            .map_err(PlaneError::InvalidFormat)?;
        if row_stride == 0 {
            return Err(PlaneError::ZeroRowStride);
        }
        let bytes_per_pixel = match storage {
            StorageLayout::Interleaved => format.bytes_per_pixel(),
            StorageLayout::Planar { .. } => Ok(format.bytes_per_sample()),
        }
        .map_err(PlaneError::InvalidFormat)?;
        let row_bytes = usize::try_from(dimensions.width())
            .map_err(|_| PlaneError::ArithmeticOverflow)?
            .checked_mul(bytes_per_pixel)
            .ok_or(PlaneError::ArithmeticOverflow)?;
        if row_stride < row_bytes {
            return Err(PlaneError::RowStrideTooSmall {
                required: row_bytes,
                actual: row_stride,
            });
        }
        let descriptor = Self {
            dimensions,
            format,
            storage,
            row_stride,
        };
        descriptor.buffer_bytes()?;
        Ok(descriptor)
    }

    #[must_use]
    pub const fn dimensions(self) -> ImageDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn format(self) -> PixelFormat {
        self.format
    }

    #[must_use]
    pub const fn storage(self) -> StorageLayout {
        self.storage
    }

    #[must_use]
    pub const fn row_stride(self) -> usize {
        self.row_stride
    }

    /// Returns the visible bytes in one row, excluding row padding.
    ///
    /// # Errors
    ///
    /// Returns an error when the row size overflows.
    pub fn row_bytes(self) -> Result<usize, PlaneError> {
        let bytes_per_pixel = match self.storage {
            StorageLayout::Interleaved => self.format.bytes_per_pixel(),
            StorageLayout::Planar { .. } => Ok(self.format.bytes_per_sample()),
        }
        .map_err(PlaneError::InvalidFormat)?;
        usize::try_from(self.dimensions.width())
            .map_err(|_| PlaneError::ArithmeticOverflow)?
            .checked_mul(bytes_per_pixel)
            .ok_or(PlaneError::ArithmeticOverflow)
    }

    /// Returns the full row-stride allocation size.
    ///
    /// # Errors
    ///
    /// Returns an error when the allocation size overflows.
    pub fn buffer_bytes(self) -> Result<usize, PlaneError> {
        let height = usize::try_from(self.dimensions.height())
            .map_err(|_| PlaneError::ArithmeticOverflow)?;
        self.row_stride
            .checked_mul(height)
            .ok_or(PlaneError::ArithmeticOverflow)
    }

    fn visible_buffer_bytes(self) -> Result<usize, PlaneError> {
        let height = usize::try_from(self.dimensions.height())
            .map_err(|_| PlaneError::ArithmeticOverflow)?;
        let last_row = height
            .checked_sub(1)
            .ok_or(PlaneError::ArithmeticOverflow)?
            .checked_mul(self.row_stride)
            .ok_or(PlaneError::ArithmeticOverflow)?;
        last_row
            .checked_add(self.row_bytes()?)
            .ok_or(PlaneError::ArithmeticOverflow)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedPlane {
    descriptor: PlaneDescriptor,
    data: Vec<u8>,
}

impl OwnedPlane {
    /// Takes ownership of a descriptor-backed byte allocation.
    ///
    /// # Errors
    ///
    /// Returns an error when the allocation cannot contain the visible plane.
    pub fn new(descriptor: PlaneDescriptor, data: Vec<u8>) -> Result<Self, PlaneError> {
        let required = descriptor.visible_buffer_bytes()?;
        if data.len() < required {
            return Err(PlaneError::BufferTooShort {
                required,
                actual: data.len(),
            });
        }
        Ok(Self { descriptor, data })
    }

    #[must_use]
    pub const fn descriptor(&self) -> PlaneDescriptor {
        self.descriptor
    }

    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    #[must_use]
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    #[must_use]
    pub fn view(&self) -> PlaneView<'_> {
        PlaneView {
            descriptor: self.descriptor,
            data: &self.data,
        }
    }

    pub fn view_mut(&mut self) -> PlaneViewMut<'_> {
        PlaneViewMut {
            descriptor: self.descriptor,
            data: &mut self.data,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PlaneView<'a> {
    descriptor: PlaneDescriptor,
    data: &'a [u8],
}

impl<'a> PlaneView<'a> {
    /// Borrows bytes using a checked descriptor.
    ///
    /// # Errors
    ///
    /// Returns an error when the byte slice is shorter than the visible plane.
    pub fn new(descriptor: PlaneDescriptor, data: &'a [u8]) -> Result<Self, PlaneError> {
        let required = descriptor.visible_buffer_bytes()?;
        if data.len() < required {
            return Err(PlaneError::BufferTooShort {
                required,
                actual: data.len(),
            });
        }
        Ok(Self { descriptor, data })
    }

    #[must_use]
    pub const fn descriptor(self) -> PlaneDescriptor {
        self.descriptor
    }

    #[must_use]
    pub fn data(self) -> &'a [u8] {
        self.data
    }

    /// Returns one visible row without its padding bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the row index or arithmetic is invalid.
    pub fn row(self, y: u32) -> Result<&'a [u8], PlaneError> {
        let row_bytes = self.descriptor.row_bytes()?;
        let start = self.row_start(y)?;
        Ok(&self.data[start..start + row_bytes])
    }

    /// Returns one complete row including padding.
    ///
    /// # Errors
    ///
    /// Returns an error when the row index or arithmetic is invalid.
    pub fn row_with_padding(self, y: u32) -> Result<&'a [u8], PlaneError> {
        let start = self.row_start(y)?;
        Ok(&self.data[start..start + self.descriptor.row_stride()])
    }

    #[must_use]
    pub fn rows(self) -> PlaneRows<'a> {
        PlaneRows {
            view: self,
            next: 0,
        }
    }

    /// Returns a checked byte offset for one typed sample.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid coordinate, channel, or arithmetic.
    pub fn sample_offset(self, x: u32, y: u32, channel: u8) -> Result<usize, PlaneError> {
        if x >= self.descriptor.dimensions().width() || y >= self.descriptor.dimensions().height() {
            return Err(PlaneError::OutOfBounds);
        }
        let format = self.descriptor.format();
        let sample = usize::from(channel);
        let sample_bytes = format.bytes_per_sample();
        let within_pixel = match self.descriptor.storage() {
            StorageLayout::Interleaved => {
                if sample >= format.channels().channels() {
                    return Err(PlaneError::OutOfBounds);
                }
                usize::try_from(x)
                    .map_err(|_| PlaneError::ArithmeticOverflow)?
                    .checked_mul(
                        format
                            .bytes_per_pixel()
                            .map_err(PlaneError::InvalidFormat)?,
                    )
                    .and_then(|offset| offset.checked_add(sample.checked_mul(sample_bytes)?))
                    .ok_or(PlaneError::ArithmeticOverflow)?
            }
            StorageLayout::Planar { plane } => {
                if channel != plane {
                    return Err(PlaneError::OutOfBounds);
                }
                usize::try_from(x)
                    .map_err(|_| PlaneError::ArithmeticOverflow)?
                    .checked_mul(sample_bytes)
                    .ok_or(PlaneError::ArithmeticOverflow)?
            }
        };
        self.row_start(y)?
            .checked_add(within_pixel)
            .ok_or(PlaneError::ArithmeticOverflow)
    }

    /// Creates a zero-copy view over a validated ROI.
    ///
    /// # Errors
    ///
    /// Returns an error when the ROI lies outside this plane.
    pub fn crop(self, rect: ImageRect) -> Result<Self, PlaneError> {
        if rect.x() >= self.descriptor.dimensions().width()
            || rect.y() >= self.descriptor.dimensions().height()
            || rect.width() > self.descriptor.dimensions().width() - rect.x()
            || rect.height() > self.descriptor.dimensions().height() - rect.y()
        {
            return Err(PlaneError::OutOfBounds);
        }
        let channel = match self.descriptor.storage() {
            StorageLayout::Interleaved => 0,
            StorageLayout::Planar { plane } => plane,
        };
        let offset = self.sample_offset(rect.x(), rect.y(), channel)?;
        let dimensions = rect.dimensions();
        let descriptor = PlaneDescriptor::new(
            dimensions,
            self.descriptor.format(),
            self.descriptor.storage(),
            self.descriptor.row_stride(),
        )?;
        Self::new(descriptor, &self.data[offset..])
    }

    fn row_start(self, y: u32) -> Result<usize, PlaneError> {
        if y >= self.descriptor.dimensions().height() {
            return Err(PlaneError::OutOfBounds);
        }
        usize::try_from(y)
            .map_err(|_| PlaneError::ArithmeticOverflow)?
            .checked_mul(self.descriptor.row_stride())
            .ok_or(PlaneError::ArithmeticOverflow)
    }
}

pub struct PlaneRows<'a> {
    view: PlaneView<'a>,
    next: u32,
}

impl<'a> Iterator for PlaneRows<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.next >= self.view.descriptor.dimensions().height() {
            return None;
        }
        let row = self.view.row(self.next).ok();
        self.next = self.next.saturating_add(1);
        row
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self
            .view
            .descriptor
            .dimensions()
            .height()
            .saturating_sub(self.next) as usize;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for PlaneRows<'_> {}

#[derive(Debug)]
pub struct PlaneViewMut<'a> {
    descriptor: PlaneDescriptor,
    data: &'a mut [u8],
}

impl PlaneViewMut<'_> {
    #[must_use]
    pub const fn descriptor(&self) -> PlaneDescriptor {
        self.descriptor
    }

    #[must_use]
    pub fn data(&self) -> &[u8] {
        self.data
    }

    #[must_use]
    pub fn data_mut(&mut self) -> &mut [u8] {
        self.data
    }
}

impl fmt::Display for PlaneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat(error) => error.fmt(formatter),
            Self::ZeroRowStride => formatter.write_str("plane row stride must be nonzero"),
            Self::RowStrideTooSmall { required, actual } => {
                write!(
                    formatter,
                    "row stride {actual} is below required {required}"
                )
            }
            Self::BufferTooShort { required, actual } => {
                write!(
                    formatter,
                    "plane buffer has {actual} bytes, requires {required}"
                )
            }
            Self::OutOfBounds => formatter.write_str("plane coordinate is out of bounds"),
            Self::ArithmeticOverflow => formatter.write_str("plane arithmetic overflowed"),
        }
    }
}

impl std::error::Error for PlaneError {}
