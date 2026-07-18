use std::fmt;

use rusttable_core::{Edit, EditId, OperationId, PhotoId, Revision};

use crate::{OperationCompileError, ProcessingOperation};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PipelineStepIndex(usize);

impl PipelineStepIndex {
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineStep {
    index: PipelineStepIndex,
    operation: ProcessingOperation,
}

impl PipelineStep {
    #[must_use]
    pub const fn index(&self) -> PipelineStepIndex {
        self.index
    }

    #[must_use]
    pub const fn operation(&self) -> &ProcessingOperation {
        &self.operation
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledPipeline {
    source_edit_id: EditId,
    source_photo_id: PhotoId,
    base_photo_revision: Revision,
    revision: Revision,
    steps: Vec<PipelineStep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineCompileError {
    Operation {
        edit_id: EditId,
        step_index: PipelineStepIndex,
        operation_id: OperationId,
        source: OperationCompileError,
    },
}

impl CompiledPipeline {
    /// Compiles every edit operation in its immutable authoring order.
    ///
    /// # Errors
    ///
    /// Returns the exact operation compilation error and its source position;
    /// no partial pipeline is returned when any operation is invalid.
    pub fn compile(edit: &Edit) -> Result<Self, PipelineCompileError> {
        let mut steps = Vec::new();
        for (index, operation) in edit.operations().enumerate() {
            let step_index = PipelineStepIndex::new(index);
            let compiled = ProcessingOperation::compile(operation).map_err(|source| {
                PipelineCompileError::Operation {
                    edit_id: edit.id(),
                    step_index,
                    operation_id: operation.id(),
                    source,
                }
            })?;
            steps.push(PipelineStep {
                index: step_index,
                operation: compiled,
            });
        }
        Ok(Self {
            source_edit_id: edit.id(),
            source_photo_id: edit.photo_id(),
            base_photo_revision: edit.base_photo_revision(),
            revision: edit.revision(),
            steps,
        })
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

    pub fn steps(&self) -> impl Iterator<Item = &PipelineStep> {
        self.steps.iter()
    }

    pub fn active_steps(&self) -> impl Iterator<Item = &PipelineStep> {
        self.steps.iter().filter(|step| step.operation.is_enabled())
    }
}

impl fmt::Display for PipelineCompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Operation {
                edit_id,
                step_index,
                operation_id,
                source,
            } => write!(
                formatter,
                "edit {edit_id} failed at pipeline step {} for operation {operation_id}: {source}",
                step_index.get()
            ),
        }
    }
}

impl std::error::Error for PipelineCompileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Operation { source, .. } => Some(source),
        }
    }
}
