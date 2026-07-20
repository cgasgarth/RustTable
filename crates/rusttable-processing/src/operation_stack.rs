use crate::descriptor::DescriptorId;
use sha2::{Digest, Sha256};
use std::fmt;

const MAX_OPERATIONS: usize = 512;
const MAX_NAME_BYTES: usize = 256;
const MAX_PARAMETER_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StackStage {
    InputPreparation,
    SensorPreparation,
    DemosaicAndInputColor,
    SceneLinear,
    CreativeAndTone,
    Geometry,
    OutputPreparation,
    Diagnostics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackStageFence {
    pub earliest: StackStage,
    pub latest: StackStage,
}

impl StackStageFence {
    /// Creates an inclusive stage fence.
    ///
    /// # Errors
    ///
    /// Returns an error when the earliest stage follows the latest stage.
    pub fn new(earliest: StackStage, latest: StackStage) -> Result<Self, OperationStackError> {
        if earliest > latest {
            return Err(OperationStackError::InvalidStageFence);
        }
        Ok(Self { earliest, latest })
    }

    #[must_use]
    pub fn accepts(self, stage: StackStage) -> bool {
        stage >= self.earliest && stage <= self.latest
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationInstance {
    id: u128,
    descriptor: DescriptorId,
    parameters: Vec<u8>,
    enabled: bool,
    opacity_basis_points: u16,
    name: Option<String>,
    stage: StackStage,
    mandatory: bool,
    multi_instance: bool,
    mask_id: Option<u128>,
    blend_id: Option<u128>,
}

impl OperationInstance {
    /// Creates a bounded operation instance with default runtime state.
    ///
    /// # Errors
    ///
    /// Returns an error for a zero ID or oversized parameter snapshot.
    pub fn new(
        id: u128,
        descriptor: DescriptorId,
        parameters: Vec<u8>,
        stage: StackStage,
        mandatory: bool,
        multi_instance: bool,
    ) -> Result<Self, OperationStackError> {
        if id == 0 || parameters.len() > MAX_PARAMETER_BYTES {
            return Err(OperationStackError::InvalidInstance);
        }
        Ok(Self {
            id,
            descriptor,
            parameters,
            enabled: true,
            opacity_basis_points: 10_000,
            name: None,
            stage,
            mandatory,
            multi_instance,
            mask_id: None,
            blend_id: None,
        })
    }

    #[must_use]
    pub const fn id(&self) -> u128 {
        self.id
    }
    #[must_use]
    pub const fn descriptor(&self) -> &DescriptorId {
        &self.descriptor
    }
    #[must_use]
    pub fn parameters(&self) -> &[u8] {
        &self.parameters
    }
    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }
    #[must_use]
    pub const fn opacity_basis_points(&self) -> u16 {
        self.opacity_basis_points
    }
    #[must_use]
    pub const fn stage(&self) -> StackStage {
        self.stage
    }
    #[must_use]
    pub const fn mandatory(&self) -> bool {
        self.mandatory
    }
    #[must_use]
    pub const fn multi_instance(&self) -> bool {
        self.multi_instance
    }
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    #[must_use]
    pub const fn mask_id(&self) -> Option<u128> {
        self.mask_id
    }

    #[must_use]
    pub const fn blend_id(&self) -> Option<u128> {
        self.blend_id
    }

    /// Attaches immutable mask and blend references to this operation.
    #[must_use]
    pub const fn with_mask_blend(mut self, mask_id: Option<u128>, blend_id: Option<u128>) -> Self {
        self.mask_id = mask_id;
        self.blend_id = blend_id;
        self
    }

    fn cache_bytes(&self, output: &mut Vec<u8>) {
        output.extend_from_slice(&self.id.to_be_bytes());
        output.extend_from_slice(self.descriptor.compatibility_name.as_bytes());
        output.push(0);
        output.extend_from_slice(self.descriptor.rust_id.as_bytes());
        output.push(0);
        output.extend_from_slice(&self.descriptor.schema_version.to_be_bytes());
        output.extend_from_slice(&self.descriptor.parameter_version.to_be_bytes());
        output.extend_from_slice(&self.descriptor.implementation_version.to_be_bytes());
        output.extend_from_slice(&(self.parameters.len() as u64).to_be_bytes());
        output.extend_from_slice(&self.parameters);
        output.extend_from_slice(&self.opacity_basis_points.to_be_bytes());
        output.push(u8::from(self.enabled));
        output.push(self.stage as u8);
        output.push(u8::from(self.mandatory));
        output.push(u8::from(self.multi_instance));
        output.extend_from_slice(&self.mask_id.unwrap_or_default().to_be_bytes());
        output.extend_from_slice(&self.blend_id.unwrap_or_default().to_be_bytes());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationStackTemplate {
    name: String,
    mandatory_descriptors: Vec<DescriptorId>,
}

impl OperationStackTemplate {
    /// Creates the initial raster workflow template.
    #[must_use]
    pub fn raster_basic() -> Self {
        Self {
            name: "RasterBasic".to_owned(),
            mandatory_descriptors: Vec::new(),
        }
    }

    /// Creates the initial raw workflow template.
    #[must_use]
    pub fn raw_basic() -> Self {
        Self {
            name: "RawBasic".to_owned(),
            mandatory_descriptors: Vec::new(),
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationStackSnapshot {
    template: OperationStackTemplate,
    operations: Vec<OperationInstance>,
    revision: u64,
}

impl OperationStackSnapshot {
    /// Creates an empty immutable snapshot for a workflow template.
    #[must_use]
    pub fn new(template: OperationStackTemplate) -> Self {
        Self {
            template,
            operations: Vec::new(),
            revision: 0,
        }
    }

    #[must_use]
    pub fn operations(&self) -> &[OperationInstance] {
        &self.operations
    }
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    #[must_use]
    pub fn template(&self) -> &OperationStackTemplate {
        &self.template
    }

    /// Applies one command without mutating this snapshot.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the command would violate stack invariants.
    pub fn apply(
        &self,
        command: StackCommand,
    ) -> Result<OperationStackResult, OperationStackError> {
        let mut next = self.clone();
        let changed = next.apply_mut(command)?;
        next.validate()?;
        let old_hash = self.identity_hash();
        let new_hash = next.identity_hash();
        Ok(OperationStackResult {
            snapshot: next,
            receipt: CommandReceipt {
                changed,
                old_hash,
                new_hash,
            },
        })
    }

    /// Checks ordering, identity, instance, and template invariants.
    ///
    /// # Errors
    ///
    /// Returns the first deterministic invariant violation.
    pub fn validate(&self) -> Result<(), OperationStackError> {
        if self.operations.len() > MAX_OPERATIONS {
            return Err(OperationStackError::OperationLimit);
        }
        let mut previous_stage = None;
        for operation in &self.operations {
            if operation.id == 0 {
                return Err(OperationStackError::InvalidInstance);
            }
            if previous_stage.is_some_and(|stage| stage > operation.stage) {
                return Err(OperationStackError::OrderViolation);
            }
            if self
                .operations
                .iter()
                .filter(|candidate| candidate.id == operation.id)
                .count()
                != 1
            {
                return Err(OperationStackError::DuplicateInstanceId);
            }
            if !operation.multi_instance
                && self
                    .operations
                    .iter()
                    .filter(|candidate| candidate.descriptor == operation.descriptor)
                    .count()
                    > 1
            {
                return Err(OperationStackError::SingleInstanceViolation);
            }
            previous_stage = Some(operation.stage);
        }
        for descriptor in &self.template.mandatory_descriptors {
            if !self
                .operations
                .iter()
                .any(|operation| &operation.descriptor == descriptor && operation.mandatory)
            {
                return Err(OperationStackError::MissingMandatory);
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn apply_mut(&mut self, command: StackCommand) -> Result<bool, OperationStackError> {
        match command {
            StackCommand::Insert {
                operation,
                position,
            } => {
                if self
                    .operations
                    .iter()
                    .any(|candidate| candidate.id == operation.id)
                {
                    return Err(OperationStackError::DuplicateInstanceId);
                }
                if !operation.multi_instance
                    && self
                        .operations
                        .iter()
                        .any(|candidate| candidate.descriptor == operation.descriptor)
                {
                    return Err(OperationStackError::SingleInstanceViolation);
                }
                let index = position.resolve(self.operations.len())?;
                self.operations.insert(index, operation);
                self.revision = self
                    .revision
                    .checked_add(1)
                    .ok_or(OperationStackError::RevisionOverflow)?;
                Ok(true)
            }
            StackCommand::Remove { id } => {
                let index = self.index(id)?;
                if self.operations[index].mandatory {
                    return Err(OperationStackError::MandatoryRemoval);
                }
                self.operations.remove(index);
                self.revision = self
                    .revision
                    .checked_add(1)
                    .ok_or(OperationStackError::RevisionOverflow)?;
                Ok(true)
            }
            StackCommand::Duplicate { id, new_id } => {
                if new_id == 0
                    || self
                        .operations
                        .iter()
                        .any(|candidate| candidate.id == new_id)
                {
                    return Err(OperationStackError::DuplicateInstanceId);
                }
                let index = self.index(id)?;
                if !self.operations[index].multi_instance {
                    return Err(OperationStackError::SingleInstanceViolation);
                }
                let mut copy = self.operations[index].clone();
                copy.id = new_id;
                copy.name = None;
                self.operations.insert(index + 1, copy);
                self.revision = self
                    .revision
                    .checked_add(1)
                    .ok_or(OperationStackError::RevisionOverflow)?;
                Ok(true)
            }
            StackCommand::Rename { id, name } => {
                if name.as_ref().is_some_and(|name| {
                    name.is_empty()
                        || name.len() > MAX_NAME_BYTES
                        || name.chars().any(char::is_control)
                }) {
                    return Err(OperationStackError::InvalidName);
                }
                let operation = self.operation_mut(id)?;
                if operation.mandatory && name.is_some() {
                    return Err(OperationStackError::RenameForbidden);
                }
                if operation.name == name {
                    return Ok(false);
                }
                operation.name = name;
                self.revision = self
                    .revision
                    .checked_add(1)
                    .ok_or(OperationStackError::RevisionOverflow)?;
                Ok(true)
            }
            StackCommand::SetEnabled { id, enabled } => {
                let operation = self.operation_mut(id)?;
                if operation.enabled == enabled {
                    return Ok(false);
                }
                operation.enabled = enabled;
                self.revision = self
                    .revision
                    .checked_add(1)
                    .ok_or(OperationStackError::RevisionOverflow)?;
                Ok(true)
            }
            StackCommand::SetOpacity { id, basis_points } => {
                if basis_points > 10_000 {
                    return Err(OperationStackError::InvalidOpacity);
                }
                let operation = self.operation_mut(id)?;
                if operation.opacity_basis_points == basis_points {
                    return Ok(false);
                }
                operation.opacity_basis_points = basis_points;
                self.revision = self
                    .revision
                    .checked_add(1)
                    .ok_or(OperationStackError::RevisionOverflow)?;
                Ok(true)
            }
            StackCommand::Move { id, target } => {
                let from = self.index(id)?;
                let operation = self.operations.remove(from);
                let index = target.resolve(self.operations.len())?;
                if index > self.operations.len() {
                    return Err(OperationStackError::InvalidMove);
                }
                self.operations.insert(index, operation);
                self.revision = self
                    .revision
                    .checked_add(1)
                    .ok_or(OperationStackError::RevisionOverflow)?;
                Ok(true)
            }
            StackCommand::Reset => {
                if self.operations.is_empty() {
                    return Ok(false);
                }
                self.operations.clear();
                self.revision = self
                    .revision
                    .checked_add(1)
                    .ok_or(OperationStackError::RevisionOverflow)?;
                Ok(true)
            }
        }
    }

    fn index(&self, id: u128) -> Result<usize, OperationStackError> {
        self.operations
            .iter()
            .position(|operation| operation.id == id)
            .ok_or(OperationStackError::UnknownInstance)
    }
    fn operation_mut(&mut self, id: u128) -> Result<&mut OperationInstance, OperationStackError> {
        let index = self.index(id)?;
        Ok(&mut self.operations[index])
    }

    fn identity_hash(&self) -> [u8; 32] {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(self.template.name.as_bytes());
        for operation in &self.operations {
            operation.cache_bytes(&mut bytes);
        }
        Sha256::digest(bytes).into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertPosition {
    Start,
    End,
    Index(usize),
}
impl InsertPosition {
    fn resolve(self, length: usize) -> Result<usize, OperationStackError> {
        match self {
            Self::Start => Ok(0),
            Self::End => Ok(length),
            Self::Index(index) if index <= length => Ok(index),
            Self::Index(_) => Err(OperationStackError::InvalidMove),
        }
    }
}

pub type MoveTarget = InsertPosition;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StackCommand {
    Insert {
        operation: OperationInstance,
        position: InsertPosition,
    },
    Remove {
        id: u128,
    },
    Duplicate {
        id: u128,
        new_id: u128,
    },
    Rename {
        id: u128,
        name: Option<String>,
    },
    SetEnabled {
        id: u128,
        enabled: bool,
    },
    SetOpacity {
        id: u128,
        basis_points: u16,
    },
    Move {
        id: u128,
        target: MoveTarget,
    },
    Reset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandReceipt {
    pub changed: bool,
    pub old_hash: [u8; 32],
    pub new_hash: [u8; 32],
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationStackResult {
    pub snapshot: OperationStackSnapshot,
    pub receipt: CommandReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpaqueOperation {
    pub source_version: u16,
    pub raw: Vec<u8>,
    pub original_index: u32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationFinding {
    UnknownVersion,
    LossyConversion,
    Blocking(String),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationOutcome {
    Executable(OperationInstance),
    Opaque(OpaqueOperation),
    Blocked(Vec<MigrationFinding>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationStackError {
    InvalidStageFence,
    InvalidInstance,
    OperationLimit,
    DuplicateInstanceId,
    UnknownInstance,
    SingleInstanceViolation,
    MissingMandatory,
    MandatoryRemoval,
    RenameForbidden,
    InvalidName,
    InvalidOpacity,
    InvalidMove,
    OrderViolation,
    RevisionOverflow,
}

impl fmt::Display for OperationStackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "operation stack error: {self:?}")
    }
}
impl std::error::Error for OperationStackError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::exposure_descriptor;

    fn operation(id: u128) -> OperationInstance {
        let descriptor = exposure_descriptor();
        OperationInstance::new(
            id,
            descriptor.id,
            vec![1, 2],
            StackStage::SceneLinear,
            false,
            true,
        )
        .expect("valid operation")
    }

    #[test]
    fn commands_are_immutable_and_ordered() {
        let stack = OperationStackSnapshot::new(OperationStackTemplate::raster_basic());
        let first = stack
            .apply(StackCommand::Insert {
                operation: operation(1),
                position: InsertPosition::End,
            })
            .expect("insert");
        assert!(stack.operations().is_empty());
        let second = first
            .snapshot
            .apply(StackCommand::Duplicate { id: 1, new_id: 2 })
            .expect("duplicate");
        assert_eq!(second.snapshot.operations().len(), 2);
        assert_ne!(first.receipt.new_hash, second.receipt.new_hash);
    }

    #[test]
    fn invalid_commands_are_atomic() {
        let stack = OperationStackSnapshot::new(OperationStackTemplate::raster_basic());
        let result = stack.apply(StackCommand::Remove { id: 9 });
        assert_eq!(result, Err(OperationStackError::UnknownInstance));
        assert!(stack.operations().is_empty());
    }
}
