#![allow(clippy::missing_errors_doc)]

use std::fmt;

use rusttable_color::ColorEncoding;
use rusttable_image::ChannelLayout;
use rusttable_masks::MaskIdentity;
use rusttable_processing::descriptor::{
    OperationDescriptor, OperationFlags, RoiKind, TilingContract,
};
use rusttable_processing::operation_stack::{OperationInstance, StackStage};
use sha2::{Digest, Sha256};

use crate::pipeline::contracts::{
    BlendStatus, ColorIdentity, ContractError, ImplementationIdentity, MaskStatus, PipelinePurpose,
    PipelineSnapshotIdentity, ResourceMetadata,
};
use crate::pipeline::snapshot::PipelineSnapshot;

/// Immutable context offered to one operation preparation call.
#[derive(Debug, Clone, Copy)]
pub struct PreparationContext {
    operation_index: usize,
    purpose: PipelinePurpose,
    input_color: ColorIdentity,
    working_color: ColorIdentity,
    output_color: ColorIdentity,
    dimensions: rusttable_image::ImageDimensions,
}

impl PreparationContext {
    #[must_use]
    pub const fn operation_index(self) -> usize {
        self.operation_index
    }
    #[must_use]
    pub const fn purpose(self) -> PipelinePurpose {
        self.purpose
    }
    #[must_use]
    pub const fn input_color(self) -> ColorIdentity {
        self.input_color
    }
    #[must_use]
    pub const fn working_color(self) -> ColorIdentity {
        self.working_color
    }
    #[must_use]
    pub const fn output_color(self) -> ColorIdentity {
        self.output_color
    }
    #[must_use]
    pub const fn dimensions(self) -> rusttable_image::ImageDimensions {
        self.dimensions
    }
}

/// Narrow safe boundary for #265's future immutable operation registry.
pub trait OperationPreparationSource {
    /// Prepares one operation into owned, detached state.
    ///
    /// # Errors
    ///
    /// Returns a bounded source error when the operation cannot be prepared.
    fn prepare(
        &self,
        operation: &OperationInstance,
        context: PreparationContext,
    ) -> Result<PreparedOperation, PreparationSourceError>;
}

/// A prepared operation returned by a registry-compatible source.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedOperation {
    descriptor: OperationDescriptor,
    implementation: ImplementationIdentity,
    resource: ResourceMetadata,
    roi_contract: crate::NodeRoiContract,
}

impl PreparedOperation {
    #[must_use]
    pub fn new(
        descriptor: OperationDescriptor,
        implementation: ImplementationIdentity,
        resource: ResourceMetadata,
    ) -> Self {
        let roi_contract = crate::NodeRoiContract::from_kind(descriptor.roi);
        Self {
            descriptor,
            implementation,
            resource,
            roi_contract,
        }
    }

    #[must_use]
    pub fn with_roi_contract(mut self, roi_contract: crate::NodeRoiContract) -> Self {
        self.roi_contract = roi_contract;
        self
    }

    #[must_use]
    pub const fn descriptor(&self) -> &OperationDescriptor {
        &self.descriptor
    }
    #[must_use]
    pub const fn implementation(&self) -> &ImplementationIdentity {
        &self.implementation
    }
    #[must_use]
    pub const fn resource(&self) -> ResourceMetadata {
        self.resource
    }

    #[must_use]
    pub fn roi_contract(&self) -> &crate::NodeRoiContract {
        &self.roi_contract
    }
}

/// A small descriptor-backed source useful for deterministic fixtures before
/// #265 supplies the processing registry.
#[derive(Debug, Clone, Default)]
pub struct DescriptorPreparationSource {
    definitions: Vec<(
        rusttable_processing::descriptor::DescriptorId,
        OperationDescriptor,
    )>,
    implementation: Option<ImplementationIdentity>,
}

impl DescriptorPreparationSource {
    #[must_use]
    pub fn new(descriptors: impl IntoIterator<Item = OperationDescriptor>) -> Self {
        Self {
            definitions: descriptors
                .into_iter()
                .map(|descriptor| (descriptor.id.clone(), descriptor))
                .collect(),
            implementation: None,
        }
    }

    #[must_use]
    pub fn with_implementation(mut self, implementation: ImplementationIdentity) -> Self {
        self.implementation = Some(implementation);
        self
    }
}

