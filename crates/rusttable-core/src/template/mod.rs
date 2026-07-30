mod ast;
mod builtins;
mod evaluate;
mod limits;
mod sanitize;
mod types;

pub use ast::{
    AST_SCHEMA_VERSION, Component, Expression, ParseError, ParseErrorKind, Span, Template,
    Transform,
};
pub use builtins::{BuiltinTemplate, BuiltinTemplateError};
pub use evaluate::{
    Cancellation, EncoderDescriptor, EvaluationError, EvaluationMode, EvaluationOptions,
    EvaluationReceipt, LogicalArtifactName,
};
pub use limits::TemplateLimits;
pub use sanitize::{
    SanitizeError, SanitizedComponent, SanitizerPolicy, sanitize_component,
    validate_relative_components,
};
pub use types::{
    Availability, Dimensions, PrivacyClass, Rational, TemplateContext, TemplateDateTime,
    TemplateValue, VariableDescriptor, VariableId, VariableRegistry, VariableType, VariableValue,
};

use std::fmt::Write;

use sha2::{Digest, Sha256};

pub(crate) fn hash_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}
