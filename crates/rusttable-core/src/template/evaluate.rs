use std::collections::BTreeSet;
use std::fmt;

use unicode_normalization::UnicodeNormalization;

use super::ast::{Expression, Template, Transform};
use super::limits::TemplateLimits;
use super::sanitize::{SanitizerPolicy, sanitize_component, validate_relative_components};
use super::types::{
    Availability, Dimensions, Rational, TemplateContext, TemplateDateTime, TemplateValue,
    VariableId, VariableType,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncoderDescriptor {
    pub name: String,
    pub extension: String,
    pub allowed_extensions: BTreeSet<String>,
}

impl EncoderDescriptor {
    pub fn new(name: impl Into<String>, extension: impl Into<String>, allowed: &[&str]) -> Self {
        let extension = extension
            .into()
            .trim_start_matches('.')
            .to_ascii_lowercase();
        let mut allowed_extensions = BTreeSet::new();
        for value in allowed {
            allowed_extensions.insert(value.trim_start_matches('.').to_ascii_lowercase());
        }
        if allowed_extensions.is_empty() {
            allowed_extensions.insert(extension.clone());
        }
        Self {
            name: name.into(),
            extension,
            allowed_extensions,
        }
    }

    #[must_use]
    pub fn accepts(&self, extension: &str) -> bool {
        self.allowed_extensions
            .contains(&extension.trim_start_matches('.').to_ascii_lowercase())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvaluationMode {
    Actual,
    Preview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvaluationOptions {
    pub limits: TemplateLimits,
    pub sanitizer: SanitizerPolicy,
    pub mode: EvaluationMode,
}

impl Default for EvaluationOptions {
    fn default() -> Self {
        Self {
            limits: TemplateLimits::default(),
            sanitizer: SanitizerPolicy::default(),
            mode: EvaluationMode::Actual,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalArtifactName {
    pub components: Vec<String>,
    pub relative_path: String,
    pub evaluation_hash: String,
    pub redacted_display_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationReceipt {
    pub schema_version: u16,
    pub ast_hash: String,
    pub evaluation_hash: String,
    pub component_hashes: Vec<String>,
    pub display_path: String,
    pub privacy_redacted: bool,
}

impl EvaluationReceipt {
    #[must_use]
    pub fn stable_encoding(&self) -> String {
        format!(
            "template-receipt-v{}\nast={}\neval={}\ncomponents={}\ndisplay={}\nredacted={}\n",
            self.schema_version,
            self.ast_hash,
            self.evaluation_hash,
            self.component_hashes.join(","),
            self.display_path,
            self.privacy_redacted
        )
    }

    #[must_use]
    pub fn receipt_hash(&self) -> String {
        super::hash_hex(self.stable_encoding().as_bytes())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvaluationError {
    MissingVariable {
        id: VariableId,
    },
    RedactedVariable {
        id: VariableId,
    },
    TypeMismatch {
        id: VariableId,
        expected: &'static str,
    },
    InvalidFormat {
        id: VariableId,
        format: String,
    },
    InvalidExtension {
        extension: String,
    },
    OutputTooLong,
    TooManyExpansions,
    Sanitization(String),
    Cancelled,
}

impl fmt::Display for EvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "template evaluation failed: {self:?}")
    }
}

impl std::error::Error for EvaluationError {}

pub trait Cancellation {
    fn is_cancelled(&self) -> bool;
}

impl Template {
    /// Evaluates a template against one immutable snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error for missing or redacted values, invalid formats or extensions,
    /// invalid output, limits, or cancellation.
    pub fn evaluate(
        &self,
        context: &TemplateContext,
        encoder: Option<&EncoderDescriptor>,
    ) -> Result<(LogicalArtifactName, EvaluationReceipt), EvaluationError> {
        self.evaluate_with(context, encoder, EvaluationOptions::default(), None)
    }

    /// Evaluates with explicit bounds and an optional cooperative cancellation source.
    ///
    /// # Errors
    ///
    /// Returns an error for missing or redacted values, invalid formats or extensions,
    /// invalid output, limits, or cancellation.
    pub fn evaluate_with(
        &self,
        context: &TemplateContext,
        encoder: Option<&EncoderDescriptor>,
        options: EvaluationOptions,
        cancellation: Option<&dyn Cancellation>,
    ) -> Result<(LogicalArtifactName, EvaluationReceipt), EvaluationError> {
        let mut expanded = Vec::with_capacity(self.components.len());
        let mut private_components = Vec::with_capacity(self.components.len());
        let mut expansion_count = 0usize;
        for component in &self.components {
            if cancellation.is_some_and(Cancellation::is_cancelled) {
                return Err(EvaluationError::Cancelled);
            }
            let mut value = String::new();
            let mut private = false;
            for expression in &component.expressions {
                let result = evaluate_expression(
                    expression,
                    context,
                    encoder,
                    options,
                    &mut expansion_count,
                )?;
                if result.availability == Availability::Redacted {
                    let Some(id) = result.id else {
                        return Err(EvaluationError::MissingVariable {
                            id: VariableId::Title,
                        });
                    };
                    return Err(EvaluationError::RedactedVariable { id });
                }
                value.push_str(&result.text);
                private |= result.private;
            }
            let sanitized = sanitize_component(&value, options.sanitizer)
                .map_err(|error| EvaluationError::Sanitization(error.to_string()))?;
            expanded.push(sanitized.value);
            private_components.push(private);
        }
        validate_relative_components(&expanded, options.sanitizer)
            .map_err(|error| EvaluationError::Sanitization(error.to_string()))?;
        let relative_path = expanded.join("/");
        if relative_path.len() > options.limits.output_bytes {
            return Err(EvaluationError::OutputTooLong);
        }
        if let Some(encoder) = encoder
            && let Some(extension) = relative_path
                .rsplit('/')
                .next()
                .and_then(|name| name.rsplit_once('.'))
                .map(|(_, extension)| extension)
            && !encoder.accepts(extension)
        {
            return Err(EvaluationError::InvalidExtension {
                extension: extension.to_owned(),
            });
        }
        let evaluation_hash = super::hash_hex(relative_path.as_bytes());
        let redacted_display_path = expanded
            .iter()
            .zip(private_components.iter())
            .map(|(value, private)| {
                if *private && options.mode == EvaluationMode::Actual {
                    "[redacted]"
                } else {
                    value
                }
            })
            .collect::<Vec<_>>()
            .join("/");
        let receipt = EvaluationReceipt {
            schema_version: 1,
            ast_hash: self.ast_hash(),
            evaluation_hash: evaluation_hash.clone(),
            component_hashes: expanded
                .iter()
                .map(|value| super::hash_hex(value.as_bytes()))
                .collect(),
            display_path: redacted_display_path.clone(),
            privacy_redacted: private_components.iter().any(|private| *private),
        };
        Ok((
            LogicalArtifactName {
                components: expanded,
                relative_path,
                evaluation_hash,
                redacted_display_path,
            },
            receipt,
        ))
    }
}

#[derive(Debug)]
struct Evaluated {
    availability: Availability,
    text: String,
    private: bool,
    id: Option<VariableId>,
}

fn evaluate_expression(
    expression: &Expression,
    context: &TemplateContext,
    encoder: Option<&EncoderDescriptor>,
    options: EvaluationOptions,
    expansions: &mut usize,
) -> Result<Evaluated, EvaluationError> {
    *expansions += 1;
    if *expansions > options.limits.expansions {
        return Err(EvaluationError::TooManyExpansions);
    }
    match expression {
        Expression::Literal(value) => Ok(Evaluated {
            availability: Availability::Available,
            text: value.clone(),
            private: false,
            id: None,
        }),
        Expression::Variable { id, format } => {
            evaluate_variable(id, format.as_deref(), context, encoder, options.limits)
        }
        Expression::Fallback { primary, fallback } => {
            let first = evaluate_expression(primary, context, encoder, options, expansions);
            match first {
                Ok(value) if value.availability != Availability::Missing => Ok(value),
                Err(EvaluationError::MissingVariable { .. }) => {
                    evaluate_expression(fallback, context, encoder, options, expansions)
                }
                Ok(_) => first,
                Err(error) => Err(error),
            }
        }
        Expression::Conditional {
            variable,
            then_branch,
            else_branch,
        } => {
            let value = context.get(variable);
            match value.availability {
                Availability::Available => {
                    evaluate_expression(then_branch, context, encoder, options, expansions)
                }
                Availability::Missing => {
                    evaluate_expression(else_branch, context, encoder, options, expansions)
                }
                Availability::Redacted => Ok(Evaluated {
                    availability: Availability::Redacted,
                    text: String::new(),
                    private: true,
                    id: Some(variable.clone()),
                }),
            }
        }
        Expression::Transform { transform, value } => {
            let mut result = evaluate_expression(value, context, encoder, options, expansions)?;
            if result.availability == Availability::Available {
                result.text = match transform {
                    Transform::Sanitize => {
                        sanitize_component(&result.text, options.sanitizer)
                            .map_err(|error| EvaluationError::Sanitization(error.to_string()))?
                            .value
                    }
                    Transform::Slug => super::sanitize::slugify(&result.text),
                };
            }
            Ok(result)
        }
    }
}

fn evaluate_variable(
    id: &VariableId,
    format: Option<&str>,
    context: &TemplateContext,
    encoder: Option<&EncoderDescriptor>,
    limits: TemplateLimits,
) -> Result<Evaluated, EvaluationError> {
    let value =
        if *id == VariableId::Extension && context.get(id).availability == Availability::Missing {
            encoder.map_or_else(super::types::VariableValue::missing, |item| {
                super::types::VariableValue::available(TemplateValue::Text(item.extension.clone()))
            })
        } else {
            context.get(id)
        };
    match value.availability {
        Availability::Missing => Err(EvaluationError::MissingVariable { id: id.clone() }),
        Availability::Redacted => Ok(Evaluated {
            availability: Availability::Redacted,
            text: String::new(),
            private: true,
            id: Some(id.clone()),
        }),
        Availability::Available => {
            let Some(actual) = value.value.as_ref() else {
                return Err(EvaluationError::TypeMismatch {
                    id: id.clone(),
                    expected: "a value",
                });
            };
            if !value_matches(id.value_type(), actual) {
                return Err(EvaluationError::TypeMismatch {
                    id: id.clone(),
                    expected: "the registered variable type",
                });
            }
            let text = format_value(id, actual, format, encoder, limits)?;
            if *id == VariableId::Extension
                && let Some(encoder) = encoder
                && !encoder.accepts(&text)
            {
                return Err(EvaluationError::InvalidExtension { extension: text });
            }
            Ok(Evaluated {
                availability: Availability::Available,
                text,
                private: is_private(id),
                id: None,
            })
        }
    }
}

fn value_matches(expected: VariableType, actual: &TemplateValue) -> bool {
    match expected {
        VariableType::Text => matches!(actual, TemplateValue::Text(_)),
        VariableType::Integer => matches!(actual, TemplateValue::Integer(_)),
        VariableType::Rational => matches!(actual, TemplateValue::Rational(_)),
        VariableType::DateTime => matches!(actual, TemplateValue::DateTime(_)),
        VariableType::Dimensions => matches!(actual, TemplateValue::Dimensions(_)),
        VariableType::Tags => matches!(actual, TemplateValue::Tags(_)),
        VariableType::Enum => matches!(actual, TemplateValue::Enum(_) | TemplateValue::Text(_)),
        VariableType::Hash => matches!(actual, TemplateValue::Hash(_) | TemplateValue::Text(_)),
    }
}

fn format_value(
    id: &VariableId,
    value: &TemplateValue,
    format: Option<&str>,
    encoder: Option<&EncoderDescriptor>,
    limits: TemplateLimits,
) -> Result<String, EvaluationError> {
    let format = format.unwrap_or("");
    match value {
        TemplateValue::Text(text) | TemplateValue::Enum(text) => {
            if format == "lower" {
                Ok(text.to_lowercase())
            } else if format == "upper" {
                Ok(text.to_uppercase())
            } else if format.is_empty() {
                Ok(text.clone())
            } else {
                Err(EvaluationError::InvalidFormat {
                    id: id.clone(),
                    format: format.to_owned(),
                })
            }
        }
        TemplateValue::Hash(text) => format_hash(id, text, format),
        TemplateValue::Integer(number) => format_integer(id, *number, format),
        TemplateValue::Rational(rational) => format_rational(id, *rational, format),
        TemplateValue::Dimensions(dimensions) => format_dimensions(id, *dimensions, format),
        TemplateValue::Tags(tags) => format_tags(id, tags, format, limits.tag_values),
        TemplateValue::DateTime(date_time) => format_datetime(id, date_time, format),
    }
    .map(|text| {
        if *id == VariableId::Extension && text.is_empty() {
            encoder.map_or_else(String::new, |item| item.extension.clone())
        } else {
            text
        }
    })
}

fn format_integer(id: &VariableId, number: i64, format: &str) -> Result<String, EvaluationError> {
    if format.is_empty() {
        return Ok(number.to_string());
    }
    let (base, uppercase) = match format {
        "x" => (16, false),
        "X" => (16, true),
        _ => (10, false),
    };
    let digits = if base == 16 {
        if uppercase {
            format!("{number:X}")
        } else {
            format!("{number:x}")
        }
    } else {
        number.to_string()
    };
    let width = format
        .strip_prefix('0')
        .and_then(|value| value.parse::<usize>().ok())
        .or_else(|| format.parse().ok());
    if format == "sign" {
        return Ok(if number >= 0 {
            format!("+{number}")
        } else {
            number.to_string()
        });
    }
    if format != "x" && format != "X" && width.is_none() {
        return Err(EvaluationError::InvalidFormat {
            id: id.clone(),
            format: format.to_owned(),
        });
    }
    Ok(if let Some(width) = width {
        format!("{digits:0>width$}")
    } else {
        digits
    })
}

fn format_rational(
    id: &VariableId,
    value: Rational,
    format: &str,
) -> Result<String, EvaluationError> {
    if format.is_empty() || format == "fraction" {
        return Ok(format!("{}/{}", value.numerator, value.denominator));
    }
    let precision = format
        .strip_prefix('.')
        .and_then(|part| part.parse::<usize>().ok());
    let Some(precision) = precision else {
        return Err(EvaluationError::InvalidFormat {
            id: id.clone(),
            format: format.to_owned(),
        });
    };
    let Some(power) = u32::try_from(precision).ok() else {
        return Err(EvaluationError::InvalidFormat {
            id: id.clone(),
            format: format.to_owned(),
        });
    };
    let scaled = (i64::from(value.numerator) * 10_i64.pow(power)) / i64::from(value.denominator);
    let negative = scaled < 0;
    let absolute = scaled.unsigned_abs();
    let scale = 10_u64.pow(power);
    let whole = absolute / scale;
    let fraction = absolute % scale;
    Ok(format!(
        "{}{whole}.{fraction:0>width$}",
        if negative { "-" } else { "" },
        width = precision
    ))
}

fn format_dimensions(
    id: &VariableId,
    dimensions: Dimensions,
    format: &str,
) -> Result<String, EvaluationError> {
    if format.is_empty() || format == "size" {
        return Ok(format!("{}x{}", dimensions.width, dimensions.height));
    }
    if format != "aspect" {
        return Err(EvaluationError::InvalidFormat {
            id: id.clone(),
            format: format.to_owned(),
        });
    }
    let divisor = gcd(dimensions.width, dimensions.height);
    Ok(format!(
        "{}:{}",
        dimensions.width / divisor,
        dimensions.height / divisor
    ))
}

fn format_tags(
    id: &VariableId,
    tags: &[String],
    format: &str,
    max_values: usize,
) -> Result<String, EvaluationError> {
    if tags.len() > max_values {
        return Err(EvaluationError::TooManyExpansions);
    }
    let mut tags = tags.to_vec();
    tags.sort_by_key(|value| {
        value
            .nfc()
            .map(|(character, _)| character)
            .collect::<String>()
    });
    let separator = format.strip_prefix("join=").unwrap_or(",");
    if separator.len() > 16 {
        return Err(EvaluationError::InvalidFormat {
            id: id.clone(),
            format: format.to_owned(),
        });
    }
    Ok(tags.join(separator))
}

fn format_hash(id: &VariableId, value: &str, format: &str) -> Result<String, EvaluationError> {
    if format.is_empty() || format == "lower" {
        return Ok(value.to_ascii_lowercase());
    }
    if format == "upper" {
        return Ok(value.to_ascii_uppercase());
    }
    let prefix = format
        .strip_prefix("prefix=")
        .unwrap_or(format)
        .parse::<usize>()
        .ok();
    let Some(prefix) = prefix.filter(|length| *length <= value.len()) else {
        return Err(EvaluationError::InvalidFormat {
            id: id.clone(),
            format: format.to_owned(),
        });
    };
    Ok(value[..prefix].to_owned())
}

fn format_datetime(
    id: &VariableId,
    date_time: &TemplateDateTime,
    format: &str,
) -> Result<String, EvaluationError> {
    if format.is_empty() || format == "date" {
        return Ok(format!(
            "{:04}-{:02}-{:02}",
            date_time.year, date_time.month, date_time.day
        ));
    }
    if format == "time" {
        return Ok(format!(
            "{:02}-{:02}-{:02}",
            date_time.hour, date_time.minute, date_time.second
        )
        .replace('-', ":"));
    }
    let mut output = String::new();
    let mut chars = format.chars();
    while let Some(character) = chars.next() {
        if character != '%' {
            output.push(character);
            continue;
        }
        let Some(token) = chars.next() else {
            return Err(EvaluationError::InvalidFormat {
                id: id.clone(),
                format: format.to_owned(),
            });
        };
        let replacement = match token {
            'Y' => return_padded(date_time.year, 4),
            'y' => return_padded(date_time.year.rem_euclid(100), 2),
            'm' => return_padded(i32::from(date_time.month), 2),
            'd' => return_padded(i32::from(date_time.day), 2),
            'H' => return_padded(i32::from(date_time.hour), 2),
            'M' => return_padded(i32::from(date_time.minute), 2),
            'S' => return_padded(i32::from(date_time.second), 2),
            'z' => {
                let sign = if date_time.offset_minutes < 0 {
                    '-'
                } else {
                    '+'
                };
                let minutes = date_time.offset_minutes.unsigned_abs();
                format!("{sign}{:02}{:02}", minutes / 60, minutes % 60)
            }
            _ => {
                return Err(EvaluationError::InvalidFormat {
                    id: id.clone(),
                    format: format.to_owned(),
                });
            }
        };
        output.push_str(&replacement);
    }
    Ok(output)
}

fn return_padded(value: i32, width: usize) -> String {
    format!("{value:0>width$}")
}
fn gcd(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}
fn is_private(id: &VariableId) -> bool {
    matches!(
        id,
        VariableId::SourceRelativeImportPath
            | VariableId::Title
            | VariableId::Creator
            | VariableId::Copyright
            | VariableId::MetadataArtist
            | VariableId::MetadataDescription
            | VariableId::MetadataLocation
            | VariableId::MetadataKeywords
            | VariableId::Camera
            | VariableId::Lens
            | VariableId::Tags
    )
}
