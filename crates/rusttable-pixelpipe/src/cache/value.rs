#![allow(clippy::missing_errors_doc)]

use std::any::Any;
use std::sync::Arc;

use crate::RgbaF32Image;
use crate::cache::CacheError;

/// Creation cost helps diagnostics explain why a result was retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CreationCost {
    Cheap,
    Normal,
    Expensive,
}

/// The typed value family retained by the in-memory cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueKind {
    Image,
    Analysis,
    Plan,
    Custom,
}

/// Immutable accounting metadata supplied with every cache value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ValueDescriptor {
    kind: ValueKind,
    resident_bytes: u64,
    auxiliary_bytes: u64,
    creation_cost: CreationCost,
    cacheable: bool,
}

impl ValueDescriptor {
    #[must_use]
    pub const fn new(
        kind: ValueKind,
        resident_bytes: u64,
        auxiliary_bytes: u64,
        creation_cost: CreationCost,
        cacheable: bool,
    ) -> Self {
        Self {
            kind,
            resident_bytes,
            auxiliary_bytes,
            creation_cost,
            cacheable,
        }
    }

    #[must_use]
    pub const fn kind(self) -> ValueKind {
        self.kind
    }
    #[must_use]
    pub const fn resident_bytes(self) -> u64 {
        self.resident_bytes
    }
    #[must_use]
    pub const fn auxiliary_bytes(self) -> u64 {
        self.auxiliary_bytes
    }
    #[must_use]
    pub const fn creation_cost(self) -> CreationCost {
        self.creation_cost
    }
    #[must_use]
    pub const fn cacheable(self) -> bool {
        self.cacheable
    }

    pub fn total_bytes(self) -> Result<u64, CacheError> {
        self.resident_bytes
            .checked_add(self.auxiliary_bytes)
            .ok_or(CacheError::CostOverflow)
    }
}

/// Validation and accounting contract for values that may be published.
pub trait CacheValue: Any + Send + Sync + 'static {
    fn descriptor(&self) -> ValueDescriptor;
    fn validate(&self) -> Result<(), String>;
}

impl CacheValue for RgbaF32Image {
    fn descriptor(&self) -> ValueDescriptor {
        let bytes = self
            .descriptor()
            .dimensions()
            .pixel_count()
            .saturating_mul(16);
        ValueDescriptor::new(ValueKind::Image, bytes, 0, CreationCost::Expensive, true)
    }

    fn validate(&self) -> Result<(), String> {
        if self.pixels().iter().all(|pixel| {
            pixel.red().is_finite()
                && pixel.green().is_finite()
                && pixel.blue().is_finite()
                && pixel.alpha().is_finite()
        }) {
            Ok(())
        } else {
            Err("image contains a non-finite component".to_owned())
        }
    }
}

/// Bounded immutable analysis data that can be cached without retaining a
/// catalog, UI, file, or device handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisValue {
    bytes: Arc<[u8]>,
}

impl AnalysisValue {
    #[must_use]
    pub fn new(bytes: impl Into<Arc<[u8]>>) -> Self {
        Self {
            bytes: bytes.into(),
        }
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl CacheValue for AnalysisValue {
    fn descriptor(&self) -> ValueDescriptor {
        ValueDescriptor::new(
            ValueKind::Analysis,
            u64::try_from(self.bytes.len()).unwrap_or(u64::MAX),
            0,
            CreationCost::Normal,
            true,
        )
    }

    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

/// Bounded immutable plan data for later render/GPU integration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanValue {
    bytes: Arc<[u8]>,
}

impl PlanValue {
    #[must_use]
    pub fn new(bytes: impl Into<Arc<[u8]>>) -> Self {
        Self {
            bytes: bytes.into(),
        }
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl CacheValue for PlanValue {
    fn descriptor(&self) -> ValueDescriptor {
        ValueDescriptor::new(
            ValueKind::Plan,
            u64::try_from(self.bytes.len()).unwrap_or(u64::MAX),
            0,
            CreationCost::Cheap,
            true,
        )
    }

    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

pub use crate::cancellation::CancellationToken;
