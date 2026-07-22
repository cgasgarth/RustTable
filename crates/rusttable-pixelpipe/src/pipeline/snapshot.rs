#![allow(clippy::missing_errors_doc)]

use rusttable_color::{ColorEncoding, Precision};
use rusttable_image::Roi;
use rusttable_masks::MaskGraph;
use rusttable_processing::OperationStackSnapshot;

use crate::RoiPlanIdentity;
use crate::pipeline::contracts::{
    BlendStatus, ColorIdentity, ContractError, ImplementationIdentity, MaskStatus,
    PipelineGeneration, PipelineInput, PipelineMode, PipelinePurpose, PipelineQuality,
    PipelineSnapshotIdentity, PublicationGeneration, RasterStatus, SnapshotDiff,
    SnapshotDiffComponent, SourceDescriptor, WORKING_COLOR, color_bytes, format_bytes,
    precision_tag, write_stack,
};

/// Detached values consumed to construct one immutable pixelpipe snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineSnapshotInput {
    generation: PipelineGeneration,
    publication_generation: PublicationGeneration,
    source: SourceDescriptor,
    input: PipelineInput,
    working: ColorIdentity,
    stack: OperationStackSnapshot,
    output: crate::OutputSpec,
    purpose: PipelinePurpose,
    quality: PipelineQuality,
    precision: Precision,
    implementation: ImplementationIdentity,
    mask_graph: Option<MaskGraph>,
}

impl PipelineSnapshotInput {
    /// Creates detached snapshot input using the decoder-declared source as the
    /// initial input boundary.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError`] when the source color boundary is invalid.
    pub fn new(
        generation: PipelineGeneration,
        source: SourceDescriptor,
        stack: OperationStackSnapshot,
        output: crate::OutputSpec,
        purpose: PipelinePurpose,
        implementation: ImplementationIdentity,
    ) -> Result<Self, ContractError> {
        let input = PipelineInput::new(source.dimensions(), source.format(), source.color())?;
        Ok(Self {
            generation,
            publication_generation: PublicationGeneration::new(generation.get()),
            source,
            input,
            working: ColorIdentity::working(),
            stack,
            output,
            purpose,
            quality: PipelineQuality::Normal,
            precision: Precision::F32,
            implementation,
            mask_graph: None,
        })
    }

    #[must_use]
    pub fn with_input(mut self, input: PipelineInput) -> Self {
        self.input = input;
        self
    }

    #[must_use]
    pub const fn with_working_color(mut self, working: ColorIdentity) -> Self {
        self.working = working;
        self
    }

    #[must_use]
    pub const fn with_quality(mut self, quality: PipelineQuality) -> Self {
        self.quality = quality;
        self
    }

    #[must_use]
    pub const fn with_precision(mut self, precision: Precision) -> Self {
        self.precision = precision;
        self
    }

    #[must_use]
    pub const fn with_publication_generation(mut self, generation: PublicationGeneration) -> Self {
        self.publication_generation = generation;
        self
    }

    #[must_use]
    pub fn with_mask_graph(mut self, graph: MaskGraph) -> Self {
        self.mask_graph = Some(graph);
        self
    }
}

/// The stable boundary between application/catalog state and pixel execution.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineSnapshot {
    schema_version: u16,
    generation: PipelineGeneration,
    publication_generation: PublicationGeneration,
    source: SourceDescriptor,
    input: PipelineInput,
    working: ColorIdentity,
    output: crate::OutputSpec,
    stack: OperationStackSnapshot,
    purpose: PipelinePurpose,
    quality: PipelineQuality,
    precision: Precision,
    implementation: ImplementationIdentity,
    mask_status: MaskStatus,
    blend_status: BlendStatus,
    raster_status: RasterStatus,
    identity: PipelineSnapshotIdentity,
    mask_graph: Option<MaskGraph>,
}

