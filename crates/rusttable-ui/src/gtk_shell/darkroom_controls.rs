//! GTK-independent typed state and actions for darkroom module controls.
use crate::viewport_presentation::ViewportGeneration;
use rusttable_core::{PhotoId, Revision};
use rusttable_processing::descriptor::{
    DescriptorError, DescriptorId, OperationDescriptor, ParameterDefault, ParameterDescriptor,
    ParameterKind,
};
use rusttable_processing::operation_stack::{
    OperationInstance, OperationStackError, OperationStackSnapshot, StackCommand,
};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DarkroomSelection {
    photo_id: PhotoId,
    generation: ViewportGeneration,
}
impl DarkroomSelection {
    #[must_use]
    pub const fn new(photo_id: PhotoId, generation: ViewportGeneration) -> Self {
        Self {
            photo_id,
            generation,
        }
    }
    #[must_use]
    pub const fn photo_id(self) -> PhotoId {
        self.photo_id
    }
    #[must_use]
    pub const fn generation(self) -> ViewportGeneration {
        self.generation
    }
}
#[derive(Debug, Clone, PartialEq)]
pub enum DarkroomParameterValue {
    Bool(bool),
    Integer(i64),
    Scalar(f64),
    Vector(Vec<f64>),
    Matrix(Vec<f64>),
    Curve(Vec<(f64, f64)>),
    Enum(String),
    Color(rusttable_color::ColorEncoding),
    ProfileRef(String),
    FileRef(String),
    ContentRef(String),
    Text(String),
}
impl DarkroomParameterValue {
    #[must_use]
    pub const fn kind(&self) -> DarkroomParameterValueKind {
        match self {
            Self::Bool(_) => DarkroomParameterValueKind::Bool,
            Self::Integer(_) => DarkroomParameterValueKind::Integer,
            Self::Scalar(_) => DarkroomParameterValueKind::Scalar,
            Self::Vector(_) => DarkroomParameterValueKind::Vector,
            Self::Matrix(_) => DarkroomParameterValueKind::Matrix,
            Self::Curve(_) => DarkroomParameterValueKind::Curve,
            Self::Enum(_) => DarkroomParameterValueKind::Enum,
            Self::Color(_) => DarkroomParameterValueKind::Color,
            Self::ProfileRef(_) => DarkroomParameterValueKind::ProfileRef,
            Self::FileRef(_) => DarkroomParameterValueKind::FileRef,
            Self::ContentRef(_) => DarkroomParameterValueKind::ContentRef,
            Self::Text(_) => DarkroomParameterValueKind::Text,
        }
    }
}
impl From<ParameterDefault> for DarkroomParameterValue {
    fn from(value: ParameterDefault) -> Self {
        match value {
            ParameterDefault::Bool(value) => Self::Bool(value),
            ParameterDefault::Integer(value) => Self::Integer(value),
            ParameterDefault::Scalar(value) => Self::Scalar(value),
            ParameterDefault::Vector(value) => Self::Vector(value),
            ParameterDefault::Matrix(value) => Self::Matrix(value),
            ParameterDefault::Curve(value) => Self::Curve(value),
            ParameterDefault::Enum(value) => Self::Enum(value),
            ParameterDefault::Color(value) => Self::Color(value),
            ParameterDefault::ProfileRef(value) => Self::ProfileRef(value),
            ParameterDefault::FileRef(value) => Self::FileRef(value),
            ParameterDefault::ContentRef(value) => Self::ContentRef(value),
            ParameterDefault::Text(value) => Self::Text(value),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DarkroomParameterValueKind {
    Bool,
    Integer,
    Scalar,
    Vector,
    Matrix,
    Curve,
    Enum,
    Color,
    ProfileRef,
    FileRef,
    ContentRef,
    Text,
}
#[derive(Debug, Clone, PartialEq)]
pub struct DarkroomParameterControl {
    descriptor: ParameterDescriptor,
    value: DarkroomParameterValue,
    default: DarkroomParameterValue,
}
impl DarkroomParameterControl {
    fn new(descriptor: ParameterDescriptor) -> Result<Self, ParameterValidationError> {
        let default = descriptor.default.clone().into();
        validate_parameter_value(&descriptor, &default)?;
        Ok(Self {
            descriptor,
            value: default.clone(),
            default,
        })
    }
    #[must_use]
    pub fn id(&self) -> &str {
        &self.descriptor.id
    }
    #[must_use]
    pub const fn value(&self) -> &DarkroomParameterValue {
        &self.value
    }
    #[must_use]
    pub const fn default_value(&self) -> &DarkroomParameterValue {
        &self.default
    }
    fn set(&mut self, value: DarkroomParameterValue) -> Result<(), ParameterValidationError> {
        validate_parameter_value(&self.descriptor, &value)?;
        self.value = value;
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq)]
pub enum ParameterValidationError {
    WrongType {
        parameter_id: String,
        expected: DarkroomParameterValueKind,
        actual: DarkroomParameterValueKind,
    },
    NonFinite(String),
    IntegerRange(String),
    ScalarRange(String),
    Length {
        parameter_id: String,
        expected: usize,
        actual: usize,
    },
    CurvePoints {
        parameter_id: String,
        maximum: usize,
        actual: usize,
    },
    EnumTag {
        parameter_id: String,
        value: String,
    },
    TextLength {
        parameter_id: String,
        maximum: usize,
        actual: usize,
    },
}
#[derive(Debug, Clone, PartialEq)]
pub struct DarkroomParameterAssignment {
    pub parameter_id: String,
    pub value: DarkroomParameterValue,
}
#[derive(Debug, Clone, PartialEq)]
pub enum DarkroomOperationStackUpdate {
    Command(StackCommand),
    SetParameter {
        descriptor: DescriptorId,
        parameter_id: String,
        value: DarkroomParameterValue,
    },
    ResetParameters {
        descriptor: DescriptorId,
        defaults: Vec<DarkroomParameterAssignment>,
    },
}
#[derive(Debug, Clone, PartialEq)]
pub struct DarkroomOperationStackUpdateMessage {
    pub selection: DarkroomSelection,
    pub operation_id: u128,
    pub module_revision: Revision,
    pub expected_stack_revision: u64,
    pub update: DarkroomOperationStackUpdate,
}
#[derive(Debug, Clone, PartialEq)]
pub enum DarkroomControlMessage {
    DisclosureChanged {
        selection: DarkroomSelection,
        operation_id: u128,
        module_revision: Revision,
        expanded: bool,
    },
    OperationStack(Box<DarkroomOperationStackUpdateMessage>),
}
#[derive(Debug, Clone, PartialEq)]
pub enum DarkroomControlFeedback {
    Ready,
    Applied {
        selection: DarkroomSelection,
        operation_id: u128,
        stack_revision: u64,
    },
    Rejected {
        selection: DarkroomSelection,
        operation_id: u128,
        error: OperationStackError,
    },
    StaleSelection {
        expected: DarkroomSelection,
        actual: Option<DarkroomSelection>,
    },
    StaleModuleRevision {
        operation_id: u128,
        expected: Revision,
        actual: Revision,
    },
    StaleStackRevision {
        expected: u64,
        actual: u64,
    },
}
#[derive(Debug, Clone, PartialEq)]
pub enum DarkroomOperationStackFeedback {
    Applied {
        selection: DarkroomSelection,
        operation_id: u128,
        stack_revision: u64,
    },
    Rejected {
        selection: DarkroomSelection,
        operation_id: u128,
        stack_revision: u64,
        error: OperationStackError,
    },
}
#[derive(Debug, Clone, PartialEq)]
pub enum DarkroomModuleControlError {
    ZeroOperationId,
    EmptyTitle,
    InvalidDescriptor(DescriptorError),
    DescriptorMismatch(Box<DescriptorMismatch>),
    Parameter(ParameterValidationError),
}
#[derive(Debug, Clone, PartialEq)]
pub struct DescriptorMismatch {
    operation_id: u128,
    operation: DescriptorId,
    descriptor: DescriptorId,
}
#[derive(Debug, Clone, PartialEq)]
pub enum DarkroomControlModelError {
    NoSelection,
    StaleSelection {
        expected: DarkroomSelection,
        actual: Option<DarkroomSelection>,
    },
    UnknownModule(u128),
    StaleModuleRevision {
        operation_id: u128,
        expected: Revision,
        actual: Revision,
    },
    ModuleNotResettable(u128),
    UnknownParameter {
        operation_id: u128,
        parameter_id: String,
    },
    Parameter(ParameterValidationError),
    RevisionOverflow,
    DuplicateOperationId(u128),
    Module(Box<DarkroomModuleControlError>),
}
#[derive(Debug, Clone, PartialEq)]
pub struct DarkroomModuleControl {
    operation_id: u128,
    descriptor: DescriptorId,
    title: String,
    expanded: bool,
    enabled: bool,
    resettable: bool,
    revision: Revision,
    parameters: Vec<DarkroomParameterControl>,
}
impl DarkroomModuleControl {
    /// Builds a module from an operation instance and matching descriptor.
    ///
    /// # Errors
    ///
    /// Returns an error when the descriptor identity, title, or parameter defaults are invalid.
    pub fn from_operation(
        operation: &OperationInstance,
        descriptor: OperationDescriptor,
        title: impl Into<String>,
        expanded: bool,
        resettable: bool,
        revision: Revision,
    ) -> Result<Self, DarkroomModuleControlError> {
        if operation.descriptor() != &descriptor.id {
            return Err(DarkroomModuleControlError::DescriptorMismatch(Box::new(
                DescriptorMismatch {
                    operation_id: operation.id(),
                    operation: operation.descriptor().clone(),
                    descriptor: descriptor.id,
                },
            )));
        }
        Self::from_descriptor(
            operation.id(),
            descriptor,
            title,
            expanded,
            operation.enabled(),
            resettable,
            revision,
        )
    }
    /// Builds a module from a validated operation descriptor.
    ///
    /// # Errors
    ///
    /// Returns an error when the operation id, title, descriptor, or parameter defaults are invalid.
    pub fn from_descriptor(
        operation_id: u128,
        descriptor: OperationDescriptor,
        title: impl Into<String>,
        expanded: bool,
        enabled: bool,
        resettable: bool,
        revision: Revision,
    ) -> Result<Self, DarkroomModuleControlError> {
        if operation_id == 0 {
            return Err(DarkroomModuleControlError::ZeroOperationId);
        }
        let title = title.into();
        if title.trim().is_empty() {
            return Err(DarkroomModuleControlError::EmptyTitle);
        }
        descriptor
            .validate()
            .map_err(DarkroomModuleControlError::InvalidDescriptor)?;
        let parameters = descriptor
            .parameters
            .iter()
            .cloned()
            .map(DarkroomParameterControl::new)
            .collect::<Result<Vec<_>, _>>()
            .map_err(DarkroomModuleControlError::Parameter)?;
        Ok(Self {
            operation_id,
            descriptor: descriptor.id,
            title,
            expanded,
            enabled,
            resettable,
            revision,
            parameters,
        })
    }
    #[must_use]
    pub const fn operation_id(&self) -> u128 {
        self.operation_id
    }
    #[must_use]
    pub const fn expanded(&self) -> bool {
        self.expanded
    }
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }
    #[must_use]
    pub fn parameter(&self, id: &str) -> Option<&DarkroomParameterControl> {
        self.parameters
            .iter()
            .find(|parameter| parameter.id() == id)
    }
    fn advance(&mut self) -> Result<Revision, DarkroomControlModelError> {
        self.revision = self
            .revision
            .checked_increment()
            .map_err(|_| DarkroomControlModelError::RevisionOverflow)?;
        Ok(self.revision)
    }
    fn disclosure(&mut self, expanded: bool) -> Result<Revision, DarkroomControlModelError> {
        self.expanded = expanded;
        self.advance()
    }
    fn enabled(&mut self, enabled: bool) -> Result<Revision, DarkroomControlModelError> {
        self.enabled = enabled;
        self.advance()
    }
    fn edit_parameter(
        &mut self,
        id: &str,
        value: DarkroomParameterValue,
    ) -> Result<Revision, DarkroomControlModelError> {
        let Some(parameter) = self.parameters.iter_mut().find(|item| item.id() == id) else {
            return Err(DarkroomControlModelError::UnknownParameter {
                operation_id: self.operation_id,
                parameter_id: id.to_owned(),
            });
        };
        parameter
            .set(value)
            .map_err(DarkroomControlModelError::Parameter)?;
        self.advance()
    }
    fn reset(
        &mut self,
    ) -> Result<(Revision, Vec<DarkroomParameterAssignment>), DarkroomControlModelError> {
        if !self.resettable {
            return Err(DarkroomControlModelError::ModuleNotResettable(
                self.operation_id,
            ));
        }
        for parameter in &mut self.parameters {
            parameter.value = parameter.default.clone();
        }
        let defaults = self
            .parameters
            .iter()
            .map(|parameter| DarkroomParameterAssignment {
                parameter_id: parameter.id().to_owned(),
                value: parameter.default.clone(),
            })
            .collect();
        Ok((self.advance()?, defaults))
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct DarkroomControlModel {
    selection: Option<DarkroomSelection>,
    stack_revision: u64,
    modules: Vec<DarkroomModuleControl>,
    feedback: DarkroomControlFeedback,
}
impl DarkroomControlModel {
    /// Creates a model using the revision of an immutable operation-stack snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when two modules target the same operation instance.
    pub fn new(
        selection: Option<DarkroomSelection>,
        stack: &OperationStackSnapshot,
        modules: Vec<DarkroomModuleControl>,
    ) -> Result<Self, DarkroomControlModelError> {
        for (index, module) in modules.iter().enumerate() {
            if modules[..index]
                .iter()
                .any(|previous| previous.operation_id() == module.operation_id())
            {
                return Err(DarkroomControlModelError::DuplicateOperationId(
                    module.operation_id(),
                ));
            }
        }
        Ok(Self {
            selection,
            stack_revision: stack.revision(),
            modules,
            feedback: DarkroomControlFeedback::Ready,
        })
    }
    #[must_use]
    pub const fn selection(&self) -> Option<DarkroomSelection> {
        self.selection
    }
    #[must_use]
    pub const fn stack_revision(&self) -> u64 {
        self.stack_revision
    }
    #[must_use]
    pub fn module(&self, operation_id: u128) -> Option<&DarkroomModuleControl> {
        self.modules
            .iter()
            .find(|item| item.operation_id() == operation_id)
    }
    #[must_use]
    pub const fn feedback(&self) -> &DarkroomControlFeedback {
        &self.feedback
    }
    pub fn select(&mut self, selection: DarkroomSelection, stack_revision: u64) {
        self.selection = Some(selection);
        self.stack_revision = stack_revision;
        self.feedback = DarkroomControlFeedback::Ready;
    }
    pub fn clear_selection(&mut self) {
        self.selection = None;
        self.feedback = DarkroomControlFeedback::Ready;
    }
    /// Changes disclosure state and returns a local GTK message.
    ///
    /// # Errors
    ///
    /// Returns an error when the selection or module revision is stale.
    pub fn set_expanded(
        &mut self,
        selection: DarkroomSelection,
        operation_id: u128,
        expected_revision: Revision,
        expanded: bool,
    ) -> Result<DarkroomControlMessage, DarkroomControlModelError> {
        self.guard(selection, operation_id, expected_revision)?;
        let module_revision = self.module_mut(operation_id)?.disclosure(expanded)?;
        Ok(DarkroomControlMessage::DisclosureChanged {
            selection,
            operation_id,
            module_revision,
            expanded,
        })
    }
    /// Enables or disables an operation and returns a stack update message.
    ///
    /// # Errors
    ///
    /// Returns an error when the selection or module revision is stale.
    pub fn set_enabled(
        &mut self,
        selection: DarkroomSelection,
        operation_id: u128,
        expected_revision: Revision,
        enabled: bool,
    ) -> Result<DarkroomControlMessage, DarkroomControlModelError> {
        self.guard(selection, operation_id, expected_revision)?;
        let stack_revision = self.stack_revision;
        let module_revision = self.module_mut(operation_id)?.enabled(enabled)?;
        Ok(Self::stack_message(
            selection,
            operation_id,
            module_revision,
            DarkroomOperationStackUpdate::Command(StackCommand::SetEnabled {
                id: operation_id,
                enabled,
            }),
            stack_revision,
        ))
    }
    /// Resets descriptor parameters and returns a typed stack update message.
    ///
    /// # Errors
    ///
    /// Returns an error when the selection, module revision, or reset capability is invalid.
    pub fn reset_module(
        &mut self,
        selection: DarkroomSelection,
        operation_id: u128,
        expected_revision: Revision,
    ) -> Result<DarkroomControlMessage, DarkroomControlModelError> {
        self.guard(selection, operation_id, expected_revision)?;
        let stack_revision = self.stack_revision;
        let (module_revision, defaults, descriptor) = {
            let module = self.module_mut(operation_id)?;
            let (module_revision, defaults) = module.reset()?;
            (module_revision, defaults, module.descriptor.clone())
        };
        Ok(Self::stack_message(
            selection,
            operation_id,
            module_revision,
            DarkroomOperationStackUpdate::ResetParameters {
                descriptor,
                defaults,
            },
            stack_revision,
        ))
    }
    /// Applies one typed descriptor parameter and returns a stack update message.
    ///
    /// # Errors
    ///
    /// Returns an error when the selection, module revision, or typed value is invalid.
    pub fn set_parameter(
        &mut self,
        selection: DarkroomSelection,
        operation_id: u128,
        expected_revision: Revision,
        parameter_id: &str,
        value: DarkroomParameterValue,
    ) -> Result<DarkroomControlMessage, DarkroomControlModelError> {
        self.guard(selection, operation_id, expected_revision)?;
        let stack_revision = self.stack_revision;
        let (module_revision, descriptor) = {
            let module = self.module_mut(operation_id)?;
            (
                module.edit_parameter(parameter_id, value.clone())?,
                module.descriptor.clone(),
            )
        };
        Ok(Self::stack_message(
            selection,
            operation_id,
            module_revision,
            DarkroomOperationStackUpdate::SetParameter {
                descriptor,
                parameter_id: parameter_id.to_owned(),
                value,
            },
            stack_revision,
        ))
    }
    #[must_use]
    pub fn receive_stack_feedback(&mut self, response: DarkroomOperationStackFeedback) -> bool {
        let (selection, operation_id, revision) = match &response {
            DarkroomOperationStackFeedback::Applied {
                selection,
                operation_id,
                stack_revision,
            }
            | DarkroomOperationStackFeedback::Rejected {
                selection,
                operation_id,
                stack_revision,
                ..
            } => (*selection, *operation_id, *stack_revision),
        };
        if self.selection != Some(selection) {
            self.feedback = DarkroomControlFeedback::StaleSelection {
                expected: selection,
                actual: self.selection,
            };
            return false;
        }
        if revision < self.stack_revision {
            self.feedback = DarkroomControlFeedback::StaleStackRevision {
                expected: self.stack_revision,
                actual: revision,
            };
            return false;
        }
        self.stack_revision = revision;
        self.feedback = match response {
            DarkroomOperationStackFeedback::Applied { .. } => DarkroomControlFeedback::Applied {
                selection,
                operation_id,
                stack_revision: revision,
            },
            DarkroomOperationStackFeedback::Rejected { error, .. } => {
                DarkroomControlFeedback::Rejected {
                    selection,
                    operation_id,
                    error,
                }
            }
        };
        true
    }
    fn stack_message(
        selection: DarkroomSelection,
        operation_id: u128,
        module_revision: Revision,
        update: DarkroomOperationStackUpdate,
        expected_stack_revision: u64,
    ) -> DarkroomControlMessage {
        DarkroomControlMessage::OperationStack(Box::new(DarkroomOperationStackUpdateMessage {
            selection,
            operation_id,
            module_revision,
            expected_stack_revision,
            update,
        }))
    }
    fn module_mut(
        &mut self,
        id: u128,
    ) -> Result<&mut DarkroomModuleControl, DarkroomControlModelError> {
        self.modules
            .iter_mut()
            .find(|item| item.operation_id() == id)
            .ok_or(DarkroomControlModelError::UnknownModule(id))
    }
    fn guard(
        &mut self,
        selection: DarkroomSelection,
        operation_id: u128,
        expected_revision: Revision,
    ) -> Result<(), DarkroomControlModelError> {
        if self.selection != Some(selection) {
            let error = if self.selection.is_none() {
                DarkroomControlModelError::NoSelection
            } else {
                DarkroomControlModelError::StaleSelection {
                    expected: selection,
                    actual: self.selection,
                }
            };
            self.feedback = DarkroomControlFeedback::StaleSelection {
                expected: selection,
                actual: self.selection,
            };
            return Err(error);
        }
        let actual = self
            .module(operation_id)
            .ok_or(DarkroomControlModelError::UnknownModule(operation_id))?
            .revision();
        if actual != expected_revision {
            self.feedback = DarkroomControlFeedback::StaleModuleRevision {
                operation_id,
                expected: expected_revision,
                actual,
            };
            return Err(DarkroomControlModelError::StaleModuleRevision {
                operation_id,
                expected: expected_revision,
                actual,
            });
        }
        Ok(())
    }
}
fn validate_parameter_value(
    descriptor: &ParameterDescriptor,
    value: &DarkroomParameterValue,
) -> Result<(), ParameterValidationError> {
    let expected = value_kind(&descriptor.kind);
    if expected != value.kind() {
        return Err(ParameterValidationError::WrongType {
            parameter_id: descriptor.id.clone(),
            expected,
            actual: value.kind(),
        });
    }
    match (&descriptor.kind, value) {
        (ParameterKind::Bool, DarkroomParameterValue::Bool(_))
        | (ParameterKind::Color { .. }, DarkroomParameterValue::Color(_))
        | (ParameterKind::ProfileRef, DarkroomParameterValue::ProfileRef(_))
        | (ParameterKind::FileRef, DarkroomParameterValue::FileRef(_))
        | (ParameterKind::ContentRef, DarkroomParameterValue::ContentRef(_)) => Ok(()),
        (ParameterKind::Integer { minimum, maximum }, DarkroomParameterValue::Integer(value)) => {
            if (*minimum..=*maximum).contains(value) {
                Ok(())
            } else {
                Err(ParameterValidationError::IntegerRange(
                    descriptor.id.clone(),
                ))
            }
        }
        (ParameterKind::Scalar { minimum, maximum }, DarkroomParameterValue::Scalar(value)) => {
            scalar(&descriptor.id, *value, *minimum, *maximum)
        }
        (
            ParameterKind::Vector {
                dimensions,
                minimum,
                maximum,
            },
            DarkroomParameterValue::Vector(values),
        ) => vector(
            &descriptor.id,
            values,
            usize::from(*dimensions),
            *minimum,
            *maximum,
        ),
        (
            ParameterKind::Matrix {
                rows,
                columns,
                minimum,
                maximum,
            },
            DarkroomParameterValue::Matrix(values),
        ) => vector(
            &descriptor.id,
            values,
            usize::from(*rows) * usize::from(*columns),
            *minimum,
            *maximum,
        ),
        (ParameterKind::Curve { maximum_points }, DarkroomParameterValue::Curve(points)) => {
            if points.len() > usize::from(*maximum_points) {
                return Err(ParameterValidationError::CurvePoints {
                    parameter_id: descriptor.id.clone(),
                    maximum: usize::from(*maximum_points),
                    actual: points.len(),
                });
            }
            if points.iter().any(|(x, y)| !x.is_finite() || !y.is_finite()) {
                return Err(ParameterValidationError::NonFinite(descriptor.id.clone()));
            }
            Ok(())
        }
        (ParameterKind::Enum { tags }, DarkroomParameterValue::Enum(value)) => {
            if tags.iter().any(|tag| tag == value) {
                Ok(())
            } else {
                Err(ParameterValidationError::EnumTag {
                    parameter_id: descriptor.id.clone(),
                    value: value.clone(),
                })
            }
        }
        (ParameterKind::Text { maximum_bytes }, DarkroomParameterValue::Text(value)) => {
            if value.len() <= usize::from(*maximum_bytes) {
                Ok(())
            } else {
                Err(ParameterValidationError::TextLength {
                    parameter_id: descriptor.id.clone(),
                    maximum: usize::from(*maximum_bytes),
                    actual: value.len(),
                })
            }
        }
        _ => unreachable!("descriptor and value kinds were checked"),
    }
}
fn scalar(
    id: &str,
    value: f64,
    minimum: f64,
    maximum: f64,
) -> Result<(), ParameterValidationError> {
    if !value.is_finite() {
        return Err(ParameterValidationError::NonFinite(id.to_owned()));
    }
    if (minimum..=maximum).contains(&value) {
        Ok(())
    } else {
        Err(ParameterValidationError::ScalarRange(id.to_owned()))
    }
}
fn vector(
    id: &str,
    values: &[f64],
    expected: usize,
    minimum: f64,
    maximum: f64,
) -> Result<(), ParameterValidationError> {
    if values.len() != expected {
        return Err(ParameterValidationError::Length {
            parameter_id: id.to_owned(),
            expected,
            actual: values.len(),
        });
    }
    values
        .iter()
        .try_for_each(|value| scalar(id, *value, minimum, maximum))
}
fn value_kind(kind: &ParameterKind) -> DarkroomParameterValueKind {
    match kind {
        ParameterKind::Bool => DarkroomParameterValueKind::Bool,
        ParameterKind::Integer { .. } => DarkroomParameterValueKind::Integer,
        ParameterKind::Scalar { .. } => DarkroomParameterValueKind::Scalar,
        ParameterKind::Vector { .. } => DarkroomParameterValueKind::Vector,
        ParameterKind::Matrix { .. } => DarkroomParameterValueKind::Matrix,
        ParameterKind::Curve { .. } => DarkroomParameterValueKind::Curve,
        ParameterKind::Enum { .. } => DarkroomParameterValueKind::Enum,
        ParameterKind::Color { .. } => DarkroomParameterValueKind::Color,
        ParameterKind::ProfileRef => DarkroomParameterValueKind::ProfileRef,
        ParameterKind::FileRef => DarkroomParameterValueKind::FileRef,
        ParameterKind::ContentRef => DarkroomParameterValueKind::ContentRef,
        ParameterKind::Text { .. } => DarkroomParameterValueKind::Text,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use rusttable_processing::descriptor::exposure_descriptor;
    use rusttable_processing::operation_stack::{OperationStackTemplate, StackStage};
    fn selection(generation: u64) -> DarkroomSelection {
        DarkroomSelection::new(
            PhotoId::new(7).expect("non-zero test photo"),
            ViewportGeneration::new(generation),
        )
    }
    fn model() -> DarkroomControlModel {
        let descriptor = exposure_descriptor();
        let operation = OperationInstance::new(
            41,
            descriptor.id,
            Vec::new(),
            StackStage::SceneLinear,
            false,
            false,
        )
        .expect("valid operation");
        let module = DarkroomModuleControl::from_operation(
            &operation,
            exposure_descriptor(),
            "Exposure",
            true,
            true,
            Revision::from_u64(3),
        )
        .expect("valid module");
        let stack = OperationStackSnapshot::new(OperationStackTemplate::raster_basic());
        DarkroomControlModel::new(Some(selection(1)), &stack, vec![module]).expect("valid model")
    }
    #[test]
    fn typed_edit_emits_descriptor_stack_update() {
        let mut model = model();
        let message = model
            .set_parameter(
                selection(1),
                41,
                Revision::from_u64(3),
                "stops",
                DarkroomParameterValue::Scalar(1.25),
            )
            .expect("typed edit");
        assert_eq!(
            model
                .module(41)
                .expect("module")
                .parameter("stops")
                .expect("parameter")
                .value(),
            &DarkroomParameterValue::Scalar(1.25)
        );
        let DarkroomControlMessage::OperationStack(message) = message else {
            panic!("typed edit must emit a stack update");
        };
        assert_eq!(message.expected_stack_revision, 0);
        assert!(matches!(
            message.update,
            DarkroomOperationStackUpdate::SetParameter {
                parameter_id,
                value: DarkroomParameterValue::Scalar(1.25),
                ..
            } if parameter_id == "stops"
        ));
    }
    #[test]
    fn expand_enable_and_reset_are_distinct_actions() {
        let mut model = model();
        assert!(matches!(
            model.set_expanded(selection(1), 41, Revision::from_u64(3), false),
            Ok(DarkroomControlMessage::DisclosureChanged {
                expanded: false,
                ..
            })
        ));
        let DarkroomControlMessage::OperationStack(message) = model
            .set_enabled(selection(1), 41, Revision::from_u64(4), false)
            .expect("disable")
        else {
            panic!("enable must emit a stack update");
        };
        assert!(matches!(
            message.update,
            DarkroomOperationStackUpdate::Command(StackCommand::SetEnabled {
                id: 41,
                enabled: false,
            })
        ));
        let DarkroomControlMessage::OperationStack(message) = model
            .reset_module(selection(1), 41, Revision::from_u64(5))
            .expect("reset")
        else {
            panic!("reset must emit a stack update");
        };
        assert!(matches!(
            message.update,
            DarkroomOperationStackUpdate::ResetParameters { defaults, .. }
               if defaults.len() == 2
        ));
    }
    #[test]
    fn invalid_and_stale_actions_do_not_mutate_state() {
        let mut model = model();
        let invalid = model.set_parameter(
            selection(1),
            41,
            Revision::from_u64(3),
            "stops",
            DarkroomParameterValue::Bool(true),
        );
        assert!(matches!(
            invalid,
            Err(DarkroomControlModelError::Parameter(_))
        ));
        assert_eq!(
            model.module(41).expect("module").revision(),
            Revision::from_u64(3)
        );
        let stale = model.set_parameter(
            selection(0),
            41,
            Revision::from_u64(3),
            "stops",
            DarkroomParameterValue::Scalar(1.0),
        );
        assert!(matches!(
            stale,
            Err(DarkroomControlModelError::StaleSelection { .. })
        ));
        assert!(matches!(
            model.feedback(),
            DarkroomControlFeedback::StaleSelection { .. }
        ));
    }
}
