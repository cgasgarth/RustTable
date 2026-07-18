use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use crate::{FiniteF64, OperationId, OperationOpacity};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationKey(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKeyError {
    Length {
        actual: usize,
        max: usize,
    },
    SegmentCount {
        actual: usize,
    },
    EmptySegment {
        segment: usize,
    },
    SegmentLength {
        segment: usize,
        actual: usize,
        max: usize,
    },
    NonAscii {
        byte_index: usize,
        byte: u8,
    },
    InvalidInitialByte {
        segment: usize,
        byte_index: usize,
        byte: u8,
    },
    InvalidSubsequentByte {
        segment: usize,
        byte_index: usize,
        byte: u8,
    },
}

impl OperationKey {
    /// Validates and stores an operation key without normalization.
    ///
    /// # Errors
    ///
    /// Returns an [`OperationKeyError`] describing the first grammar violation.
    pub fn new(value: impl Into<String>) -> Result<Self, OperationKeyError> {
        Self::try_from(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for OperationKey {
    type Error = OperationKeyError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        validate_operation_key(&value)?;
        Ok(Self(value))
    }
}

impl TryFrom<&str> for OperationKey {
    type Error = OperationKeyError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from(value.to_owned())
    }
}

impl FromStr for OperationKey {
    type Err = OperationKeyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value)
    }
}

impl fmt::Display for OperationKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

