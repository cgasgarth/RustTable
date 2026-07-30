use std::fmt;

use rand::random;
use sha2::{Digest, Sha256};

use crate::event::DiagnosticsError;

const MAX_FIELD_KEY_BYTES: usize = 64;
const MAX_FIELD_VALUE_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

    #[must_use]
    pub const fn is_serializable(self) -> bool {
        !matches!(self, Self::Secret | Self::Payload)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FieldValue {
    Text(String),
    Integer(i64),
    Unsigned(u64),
    Boolean(bool),
    Float(String),
    Bytes(usize),
}

#[derive(Clone, Eq, PartialEq)]
pub struct DiagnosticField {
    pub(crate) key: String,
    pub(crate) privacy: PrivacyClass,
    value: FieldValue,
}

impl fmt::Debug for DiagnosticField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DiagnosticField")
            .field("key", &self.key)
            .field("privacy", &self.privacy)
            .field("value", &"[redacted]")
            .finish()
    }
}

impl DiagnosticField {
    /// # Errors
    ///
    /// Returns an error when the key or value exceeds its bound.
    pub fn public_text(key: &str, value: &str) -> Result<Self, DiagnosticsError> {
        Self::text(key, PrivacyClass::Public, value)
    }

    /// # Errors
    ///
    /// Returns an error when the key or value exceeds its bound.
    pub fn operational_text(key: &str, value: &str) -> Result<Self, DiagnosticsError> {
        Self::text(key, PrivacyClass::Operational, value)
    }

    /// # Errors
    ///
    /// Returns an error when the key or value exceeds its bound.
    pub fn private_text(key: &str, value: &str) -> Result<Self, DiagnosticsError> {
        Self::text(key, PrivacyClass::Private, value)
    }

    /// # Errors
    ///
    /// Returns an error when the key or value exceeds its bound.
    pub fn secret_text(key: &str, value: &str) -> Result<Self, DiagnosticsError> {
        Self::text(key, PrivacyClass::Secret, value)
    }

    /// # Errors
    ///
    /// Returns an error when the key or value exceeds its bound.
    pub fn payload_bytes(key: &str, value: &[u8]) -> Result<Self, DiagnosticsError> {
        Self::bounded(key, PrivacyClass::Payload, value.len()).map(|(key, privacy)| Self {
            key,
            privacy,
            value: FieldValue::Bytes(value.len()),
        })
    }

    /// # Errors
    ///
    /// Returns an error when the key is invalid.
    pub fn integer(key: &str, value: i64) -> Result<Self, DiagnosticsError> {
        Self::bounded(
            key,
            PrivacyClass::Operational,
            std::mem::size_of_val(&value),
        )
        .map(|(key, privacy)| Self {
            key,
            privacy,
            value: FieldValue::Integer(value),
        })
    }

    /// # Errors
    ///
    /// Returns an error when the key is invalid.
    pub fn unsigned(key: &str, value: u64) -> Result<Self, DiagnosticsError> {
        Self::bounded(
            key,
            PrivacyClass::Operational,
            std::mem::size_of_val(&value),
        )
        .map(|(key, privacy)| Self {
            key,
            privacy,
            value: FieldValue::Unsigned(value),
        })
    }

    /// # Errors
    ///
    /// Returns an error when the key is invalid.
    pub fn boolean(key: &str, value: bool) -> Result<Self, DiagnosticsError> {
        Self::bounded(key, PrivacyClass::Operational, 1).map(|(key, privacy)| Self {
            key,
            privacy,
            value: FieldValue::Boolean(value),
        })
    }

    /// # Errors
    ///
    /// Returns an error when the key is invalid or the value is non-finite.
    pub fn finite_float(key: &str, value: f64) -> Result<Self, DiagnosticsError> {
        if !value.is_finite() {
            return Err(DiagnosticsError::InvalidIdentifier("finite numeric field"));
        }
        Self::bounded(key, PrivacyClass::Operational, 8).map(|(key, privacy)| Self {
            key,
            privacy,
            value: FieldValue::Float(value.to_string()),
        })
    }

    /// # Errors
    ///
    /// Returns an error when the value exceeds its bound.
    pub fn path(value: &str) -> Result<Self, DiagnosticsError> {
        Self::private_text("path", value)
    }

    /// # Errors
    ///
    /// Returns an error when the value exceeds its bound.
    pub fn filename(value: &str) -> Result<Self, DiagnosticsError> {
        Self::private_text("filename", value)
    }

    /// # Errors
    ///
    /// Returns an error when the value exceeds its bound.
    pub fn url(value: &str) -> Result<Self, DiagnosticsError> {
        Self::private_text("url", value)
    }

    /// # Errors
    ///
    /// Returns an error when the value exceeds its bound.
    pub fn email(value: &str) -> Result<Self, DiagnosticsError> {
        Self::private_text("email", value)
    }

