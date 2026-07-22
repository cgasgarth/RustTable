use sha2::{Digest, Sha256};

use rusttable_core::OperationId;

use crate::{
    CpuPixelpipeOutputMode, CpuPixelpipeSnapshotIdentity, RgbaF32Descriptor, SourceRasterIdentity,
};
use rusttable_processing::WorkingFrameDescriptor;

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

/// A receipt did not retain the immutable source identity required for publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuPipelineReceiptError {
    SourceIdentityMismatch {
        expected: SourceRasterIdentity,
        actual: SourceRasterIdentity,
    },
}

impl std::fmt::Display for CpuPipelineReceiptError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SourceIdentityMismatch { expected, actual } => write!(
                formatter,
                "pixelpipe receipt source identity mismatch: expected {expected:?}, got {actual:?}"
            ),
        }
    }
}

impl std::error::Error for CpuPipelineReceiptError {}

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
    source_identity: SourceRasterIdentity,
    input_identity: PixelIdentity,
    output_identity: PixelIdentity,
    snapshot_identity: CpuPixelpipeSnapshotIdentity,
    basicadj_plan_identity: [u8; 32],
    output_mode: CpuPixelpipeOutputMode,
    working_profile: WorkingFrameDescriptor,
    nodes: Vec<CpuNodeReceipt>,
}

impl CpuPipelineReceipt {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        input_descriptor: RgbaF32Descriptor,
        output_descriptor: RgbaF32Descriptor,
        source_identity: SourceRasterIdentity,
        pixel_identities: (PixelIdentity, PixelIdentity),
        snapshot_identity: CpuPixelpipeSnapshotIdentity,
        basicadj_plan_identity: [u8; 32],
        output_mode: CpuPixelpipeOutputMode,
        working_profile: WorkingFrameDescriptor,
        nodes: Vec<CpuNodeReceipt>,
    ) -> Self {
        Self {
            implementation: CpuImplementation::ScalarReferenceV1,
            input_descriptor,
            output_descriptor,
            source_identity,
            input_identity: pixel_identities.0,
            output_identity: pixel_identities.1,
            snapshot_identity,
            basicadj_plan_identity,
            output_mode,
            working_profile,
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

    /// Returns the immutable decoded-source evidence bound to this execution.
    #[must_use]
    pub const fn source_identity(&self) -> SourceRasterIdentity {
        self.source_identity
    }

    /// Authorizes publication only for the source raster used by this execution.
    ///
    /// # Errors
    ///
    /// Returns an error when a source was replaced or does not match the
    /// immutable input evidence captured in this receipt.
    pub fn authorize_publication_for(
        &self,
        expected_source_identity: SourceRasterIdentity,
    ) -> Result<(), CpuPipelineReceiptError> {
        if self.source_identity == expected_source_identity {
            Ok(())
        } else {
            Err(CpuPipelineReceiptError::SourceIdentityMismatch {
                expected: expected_source_identity,
                actual: self.source_identity,
            })
        }
    }

    #[must_use]
    pub const fn input_identity(&self) -> PixelIdentity {
        self.input_identity
    }

    #[must_use]
    pub const fn output_identity(&self) -> PixelIdentity {
        self.output_identity
    }

    /// Returns the immutable preparation identity consumed by this execution.
    #[must_use]
    pub const fn snapshot_identity(&self) -> CpuPixelpipeSnapshotIdentity {
        self.snapshot_identity
    }

    #[must_use]
    pub const fn output_mode(&self) -> CpuPixelpipeOutputMode {
        self.output_mode
    }

    #[must_use]
    pub const fn working_profile(&self) -> WorkingFrameDescriptor {
        self.working_profile
    }

    /// Returns the frozen automatic-basicadj resolution identity used by the
    /// execution. Zero means that no automatic basicadj node was present.
    #[must_use]
    pub const fn basicadj_plan_identity(&self) -> [u8; 32] {
        self.basicadj_plan_identity
    }

    #[must_use]
    pub fn nodes(&self) -> &[CpuNodeReceipt] {
        &self.nodes
    }
}
