use std::fmt;

use crate::{ColorEncoding, ColorProfileRef, ImageDimensions, PixelFormat, Roi, StorageLayout};

/// One checked plane's row layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlaneDescriptor {
    width: u32,
    height: u32,
    row_stride: usize,
    byte_offset: usize,
    byte_length: usize,
}

impl PlaneDescriptor {
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }

    #[must_use]
    pub const fn row_stride(self) -> usize {
        self.row_stride
    }

    #[must_use]
    pub const fn byte_offset(self) -> usize {
        self.byte_offset
    }

    #[must_use]
    pub const fn byte_length(self) -> usize {
        self.byte_length
    }

    #[must_use]
    pub const fn end(self) -> usize {
        self.byte_offset + self.byte_length
    }
}

/// A storage-neutral description shared by all image consumers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageDescriptor {
    dimensions: ImageDimensions,
    format: PixelFormat,
    color_encoding: ColorEncoding,
    profile: Option<ColorProfileRef>,
    orientation: crate::Orientation,
    planes: Vec<PlaneDescriptor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageDescriptorError {
    ArithmeticOverflow,
    InvalidPlaneCount,
    StrideTooSmall {
        plane: usize,
        minimum: usize,
        actual: usize,
    },
    TooManyBytes,
}

impl ImageDescriptor {
    /// Creates tightly packed planes for a format.
    ///
    /// # Errors
    ///
    /// Returns a descriptor error when checked stride or allocation arithmetic
    /// cannot be represented.
    pub fn new(
        dimensions: ImageDimensions,
        format: PixelFormat,
        color_encoding: ColorEncoding,
        orientation: crate::Orientation,
    ) -> Result<Self, ImageDescriptorError> {
        let strides = (0..format.plane_count())
            .map(|_| default_stride(dimensions.width(), format))
            .collect::<Result<Vec<_>, _>>()?;
        Self::with_strides(
            dimensions,
            format,
            color_encoding,
            None,
            orientation,
            &strides,
        )
    }

    /// Creates a descriptor with explicit checked row strides.
    ///
    /// # Errors
    ///
    /// Returns an error for a wrong plane count, short stride, or overflow.
    pub fn with_strides(
        dimensions: ImageDimensions,
        format: PixelFormat,
        color_encoding: ColorEncoding,
        profile: Option<ColorProfileRef>,
        orientation: crate::Orientation,
        row_strides: &[usize],
    ) -> Result<Self, ImageDescriptorError> {
        if row_strides.len() != format.plane_count() {
            return Err(ImageDescriptorError::InvalidPlaneCount);
        }
        let mut offset = 0usize;
        let mut planes = Vec::with_capacity(row_strides.len());
        for (plane, stride) in row_strides.iter().copied().enumerate() {
            let minimum = minimum_stride(dimensions.width(), format);
            if stride < minimum {
                return Err(ImageDescriptorError::StrideTooSmall {
                    plane,
                    minimum,
                    actual: stride,
                });
            }
            let byte_length = stride
                .checked_mul(
                    usize::try_from(dimensions.height())
                        .map_err(|_| ImageDescriptorError::ArithmeticOverflow)?,
                )
                .ok_or(ImageDescriptorError::ArithmeticOverflow)?;
            let end = offset
                .checked_add(byte_length)
                .ok_or(ImageDescriptorError::ArithmeticOverflow)?;
            planes.push(PlaneDescriptor {
                width: dimensions.width(),
                height: dimensions.height(),
                row_stride: stride,
                byte_offset: offset,
                byte_length,
            });
            offset = end;
        }
        Ok(Self {
            dimensions,
            format,
            color_encoding,
            profile,
            orientation,
            planes,
        })
    }

    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn format(&self) -> PixelFormat {
        self.format
    }

    #[must_use]
    pub const fn color_encoding(&self) -> ColorEncoding {
        self.color_encoding
    }

    #[must_use]
    pub const fn orientation(&self) -> crate::Orientation {
        self.orientation
    }

