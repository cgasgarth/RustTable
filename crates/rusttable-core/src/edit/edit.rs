use std::collections::BTreeSet;
use std::fmt;

use super::operation::Operation;
use crate::{EditId, OperationId, PhotoId, Revision};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    id: EditId,
    photo_id: PhotoId,
    base_photo_revision: Revision,
    revision: Revision,
    operations: Vec<Operation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditBuildError {
    DuplicateOperationId { operation_id: OperationId },
}

impl fmt::Display for EditBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateOperationId { operation_id } => {
                write!(
                    formatter,
                    "operation ID {operation_id} was supplied more than once"
                )
            }
        }
    }
}

impl std::error::Error for EditBuildError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditRevisionError {
    RevisionOverflow,
    InvalidReplacementOperations { source: EditBuildError },
}

impl fmt::Display for EditRevisionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RevisionOverflow => formatter.write_str("edit revision cannot be incremented"),
            Self::InvalidReplacementOperations { source } => source.fmt(formatter),
        }
    }
}

impl std::error::Error for EditRevisionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::RevisionOverflow => None,
            Self::InvalidReplacementOperations { source } => Some(source),
        }
    }
}

impl Edit {
    /// Creates an edit at revision zero.
    ///
    /// # Errors
    ///
    /// Returns [`EditBuildError::DuplicateOperationId`] if an operation ID repeats.
    pub fn new<I>(
        id: EditId,
        photo_id: PhotoId,
        base_photo_revision: Revision,
        operations: I,
    ) -> Result<Self, EditBuildError>
    where
        I: IntoIterator<Item = Operation>,
    {
        Self::from_parts(
            id,
            photo_id,
            base_photo_revision,
            Revision::ZERO,
            operations,
        )
    }

    /// Reconstructs an edit at a caller-supplied edit revision.
    ///
    /// # Errors
    ///
    /// Returns [`EditBuildError::DuplicateOperationId`] if an operation ID repeats.
    pub fn from_parts<I>(
        id: EditId,
        photo_id: PhotoId,
        base_photo_revision: Revision,
        revision: Revision,
        operations: I,
    ) -> Result<Self, EditBuildError>
    where
        I: IntoIterator<Item = Operation>,
    {
        Ok(Self {
            id,
            photo_id,
            base_photo_revision,
            revision,
            operations: collect_operations(operations)?,
        })
    }

    /// Returns a new edit with one checked revision increment and replacement operations.
    ///
    /// # Errors
    ///
    /// Returns [`EditRevisionError::RevisionOverflow`] before consuming replacements when the
    /// current revision is maximal, or preserves a duplicate-operation build error otherwise.
    pub fn revised<I>(&self, operations: I) -> Result<Self, EditRevisionError>
    where
        I: IntoIterator<Item = Operation>,
    {
        let revision = self
            .revision
            .checked_increment()
            .map_err(|_| EditRevisionError::RevisionOverflow)?;
        let operations = collect_operations(operations)
            .map_err(|source| EditRevisionError::InvalidReplacementOperations { source })?;
        Ok(Self {
            id: self.id,
            photo_id: self.photo_id,
            base_photo_revision: self.base_photo_revision,
            revision,
            operations,
        })
    }

    #[must_use]
    pub const fn id(&self) -> EditId {
        self.id
    }

    #[must_use]
    pub const fn photo_id(&self) -> PhotoId {
        self.photo_id
    }

    #[must_use]
    pub const fn base_photo_revision(&self) -> Revision {
        self.base_photo_revision
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    pub fn operations(&self) -> impl Iterator<Item = &Operation> {
        self.operations.iter()
    }
}

fn collect_operations<I>(operations: I) -> Result<Vec<Operation>, EditBuildError>
where
    I: IntoIterator<Item = Operation>,
{
    let mut seen = BTreeSet::new();
    let mut collected = Vec::new();
    for operation in operations {
        let operation_id = operation.id();
        if !seen.insert(operation_id) {
            return Err(EditBuildError::DuplicateOperationId { operation_id });
        }
        collected.push(operation);
    }
    Ok(collected)
}
