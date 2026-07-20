use std::fmt;

use rusttable_core::{Edit, EditId, OperationId, PhotoId, Revision};

use crate::{
    CompiledPipeline, PipelineCompileError, PipelineStepIndex, PreparedCpuOperation,
    ProcessingOperation,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationGraphNodeIndex(usize);

impl OperationGraphNodeIndex {
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationGraphInput {
    Source,
    Node(OperationGraphNodeIndex),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationGraphOutput {
    Source,
    Node(OperationGraphNodeIndex),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationGraphNode {
    index: OperationGraphNodeIndex,
    pipeline_step_index: PipelineStepIndex,
    input: OperationGraphInput,
    prepared: PreparedCpuOperation,
}

impl OperationGraphNode {
    #[must_use]
    pub const fn index(&self) -> OperationGraphNodeIndex {
        self.index
    }

    #[must_use]
    pub const fn pipeline_step_index(&self) -> PipelineStepIndex {
        self.pipeline_step_index
    }

    #[must_use]
    pub const fn input(&self) -> OperationGraphInput {
        self.input
    }

    #[must_use]
    pub const fn operation(&self) -> &ProcessingOperation {
        self.prepared.operation()
    }

    pub(crate) const fn prepared(&self) -> &PreparedCpuOperation {
        &self.prepared
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledOperationGraph {
    source_edit_id: EditId,
    source_photo_id: PhotoId,
    base_photo_revision: Revision,
    revision: Revision,
    nodes: Vec<OperationGraphNode>,
    output: OperationGraphOutput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationGraphCompileError {
    Pipeline(Box<PipelineCompileError>),
}

impl CompiledOperationGraph {
    /// Compiles an edit and makes its authored operation topology explicit.
    ///
    /// # Errors
    ///
    /// Returns the nested pipeline compilation error without returning a partial graph.
    pub fn compile(edit: &Edit) -> Result<Self, OperationGraphCompileError> {
        let pipeline = CompiledPipeline::compile(edit)
            .map_err(|source| OperationGraphCompileError::Pipeline(Box::new(source)))?;
        Ok(Self::from_pipeline(&pipeline))
    }

    #[must_use]
    pub fn from_pipeline(pipeline: &CompiledPipeline) -> Self {
        let nodes = pipeline
            .steps()
            .enumerate()
            .map(|(index, step)| OperationGraphNode {
                index: OperationGraphNodeIndex::new(index),
                pipeline_step_index: step.index(),
                input: if index == 0 {
                    OperationGraphInput::Source
                } else {
                    OperationGraphInput::Node(OperationGraphNodeIndex::new(index - 1))
                },
                prepared: step.prepared().clone(),
            })
            .collect::<Vec<_>>();
        let output = nodes.last().map_or(OperationGraphOutput::Source, |node| {
            OperationGraphOutput::Node(node.index())
        });
        Self {
            source_edit_id: pipeline.source_edit_id(),
            source_photo_id: pipeline.source_photo_id(),
            base_photo_revision: pipeline.base_photo_revision(),
            revision: pipeline.revision(),
            nodes,
            output,
        }
    }

    #[must_use]
    pub const fn source_edit_id(&self) -> EditId {
        self.source_edit_id
    }

    #[must_use]
    pub const fn source_photo_id(&self) -> PhotoId {
        self.source_photo_id
    }

    #[must_use]
    pub const fn base_photo_revision(&self) -> Revision {
        self.base_photo_revision
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    pub fn nodes(&self) -> impl Iterator<Item = &OperationGraphNode> {
        self.nodes.iter()
    }

    #[must_use]
    pub fn node(&self, index: OperationGraphNodeIndex) -> Option<&OperationGraphNode> {
        self.nodes.get(index.get())
    }

    #[must_use]
    pub fn node_by_operation_id(&self, operation_id: OperationId) -> Option<&OperationGraphNode> {
        self.nodes
            .iter()
            .find(|node| node.operation().operation_id() == operation_id)
    }

    #[must_use]
    pub const fn output(&self) -> OperationGraphOutput {
        self.output
    }
}

impl fmt::Display for OperationGraphCompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pipeline(source) => {
                write!(formatter, "operation graph pipeline failure: {source}")
            }
        }
    }
}

impl std::error::Error for OperationGraphCompileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Pipeline(source) => Some(source.as_ref()),
        }
    }
}