fn validate_operation_key(value: &str) -> Result<(), OperationKeyError> {
    if value.is_empty() {
        return Err(OperationKeyError::EmptySegment { segment: 0 });
    }
    if let Some((byte_index, byte)) = value.bytes().enumerate().find(|(_, byte)| !byte.is_ascii()) {
        return Err(OperationKeyError::NonAscii { byte_index, byte });
    }
    if value.len() > 128 {
        return Err(OperationKeyError::Length {
            actual: value.len(),
            max: 128,
        });
    }
    let segments = value.split('.').collect::<Vec<_>>();
    if !(2..=4).contains(&segments.len()) {
        return Err(OperationKeyError::SegmentCount {
            actual: segments.len(),
        });
    }
    let mut byte_offset = 0;
    for (segment_index, segment) in segments.iter().enumerate() {
        if segment.is_empty() {
            return Err(OperationKeyError::EmptySegment {
                segment: segment_index,
            });
        }
        if segment.len() > 32 {
            return Err(OperationKeyError::SegmentLength {
                segment: segment_index,
                actual: segment.len(),
                max: 32,
            });
        }
        let bytes = segment.as_bytes();
        if !bytes[0].is_ascii_lowercase() {
            return Err(OperationKeyError::InvalidInitialByte {
                segment: segment_index,
                byte_index: byte_offset,
                byte: bytes[0],
            });
        }
        for (position, byte) in bytes.iter().copied().enumerate().skip(1) {
            if !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_') {
                return Err(OperationKeyError::InvalidSubsequentByte {
                    segment: segment_index,
                    byte_index: byte_offset + position,
                    byte,
                });
            }
        }
        byte_offset += segment.len() + 1;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParameterName(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterNameError {
    Empty,
    Length { actual: usize, max: usize },
    NonAscii { byte_index: usize, byte: u8 },
    InvalidInitialByte { byte_index: usize, byte: u8 },
    InvalidSubsequentByte { byte_index: usize, byte: u8 },
}

impl ParameterName {
    /// Validates and stores a parameter name without normalization.
    ///
    /// # Errors
    ///
    /// Returns a [`ParameterNameError`] describing the first grammar violation.
    pub fn new(value: impl Into<String>) -> Result<Self, ParameterNameError> {
        Self::try_from(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ParameterName {
    type Error = ParameterNameError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        validate_parameter_name(&value)?;
        Ok(Self(value))
    }
}

impl TryFrom<&str> for ParameterName {
    type Error = ParameterNameError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from(value.to_owned())
    }
}

impl FromStr for ParameterName {
    type Err = ParameterNameError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value)
    }
}

impl fmt::Display for ParameterName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

fn validate_parameter_name(value: &str) -> Result<(), ParameterNameError> {
    if value.is_empty() {
        return Err(ParameterNameError::Empty);
    }
    if let Some((byte_index, byte)) = value.bytes().enumerate().find(|(_, byte)| !byte.is_ascii()) {
        return Err(ParameterNameError::NonAscii { byte_index, byte });
    }
    if value.len() > 64 {
        return Err(ParameterNameError::Length {
            actual: value.len(),
            max: 64,
        });
    }
    let bytes = value.as_bytes();
    if !bytes[0].is_ascii_lowercase() {
        return Err(ParameterNameError::InvalidInitialByte {
            byte_index: 0,
            byte: bytes[0],
        });
    }
    for (byte_index, byte) in bytes.iter().copied().enumerate().skip(1) {
        if !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_') {
            return Err(ParameterNameError::InvalidSubsequentByte { byte_index, byte });
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParameterText(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterTextError {
    Length { actual: usize, max: usize },
    EmbeddedNul { byte_index: usize },
}

impl ParameterText {
    /// Validates and stores parameter text byte-for-byte.
    ///
    /// # Errors
    ///
    /// Returns a [`ParameterTextError`] for embedded NUL or text exceeding 4,096 bytes.
    pub fn new(value: impl Into<String>) -> Result<Self, ParameterTextError> {
        Self::try_from(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ParameterText {
    type Error = ParameterTextError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if let Some(byte_index) = value.bytes().position(|byte| byte == 0) {
            return Err(ParameterTextError::EmbeddedNul { byte_index });
        }
        if value.len() > 4_096 {
            return Err(ParameterTextError::Length {
                actual: value.len(),
                max: 4_096,
            });
        }
        Ok(Self(value))
    }
}

impl TryFrom<&str> for ParameterText {
    type Error = ParameterTextError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from(value.to_owned())
    }
}

impl FromStr for ParameterText {
    type Err = ParameterTextError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value)
    }
}

impl fmt::Display for ParameterText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ParameterValue {
    Bool(bool),
    Integer(i64),
    Scalar(FiniteF64),
    Text(ParameterText),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationBuildError {
    DuplicateParameterName { name: ParameterName },
}

impl fmt::Display for OperationBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateParameterName { name } => write!(
                formatter,
                "parameter name {name} was supplied more than once"
            ),
        }
    }
}

impl std::error::Error for OperationBuildError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation {
    id: OperationId,
    key: OperationKey,
    enabled: bool,
    opacity: OperationOpacity,
    parameters: BTreeMap<ParameterName, ParameterValue>,
}

impl Operation {
    /// Creates an immutable operation from a sequence of unique named parameters.
    ///
    /// # Errors
    ///
    /// Returns [`OperationBuildError::DuplicateParameterName`] when a name occurs more than once.
    pub fn new<I>(
        id: OperationId,
        key: OperationKey,
        enabled: bool,
        parameters: I,
    ) -> Result<Self, OperationBuildError>
    where
        I: IntoIterator<Item = (ParameterName, ParameterValue)>,
    {
        Self::new_with_opacity(id, key, enabled, OperationOpacity::ONE, parameters)
    }

    /// Creates an immutable operation with an explicit opacity.
    ///
    /// # Errors
    ///
    /// Returns [`OperationBuildError::DuplicateParameterName`] when a name
    /// occurs more than once.
    pub fn new_with_opacity<I>(
        id: OperationId,
        key: OperationKey,
        enabled: bool,
        opacity: OperationOpacity,
        parameters: I,
    ) -> Result<Self, OperationBuildError>
    where
        I: IntoIterator<Item = (ParameterName, ParameterValue)>,
    {
        let mut collected = BTreeMap::new();
        for (name, value) in parameters {
            if collected.insert(name.clone(), value).is_some() {
                return Err(OperationBuildError::DuplicateParameterName { name });
            }
        }
        Ok(Self {
            id,
            key,
            enabled,
            opacity,
            parameters: collected,
        })
    }

    #[must_use]
    pub const fn id(&self) -> OperationId {
        self.id
    }

    #[must_use]
    pub fn key(&self) -> &OperationKey {
        &self.key
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub const fn opacity(&self) -> OperationOpacity {
        self.opacity
    }

    #[must_use]
    pub fn parameter(&self, name: &ParameterName) -> Option<&ParameterValue> {
        self.parameters.get(name)
    }

    pub fn parameters(&self) -> impl Iterator<Item = (&ParameterName, &ParameterValue)> {
        self.parameters.iter()
    }
}
