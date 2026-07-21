use crate::{Provider, ProviderPolicy};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GamutPolicy {
    Disabled,
    Preserve { margin: f32 },
}

impl GamutPolicy {
    #[must_use]
    pub const fn enabled(self) -> bool {
        matches!(self, Self::Preserve { .. })
    }

    #[must_use]
    pub const fn margin(self) -> f32 {
        match self {
            Self::Disabled => 0.01,
            Self::Preserve { margin } => margin,
        }
    }

    pub fn validate(self) -> Result<(), PolicyError> {
        if !self.margin().is_finite() || !(0.0..=0.5).contains(&self.margin()) {
            return Err(PolicyError::InvalidGamutMargin);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowDecision {
    enabled: bool,
    sampled_pixels: u64,
    deep_shadow_pixels: u64,
}

impl ShadowDecision {
    #[must_use]
    pub const fn new(enabled: bool, sampled_pixels: u64, deep_shadow_pixels: u64) -> Self {
        Self {
            enabled,
            sampled_pixels,
            deep_shadow_pixels,
        }
    }
    #[must_use]
    pub const fn enabled(self) -> bool {
        self.enabled
    }
    #[must_use]
    pub const fn sampled_pixels(self) -> u64 {
        self.sampled_pixels
    }
    #[must_use]
    pub const fn deep_shadow_pixels(self) -> u64 {
        self.deep_shadow_pixels
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileReceipt {
    pub count: u64,
    pub tile_width: u32,
    pub tile_height: u32,
    pub overlap: u32,
    pub scale: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryBudget {
    pub max_total_bytes: u64,
    pub max_tile_bytes: u64,
    pub max_concurrency: u32,
}

impl Default for MemoryBudget {
    fn default() -> Self {
        Self {
            max_total_bytes: 1024 * 1024 * 1024,
            max_tile_bytes: 256 * 1024 * 1024,
            max_concurrency: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryReceipt {
    pub source_bytes: u64,
    pub mask_bytes: u64,
    pub tile_input_bytes: u64,
    pub tile_output_bytes: u64,
    pub row_buffer_bytes: u64,
    pub model_output_bytes: u64,
    pub detail_bytes: u64,
    pub session_bytes: u64,
    pub concurrency: u32,
    pub total_bytes: u64,
}

impl MemoryReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn checked(
        source_bytes: u64,
        mask_bytes: u64,
        tile_input_bytes: u64,
        tile_output_bytes: u64,
        row_buffer_bytes: u64,
        model_output_bytes: u64,
        detail_bytes: u64,
        session_bytes: u64,
        budget: MemoryBudget,
    ) -> Result<Self, PolicyError> {
        if budget.max_concurrency == 0 {
            return Err(PolicyError::InvalidConcurrency);
        }
        let tile_bytes = tile_input_bytes
            .checked_add(tile_output_bytes)
            .and_then(|value| value.checked_add(model_output_bytes))
            .ok_or(PolicyError::ArithmeticOverflow)?;
        if tile_bytes > budget.max_tile_bytes {
            return Err(PolicyError::TileMemoryLimit);
        }
        let fixed = source_bytes
            .checked_add(mask_bytes)
            .and_then(|value| value.checked_add(row_buffer_bytes))
            .and_then(|value| value.checked_add(detail_bytes))
            .and_then(|value| value.checked_add(session_bytes))
            .ok_or(PolicyError::ArithmeticOverflow)?;
        let total = fixed
            .checked_add(
                tile_bytes
                    .checked_mul(u64::from(budget.max_concurrency))
                    .ok_or(PolicyError::ArithmeticOverflow)?,
            )
            .ok_or(PolicyError::ArithmeticOverflow)?;
        if total > budget.max_total_bytes {
            return Err(PolicyError::MemoryLimit {
                estimated: total,
                limit: budget.max_total_bytes,
            });
        }
        Ok(Self {
            source_bytes,
            mask_bytes,
            tile_input_bytes,
            tile_output_bytes,
            row_buffer_bytes,
            model_output_bytes,
            detail_bytes,
            session_bytes,
            concurrency: budget.max_concurrency,
            total_bytes: total,
        })
    }
}

pub(crate) fn selected_provider(policy: ProviderPolicy, providers: &[Provider]) -> Provider {
    match policy {
        ProviderPolicy::Cpu => Provider::Cpu,
        ProviderPolicy::Explicit(provider) => provider,
        ProviderPolicy::Auto => providers
            .iter()
            .copied()
            .find(|provider| *provider != Provider::Cpu)
            .unwrap_or(Provider::Cpu),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyError {
    InvalidGamutMargin,
    UnsupportedScaleGamut,
    InvalidShadowThreshold,
    UnsupportedScaleDetail,
    ProviderUnavailable,
    InvalidConcurrency,
    TileMemoryLimit,
    MemoryLimit { estimated: u64, limit: u64 },
    ArithmeticOverflow,
}

impl std::fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "RGB AI policy error: {self:?}")
    }
}
impl std::error::Error for PolicyError {}
