use std::collections::BTreeSet;
use std::fmt;

use crate::contracts::TileContract;
use crate::{ImageDimensions, ModelIdentity, Provider, ProviderPolicy};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InferenceLimits {
    pub max_output_bytes: u64,
    pub max_tile_bytes: u64,
    pub max_session_bytes: u64,
    pub max_concurrency: u32,
}

impl Default for InferenceLimits {
    fn default() -> Self {
        Self {
            max_output_bytes: 512 * 1024 * 1024,
            max_tile_bytes: 256 * 1024 * 1024,
            max_session_bytes: 512 * 1024 * 1024,
            max_concurrency: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InferenceTile {
    input_x: u32,
    input_y: u32,
    input_width: u32,
    input_height: u32,
    output_x: u32,
    output_y: u32,
    output_width: u32,
    output_height: u32,
    padded: bool,
}

impl InferenceTile {
    #[must_use]
    pub const fn input_origin(self) -> (u32, u32) {
        (self.input_x, self.input_y)
    }
    #[must_use]
    pub const fn input_dimensions(self) -> (u32, u32) {
        (self.input_width, self.input_height)
    }
    #[must_use]
    pub const fn output_origin(self) -> (u32, u32) {
        (self.output_x, self.output_y)
    }
    #[must_use]
    pub const fn output_dimensions(self) -> (u32, u32) {
        (self.output_width, self.output_height)
    }
    #[must_use]
    pub const fn padded(self) -> bool {
        self.padded
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InferencePlan {
    model: ModelIdentity,
    provider: Provider,
    source: ImageDimensions,
    output: ImageDimensions,
    tiles: Vec<InferenceTile>,
    estimated_bytes: u64,
}

impl InferencePlan {
    pub fn build(
        model: ModelIdentity,
        source: ImageDimensions,
        tile: &TileContract,
        provider_policy: ProviderPolicy,
        limits: InferenceLimits,
        channels: u32,
        session_bytes: u64,
    ) -> Result<Self, PlanningError> {
        let provider = provider_policy.selected();
        if source.width() < tile.minimum_width || source.height() < tile.minimum_height {
            return Err(PlanningError::ImageTooSmall);
        }
        validate_tile(tile)?;
        if channels == 0 {
            return Err(PlanningError::InvalidChannels);
        }
        if limits.max_concurrency == 0 {
            return Err(PlanningError::ConcurrencyLimit);
        }
        let output_width = source
            .width()
            .checked_mul(tile.scale)
            .ok_or(PlanningError::ArithmeticOverflow)?;
        let output_height = source
            .height()
            .checked_mul(tile.scale)
            .ok_or(PlanningError::ArithmeticOverflow)?;
        let output = ImageDimensions::new(output_width, output_height)
            .map_err(|_| PlanningError::ArithmeticOverflow)?;
        let core_width = tile
            .width
            .checked_sub(
                tile.valid_crop
                    .left
                    .checked_add(tile.valid_crop.right)
                    .ok_or(PlanningError::ArithmeticOverflow)?,
            )
            .ok_or(PlanningError::InvalidTile)?;
        let core_height = tile
            .height
            .checked_sub(
                tile.valid_crop
                    .top
                    .checked_add(tile.valid_crop.bottom)
                    .ok_or(PlanningError::ArithmeticOverflow)?,
            )
            .ok_or(PlanningError::InvalidTile)?;
        let mut tiles = Vec::new();
        let mut seen = BTreeSet::new();
        let mut y = 0_u32;
        while y < source.height() {
            let mut x = 0_u32;
            while x < source.width() {
                let input_x = x.saturating_sub(tile.valid_crop.left);
                let input_y = y.saturating_sub(tile.valid_crop.top);
                let input_x = input_x.min(source.width().saturating_sub(tile.width));
                let input_y = input_y.min(source.height().saturating_sub(tile.height));
                if seen.insert((input_x, input_y)) {
                    let output_width = core_width
                        .min(source.width() - x)
                        .checked_mul(tile.scale)
                        .ok_or(PlanningError::ArithmeticOverflow)?;
                    let output_height = core_height
                        .min(source.height() - y)
                        .checked_mul(tile.scale)
                        .ok_or(PlanningError::ArithmeticOverflow)?;
                    tiles.push(InferenceTile {
                        input_x,
                        input_y,
                        input_width: tile.width,
                        input_height: tile.height,
                        output_x: x
                            .checked_mul(tile.scale)
                            .ok_or(PlanningError::ArithmeticOverflow)?,
                        output_y: y
                            .checked_mul(tile.scale)
                            .ok_or(PlanningError::ArithmeticOverflow)?,
                        output_width,
                        output_height,
                        padded: source.width() < tile.width
                            || source.height() < tile.height
                            || input_x + tile.width > source.width()
                            || input_y + tile.height > source.height(),
                    });
                }
                x = x
                    .checked_add(core_width)
                    .ok_or(PlanningError::ArithmeticOverflow)?;
            }
            y = y
                .checked_add(core_height)
                .ok_or(PlanningError::ArithmeticOverflow)?;
        }
        let output_bytes = u64::from(output_width)
            .checked_mul(u64::from(output_height))
            .and_then(|pixels| pixels.checked_mul(u64::from(channels)))
            .and_then(|bytes| bytes.checked_mul(4))
            .ok_or(PlanningError::ArithmeticOverflow)?;
        let tile_bytes = u64::from(tile.width)
            .checked_mul(u64::from(tile.height))
            .and_then(|pixels| pixels.checked_mul(u64::from(channels)))
            .and_then(|bytes| bytes.checked_mul(4))
            .ok_or(PlanningError::ArithmeticOverflow)?;
        let estimated = output_bytes
            .checked_add(
                tile_bytes
                    .checked_mul(2)
                    .ok_or(PlanningError::ArithmeticOverflow)?,
            )
            .and_then(|value| value.checked_add(session_bytes))
            .ok_or(PlanningError::ArithmeticOverflow)?;
        let limit = limits
            .max_output_bytes
            .checked_add(limits.max_tile_bytes)
            .and_then(|value| value.checked_add(limits.max_session_bytes))
            .ok_or(PlanningError::ArithmeticOverflow)?;
        if output_bytes > limits.max_output_bytes
            || tile_bytes > limits.max_tile_bytes
            || session_bytes > limits.max_session_bytes
            || estimated > limit
        {
            return Err(PlanningError::MemoryLimit { estimated, limit });
        }
        Ok(Self {
            model,
            provider,
            source,
            output,
            tiles,
            estimated_bytes: estimated,
        })
    }

    #[must_use]
    pub const fn model(&self) -> ModelIdentity {
        self.model
    }
    #[must_use]
    pub const fn provider(&self) -> Provider {
        self.provider
    }
    #[must_use]
    pub const fn source(&self) -> ImageDimensions {
        self.source
    }
    #[must_use]
    pub const fn output(&self) -> ImageDimensions {
        self.output
    }
    #[must_use]
    pub fn tiles(&self) -> &[InferenceTile] {
        &self.tiles
    }
    #[must_use]
    pub const fn estimated_bytes(&self) -> u64 {
        self.estimated_bytes
    }
}

fn validate_tile(tile: &TileContract) -> Result<(), PlanningError> {
    if tile.width == 0
        || tile.height == 0
        || tile.alignment == 0
        || !tile.width.is_multiple_of(tile.alignment)
        || !tile.height.is_multiple_of(tile.alignment)
        || tile.overlap >= tile.width.min(tile.height)
    {
        return Err(PlanningError::InvalidTile);
    }
    let crop_x = tile
        .valid_crop
        .left
        .checked_add(tile.valid_crop.right)
        .ok_or(PlanningError::ArithmeticOverflow)?;
    let crop_y = tile
        .valid_crop
        .top
        .checked_add(tile.valid_crop.bottom)
        .ok_or(PlanningError::ArithmeticOverflow)?;
    if crop_x >= tile.width || crop_y >= tile.height {
        return Err(PlanningError::InvalidTile);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningError {
    ImageTooSmall,
    InvalidChannels,
    InvalidTile,
    ConcurrencyLimit,
    ArithmeticOverflow,
    MemoryLimit { estimated: u64, limit: u64 },
}

impl fmt::Display for PlanningError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid bounded inference plan: {self:?}")
    }
}

impl std::error::Error for PlanningError {}