impl PipelineSnapshot {
    /// Validates and detaches a complete source/edit/output generation.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError`] when stack, source, input, or output invariants
    /// cannot be proven.
    pub fn new(input: PipelineSnapshotInput) -> Result<Self, ContractError> {
        input
            .stack
            .validate()
            .map_err(|error| ContractError::Stack(error.to_string()))?;
        if input.input.dimensions() != input.source.dimensions()
            || input.input.format() != input.source.format()
            || input.input.color() != input.source.color()
        {
            return Err(ContractError::InvalidFormat);
        }
        if input.output.dimensions().width() == 0 || input.output.dimensions().height() == 0 {
            return Err(ContractError::InvalidDimensions);
        }
        let mask_status = if input.mask_graph.is_some()
            || input
                .stack
                .operations()
                .iter()
                .any(|operation| operation.mask_id().is_some())
        {
            MaskStatus::Referenced
        } else {
            MaskStatus::NotReferenced
        };
        let blend_status = if input
            .stack
            .operations()
            .iter()
            .any(|operation| operation.blend_id().is_some())
        {
            BlendStatus::Referenced
        } else {
            BlendStatus::NotReferenced
        };
        let mut snapshot = Self {
            schema_version: crate::PIPELINE_SCHEMA_VERSION,
            generation: input.generation,
            publication_generation: input.publication_generation,
            source: input.source,
            input: input.input,
            working: input.working,
            output: input.output,
            stack: input.stack,
            purpose: input.purpose,
            quality: input.quality,
            precision: input.precision,
            implementation: input.implementation,
            mask_status,
            blend_status,
            raster_status: RasterStatus::Validated,
            identity: PipelineSnapshotIdentity::from_bytes(&[]),
            mask_graph: input.mask_graph,
        };
        snapshot.identity = PipelineSnapshotIdentity::from_bytes(&snapshot.canonical_bytes());
        Ok(snapshot)
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }
    #[must_use]
    pub const fn generation(&self) -> PipelineGeneration {
        self.generation
    }
    #[must_use]
    pub const fn publication_generation(&self) -> PublicationGeneration {
        self.publication_generation
    }
    #[must_use]
    pub const fn source(&self) -> &SourceDescriptor {
        &self.source
    }
    #[must_use]
    pub const fn input(&self) -> PipelineInput {
        self.input
    }
    #[must_use]
    pub const fn working_color(&self) -> ColorIdentity {
        self.working
    }
    #[must_use]
    pub const fn output(&self) -> &crate::OutputSpec {
        &self.output
    }
    #[must_use]
    pub const fn stack(&self) -> &OperationStackSnapshot {
        &self.stack
    }
    #[must_use]
    pub const fn purpose(&self) -> PipelinePurpose {
        self.purpose
    }
    #[must_use]
    pub const fn mode(&self) -> PipelineMode {
        self.purpose.mode()
    }
    #[must_use]
    pub const fn quality(&self) -> PipelineQuality {
        self.quality
    }
    #[must_use]
    pub const fn precision(&self) -> Precision {
        self.precision
    }
    #[must_use]
    pub const fn implementation(&self) -> &ImplementationIdentity {
        &self.implementation
    }
    #[must_use]
    pub const fn mask_status(&self) -> MaskStatus {
        self.mask_status
    }
    #[must_use]
    pub const fn blend_status(&self) -> BlendStatus {
        self.blend_status
    }
    #[must_use]
    pub const fn raster_status(&self) -> RasterStatus {
        self.raster_status
    }
    #[must_use]
    pub const fn identity(&self) -> PipelineSnapshotIdentity {
        self.identity
    }

    #[must_use]
    pub const fn mask_graph(&self) -> Option<&MaskGraph> {
        self.mask_graph.as_ref()
    }
    #[must_use]
    pub fn mask_graph_identity(&self) -> [u8; 32] {
        self.mask_graph
            .as_ref()
            .map_or([0; 32], MaskGraph::identity)
    }

