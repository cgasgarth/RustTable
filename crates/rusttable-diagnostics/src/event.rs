use std::fmt;

pub const SCHEMA_VERSION: u16 = 1;
pub const MAX_CODE_BYTES: usize = 64;
pub const MAX_FIELD_BYTES: usize = 65_536;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DiagnosticCode(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeError {
    Empty,
    TooLong,
    InvalidByte(u8),
}

impl DiagnosticCode {
    /// Creates a checked stable code.
    ///
    /// # Errors
    ///
    /// Returns [`CodeError`] for an empty, oversized, or non-ASCII code.
    pub fn new(value: impl Into<String>) -> Result<Self, CodeError> {
        let value = value.into();
        if value.is_empty() {
            return Err(CodeError::Empty);
        }
        if value.len() > MAX_CODE_BYTES {
            return Err(CodeError::TooLong);
        }
        if value.bytes().any(|byte| {
            !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte))
        }) {
            return Err(value
                .bytes()
                .find(|byte| {
                    !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(byte))
                })
                .map_or(CodeError::InvalidByte(0), CodeError::InvalidByte));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiagnosticSeverity {
    Trace,
    Debug,
    Info,
    Warning,
    Error,
    Critical,
}

impl DiagnosticSeverity {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Critical => "critical",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiagnosticSubsystem {
    Application,
    Catalog,
    Configuration,
    Import,
    Image,
    Metadata,
    Processing,
    Gpu,
    Ui,
    Storage,
    System,
}

impl DiagnosticSubsystem {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Application => "application",
            Self::Catalog => "catalog",
            Self::Configuration => "configuration",
            Self::Import => "import",
            Self::Image => "image",
            Self::Metadata => "metadata",
            Self::Processing => "processing",
            Self::Gpu => "gpu",
            Self::Ui => "ui",
            Self::Storage => "storage",
            Self::System => "system",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PrivacyClass {
    Public,
    Operational,
    Private,
    Secret,
    Payload,
}

impl PrivacyClass {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Operational => "operational",
            Self::Private => "private",
            Self::Secret => "secret",
            Self::Payload => "payload",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    Text(String),
    Unsigned(u64),
    Signed(i64),
    Float(f64),
    Boolean(bool),
}

impl FieldValue {
    pub(crate) fn encoded_len(&self) -> usize {
        match self {
            Self::Text(value) => value.len(),
            Self::Unsigned(_) | Self::Signed(_) | Self::Float(_) | Self::Boolean(_) => 24,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiagnosticField {
    name: String,
    value: FieldValue,
    privacy: PrivacyClass,
}

impl DiagnosticField {
    /// Creates a typed text field.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when the name or bounded value is invalid.
    pub fn text(
        name: impl Into<String>,
        value: impl Into<String>,
        privacy: PrivacyClass,
    ) -> Result<Self, FieldError> {
        let name = name.into();
        let value = value.into();
        validate_field(&name, &value)?;
        Ok(Self {
            privacy: effective_privacy(&name, privacy),
            name,
            value: FieldValue::Text(value),
        })
    }

    /// Creates an unsigned field.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when the name is invalid.
    pub fn unsigned(
        name: impl Into<String>,
        value: u64,
        privacy: PrivacyClass,
    ) -> Result<Self, FieldError> {
        Self::scalar(name, FieldValue::Unsigned(value), privacy)
    }
    /// Creates a signed field.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when the name is invalid.
    pub fn signed(
        name: impl Into<String>,
        value: i64,
        privacy: PrivacyClass,
    ) -> Result<Self, FieldError> {
        Self::scalar(name, FieldValue::Signed(value), privacy)
    }
    /// Creates a finite floating-point field.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] for a non-finite value or invalid name.
    pub fn float(
        name: impl Into<String>,
        value: f64,
        privacy: PrivacyClass,
    ) -> Result<Self, FieldError> {
        if !value.is_finite() {
            return Err(FieldError::NonFinite);
        }
        Self::scalar(name, FieldValue::Float(value), privacy)
    }
    /// Creates a boolean field.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when the name is invalid.
    pub fn boolean(
        name: impl Into<String>,
        value: bool,
        privacy: PrivacyClass,
    ) -> Result<Self, FieldError> {
        Self::scalar(name, FieldValue::Boolean(value), privacy)
    }

    fn scalar(
        name: impl Into<String>,
        value: FieldValue,
        privacy: PrivacyClass,
    ) -> Result<Self, FieldError> {
        let name = name.into();
        validate_field_name(&name)?;
        Ok(Self {
            privacy: effective_privacy(&name, privacy),
            name,
            value,
        })
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn value(&self) -> &FieldValue {
        &self.value
    }
    #[must_use]
    pub const fn privacy(&self) -> PrivacyClass {
        self.privacy
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldError {
    EmptyName,
    NameTooLong,
    InvalidName,
    TooLong,
    NonFinite,
}

impl fmt::Display for CodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("diagnostic code is empty"),
            Self::TooLong => formatter.write_str("diagnostic code is too long"),
            Self::InvalidByte(byte) => write!(formatter, "invalid diagnostic code byte {byte}"),
        }
    }
}
impl std::error::Error for CodeError {}
impl fmt::Display for FieldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyName => "diagnostic field name is empty",
            Self::NameTooLong => "diagnostic field name is too long",
            Self::InvalidName => "diagnostic field name is invalid",
            Self::TooLong => "diagnostic field value is too long",
            Self::NonFinite => "diagnostic field value is not finite",
        })
    }
}
impl std::error::Error for FieldError {}

fn validate_field_name(name: &str) -> Result<(), FieldError> {
    if name.is_empty() {
        return Err(FieldError::EmptyName);
    }
    if name.len() > 128 {
        return Err(FieldError::NameTooLong);
    }
    if name
        .bytes()
        .any(|byte| !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_.-".contains(&byte)))
    {
        return Err(FieldError::InvalidName);
    }
    Ok(())
}

fn validate_field(name: &str, value: &str) -> Result<(), FieldError> {
    validate_field_name(name)?;
    if value.len() > MAX_FIELD_BYTES {
        return Err(FieldError::TooLong);
    }
    Ok(())
}

fn effective_privacy(name: &str, requested: PrivacyClass) -> PrivacyClass {
    let name = name.to_ascii_lowercase();
    if [
        "token",
        "password",
        "secret",
        "cookie",
        "credential",
        "private_key",
    ]
    .iter()
    .any(|part| name.contains(part))
    {
        PrivacyClass::Secret
    } else if [
        "path",
        "filename",
        "url",
        "email",
        "serial",
        "owner",
        "gps",
        "metadata",
        "free_text",
        "database",
    ]
    .iter()
    .any(|part| name.contains(part))
    {
        requested.max(PrivacyClass::Private)
    } else {
        requested
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiagnosticContext {
    request: Option<String>,
    photo: Option<String>,
    edit: Option<String>,
    task: Option<String>,
    device: Option<String>,
}

impl DiagnosticContext {
    #[must_use]
    pub fn request(mut self, value: impl Into<String>) -> Self {
        self.request = bounded_alias(value.into());
        self
    }
    #[must_use]
    pub fn photo(mut self, value: impl Into<String>) -> Self {
        self.photo = bounded_alias(value.into());
        self
    }
    #[must_use]
    pub fn edit(mut self, value: impl Into<String>) -> Self {
        self.edit = bounded_alias(value.into());
        self
    }
    #[must_use]
    pub fn task(mut self, value: impl Into<String>) -> Self {
        self.task = bounded_alias(value.into());
        self
    }
    #[must_use]
    pub fn device(mut self, value: impl Into<String>) -> Self {
        self.device = bounded_alias(value.into());
        self
    }
    pub(crate) fn values(&self) -> [(&'static str, Option<&str>); 5] {
        [
            ("request", self.request.as_deref()),
            ("photo", self.photo.as_deref()),
            ("edit", self.edit.as_deref()),
            ("task", self.task.as_deref()),
            ("device", self.device.as_deref()),
        ]
    }
}

fn bounded_alias(value: String) -> Option<String> {
    (!value.is_empty() && value.len() <= 128 && !value.contains('\0')).then_some(value)
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiagnosticRecord {
    pub code: DiagnosticCode,
    pub severity: DiagnosticSeverity,
    pub subsystem: DiagnosticSubsystem,
    pub operation: String,
    pub context: DiagnosticContext,
    pub fields: Vec<DiagnosticField>,
}

impl DiagnosticRecord {
    /// Creates an empty typed diagnostic record.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when the operation name is invalid.
    pub fn new(
        code: DiagnosticCode,
        severity: DiagnosticSeverity,
        subsystem: DiagnosticSubsystem,
        operation: impl Into<String>,
    ) -> Result<Self, FieldError> {
        let operation = operation.into();
        validate_field_name(&operation)?;
        Ok(Self {
            code,
            severity,
            subsystem,
            operation,
            context: DiagnosticContext::default(),
            fields: Vec::new(),
        })
    }
    #[must_use]
    pub fn with_context(mut self, context: DiagnosticContext) -> Self {
        self.context = context;
        self
    }
    #[must_use]
    pub fn with_field(mut self, field: DiagnosticField) -> Self {
        self.fields.push(field);
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticEvent {
    Startup,
    Shutdown,
    ApplicationFailure(ApplicationFailureCode),
}

impl DiagnosticEvent {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::Shutdown => "shutdown",
            Self::ApplicationFailure(_) => "application_failure",
        }
    }
    pub(crate) fn record(self) -> DiagnosticRecord {
        let code = match self {
            Self::Startup => "application.startup",
            Self::Shutdown => "application.shutdown",
            Self::ApplicationFailure(_) => "application.failure",
        };
        DiagnosticRecord::new(
            DiagnosticCode::new(code).expect("static diagnostic code"),
            if matches!(self, Self::ApplicationFailure(_)) {
                DiagnosticSeverity::Error
            } else {
                DiagnosticSeverity::Info
            },
            DiagnosticSubsystem::Application,
            self.name(),
        )
        .expect("static diagnostic operation")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplicationFailureCode {
    IcedRun,
}

#[derive(Debug)]
pub enum DiagnosticsError {
    DirectoryUnavailable,
    InvalidPath,
    SymlinkRefused(&'static str),
    Storage(std::io::Error),
    StorageUnavailable,
    Poisoned,
}

impl fmt::Display for DiagnosticsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DirectoryUnavailable => formatter.write_str("diagnostics directory unavailable"),
            Self::InvalidPath => formatter.write_str("diagnostics path is invalid"),
            Self::SymlinkRefused(path) => write!(formatter, "diagnostics symlink refused: {path}"),
            Self::Storage(error) => write!(formatter, "diagnostics storage failure: {error}"),
            Self::StorageUnavailable => {
                formatter.write_str("all diagnostics sinks are unavailable")
            }
            Self::Poisoned => formatter.write_str("diagnostics storage lock poisoned"),
        }
    }
}
impl std::error::Error for DiagnosticsError {}
impl From<std::io::Error> for DiagnosticsError {
    fn from(error: std::io::Error) -> Self {
        Self::Storage(error)
    }
}
