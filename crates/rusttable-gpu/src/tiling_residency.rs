use super::ResidencyError;
use crate::transfer::{HostTransferDescriptor, TransferRegion};
use crate::{DeviceGeneration, ResourceClass};
use rusttable_color::ColorEncoding;
use rusttable_image::{ImageDimensions, PixelFormat, Roi};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResidentIntermediate {
    pub node: u32,
    pub roi: Roi,
    pub class: ResourceClass,
    pub bytes: u64,
    pub lifetime: Lifetime,
    pub compatibility: u64,
}

impl ResidentIntermediate {
    pub const fn new(
        node: u32,
        roi: Roi,
        class: ResourceClass,
        bytes: u64,
        lifetime: Lifetime,
        compatibility: u64,
    ) -> Result<Self, ResidencyError> {
        if bytes == 0 {
            return Err(ResidencyError::ZeroBytes);
        }
        if lifetime.start >= lifetime.end {
            return Err(ResidencyError::InvalidLifetime);
        }
        if roi.is_empty() {
            return Err(ResidencyError::EmptyRoi);
        }
        Ok(Self {
            node,
            roi,
            class,
            bytes,
            lifetime,
            compatibility,
        })
    }
    #[must_use]
    pub fn compatible_with(&self, next: &Self) -> bool {
        self.lifetime.end == next.lifetime.start
            && self.class.generation == next.class.generation
            && self.class.kind == next.class.kind
            && self.class.format == next.class.format
            && self.class.usage == next.class.usage
            && self.compatibility == next.compatibility
            && self.roi == next.roi
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Lifetime {
    pub start: u32,
    pub end: u32,
}

impl Lifetime {
    pub const fn new(start: u32, end: u32) -> Result<Self, ResidencyError> {
        if start >= end {
            return Err(ResidencyError::InvalidLifetime);
        }
        Ok(Self { start, end })
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostBoundary {
    pub from_node: u32,
    pub to_node: u32,
    pub descriptor: HostTransferDescriptor,
}

impl HostBoundary {
    pub fn new(
        from_node: u32,
        to_node: u32,
        dimensions: ImageDimensions,
        region: TransferRegion,
        row_stride: u64,
        pixel_format: PixelFormat,
        byte_length: u64,
    ) -> Result<Self, ResidencyError> {
        if from_node >= to_node {
            return Err(ResidencyError::InvalidHostBoundary);
        }
        let pixel_bytes = u64::try_from(pixel_format.bytes_per_pixel())
            .map_err(|_| ResidencyError::ArithmeticOverflow)?;
        let required_bytes = u64::from(region.y)
            .checked_add(u64::from(region.height))
            .and_then(|rows| rows.checked_sub(1))
            .and_then(|last_row| last_row.checked_mul(row_stride))
            .and_then(|offset| offset.checked_add(u64::from(region.x).checked_mul(pixel_bytes)?))
            .and_then(|offset| {
                offset.checked_add(u64::from(region.width).checked_mul(pixel_bytes)?)
            })
            .ok_or(ResidencyError::ArithmeticOverflow)?;
        if byte_length < required_bytes {
            return Err(ResidencyError::Transfer(
                crate::transfer::TransferError::SourceLengthMismatch,
            ));
        }
        let descriptor =
            HostTransferDescriptor::new(dimensions, region, row_stride, pixel_format, byte_length)?
                .with_color_encoding(ColorEncoding::Unspecified);
        Ok(Self {
            from_node,
            to_node,
            descriptor,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResidencyPlan {
    generation: DeviceGeneration,
    budget_bytes: u64,
    intermediates: Vec<ResidentIntermediate>,
    host_boundaries: Vec<HostBoundary>,
}

impl ResidencyPlan {
    #[must_use]
    pub const fn new(generation: DeviceGeneration, budget_bytes: u64) -> Self {
        Self {
            generation,
            budget_bytes,
            intermediates: Vec::new(),
            host_boundaries: Vec::new(),
        }
    }

    pub fn push_intermediate(
        &mut self,
        intermediate: ResidentIntermediate,
    ) -> Result<(), ResidencyError> {
        if intermediate.class.generation != self.generation {
            return Err(ResidencyError::GenerationMismatch);
        }
        let end = self
            .resident_bytes()
            .checked_add(intermediate.bytes)
            .ok_or(ResidencyError::ArithmeticOverflow)?;
        if end > self.budget_bytes {
            return Err(ResidencyError::BudgetExceeded {
                required: end,
                budget: self.budget_bytes,
            });
        }
        self.intermediates.push(intermediate);
        Ok(())
    }

    pub fn push_host_boundary(&mut self, boundary: HostBoundary) -> Result<(), ResidencyError> {
        if self.host_boundaries.iter().any(|existing| {
            existing.from_node == boundary.from_node && existing.to_node == boundary.to_node
        }) {
            return Err(ResidencyError::DuplicateHostBoundary);
        }
        self.host_boundaries.push(boundary);
        self.host_boundaries
            .sort_by_key(|value| (value.from_node, value.to_node));
        Ok(())
    }

    #[must_use]
    pub const fn generation(&self) -> DeviceGeneration {
        self.generation
    }

    #[must_use]
    pub fn resident_bytes(&self) -> u64 {
        self.intermediates.iter().map(|value| value.bytes).sum()
    }

    #[must_use]
    pub fn intermediates(&self) -> &[ResidentIntermediate] {
        &self.intermediates
    }

    #[must_use]
    pub fn host_boundaries(&self) -> &[HostBoundary] {
        &self.host_boundaries
    }
    pub fn validate(&self) -> Result<(), ResidencyError> {
        let bytes = self.resident_bytes();
        if bytes > self.budget_bytes {
            return Err(ResidencyError::BudgetExceeded {
                required: bytes,
                budget: self.budget_bytes,
            });
        }
        if self
            .intermediates
            .iter()
            .any(|value| value.class.generation != self.generation)
        {
            return Err(ResidencyError::GenerationMismatch);
        }
        Ok(())
    }
}