impl OperationPreparationSource for DescriptorPreparationSource {
    fn prepare(
        &self,
        operation: &OperationInstance,
        context: PreparationContext,
    ) -> Result<PreparedOperation, PreparationSourceError> {
        let descriptor = self
            .definitions
            .iter()
            .find(|(id, _)| id == operation.descriptor())
            .map(|(_, descriptor)| descriptor.clone())
            .ok_or(PreparationSourceError::UnknownDescriptor)?;
        let implementation = self
            .implementation
            .clone()
            .ok_or(PreparationSourceError::ImplementationUnavailable)?;
        let pixels = context
            .dimensions()
            .pixel_count()
            .map_err(|_| PreparationSourceError::ResourceOverflow)?;
        let bytes = pixels
            .checked_mul(16)
            .ok_or(PreparationSourceError::ResourceOverflow)?;
        let resource = ResourceMetadata::new(bytes, bytes, bytes, 16)
            .map_err(|_| PreparationSourceError::ResourceOverflow)?;
        Ok(PreparedOperation::new(descriptor, implementation, resource))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreparationSourceError {
    UnknownDescriptor,
    ImplementationUnavailable,
    ResourceOverflow,
    Rejected,
}

impl fmt::Display for PreparationSourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnknownDescriptor => "descriptor is unavailable",
            Self::ImplementationUnavailable => "implementation identity is unavailable",
            Self::ResourceOverflow => "resource metadata overflowed",
            Self::Rejected => "operation source rejected preparation",
        })
    }
}

impl std::error::Error for PreparationSourceError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PreparedNodeIdentity([u8; 32]);

impl PreparedNodeIdentity {
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Display for PreparedNodeIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeCacheability {
    Cacheable,
    NotCacheable,
}

/// Owned, immutable metadata for one executable operation node.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedNode {
    index: usize,
    operation_id: u128,
    descriptor: rusttable_processing::descriptor::DescriptorId,
    identity: PreparedNodeIdentity,
    roi: RoiKind,
    tiling: TilingContract,
    resource: ResourceMetadata,
    implementation: ImplementationIdentity,
    mask: MaskStatus,
    blend: BlendStatus,
    cacheability: NodeCacheability,
    roi_contract: crate::NodeRoiContract,
}

impl PreparedNode {
    #[must_use]
    pub const fn index(&self) -> usize {
        self.index
    }
    #[must_use]
    pub const fn operation_id(&self) -> u128 {
        self.operation_id
    }
    #[must_use]
    pub const fn descriptor(&self) -> &rusttable_processing::descriptor::DescriptorId {
        &self.descriptor
    }
    #[must_use]
    pub const fn identity(&self) -> PreparedNodeIdentity {
        self.identity
    }
    #[must_use]
    pub const fn roi(&self) -> RoiKind {
        self.roi
    }
    #[must_use]
    pub const fn tiling(&self) -> TilingContract {
        self.tiling
    }
    #[must_use]
    pub const fn resource(&self) -> ResourceMetadata {
        self.resource
    }
    #[must_use]
    pub const fn implementation(&self) -> &ImplementationIdentity {
        &self.implementation
    }
    #[must_use]
    pub const fn mask_status(&self) -> MaskStatus {
        self.mask
    }
    #[must_use]
    pub const fn blend_status(&self) -> BlendStatus {
        self.blend
    }
    #[must_use]
    pub const fn cacheability(&self) -> NodeCacheability {
        self.cacheability
    }

    #[must_use]
    pub fn roi_contract(&self) -> &crate::NodeRoiContract {
        &self.roi_contract
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreparationError {
    Snapshot(ContractError),
    DisabledMandatory {
        operation_id: u128,
    },
    UnknownOperation {
        operation_id: u128,
    },
    Source {
        operation_id: u128,
        source: PreparationSourceError,
    },
    DescriptorInvalid {
        operation_id: u128,
        reason: String,
    },
    DescriptorMismatch {
        operation_id: u128,
    },
    StageMismatch {
        operation_id: u128,
    },
    UnsupportedPurpose {
        operation_id: u128,
        purpose: PipelinePurpose,
    },
    CpuUnavailable {
        operation_id: u128,
    },
    InputColorTransition {
        operation_id: u128,
    },
    OutputColorTransition {
        operation_id: u128,
    },
    ChannelTransition {
        operation_id: u128,
    },
    DanglingMask {
        operation_id: u128,
        mask_id: u128,
    },
    DanglingTypedMask {
        operation_id: u128,
        mask_id: MaskIdentity,
    },
    DanglingBlend {
        operation_id: u128,
        blend_id: u128,
    },
    PreparedIdentityMismatch {
        operation_id: u128,
    },
}

impl fmt::Display for PreparationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "pixelpipe preparation error: {self:?}")
    }
}