    #[must_use]
    pub fn profile(&self) -> Option<&ColorProfileRef> {
        self.profile.as_ref()
    }

    #[must_use]
    pub fn planes(&self) -> &[PlaneDescriptor] {
        &self.planes
    }

    #[must_use]
    pub fn byte_length(&self) -> usize {
        self.planes.last().map_or(0, |plane| plane.end())
    }

    /// Returns the byte offset of an interleaved pixel after checked bounds.
    /// Returns the checked byte offset of one interleaved pixel.
    ///
    /// # Errors
    ///
    /// Returns an error for out-of-bounds coordinates, planar storage, or
    /// arithmetic overflow.
    pub fn pixel_offset(&self, x: u32, y: u32) -> Result<usize, ImageViewError> {
        if x >= self.dimensions.width() || y >= self.dimensions.height() {
            return Err(ImageViewError::OutOfBounds);
        }
        if self.format.storage() != StorageLayout::Interleaved {
            return Err(ImageViewError::NotInterleaved);
        }
        let plane = self
            .planes
            .first()
            .ok_or(ImageViewError::InvalidDescriptor)?;
        let row = usize::try_from(y)
            .map_err(|_| ImageViewError::ArithmeticOverflow)?
            .checked_mul(plane.row_stride())
            .ok_or(ImageViewError::ArithmeticOverflow)?;
        let column = usize::try_from(x)
            .map_err(|_| ImageViewError::ArithmeticOverflow)?
            .checked_mul(self.format.bytes_per_pixel())
            .ok_or(ImageViewError::ArithmeticOverflow)?;
        plane
            .byte_offset()
            .checked_add(row)
            .and_then(|offset| offset.checked_add(column))
            .ok_or(ImageViewError::ArithmeticOverflow)
    }
}

fn default_stride(width: u32, format: PixelFormat) -> Result<usize, ImageDescriptorError> {
    let width = usize::try_from(width).map_err(|_| ImageDescriptorError::ArithmeticOverflow)?;
    width
        .checked_mul(format.bytes_per_pixel())
        .ok_or(ImageDescriptorError::ArithmeticOverflow)
}

fn minimum_stride(width: u32, format: PixelFormat) -> usize {
    usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(format.bytes_per_pixel()))
        .unwrap_or(usize::MAX)
}

/// An immutable checked view over one or more image planes.
#[derive(Debug, Clone, Copy)]
pub struct ImageView<'a> {
    descriptor: &'a ImageDescriptor,
    bytes: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageViewError {
    BufferTooShort { required: usize, actual: usize },
    PlaneOutOfBounds,
    RowOutOfBounds,
    OutOfBounds,
    NotInterleaved,
    InvalidDescriptor,
    ArithmeticOverflow,
}

impl<'a> ImageView<'a> {
    /// Borrows storage after checking the complete descriptor extent.
    ///
    /// # Errors
    ///
    /// Returns [`ImageViewError::BufferTooShort`] when backing storage is short.
    pub fn new(descriptor: &'a ImageDescriptor, bytes: &'a [u8]) -> Result<Self, ImageViewError> {
        if bytes.len() < descriptor.byte_length() {
            return Err(ImageViewError::BufferTooShort {
                required: descriptor.byte_length(),
                actual: bytes.len(),
            });
        }
        Ok(Self { descriptor, bytes })
    }

    #[must_use]
    pub const fn descriptor(self) -> &'a ImageDescriptor {
        self.descriptor
    }

    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.bytes
    }

    /// Returns one complete plane including row padding.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid plane index.
    pub fn plane(self, index: usize) -> Result<&'a [u8], ImageViewError> {
        let plane = self
            .descriptor
            .planes()
            .get(index)
            .copied()
            .ok_or(ImageViewError::PlaneOutOfBounds)?;
        Ok(&self.bytes[plane.byte_offset()..plane.end()])
    }

    /// Returns one checked row including its padding bytes.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid plane or row, or arithmetic overflow.
    pub fn row(self, plane_index: usize, row: u32) -> Result<&'a [u8], ImageViewError> {
        let plane = self
            .descriptor
            .planes()
            .get(plane_index)
            .copied()
            .ok_or(ImageViewError::PlaneOutOfBounds)?;
        if row >= plane.height() {
            return Err(ImageViewError::RowOutOfBounds);
        }
        let start = plane
            .byte_offset()
            .checked_add(
                usize::try_from(row)
                    .map_err(|_| ImageViewError::ArithmeticOverflow)?
                    .checked_mul(plane.row_stride())
                    .ok_or(ImageViewError::ArithmeticOverflow)?,
            )
            .ok_or(ImageViewError::ArithmeticOverflow)?;
        Ok(&self.bytes[start..start + plane.row_stride()])
    }
}

