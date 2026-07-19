use std::fmt;

use super::limits::TemplateLimits;
use super::types::{VariableDescriptor, VariableId, VariableRegistry};

pub const AST_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transform {
    Sanitize,
    Slug,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expression {
    Literal(String),
    Variable {
        id: VariableId,
        format: Option<String>,
    },
    Fallback {
        primary: Box<Expression>,
        fallback: Box<Expression>,
    },
    Conditional {
        variable: VariableId,
        then_branch: Box<Expression>,
        else_branch: Box<Expression>,
    },
    Transform {
        transform: Transform,
        value: Box<Expression>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    pub(crate) expressions: Vec<Expression>,
}

impl Component {
    #[must_use]
    pub fn expressions(&self) -> &[Expression] {
        &self.expressions
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pub(crate) components: Vec<Component>,
}

impl Template {
    /// Parses a portable relative template with the standard variable registry.
    ///
    /// # Errors
    ///
    /// Returns a span-bearing error for malformed syntax or a violated limit.
    pub fn parse(source: &str) -> Result<Self, ParseError> {
        Self::parse_with_limits(
            source,
            &VariableRegistry::standard(),
            TemplateLimits::default(),
        )
    }

    /// Parses a template using an explicit registry and resource limits.
    ///
    /// # Errors
    ///
    /// Returns a span-bearing error for malformed syntax or a violated limit.
    pub fn parse_with_limits(
        source: &str,
        registry: &VariableRegistry,
        limits: TemplateLimits,
    ) -> Result<Self, ParseError> {
        let mut parser = Parser {
            source,
            registry,
            limits,
            expressions: 0,
            literal_bytes: 0,
            nesting: 0,
        };
        parser.parse_template()
    }

    #[must_use]
    pub fn components(&self) -> &[Component] {
        &self.components
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        AST_SCHEMA_VERSION
    }

    #[must_use]
    pub fn canonical_encoding(&self) -> String {
        let mut output = String::from("template-v1\0");
        for component in &self.components {
            output.push_str("component\0");
            for expression in &component.expressions {
                encode_expression(expression, &mut output);
            }
            output.push('\0');
        }
        output
    }

    #[must_use]
    pub fn ast_hash(&self) -> String {
        super::hash_hex(self.canonical_encoding().as_bytes())
    }

    #[must_use]
    pub fn referenced_variables(&self) -> Vec<VariableId> {
        let mut variables = Vec::new();
        for component in &self.components {
            for expression in &component.expressions {
                collect_variables(expression, &mut variables);
            }
        }
        variables.sort_by(|left, right| left.name().cmp(right.name()));
        variables.dedup();
        variables
    }
}

fn collect_variables(expression: &Expression, variables: &mut Vec<VariableId>) {
    match expression {
        Expression::Literal(_) => {}
        Expression::Variable { id, .. } => variables.push(id.clone()),
        Expression::Fallback { primary, fallback } => {
            collect_variables(primary, variables);
            collect_variables(fallback, variables);
        }
        Expression::Conditional {
            variable,
            then_branch,
            else_branch,
        } => {
            variables.push(variable.clone());
            collect_variables(then_branch, variables);
            collect_variables(else_branch, variables);
        }
        Expression::Transform { value, .. } => collect_variables(value, variables),
    }
}

fn encode_expression(expression: &Expression, output: &mut String) {
    match expression {
        Expression::Literal(value) => {
            output.push_str("literal:");
            output.push_str(&value.len().to_string());
            output.push(':');
            output.push_str(value);
        }
        Expression::Variable { id, format } => {
            output.push_str("variable:");
            output.push_str(id.name());
            output.push(':');
            output.push_str(format.as_deref().unwrap_or(""));
        }
        Expression::Fallback { primary, fallback } => {
            output.push_str("fallback:");
            encode_expression(primary, output);
            output.push(':');
            encode_expression(fallback, output);
        }
        Expression::Conditional {
            variable,
            then_branch,
            else_branch,
        } => {
            output.push_str("conditional:");
            output.push_str(variable.name());
            output.push(':');
            encode_expression(then_branch, output);
            output.push(':');
            encode_expression(else_branch, output);
        }
        Expression::Transform { transform, value } => {
            output.push_str("transform:");
            output.push_str(match transform {
                Transform::Sanitize => "sanitize",
                Transform::Slug => "slug",
            });
            output.push(':');
            encode_expression(value, output);
        }
    }
}

struct Parser<'a> {
    source: &'a str,
    registry: &'a VariableRegistry,
    limits: TemplateLimits,
    expressions: usize,
    literal_bytes: usize,
    nesting: usize,
}

impl Parser<'_> {
    fn parse_template(&mut self) -> Result<Template, ParseError> {
        if self.source.is_empty() {
            return Err(ParseError::new(ParseErrorKind::EmptyTemplate, 0, 0));
        }
        if self.source.starts_with('/') || self.source.starts_with('\\') {
            return Err(ParseError::new(ParseErrorKind::AbsolutePath, 0, 1));
        }
        if self.source.len() >= 2
            && self.source.as_bytes()[0].is_ascii_alphabetic()
            && self.source.as_bytes()[1] == b':'
        {
            return Err(ParseError::new(ParseErrorKind::AbsolutePath, 0, 2));
        }
        let mut components = Vec::new();
        let mut current = Vec::new();
        let mut literal = String::new();
        let mut index = 0;
        while index < self.source.len() {
            let character = self.source[index..]
                .chars()
                .next()
                .expect("valid UTF-8 index");
            match character {
                '/' => {
                    self.push_literal(&mut current, &mut literal, index)?;
                    self.push_component(&mut components, current, index)?;
                    current = Vec::new();
                    index += 1;
                }
                '\\' => {
                    index += character.len_utf8();
                    let Some(escaped) = self.source[index..].chars().next() else {
                        return Err(ParseError::new(
                            ParseErrorKind::TrailingEscape,
                            index - 1,
                            index,
                        ));
                    };
                    literal.push(escaped);
                    index += escaped.len_utf8();
                }
                '$' if self.source[index..].starts_with("$$") => {
                    literal.push('$');
                    index += 2;
                }
                '$' if self.source[index..].starts_with("${") => {
                    self.push_literal(&mut current, &mut literal, index)?;
                    let (inner, end) = self.braced(index + 2)?;
                    let inner = inner.to_owned();
                    self.push_expression(&mut current, &inner, index, end)?;
                    index = end + 1;
                }
                '$' => {
                    literal.push('$');
                    index += 1;
                }
                _ => {
                    literal.push(character);
                    index += character.len_utf8();
                }
            }
        }
        self.push_literal(&mut current, &mut literal, self.source.len())?;
        self.push_component(&mut components, current, self.source.len())?;
        validate_extension_position(&components)?;
        Ok(Template { components })
    }

    fn braced(&self, start: usize) -> Result<(&str, usize), ParseError> {
        let mut depth = 0usize;
        let mut index = start;
        while index < self.source.len() {
            if self.source[index..].starts_with("${") {
                depth += 1;
                index += 2;
                continue;
            }
            let character = self.source[index..]
                .chars()
                .next()
                .expect("valid UTF-8 index");
            if character == '}' {
                if depth == 0 {
                    return Ok((&self.source[start..index], index));
                }
                depth -= 1;
            }
            index += character.len_utf8();
        }
        Err(ParseError::new(
            ParseErrorKind::UnclosedExpression,
            start - 2,
            self.source.len(),
        ))
    }

    fn push_literal(
        &mut self,
        component: &mut Vec<Expression>,
        literal: &mut String,
        span: usize,
    ) -> Result<(), ParseError> {
        if literal.is_empty() {
            return Ok(());
        }
        self.literal_bytes = self
            .literal_bytes
            .checked_add(literal.len())
            .ok_or_else(|| ParseError::new(ParseErrorKind::LimitExceeded, span, span))?;
        if self.literal_bytes > self.limits.literal_bytes {
            return Err(ParseError::new(ParseErrorKind::LimitExceeded, span, span));
        }
        component.push(Expression::Literal(std::mem::take(literal)));
        Ok(())
    }

    fn push_expression(
        &mut self,
        component: &mut Vec<Expression>,
        inner: &str,
        start: usize,
        end: usize,
    ) -> Result<(), ParseError> {
        self.expressions += 1;
        if self.expressions > self.limits.expressions {
            return Err(ParseError::new(ParseErrorKind::LimitExceeded, start, end));
        }
        component.push(self.parse_expression(inner, start + 2)?);
        Ok(())
    }

    fn push_component(
        &self,
        components: &mut Vec<Component>,
        expressions: Vec<Expression>,
        span: usize,
    ) -> Result<(), ParseError> {
        if expressions.is_empty() {
            return Err(ParseError::new(ParseErrorKind::EmptyComponent, span, span));
        }
        if expressions.len() == 1
            && matches!(expressions.first(), Some(Expression::Literal(value)) if value == "." || value == "..")
        {
            return Err(ParseError::new(ParseErrorKind::Traversal, span, span));
        }
        if components.len() >= self.limits.components {
            return Err(ParseError::new(ParseErrorKind::LimitExceeded, span, span));
        }
        components.push(Component { expressions });
        Ok(())
    }

    fn parse_expression(&mut self, source: &str, offset: usize) -> Result<Expression, ParseError> {
        self.nesting += 1;
        if self.nesting > self.limits.nesting {
            self.nesting -= 1;
            return Err(ParseError::new(
                ParseErrorKind::LimitExceeded,
                offset,
                offset + source.len(),
            ));
        }
        let result = self.parse_expression_inner(source, offset);
        self.nesting -= 1;
        result
    }

    fn parse_expression_inner(
        &mut self,
        source: &str,
        offset: usize,
    ) -> Result<Expression, ParseError> {
        if source.is_empty() {
            return Err(ParseError::new(
                ParseErrorKind::EmptyExpression,
                offset,
                offset,
            ));
        }
        if let Some(parts) = split_top_level(source, '|') {
            if parts.len() != 2 {
                return Err(ParseError::new(
                    ParseErrorKind::InvalidFallback,
                    offset,
                    offset + source.len(),
                ));
            }
            return Ok(Expression::Fallback {
                primary: Box::new(self.parse_atom(parts[0], offset)?),
                fallback: Box::new(self.parse_atom(parts[1], offset + parts[0].len() + 1)?),
            });
        }
        if let Some(rest) = source.strip_prefix('?') {
            let parts = split_top_level(rest, ':').ok_or_else(|| {
                ParseError::new(
                    ParseErrorKind::InvalidConditional,
                    offset,
                    offset + source.len(),
                )
            })?;
            if parts.len() != 3 {
                return Err(ParseError::new(
                    ParseErrorKind::InvalidConditional,
                    offset,
                    offset + source.len(),
                ));
            }
            let variable = self.variable(parts[0], offset + 1)?;
            return Ok(Expression::Conditional {
                variable,
                then_branch: Box::new(self.parse_atom(parts[1], offset + 1 + parts[0].len() + 1)?),
                else_branch: Box::new(
                    self.parse_atom(parts[2], offset + source.len() - parts[2].len())?,
                ),
            });
        }
        if let Some(parts) = split_top_level(source, ':') {
            if parts.len() != 2 {
                return Err(ParseError::new(
                    ParseErrorKind::InvalidExpression,
                    offset,
                    offset + source.len(),
                ));
            }
            if let Some(transform) = match parts[0] {
                "sanitize" => Some(Transform::Sanitize),
                "slug" => Some(Transform::Slug),
                _ => None,
            } {
                return Ok(Expression::Transform {
                    transform,
                    value: Box::new(self.parse_atom(parts[1], offset + parts[0].len() + 1)?),
                });
            }
            let id = self.variable(parts[0], offset)?;
            if parts[1].len() > self.limits.format_width {
                return Err(ParseError::new(
                    ParseErrorKind::FormatTooWide,
                    offset,
                    offset + source.len(),
                ));
            }
            if parts[1].contains('$') || parts[1].contains('}') {
                return Err(ParseError::new(
                    ParseErrorKind::InvalidFormat,
                    offset,
                    offset + source.len(),
                ));
            }
            return Ok(Expression::Variable {
                id,
                format: Some(parts[1].to_owned()),
            });
        }
        Ok(Expression::Variable {
            id: self.variable(source, offset)?,
            format: None,
        })
    }

    fn parse_atom(&mut self, source: &str, offset: usize) -> Result<Expression, ParseError> {
        let source = source.trim();
        if source.starts_with("${") && source.ends_with('}') {
            return self.parse_expression(&source[2..source.len() - 1], offset + 2);
        }
        if let Some(id) = self
            .registry
            .resolve(source)
            .map(|descriptor| descriptor.id)
        {
            return Ok(Expression::Variable { id, format: None });
        }
        if source.contains("${") || source.contains('}') {
            return Err(ParseError::new(
                ParseErrorKind::InvalidExpression,
                offset,
                offset + source.len(),
            ));
        }
        Ok(Expression::Literal(source.to_owned()))
    }

    fn variable(&self, source: &str, offset: usize) -> Result<VariableId, ParseError> {
        self.registry
            .resolve(source.trim())
            .map(|descriptor: VariableDescriptor| descriptor.id)
            .ok_or_else(|| {
                ParseError::new(
                    ParseErrorKind::UnknownVariable,
                    offset,
                    offset + source.len(),
                )
            })
    }
}

fn split_top_level(source: &str, delimiter: char) -> Option<Vec<&str>> {
    let mut depth = 0usize;
    let mut start = 0;
    let mut parts = Vec::new();
    let mut index = 0;
    while index < source.len() {
        if source[index..].starts_with("${") {
            depth += 1;
            index += 2;
            continue;
        }
        let character = source[index..].chars().next()?;
        if character == '}' && depth > 0 {
            depth -= 1;
        } else if character == delimiter && depth == 0 {
            parts.push(&source[start..index]);
            start = index + character.len_utf8();
        }
        index += character.len_utf8();
    }
    if parts.is_empty() {
        None
    } else {
        parts.push(&source[start..]);
        Some(parts)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseErrorKind {
    EmptyTemplate,
    EmptyComponent,
    EmptyExpression,
    AbsolutePath,
    Traversal,
    TrailingEscape,
    UnclosedExpression,
    UnknownVariable,
    InvalidExpression,
    InvalidFallback,
    InvalidConditional,
    InvalidFormat,
    ExtensionPosition,
    FormatTooWide,
    LimitExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub kind: ParseErrorKind,
    pub span: Span,
}

impl ParseError {
    const fn new(kind: ParseErrorKind, start: usize, end: usize) -> Self {
        Self {
            kind,
            span: Span { start, end },
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "template parse error {:?} at {}..{}",
            self.kind, self.span.start, self.span.end
        )
    }
}

fn validate_extension_position(components: &[Component]) -> Result<(), ParseError> {
    for (component_index, component) in components.iter().enumerate() {
        for (expression_index, expression) in component.expressions.iter().enumerate() {
            if contains_variable(expression, &VariableId::Extension)
                && (component_index + 1 != components.len()
                    || expression_index + 1 != component.expressions.len())
            {
                return Err(ParseError::new(ParseErrorKind::ExtensionPosition, 0, 0));
            }
        }
    }
    Ok(())
}

fn contains_variable(expression: &Expression, id: &VariableId) -> bool {
    match expression {
        Expression::Literal(_) => false,
        Expression::Variable { id: variable, .. } => variable == id,
        Expression::Fallback { primary, fallback } => {
            contains_variable(primary, id) || contains_variable(fallback, id)
        }
        Expression::Conditional {
            variable,
            then_branch,
            else_branch,
        } => {
            variable == id
                || contains_variable(then_branch, id)
                || contains_variable(else_branch, id)
        }
        Expression::Transform { value, .. } => contains_variable(value, id),
    }
}

impl std::error::Error for ParseError {}
