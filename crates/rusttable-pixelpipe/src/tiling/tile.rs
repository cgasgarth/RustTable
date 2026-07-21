use std::{fmt, num::NonZeroU32};

use rusttable_processing::RasterDimensions;

/// A non-overlapping rectangular region within a raster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CpuPixelpipeTile {
    origin_x: u32,
    origin_y: u32,
    dimensions: RasterDimensions,
}

impl CpuPixelpipeTile {
    const fn new(origin_x: u32, origin_y: u32, dimensions: RasterDimensions) -> Self {
        Self {
            origin_x,
            origin_y,
            dimensions,
        }
    }

    /// Horizontal source-pixel coordinate of this tile's top-left corner.
    #[must_use]
    pub const fn origin_x(self) -> u32 {
        self.origin_x
    }

    /// Vertical source-pixel coordinate of this tile's top-left corner.
    #[must_use]
    pub const fn origin_y(self) -> u32 {
        self.origin_y
    }

    /// Exact dimensions of this tile, including any right or bottom edge tile.
    #[must_use]
    pub const fn dimensions(self) -> RasterDimensions {
        self.dimensions
    }
}

/// Checked, deterministic dimensions for scalar CPU pixelpipe tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CpuTilePlan {
    tile_width: NonZeroU32,
    tile_height: NonZeroU32,
}

impl CpuTilePlan {
    /// Creates a tile plan with nonzero horizontal and vertical extents.
    ///
    /// # Errors
    ///
    /// Returns a typed error when either tile dimension is zero.
    pub const fn new(tile_width: u32, tile_height: u32) -> Result<Self, CpuTilePlanError> {
        let Some(tile_width) = NonZeroU32::new(tile_width) else {
            return Err(CpuTilePlanError::ZeroTileWidth);
        };
        let Some(tile_height) = NonZeroU32::new(tile_height) else {
            return Err(CpuTilePlanError::ZeroTileHeight);
        };
        Ok(Self {
            tile_width,
            tile_height,
        })
    }

    /// Requested tile width in pixels.
    #[must_use]
    pub const fn tile_width(self) -> u32 {
        self.tile_width.get()
    }

    /// Requested tile height in pixels.
    #[must_use]
    pub const fn tile_height(self) -> u32 {
        self.tile_height.get()
    }

    /// Creates a deterministic row-major tile grid for one raster.
    ///
    /// Every grid tile is nonempty, in bounds, and appears exactly once. Right
    /// and bottom edge tiles are reduced to the remaining source extent.
    ///
    /// # Errors
    ///
    /// Returns a typed error if the tile grid cannot be represented on this
    /// platform.
    pub fn grid_for(self, dimensions: RasterDimensions) -> Result<CpuTileGrid, CpuTilePlanError> {
        let columns = tile_count_for_axis(dimensions.width(), self.tile_width())?;
        let rows = tile_count_for_axis(dimensions.height(), self.tile_height())?;
        let tile_count = u64::from(columns)
            .checked_mul(u64::from(rows))
            .ok_or(CpuTilePlanError::TileCountOverflow)?;
        Ok(CpuTileGrid {
            plan: self,
            dimensions,
            columns,
            rows,
            tile_count,
        })
    }
}

/// One checked, lazily-addressable row-major CPU tile grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CpuTileGrid {
    plan: CpuTilePlan,
    dimensions: RasterDimensions,
    columns: u32,
    rows: u32,
    tile_count: u64,
}

impl CpuTileGrid {
    /// Number of tiles in this grid.
    #[must_use]
    pub const fn tile_count(self) -> u64 {
        self.tile_count
    }

    /// Number of horizontal tiles.
    #[must_use]
    pub const fn columns(self) -> u32 {
        self.columns
    }

    /// Number of vertical tiles.
    #[must_use]
    pub const fn rows(self) -> u32 {
        self.rows
    }

    /// Returns one tile by its row-major index.
    ///
    /// # Errors
    ///
    /// Returns a typed error if checked tile-coordinate arithmetic cannot be
    /// represented. An index outside the grid returns `Ok(None)`.
    pub fn tile_at(self, index: u64) -> Result<Option<CpuPixelpipeTile>, CpuTilePlanError> {
        if index >= self.tile_count {
            return Ok(None);
        }
        let columns = u64::from(self.columns);
        let row = index / columns;
        let column = index % columns;
        let origin_x = checked_origin(column, self.plan.tile_width())?;
        let origin_y = checked_origin(row, self.plan.tile_height())?;
        let tile_width = self
            .dimensions
            .width()
            .checked_sub(origin_x)
            .ok_or(CpuTilePlanError::TileCoordinateOverflow)?
            .min(self.plan.tile_width());
        let tile_height = self
            .dimensions
            .height()
            .checked_sub(origin_y)
            .ok_or(CpuTilePlanError::TileCoordinateOverflow)?
            .min(self.plan.tile_height());
        let dimensions = RasterDimensions::new(tile_width, tile_height)
            .map_err(|_| CpuTilePlanError::TileCoordinateOverflow)?;
        Ok(Some(CpuPixelpipeTile::new(origin_x, origin_y, dimensions)))
    }
}

fn checked_origin(index: u64, tile_length: u32) -> Result<u32, CpuTilePlanError> {
    let origin = index
        .checked_mul(u64::from(tile_length))
        .ok_or(CpuTilePlanError::TileCoordinateOverflow)?;
    u32::try_from(origin).map_err(|_| CpuTilePlanError::TileCoordinateOverflow)
}

fn tile_count_for_axis(length: u32, tile_length: u32) -> Result<u32, CpuTilePlanError> {
    let whole_tiles = length / tile_length;
    let has_remainder = u32::from(!length.is_multiple_of(tile_length));
    whole_tiles
        .checked_add(has_remainder)
        .ok_or(CpuTilePlanError::TileCountOverflow)
}

/// Rejection reason for a checked CPU tile plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuTilePlanError {
    ZeroTileWidth,
    ZeroTileHeight,
    TileCountOverflow,
    TileCoordinateOverflow,
}

impl fmt::Display for CpuTilePlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroTileWidth => formatter.write_str("CPU tile width must be nonzero"),
            Self::ZeroTileHeight => formatter.write_str("CPU tile height must be nonzero"),
            Self::TileCountOverflow => formatter.write_str("CPU tile grid count overflowed"),
            Self::TileCoordinateOverflow => {
                formatter.write_str("CPU tile coordinate calculation overflowed")
            }
        }
    }
}

impl std::error::Error for CpuTilePlanError {}