impl std::error::Error for PreparationError {}

/// Bounded evidence of one successful all-or-nothing preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparationReceipt {
    snapshot_identity: PipelineSnapshotIdentity,
    prepared_identity: PipelineSnapshotIdentity,
    source_identity: crate::SourceIdentity,
    generation: crate::PipelineGeneration,
    publication_generation: crate::PublicationGeneration,
    node_count: usize,
}

impl PreparationReceipt {
    #[must_use]
    pub const fn snapshot_identity(&self) -> PipelineSnapshotIdentity {
        self.snapshot_identity
    }
    #[must_use]
    pub const fn prepared_identity(&self) -> PipelineSnapshotIdentity {
        self.prepared_identity
    }
    #[must_use]
    pub const fn source_identity(&self) -> crate::SourceIdentity {
        self.source_identity
    }
    #[must_use]
    pub const fn generation(&self) -> crate::PipelineGeneration {
        self.generation
    }
    #[must_use]
    pub const fn publication_generation(&self) -> crate::PublicationGeneration {
        self.publication_generation
    }
    #[must_use]
    pub const fn node_count(&self) -> usize {
        self.node_count
    }
}

/// Immutable prepared pipeline data. No execution resources or mutable owners
/// are retained here; later issues own scheduling, pools, masks, and devices.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedPipeline {
    snapshot: PipelineSnapshot,
    nodes: Vec<PreparedNode>,
    identity: PipelineSnapshotIdentity,
    receipt: PreparationReceipt,
}

impl PreparedPipeline {
    #[must_use]
    pub const fn snapshot(&self) -> &PipelineSnapshot {
        &self.snapshot
    }
    #[must_use]
    pub fn nodes(&self) -> &[PreparedNode] {
        &self.nodes
    }
    #[must_use]
    pub const fn identity(&self) -> PipelineSnapshotIdentity {
        self.identity
    }
    #[must_use]
    pub const fn receipt(&self) -> &PreparationReceipt {
        &self.receipt
    }

    /// Plans a final output request against this immutable prepared snapshot.
    pub fn plan_roi(
        &self,
        request: crate::RoiRequest,
    ) -> Result<crate::RoiPlan, crate::RoiPlanningError> {
        let source = crate::RoiDescriptor::source(
            self.snapshot.source().dimensions(),
            self.snapshot.source().bounds().into(),
            self.snapshot.source().identity().as_bytes(),
        )
        .map_err(crate::RoiPlanningError::InvalidSource)?;
        let nodes = self
            .nodes
            .iter()
            .map(|node| {
                crate::RoiNode::new(
                    node.operation_id(),
                    node.descriptor().compatibility_name.clone(),
                    node.roi_contract().clone(),
                )
            })
            .collect::<Vec<_>>();
        crate::RoiPlanner.plan(source, &nodes, request)
    }
}

/// Validates a snapshot and prepares every enabled operation exactly once.
#[derive(Clone, Copy)]
pub struct PipelinePreparer<'a> {
    source: &'a dyn OperationPreparationSource,
}

impl<'a> PipelinePreparer<'a> {
    #[must_use]
    pub const fn new(source: &'a dyn OperationPreparationSource) -> Self {
        Self { source }
    }

    /// Prepares one immutable snapshot through the source boundary.
    ///
    /// # Errors
    ///
    /// Returns [`PreparationError`] when any node or boundary fails validation.
    pub fn prepare(
        &self,
        snapshot: &PipelineSnapshot,
    ) -> Result<PreparedPipeline, PreparationError> {
        Self::prepare_with(snapshot, self.source)
    }

