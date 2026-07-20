use sha2::{Digest, Sha256};

use rusttable_core::OperationId;

use crate::{CpuPixelpipeOutputMode, RgbaF32Descriptor};

/// Identifies the deterministic CPU implementation that produced a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CpuImplementation {
    ScalarReferenceV1,
}

/// A canonical SHA-256 identity for packed RGBA f32 pixel bits.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PixelIdentity([u8; 32]);

impl PixelIdentity {
    #[must_use]
    pub(crate) fn from_components(components: impl IntoIterator<Item = f32>) -> Self {
        let mut hasher = Sha256::new();
        for component in components {
            hasher.update(component.to_bits().to_le_bytes());
        }
        Self(hasher.finalize().into())
    }

    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl std::fmt::Debug for PixelIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Ordered execution evidence for one registered operation node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CpuNodeReceipt {
    index: usize,
    operation_id: OperationId,
}

impl CpuNodeReceipt {
    #[must_use]
    pub(crate) const fn new(index: usize, operation_id: OperationId) -> Self {
        Self {
            index,
            operation_id,
        }
    }

    #[must_use]
    pub const fn index(self) -> usize {
        self.index
    }

    #[must_use]
    pub const fn operation_id(self) -> OperationId {
        self.operation_id
    }
}

/// Immutable evidence describing a completed CPU pixelpipe execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuPipelineReceipt {
    implementation: CpuImplementation,
    input_descriptor: RgbaF32Descriptor,
    output_descriptor: RgbaF32Descriptor,
    input_identity: PixelIdentity,
    output_identity: PixelIdentity,
    output_mode: CpuPixelpipeOutputMode,
    nodes: Vec<CpuNodeReceipt>,
}

impl CpuPipelineReceipt {
    #[must_use]
    pub(crate) fn new(
        input_descriptor: RgbaF32Descriptor,
        output_descriptor: RgbaF32Descriptor,
        input_identity: PixelIdentity,
        output_identity: PixelIdentity,
        output_mode: CpuPixelpipeOutputMode,
        nodes: Vec<CpuNodeReceipt>,
    ) -> Self {
        Self {
            implementation: CpuImplementation::ScalarReferenceV1,
            input_descriptor,
            output_descriptor,
            input_identity,
            output_identity,
            output_mode,
            nodes,
        }
    }

    #[must_use]
    pub const fn implementation(&self) -> CpuImplementation {
        self.implementation
    }

    #[must_use]
    pub const fn input_descriptor(&self) -> RgbaF32Descriptor {
        self.input_descriptor
    }

    #[must_use]
    pub const fn output_descriptor(&self) -> RgbaF32Descriptor {
        self.output_descriptor
    }

    #[must_use]
    pub const fn input_identity(&self) -> PixelIdentity {
        self.input_identity
    }

    #[must_use]
    pub const fn output_identity(&self) -> PixelIdentity {
        self.output_identity
    }

    #[must_use]
    pub const fn output_mode(&self) -> CpuPixelpipeOutputMode {
        self.output_mode
    }

    #[must_use]
    pub fn nodes(&self) -> &[CpuNodeReceipt] {
        &self.nodes
    }
}
