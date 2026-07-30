#![allow(clippy::missing_errors_doc)]

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const MAX_VALUE_BYTES: usize = 4096;
const MAX_VARIABLES: usize = 128;
const MAX_EXPANDED_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatermarkContext {
    filename: String,
    sequence: u64,
    version: u64,
    width: u32,
    height: u32,
    rating: i8,
    labels: Vec<String>,
    capture_date: Option<String>,
    export_date: Option<String>,
    user_variables: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatermarkContextError {
    ValueTooLong,
    TooManyVariables,
    InvalidVariableName,
    InvalidVariableValue,
    ExpandedTooLarge,
}

impl std::fmt::Display for WatermarkContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ValueTooLong => f.write_str("watermark context value is too long"),
            Self::TooManyVariables => f.write_str("watermark context has too many variables"),
            Self::InvalidVariableName => f.write_str("watermark variable name is invalid"),
            Self::InvalidVariableValue => f.write_str("watermark variable value is invalid"),
            Self::ExpandedTooLarge => f.write_str("expanded watermark SVG is too large"),
        }
    }
}

impl std::error::Error for WatermarkContextError {}

impl WatermarkContext {
    pub fn new(
        filename: impl Into<String>,
        width: u32,
        height: u32,
    ) -> Result<Self, WatermarkContextError> {
        let filename = bounded_value(filename.into())?;
        if width == 0 || height == 0 {
            return Err(WatermarkContextError::InvalidVariableValue);
        }
        Ok(Self {
            filename,
            width,
            height,
            ..Self::default()
        })
    }

    #[must_use]
    pub fn with_sequence(mut self, sequence: u64, version: u64) -> Self {
        self.sequence = sequence;
        self.version = version;
        self
    }

    pub fn with_rating(mut self, rating: i8) -> Result<Self, WatermarkContextError> {
        if !(-1..=5).contains(&rating) {
            return Err(WatermarkContextError::InvalidVariableValue);
        }
        self.rating = rating;
        Ok(self)
    }

    pub fn with_labels(
        mut self,
        labels: impl IntoIterator<Item = String>,
    ) -> Result<Self, WatermarkContextError> {
        self.labels = labels
            .into_iter()
            .map(bounded_value)
            .collect::<Result<_, _>>()?;
        Ok(self)
    }

    pub fn with_dates(
        mut self,
        capture: Option<String>,
        export: Option<String>,
    ) -> Result<Self, WatermarkContextError> {
        self.capture_date = capture.map(bounded_value).transpose()?;
        self.export_date = export.map(bounded_value).transpose()?;
        Ok(self)
    }

    pub fn with_variable(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, WatermarkContextError> {
        let name = name.into();
        if self.user_variables.len() >= MAX_VARIABLES && !self.user_variables.contains_key(&name) {
            return Err(WatermarkContextError::TooManyVariables);
        }
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.')
        {
            return Err(WatermarkContextError::InvalidVariableName);
        }
        self.user_variables
            .insert(name, bounded_value(value.into())?);
        Ok(self)
    }

    #[must_use]
    pub fn rendered_hash(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable.watermark.context.v1");
        for (name, value) in self.values() {
            hasher.update((name.len() as u64).to_le_bytes());
            hasher.update(name.as_bytes());
            hasher.update((value.len() as u64).to_le_bytes());
            hasher.update(value.as_bytes());
        }
        hasher.finalize().into()
    }

    pub fn expand(&self, source: &[u8]) -> Result<ExpandedWatermark, WatermarkContextError> {
        let source =
            std::str::from_utf8(source).map_err(|_| WatermarkContextError::InvalidVariableValue)?;
        let values = self.values().into_iter().collect::<BTreeMap<_, _>>();
        let mut output = Vec::with_capacity(source.len());
        let mut findings = Vec::new();
        let mut cursor = 0;
        while let Some(open) = source[cursor..].find("{{") {
            let open = cursor + open;
            output.extend_from_slice(&source.as_bytes()[cursor..open]);
            let Some(close) = source[open + 2..].find("}}") else {
                output.extend_from_slice(&source.as_bytes()[open..]);
                cursor = source.len();
                break;
            };
            let close = open + 2 + close;
            let name = &source[open + 2..close];
            if name.is_empty()
                || name.len() > 64
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.')
            {
                findings.push("invalid-variable".to_owned());
            } else if let Some(value) = values.get(name) {
                xml_escape(value, &mut output);
            } else {
                findings.push(name.to_owned());
            }
            cursor = close + 2;
            if output.len() > MAX_EXPANDED_BYTES {
                return Err(WatermarkContextError::ExpandedTooLarge);
            }
        }
        output.extend_from_slice(&source.as_bytes()[cursor..]);
        if output.len() > MAX_EXPANDED_BYTES {
            return Err(WatermarkContextError::ExpandedTooLarge);
        }
        let hash = Sha256::digest(&output).into();
        Ok(ExpandedWatermark {
            bytes: output,
            hash,
            findings,
        })
    }

    fn values(&self) -> Vec<(String, String)> {
        let mut values = vec![
            ("filename".to_owned(), self.filename.clone()),
            ("sequence".to_owned(), self.sequence.to_string()),
            ("version".to_owned(), self.version.to_string()),
            ("width".to_owned(), self.width.to_string()),
            ("height".to_owned(), self.height.to_string()),
            ("rating".to_owned(), self.rating.to_string()),
            ("labels".to_owned(), self.labels.join(",")),
            (
                "capture_date".to_owned(),
                self.capture_date.clone().unwrap_or_default(),
            ),
            (
                "export_date".to_owned(),
                self.export_date.clone().unwrap_or_default(),
            ),
        ];
        values.extend(
            self.user_variables
                .iter()
                .map(|(name, value)| (name.clone(), value.clone())),
        );
        values.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        values
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedWatermark {
    pub(crate) bytes: Vec<u8>,
    pub(crate) hash: [u8; 32],
    pub(crate) findings: Vec<String>,
}

impl ExpandedWatermark {
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub const fn hash(&self) -> [u8; 32] {
        self.hash
    }

    #[must_use]
    pub fn findings(&self) -> &[String] {
        &self.findings
    }
}

fn bounded_value(value: String) -> Result<String, WatermarkContextError> {
    if value.len() > MAX_VALUE_BYTES || value.bytes().any(|byte| byte == 0) {
        return Err(WatermarkContextError::ValueTooLong);
    }
    Ok(value)
}

fn xml_escape(value: &str, output: &mut Vec<u8>) {
    for character in value.chars() {
        match character {
            '&' => output.extend_from_slice(b"&amp;"),
            '<' => output.extend_from_slice(b"&lt;"),
            '>' => output.extend_from_slice(b"&gt;"),
            '"' => output.extend_from_slice(b"&quot;"),
            '\'' => output.extend_from_slice(b"&apos;"),
            character => {
                let mut buffer = [0; 4];
                output.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
            }
        }
    }
}