    /// Prepares one snapshot with an explicitly supplied source boundary.
    ///
    /// # Errors
    ///
    /// Returns [`PreparationError`] when preparation is not all-or-nothing valid.
    pub fn prepare_with(
        snapshot: &PipelineSnapshot,
        source: &dyn OperationPreparationSource,
    ) -> Result<PreparedPipeline, PreparationError> {
        snapshot
            .stack()
            .validate()
            .map_err(|error| PreparationError::Snapshot(ContractError::Stack(error.to_string())))?;
        validate_boundaries(snapshot)?;
        validate_references(snapshot)?;

        let mut nodes = Vec::new();
        for (index, operation) in snapshot.stack().operations().iter().enumerate() {
            if !operation.enabled() {
                if operation.mandatory() {
                    return Err(PreparationError::DisabledMandatory {
                        operation_id: operation.id(),
                    });
                }
                continue;
            }
            let context = PreparationContext {
                operation_index: index,
                purpose: snapshot.purpose(),
                input_color: snapshot.input().color(),
                working_color: snapshot.working_color(),
                output_color: snapshot.output().color(),
                dimensions: snapshot.output().dimensions(),
            };
            let prepared =
                source
                    .prepare(operation, context)
                    .map_err(|source| PreparationError::Source {
                        operation_id: operation.id(),
                        source,
                    })?;
            validate_prepared(operation, snapshot, &prepared)?;
            nodes.push(prepared_node(index, operation, snapshot, prepared));
        }

        let mut identity_bytes = snapshot.canonical_bytes();
        for node in &nodes {
            identity_bytes.extend_from_slice(&node.identity().as_bytes());
        }
        let identity = PipelineSnapshotIdentity::from_bytes(&identity_bytes);
        let prepared_snapshot = snapshot.clone().prepared();
        let receipt = PreparationReceipt {
            snapshot_identity: snapshot.identity(),
            prepared_identity: identity,
            source_identity: snapshot.source().identity(),
            generation: snapshot.generation(),
            publication_generation: snapshot.publication_generation(),
            node_count: nodes.len(),
        };
        Ok(PreparedPipeline {
            snapshot: prepared_snapshot,
            nodes,
            identity,
            receipt,
        })
    }
}

fn validate_boundaries(snapshot: &PipelineSnapshot) -> Result<(), PreparationError> {
    if snapshot.input().color().encoding() == ColorEncoding::Unspecified
        || (!snapshot.working_color().encoding().is_linear()
            && !matches!(
                snapshot.working_color().encoding(),
                ColorEncoding::External(_)
            ))
        || snapshot.output().color().encoding() == ColorEncoding::Unspecified
    {
        return Err(PreparationError::Snapshot(ContractError::UntaggedColor));
    }
    if snapshot.input().format().channels() != snapshot.source().format().channels()
        || snapshot.input().format().alpha() != snapshot.source().format().alpha()
    {
        return Err(PreparationError::Snapshot(ContractError::InvalidFormat));
    }
    Ok(())
}

fn validate_references(snapshot: &PipelineSnapshot) -> Result<(), PreparationError> {
    for operation in snapshot.stack().operations() {
        if let Some(mask_id) = operation.mask_id()
            && !snapshot
                .stack()
                .operations()
                .iter()
                .any(|candidate| candidate.id() == mask_id)
        {
            return Err(PreparationError::DanglingMask {
                operation_id: operation.id(),
                mask_id,
            });
        }
        if let Some(blend_id) = operation.blend_id()
            && !snapshot
                .stack()
                .operations()
                .iter()
                .any(|candidate| candidate.id() == blend_id)
        {
            return Err(PreparationError::DanglingBlend {
                operation_id: operation.id(),
                blend_id,
            });
        }
    }
    if let Some(graph) = snapshot.mask_graph() {
        for operation in snapshot.stack().operations() {
            if let Some(reference) = operation.mask_reference()
                && (reference.consumer_operation() != operation.id()
                    || graph.node(reference.identity()).is_none())
            {
                return Err(PreparationError::DanglingTypedMask {
                    operation_id: operation.id(),
                    mask_id: reference.identity(),
                });
            }
        }
    }
    Ok(())
}

fn validate_prepared(
    operation: &OperationInstance,
    snapshot: &PipelineSnapshot,
    prepared: &PreparedOperation,
) -> Result<(), PreparationError> {
    if prepared.descriptor().id != *operation.descriptor() {
        return Err(PreparationError::DescriptorMismatch {
            operation_id: operation.id(),
        });
    }
    prepared
        .descriptor()
        .validate()
        .map_err(|error| PreparationError::DescriptorInvalid {
            operation_id: operation.id(),
            reason: error.to_string(),
        })?;
    if !prepared.descriptor().capability.cpu_supported {
        return Err(PreparationError::CpuUnavailable {
            operation_id: operation.id(),
        });
    }
    if !prepared.descriptor().capability.modes.is_empty()
        && !prepared
            .descriptor()
            .capability
            .modes
            .iter()
            .any(|mode| mode == snapshot.purpose().tag() || mode == snapshot.mode_tag())
    {
        return Err(PreparationError::UnsupportedPurpose {
            operation_id: operation.id(),
            purpose: snapshot.purpose(),
        });
    }
    if !prepared
        .descriptor()
        .io
        .input
        .encodings
        .contains(&ColorEncoding::LinearSrgbD65)
        || !prepared
            .descriptor()
            .io
            .output
            .encodings
            .contains(&ColorEncoding::LinearSrgbD65)
    {
        return Err(PreparationError::InputColorTransition {
            operation_id: operation.id(),
        });
    }
    if !channel_compatible(
        prepared.descriptor().io.input.channels,
        snapshot.input().format().channels(),
    ) || !channel_compatible(
        prepared.descriptor().io.output.channels,
        snapshot.output().format().channels(),
    ) {
        return Err(PreparationError::ChannelTransition {
            operation_id: operation.id(),
        });
    }
    if !stage_matches(prepared.descriptor().stage.as_str(), operation.stage()) {
        return Err(PreparationError::StageMismatch {
            operation_id: operation.id(),
        });
    }
    Ok(())
}

