use std::alloc::Layout;
use std::fmt;
use std::mem::{align_of, size_of};

use crate::ImageDimensions;

#[repr(C, align(64))]
#[derive(Debug, Clone, Copy, PartialEq)]
struct AlignedRgba([f32; 4]);

impl AlignedRgba {
    const ZERO: Self = Self([0.0; 4]);

    #[must_use]
    const fn as_array(&self) -> &[f32; 4] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferAllocationError {
    ArithmeticOverflow,
    CapacityOverflow,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalRgbaBuffer {
    dimensions: ImageDimensions,
    pixels: Vec<AlignedRgba>,
}

impl CanonicalRgbaBuffer {
    /// Allocates a zeroed canonical RGBA buffer.
    ///
    /// # Errors
    ///
    /// Returns an error when the pixel count cannot be represented or the
    /// allocation cannot be reserved.
    pub fn new(dimensions: ImageDimensions) -> Result<Self, BufferAllocationError> {
        let count = usize::try_from(
            dimensions
                .pixel_count()
                .map_err(|_| BufferAllocationError::ArithmeticOverflow)?,
        )
        .map_err(|_| BufferAllocationError::CapacityOverflow)?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(count)
            .map_err(|_| BufferAllocationError::CapacityOverflow)?;
        pixels.resize(count, AlignedRgba::ZERO);
        Ok(Self { dimensions, pixels })
    }

    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn pixel_alignment() -> usize {
        16
    }

    #[must_use]
    pub const fn allocation_alignment() -> usize {
        align_of::<AlignedRgba>()
    }

    /// Returns the logical tightly packed byte size of the buffer.
    ///
    /// # Errors
    ///
    /// Returns an error when the size multiplication overflows.
    pub fn byte_size(&self) -> Result<usize, BufferAllocationError> {
        self.pixels
            .len()
            .checked_mul(Self::pixel_alignment())
            .ok_or(BufferAllocationError::ArithmeticOverflow)
    }

    /// Returns the physical byte size including alignment padding.
    ///
    /// # Errors
    ///
    /// Returns an error when the size multiplication overflows.
    pub fn allocated_byte_size(&self) -> Result<usize, BufferAllocationError> {
        self.pixels
            .len()
            .checked_mul(size_of::<AlignedRgba>())
            .ok_or(BufferAllocationError::ArithmeticOverflow)
    }

    #[must_use]
    pub fn pixels(&self) -> impl ExactSizeIterator<Item = &[f32; 4]> {
        self.pixels.iter().map(AlignedRgba::as_array)
    }

    pub fn pixel(&self, x: u32, y: u32) -> Option<&[f32; 4]> {
        let index = self.index(x, y)?;
        self.pixels.get(index).map(AlignedRgba::as_array)
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, value: [f32; 4]) -> bool {
        let Some(index) = self.index(x, y) else {
            return false;
        };
        self.pixels[index] = AlignedRgba(value);
        true
    }

    fn index(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.dimensions.width() || y >= self.dimensions.height() {
            return None;
        }
        let index = u64::from(y)
            .checked_mul(u64::from(self.dimensions.width()))?
            .checked_add(u64::from(x))?;
        usize::try_from(index).ok()
    }
}

pub trait BufferPool: Send + Sync {
    /// Allocates a canonical processing buffer for checked dimensions.
    ///
    /// # Errors
    ///
    /// Returns an error when the dimensions or allocation are unsupported.
    fn allocate_canonical(
        &self,
        dimensions: ImageDimensions,
    ) -> Result<CanonicalRgbaBuffer, BufferAllocationError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultBufferPool;

impl BufferPool for DefaultBufferPool {
    fn allocate_canonical(
        &self,
        dimensions: ImageDimensions,
    ) -> Result<CanonicalRgbaBuffer, BufferAllocationError> {
        CanonicalRgbaBuffer::new(dimensions)
    }
}

impl fmt::Display for BufferAllocationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ArithmeticOverflow => "canonical buffer arithmetic overflowed",
            Self::CapacityOverflow => "canonical buffer capacity is unsupported",
        })
    }
}

impl std::error::Error for BufferAllocationError {}

const _: () = assert!(Layout::new::<AlignedRgba>().align() >= 64);