/// An exclusive checked mutable view. Rust's borrow rules provide the alias
/// safety guarantee for writable image operations.
pub struct ImageViewMut<'a> {
    descriptor: &'a ImageDescriptor,
    bytes: &'a mut [u8],
}

impl<'a> ImageViewMut<'a> {
    /// Borrows exclusive storage after checking the complete descriptor extent.
    ///
    /// # Errors
    ///
    /// Returns [`ImageViewError::BufferTooShort`] when backing storage is short.
    pub fn new(
        descriptor: &'a ImageDescriptor,
        bytes: &'a mut [u8],
    ) -> Result<Self, ImageViewError> {
        if bytes.len() < descriptor.byte_length() {
            return Err(ImageViewError::BufferTooShort {
                required: descriptor.byte_length(),
                actual: bytes.len(),
            });
        }
        Ok(Self { descriptor, bytes })
    }

    #[must_use]
    pub const fn descriptor(&self) -> &'a ImageDescriptor {
        self.descriptor
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.bytes
    }

    /// Returns one exclusive mutable row including padding bytes.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid plane or row, or arithmetic overflow.
    pub fn row_mut(&mut self, plane_index: usize, row: u32) -> Result<&mut [u8], ImageViewError> {
        let plane = self
            .descriptor
            .planes()
            .get(plane_index)
            .copied()
            .ok_or(ImageViewError::PlaneOutOfBounds)?;
        if row >= plane.height() {
            return Err(ImageViewError::RowOutOfBounds);
        }
        let start = plane
            .byte_offset()
            .checked_add(
                usize::try_from(row)
                    .map_err(|_| ImageViewError::ArithmeticOverflow)?
                    .checked_mul(plane.row_stride())
                    .ok_or(ImageViewError::ArithmeticOverflow)?,
            )
            .ok_or(ImageViewError::ArithmeticOverflow)?;
        Ok(&mut self.bytes[start..start + plane.row_stride()])
    }
}

/// Owned storage paired permanently with its validated descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedImage {
    descriptor: ImageDescriptor,
    bytes: Vec<u8>,
}

impl OwnedImage {
    /// Creates owned storage whose length exactly matches its descriptor.
    ///
    /// # Errors
    ///
    /// Returns [`ImageViewError::BufferTooShort`] for any length mismatch.
    pub fn new(descriptor: ImageDescriptor, bytes: Vec<u8>) -> Result<Self, ImageViewError> {
        if bytes.len() != descriptor.byte_length() {
            return Err(ImageViewError::BufferTooShort {
                required: descriptor.byte_length(),
                actual: bytes.len(),
            });
        }
        Ok(Self { descriptor, bytes })
    }

    #[must_use]
    pub const fn descriptor(&self) -> &ImageDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Borrows the owned image through its descriptor.
    ///
    /// # Errors
    ///
    /// Returns a view error if the owned invariant has been violated.
    pub fn view(&self) -> Result<ImageView<'_>, ImageViewError> {
        ImageView::new(&self.descriptor, &self.bytes)
    }

    /// Borrows an exclusive mutable view of the owned image.
    ///
    /// # Errors
    ///
    /// Returns a view error if the owned invariant has been violated.
    pub fn view_mut(&mut self) -> Result<ImageViewMut<'_>, ImageViewError> {
        ImageViewMut::new(&self.descriptor, &mut self.bytes)
    }
}

