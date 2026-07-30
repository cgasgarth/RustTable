use super::TilingError;
use crate::{InitializationPolicy, ResourceClass};
const DEFAULT_ALIGNMENT: u32 = 1;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct EdgeOverlap {
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
}
impl EdgeOverlap {
    #[must_use]
    pub const fn new(left: u32, top: u32, right: u32, bottom: u32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }
    #[must_use]
    pub const fn uniform(value: u32) -> Self {
        Self::new(value, value, value, value)
    }
    #[must_use]
    pub const fn left(self) -> u32 {
        self.left
    }

    #[must_use]
    pub const fn top(self) -> u32 {
        self.top
    }

    #[must_use]
    pub const fn right(self) -> u32 {
        self.right
    }

    #[must_use]
    pub const fn bottom(self) -> u32 {
        self.bottom
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileAlignment {
    origin_x: u32,
    origin_y: u32,
    extent_x: u32,
    extent_y: u32,
}

impl TileAlignment {
    pub const fn new(
        origin_x: u32,
        origin_y: u32,
        extent_x: u32,
        extent_y: u32,
    ) -> Result<Self, TilingError> {
        if origin_x == 0 || origin_y == 0 || extent_x == 0 || extent_y == 0 {
            return Err(TilingError::ZeroAlignment);
        }
        Ok(Self {
            origin_x,
            origin_y,
            extent_x,
            extent_y,
        })
    }
    #[must_use]
    pub const fn unit() -> Self {
        Self {
            origin_x: DEFAULT_ALIGNMENT,
            origin_y: DEFAULT_ALIGNMENT,
            extent_x: DEFAULT_ALIGNMENT,
            extent_y: DEFAULT_ALIGNMENT,
        }
    }

    #[must_use]
    pub const fn origin_x(self) -> u32 {
        self.origin_x
    }

    #[must_use]
    pub const fn origin_y(self) -> u32 {
        self.origin_y
    }

    #[must_use]
    pub const fn extent_x(self) -> u32 {
        self.extent_x
    }

    #[must_use]
    pub const fn extent_y(self) -> u32 {
        self.extent_y
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileMemoryBudget {
    hard_bytes: u64,
    reserve_bytes: u64,
    max_allocation_bytes: u64,
    backend_overhead: u64,
}

impl TileMemoryBudget {
    pub const fn new(
        hard_bytes: u64,
        reserve_bytes: u64,
        max_allocation_bytes: u64,
        backend_overhead: u64,
    ) -> Result<Self, TilingError> {
        if hard_bytes == 0 || max_allocation_bytes == 0 {
            return Err(TilingError::ZeroBudget);
        }
        if reserve_bytes > hard_bytes {
            return Err(TilingError::ReserveExceedsBudget);
        }
        Ok(Self {
            hard_bytes,
            reserve_bytes,
            max_allocation_bytes,
            backend_overhead,
        })
    }

    #[must_use]
    pub const fn hard_bytes(self) -> u64 {
        self.hard_bytes
    }

    #[must_use]
    pub const fn reserve_bytes(self) -> u64 {
        self.reserve_bytes
    }

    #[must_use]
    pub const fn max_allocation_bytes(self) -> u64 {
        self.max_allocation_bytes
    }

    #[must_use]
    pub const fn backend_overhead(self) -> u64 {
        self.backend_overhead
    }

    #[must_use]
    pub const fn available_bytes(self) -> u64 {
        self.hard_bytes - self.reserve_bytes
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TileArea {
    Input,
    Output,
    Larger,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TileResourceSpec {
    pub name: String,
    pub class: ResourceClass,
    pub area: TileArea,
    pub bytes_per_pixel: u64,
    pub fixed_bytes: u64,
    pub initialization: InitializationPolicy,
    pub resident: bool,
}

impl TileResourceSpec {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: impl Into<String>,
        class: ResourceClass,
        area: TileArea,
        bytes_per_pixel: u64,
    ) -> Result<Self, TilingError> {
        if bytes_per_pixel == 0 {
            return Err(TilingError::ZeroBytesPerPixel);
        }
        Ok(Self {
            name: name.into(),
            class,
            area,
            bytes_per_pixel,
            fixed_bytes: 0,
            initialization: InitializationPolicy::FullOverwrite,
            resident: false,
        })
    }

    #[must_use]
    pub const fn with_fixed_bytes(mut self, fixed_bytes: u64) -> Self {
        self.fixed_bytes = fixed_bytes;
        self
    }

    #[must_use]
    pub const fn with_initialization(mut self, initialization: InitializationPolicy) -> Self {
        self.initialization = initialization;
        self
    }

    #[must_use]
    pub const fn with_residency(mut self, resident: bool) -> Self {
        self.resident = resident;
        self
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileMemoryEstimate {
    pub tile_bytes: u64,
    pub resident_bytes: u64,
    pub reserve_bytes: u64,
    pub required_bytes: u64,
    pub available_bytes: u64,
    pub allocations: Vec<ResourceAllocationEstimate>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceAllocationEstimate {
    pub name: String,
    pub bytes: u64,
    pub class: ResourceClass,
    pub resident: bool,
}