fn prepared_node(
    index: usize,
    operation: &OperationInstance,
    snapshot: &PipelineSnapshot,
    prepared: PreparedOperation,
) -> PreparedNode {
    let descriptor = prepared.descriptor().clone();
    let mut identity_bytes = Vec::new();
    identity_bytes.extend_from_slice(b"rusttable.pixelpipe.node.v1");
    identity_bytes.extend_from_slice(&operation.id().to_le_bytes());
    identity_bytes.extend_from_slice(&descriptor.canonical_hash().unwrap_or([0; 32]));
    identity_bytes.extend_from_slice(&snapshot.mask_graph_identity());
    identity_bytes.extend_from_slice(format!("{:?}", prepared.roi_contract()).as_bytes());
    identity_bytes.extend_from_slice(&(operation.parameters().len() as u64).to_le_bytes());
    identity_bytes.extend_from_slice(operation.parameters());
    identity_bytes.extend_from_slice(&operation.opacity_basis_points().to_le_bytes());
    identity_bytes.push(snapshot.purpose().tag().as_bytes()[0]);
    identity_bytes.extend_from_slice(prepared.implementation().name().as_bytes());
    identity_bytes.extend_from_slice(&prepared.implementation().version().to_le_bytes());
    identity_bytes.extend_from_slice(prepared.implementation().build().as_bytes());
    let roi_contract = prepared.roi_contract().clone();
    let identity = PreparedNodeIdentity(Sha256::digest(identity_bytes).into());
    PreparedNode {
        index,
        operation_id: operation.id(),
        descriptor: descriptor.id,
        identity,
        roi: descriptor.roi,
        tiling: descriptor.tiling,
        resource: prepared.resource,
        implementation: prepared.implementation,
        mask: if operation.mask_id().is_some() || operation.mask_reference().is_some() {
            MaskStatus::Deferred
        } else {
            MaskStatus::NotReferenced
        },
        blend: if operation.blend_id().is_some() {
            BlendStatus::Deferred
        } else {
            BlendStatus::NotReferenced
        },
        cacheability: if descriptor.flags.contains(OperationFlags::ANALYSIS) {
            NodeCacheability::NotCacheable
        } else {
            NodeCacheability::Cacheable
        },
        roi_contract,
    }
}

fn channel_compatible(required: u8, actual: ChannelLayout) -> bool {
    matches!(
        (required, actual),
        (
            1,
            ChannelLayout::Gray | ChannelLayout::Bayer | ChannelLayout::XTrans
        ) | (2, ChannelLayout::GrayA)
            | (3, ChannelLayout::Rgb | ChannelLayout::Rgba)
            | (4, ChannelLayout::Rgba)
    )
}

fn stage_matches(descriptor: &str, actual: StackStage) -> bool {
    matches!(
        (descriptor, actual),
        ("input-preparation", StackStage::InputPreparation)
            | ("sensor-preparation", StackStage::SensorPreparation)
            | (
                "demosaic-and-input-color",
                StackStage::DemosaicAndInputColor
            )
            | ("scene-linear", StackStage::SceneLinear)
            | ("creative-and-tone", StackStage::CreativeAndTone)
            | ("geometry", StackStage::Geometry)
            | ("output-preparation", StackStage::OutputPreparation)
            | ("diagnostics", StackStage::Diagnostics)
    )
}

trait SnapshotModeTag {
    fn mode_tag(&self) -> &'static str;
}

impl SnapshotModeTag for PipelineSnapshot {
    fn mode_tag(&self) -> &'static str {
        match self.mode() {
            crate::PipelineMode::Interactive => "interactive",
            crate::PipelineMode::Batch => "batch",
        }
    }
}