    /// Returns the canonical bytes used for identity and cache keys.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"rusttable.pixelpipe.snapshot.v1");
        bytes.extend_from_slice(&self.schema_version.to_le_bytes());
        bytes.extend_from_slice(&self.generation.get().to_le_bytes());
        bytes.extend_from_slice(&self.source.identity().as_bytes());
        write_dimensions(self.source.dimensions(), &mut bytes);
        bytes.push(self.source.orientation() as u8);
        write_roi(self.source.bounds(), &mut bytes);
        format_bytes(self.source.format(), &mut bytes);
        color_bytes(self.source.color(), &mut bytes);
        write_dimensions(self.input.dimensions(), &mut bytes);
        format_bytes(self.input.format(), &mut bytes);
        color_bytes(self.input.color(), &mut bytes);
        color_bytes(self.working, &mut bytes);
        write_dimensions(self.output.dimensions(), &mut bytes);
        write_roi(self.output.roi(), &mut bytes);
        format_bytes(self.output.format(), &mut bytes);
        color_bytes(self.output.color(), &mut bytes);
        bytes.extend_from_slice(
            &self
                .output
                .background()
                .rgba()
                .map(f32::to_bits)
                .map(u32::to_le_bytes)
                .concat(),
        );
        write_stack(&self.stack, &mut bytes);
        bytes.extend_from_slice(&self.mask_graph_identity());
        bytes.push(purpose_tag(self.purpose));
        bytes.push(quality_tag(self.quality));
        bytes.push(precision_tag(self.precision));
        write_text(self.implementation.name(), &mut bytes);
        bytes.extend_from_slice(&self.implementation.version().to_le_bytes());
        write_text(self.implementation.build(), &mut bytes);
        bytes
    }

    /// Describes the output-affecting identity components changed between snapshots.
    #[must_use]
    pub fn diff(&self, other: &Self) -> SnapshotDiff {
        let mut components = Vec::new();
        if self.source != other.source {
            components.push(SnapshotDiffComponent::Source);
        }
        if self.stack != other.stack {
            components.push(SnapshotDiffComponent::Stack);
        }
        if self.input.color() != other.input.color() {
            components.push(SnapshotDiffComponent::InputColor);
        }
        if self.working != other.working {
            components.push(SnapshotDiffComponent::WorkingColor);
        }
        if self.output.color() != other.output.color() {
            components.push(SnapshotDiffComponent::OutputColor);
        }
        if self.output.dimensions() != other.output.dimensions()
            || self.output.roi() != other.output.roi()
        {
            components.push(SnapshotDiffComponent::OutputGeometry);
        }
        if self.output.format() != other.output.format() {
            components.push(SnapshotDiffComponent::OutputFormat);
        }
        if self.purpose != other.purpose {
            components.push(SnapshotDiffComponent::Purpose);
        }
        if self.quality != other.quality {
            components.push(SnapshotDiffComponent::Quality);
        }
        if self.precision != other.precision {
            components.push(SnapshotDiffComponent::Precision);
        }
        if self.implementation != other.implementation {
            components.push(SnapshotDiffComponent::Implementation);
        }
        SnapshotDiff::new(components)
    }

    /// Adds an attributable ROI component to the ordinary snapshot diff.
    #[must_use]
    pub fn diff_with_roi(
        &self,
        other: &Self,
        left: RoiPlanIdentity,
        right: RoiPlanIdentity,
    ) -> SnapshotDiff {
        let mut diff = self.diff(other);
        if left != right && !diff.components().contains(&SnapshotDiffComponent::Roi) {
            diff.push(SnapshotDiffComponent::Roi);
        }
        diff
    }

    /// Publication is valid only for the generation that produced the output.
    #[must_use]
    pub fn publication_is_current(&self, current: PublicationGeneration) -> bool {
        self.publication_generation.get() == current.get()
    }

    pub(crate) fn prepared(mut self) -> Self {
        self.raster_status = RasterStatus::Prepared;
        self
    }
}

fn write_dimensions(dimensions: rusttable_image::ImageDimensions, output: &mut Vec<u8>) {
    output.extend_from_slice(&dimensions.width().to_le_bytes());
    output.extend_from_slice(&dimensions.height().to_le_bytes());
}

fn write_roi(roi: Roi, output: &mut Vec<u8>) {
    output.extend_from_slice(&roi.x().to_le_bytes());
    output.extend_from_slice(&roi.y().to_le_bytes());
    output.extend_from_slice(&roi.width().to_le_bytes());
    output.extend_from_slice(&roi.height().to_le_bytes());
}

fn write_text(value: &str, output: &mut Vec<u8>) {
    output.extend_from_slice(&(value.len() as u64).to_le_bytes());
    output.extend_from_slice(value.as_bytes());
}

const fn purpose_tag(value: PipelinePurpose) -> u8 {
    match value {
        PipelinePurpose::Preview => 0,
        PipelinePurpose::Full => 1,
        PipelinePurpose::Thumbnail => 2,
        PipelinePurpose::Export => 3,
    }
}

const fn quality_tag(value: PipelineQuality) -> u8 {
    match value {
        PipelineQuality::Draft => 0,
        PipelineQuality::Normal => 1,
        PipelineQuality::High => 2,
        PipelineQuality::Maximum => 3,
    }
}

#[allow(dead_code)]
const fn _working_color_is_fixed() -> ColorEncoding {
    WORKING_COLOR
}
