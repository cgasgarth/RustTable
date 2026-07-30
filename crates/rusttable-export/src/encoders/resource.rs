#![allow(clippy::missing_errors_doc)]

use std::fmt;

use rusttable_pixelpipe::{CancellationToken, ResourceClaim};

const DEFAULT_MEMORY_BYTES: u64 = 512 * 1024 * 1024;
const DEFAULT_THREADS: u16 = 1;
const MAX_THREADS: u16 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncodeBudget {
    memory_bytes: u64,
    worker_tokens: u16,
    max_parallelism: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetError {
    ZeroMemory,
    ZeroWorkers,
    ParallelismExceedsWorkers,
    ParallelismExceedsCodecLimit,
}

impl fmt::Display for BudgetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ZeroMemory => "encode budget memory is zero",
            Self::ZeroWorkers => "encode budget has no worker tokens",
            Self::ParallelismExceedsWorkers => "encode parallelism exceeds worker tokens",
            Self::ParallelismExceedsCodecLimit => "encode parallelism exceeds codec limit",
        })
    }
}

impl std::error::Error for BudgetError {}

impl EncodeBudget {
    /// Creates the bounded reservation supplied by the export queue.
    pub fn new(
        memory_bytes: u64,
        worker_tokens: u16,
        max_parallelism: u16,
    ) -> Result<Self, BudgetError> {
        if memory_bytes == 0 {
            return Err(BudgetError::ZeroMemory);
        }
        if worker_tokens == 0 {
            return Err(BudgetError::ZeroWorkers);
        }
        if max_parallelism > worker_tokens {
            return Err(BudgetError::ParallelismExceedsWorkers);
        }
        if max_parallelism > MAX_THREADS {
            return Err(BudgetError::ParallelismExceedsCodecLimit);
        }
        Ok(Self {
            memory_bytes,
            worker_tokens,
            max_parallelism,
        })
    }

    pub fn from_resource_claim(claim: &ResourceClaim) -> Result<Self, BudgetError> {
        Self::new(
            claim.memory_bytes(),
            claim.worker_tokens(),
            claim.max_parallelism(),
        )
    }

    #[must_use]
    pub const fn default_single_thread() -> Self {
        Self {
            memory_bytes: DEFAULT_MEMORY_BYTES,
            worker_tokens: DEFAULT_THREADS,
            max_parallelism: DEFAULT_THREADS,
        }
    }

    #[must_use]
    pub const fn memory_bytes(self) -> u64 {
        self.memory_bytes
    }

    #[must_use]
    pub const fn worker_tokens(self) -> u16 {
        self.worker_tokens
    }

    #[must_use]
    pub const fn max_parallelism(self) -> u16 {
        self.max_parallelism
    }

    #[must_use]
    pub const fn threads(self) -> u16 {
        if self.max_parallelism < self.worker_tokens {
            self.max_parallelism
        } else {
            self.worker_tokens
        }
    }
}

impl Default for EncodeBudget {
    fn default() -> Self {
        Self::default_single_thread()
    }
}

pub trait EncodeCancellation {
    fn is_cancelled(&self) -> bool;
}

impl EncodeCancellation for CancellationToken {
    fn is_cancelled(&self) -> bool {
        Self::is_cancelled(self)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NeverCancel;

impl EncodeCancellation for NeverCancel {
    fn is_cancelled(&self) -> bool {
        false
    }
}

pub(crate) fn checked_memory(
    dimensions: rusttable_image::ImageDimensions,
    channels: usize,
    sample_bytes: usize,
    budget: EncodeBudget,
) -> Result<(), &'static str> {
    let required = u64::from(dimensions.width())
        .checked_mul(u64::from(dimensions.height()))
        .and_then(|value| value.checked_mul(u64::try_from(channels).ok()?))
        .and_then(|value| value.checked_mul(u64::try_from(sample_bytes).ok()?))
        .ok_or("pixel allocation overflow")?;
    if required > budget.memory_bytes() {
        return Err("pixel allocation exceeds queue reservation");
    }
    Ok(())
}
