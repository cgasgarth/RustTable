#[path = "edit.rs"]
mod aggregate;
mod operation;

pub use aggregate::{Edit, EditBuildError, EditRevisionError};
pub use operation::{
    Operation, OperationBuildError, OperationKey, OperationKeyError, ParameterName,
    ParameterNameError, ParameterText, ParameterTextError, ParameterValue,
};