/// One 16-byte interleaved RGBA pixel.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlignedRgbaF32 {
    channels: [f32; 4],
}

#[repr(C, align(64))]
#[derive(Debug, Clone, Copy, PartialEq)]
struct AlignedRgbaChunk {
    pixels: [AlignedRgbaF32; 4],
}

impl AlignedRgbaF32 {
    #[must_use]
    pub const fn new(channels: [f32; 4]) -> Self {
        Self { channels }
    }

    #[must_use]
    pub const fn channels(self) -> [f32; 4] {
        self.channels
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalRgbaF32 {
    dimensions: ImageDimensions,
    chunks: Vec<AlignedRgbaChunk>,
    length: usize,
    color_encoding: ColorEncoding,
}

impl CanonicalRgbaF32 {
    /// Creates a finite canonical linear-light RGBA image.
    ///
    /// # Errors
    ///
    /// Returns an error when pixel count or component finiteness is invalid.
    pub fn new(
        dimensions: ImageDimensions,
        pixels: impl IntoIterator<Item = AlignedRgbaF32>,
    ) -> Result<Self, ImageViewError> {
        let pixels = pixels.into_iter().collect::<Vec<_>>();
        if dimensions.pixel_count() != Ok(u64::try_from(pixels.len()).unwrap_or(u64::MAX)) {
            return Err(ImageViewError::InvalidDescriptor);
        }
        if pixels.iter().any(|pixel| {
            pixel
                .channels()
                .into_iter()
                .any(|channel| !channel.is_finite())
        }) {
            return Err(ImageViewError::InvalidDescriptor);
        }
        let length = pixels.len();
        let mut chunks = Vec::with_capacity(length.div_ceil(4));
        for group in pixels.chunks(4) {
            let mut chunk = [AlignedRgbaF32::new([0.0; 4]); 4];
            chunk[..group.len()].copy_from_slice(group);
            chunks.push(AlignedRgbaChunk { pixels: chunk });
        }
        Ok(Self {
            dimensions,
            chunks,
            length,
            color_encoding: ColorEncoding::LinearSrgb,
        })
    }

    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn format(&self) -> PixelFormat {
        PixelFormat::canonical_rgba_f32()
    }

    #[must_use]
    pub const fn color_encoding(&self) -> ColorEncoding {
        self.color_encoding
    }

    pub fn pixels(&self) -> impl Iterator<Item = &AlignedRgbaF32> {
        self.chunks
            .iter()
            .flat_map(|chunk| chunk.pixels.iter())
            .take(self.length)
    }

    #[must_use]
    pub fn pixel(&self, index: usize) -> Option<&AlignedRgbaF32> {
        if index >= self.length {
            return None;
        }
        self.chunks
            .get(index / 4)
            .and_then(|chunk| chunk.pixels.get(index % 4))
    }

    /// Reports whether the backing allocation begins on a 64-byte boundary.
    #[must_use]
    pub fn is_cacheline_aligned(&self) -> bool {
        self.chunks.as_ptr().align_offset(64) == 0
    }
}

impl fmt::Display for ImageViewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BufferTooShort { required, actual } => {
                write!(
                    formatter,
                    "image buffer has {actual} bytes, needs {required}"
                )
            }
            Self::PlaneOutOfBounds => formatter.write_str("image plane index is out of bounds"),
            Self::RowOutOfBounds => formatter.write_str("image row is out of bounds"),
            Self::OutOfBounds => formatter.write_str("image coordinate is out of bounds"),
            Self::NotInterleaved => {
                formatter.write_str("pixel offsets require interleaved storage")
            }
            Self::InvalidDescriptor => formatter.write_str("image descriptor invariant failed"),
            Self::ArithmeticOverflow => formatter.write_str("image view arithmetic overflowed"),
        }
    }
}

impl std::error::Error for ImageViewError {}

impl From<ImageDimensions> for Roi {
    fn from(dimensions: ImageDimensions) -> Self {
        Self::full(dimensions)
    }
}