    /// # Errors
    ///
    /// Returns an error when the value exceeds its bound.
    pub fn camera_serial(value: &str) -> Result<Self, DiagnosticsError> {
        Self::private_text("camera_serial", value)
    }

    /// # Errors
    ///
    /// Returns an error when the value exceeds its bound.
    pub fn owner_name(value: &str) -> Result<Self, DiagnosticsError> {
        Self::private_text("owner_name", value)
    }

    /// # Errors
    ///
    /// Returns an error when the value exceeds its bound.
    pub fn gps(value: &str) -> Result<Self, DiagnosticsError> {
        Self::private_text("gps", value)
    }

    /// # Errors
    ///
    /// Returns an error when the value exceeds its bound.
    pub fn metadata(value: &str) -> Result<Self, DiagnosticsError> {
        Self::private_text("metadata", value)
    }

    /// # Errors
    ///
    /// Returns an error when the value exceeds its bound.
    pub fn free_text(value: &str) -> Result<Self, DiagnosticsError> {
        Self::private_text("free_text", value)
    }

    /// # Errors
    ///
    /// Returns an error when the value exceeds its bound.
    pub fn credential(value: &str) -> Result<Self, DiagnosticsError> {
        Self::secret_text("credential", value)
    }

    /// # Errors
    ///
    /// Returns an error when the value exceeds its bound.
    pub fn database_record(value: &str) -> Result<Self, DiagnosticsError> {
        Self::payload_bytes("database_record", value.as_bytes())
    }

    /// # Errors
    ///
    /// Returns an error when the value exceeds its bound.
    pub fn pixel_data(value: &[u8]) -> Result<Self, DiagnosticsError> {
        Self::payload_bytes("pixel_data", value)
    }

    /// # Errors
    ///
    /// Returns an error when the value exceeds its bound.
    pub fn driver_dump(value: &str) -> Result<Self, DiagnosticsError> {
        Self::payload_bytes("driver_dump", value.as_bytes())
    }

    #[must_use]
    pub const fn privacy(&self) -> PrivacyClass {
        self.privacy
    }

    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    fn text(key: &str, privacy: PrivacyClass, value: &str) -> Result<Self, DiagnosticsError> {
        Self::bounded(key, privacy, value.len()).map(|(key, privacy)| Self {
            key,
            privacy,
            value: FieldValue::Text(value.to_owned()),
        })
    }

    fn bounded(
        key: &str,
        privacy: PrivacyClass,
        bytes: usize,
    ) -> Result<(String, PrivacyClass), DiagnosticsError> {
        if key.is_empty() || key.len() > MAX_FIELD_KEY_BYTES || !key.is_ascii() {
            return Err(DiagnosticsError::InvalidIdentifier("field key"));
        }
        if bytes > MAX_FIELD_VALUE_BYTES {
            return Err(DiagnosticsError::FieldTooLarge);
        }
        Ok((key.to_owned(), privacy))
    }

    pub(crate) fn value(&self) -> &FieldValue {
        &self.value
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct Alias(String);

impl Alias {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Alias {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("Alias").field(&self.0).finish()
    }
}

#[derive(Clone)]
pub struct Redactor {
    key: [u8; 32],
}

impl Default for Redactor {
    fn default() -> Self {
        Self::new()
    }
}

impl Redactor {
    #[must_use]
    pub fn new() -> Self {
        Self { key: random() }
    }

    #[must_use]
    pub fn alias(&self, value: &str) -> Alias {
        let mut digest = Sha256::new();
        digest.update(self.key);
        digest.update(value.as_bytes());
        let bytes = digest.finalize();
        let mut alias = String::from("alias-");
        for byte in bytes.iter().take(12) {
            use std::fmt::Write;
            write!(&mut alias, "{byte:02x}").expect("String cannot fail");
        }
        Alias(alias)
    }
}

pub(crate) enum VisibleValue<'a> {
    Text(&'a str),
    Integer(i64),
    Unsigned(u64),
    Boolean(bool),
    Float(&'a str),
    PrivateAlias(Alias),
}

pub(crate) fn visible_value<'a>(
    field: &'a DiagnosticField,
    redactor: &Redactor,
) -> Option<VisibleValue<'a>> {
    match (&field.privacy, field.value()) {
        (PrivacyClass::Secret | PrivacyClass::Payload, _) | (_, FieldValue::Bytes(_)) => None,
        (PrivacyClass::Private, FieldValue::Text(value)) => {
            Some(VisibleValue::PrivateAlias(redactor.alias(value)))
        }
        (_, FieldValue::Text(value)) => Some(VisibleValue::Text(value)),
        (_, FieldValue::Integer(value)) => Some(VisibleValue::Integer(*value)),
        (_, FieldValue::Unsigned(value)) => Some(VisibleValue::Unsigned(*value)),
        (_, FieldValue::Boolean(value)) => Some(VisibleValue::Boolean(*value)),
        (_, FieldValue::Float(value)) => Some(VisibleValue::Float(value)),
    }
}
