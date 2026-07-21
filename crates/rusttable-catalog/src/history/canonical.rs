use std::fmt;

use rusttable_core::{Edit, OperationId, ParameterName, ParameterValue};
use sha2::{Digest, Sha256};

use super::types::{HistoryOperationKind, HistoryPayload};

pub const EDIT_BLOB_SCHEMA: u16 = 1;
pub const MASK_BLEND_BLOB_SCHEMA: u16 = 1;
pub const PIPELINE_BLOB_SCHEMA: u16 = 1;
pub const MAX_CANONICAL_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContentBlobKind {
    Edit,
    MaskBlend,
    Pipeline,
}

impl ContentBlobKind {
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Edit => 1,
            Self::MaskBlend => 2,
            Self::Pipeline => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentBlobId {
    kind: ContentBlobKind,
    schema: u16,
    length: u64,
    digest: [u8; 32],
}

impl ContentBlobId {
    #[must_use]
    pub fn new(kind: ContentBlobKind, schema: u16, bytes: &[u8]) -> Self {
        Self {
            kind,
            schema,
            length: bytes.len() as u64,
            digest: Sha256::digest(bytes).into(),
        }
    }

    #[must_use]
    pub const fn from_parts(
        kind: ContentBlobKind,
        schema: u16,
        length: u64,
        digest: [u8; 32],
    ) -> Self {
        Self {
            kind,
            schema,
            length,
            digest,
        }
    }

    #[must_use]
    pub const fn kind(self) -> ContentBlobKind {
        self.kind
    }

    #[must_use]
    pub const fn schema(self) -> u16 {
        self.schema
    }

    #[must_use]
    pub const fn length(self) -> u64 {
        self.length
    }

    #[must_use]
    pub const fn digest(self) -> [u8; 32] {
        self.digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalBlob {
    id: ContentBlobId,
    bytes: Vec<u8>,
}

impl CanonicalBlob {
    /// Creates a content-addressed blob after checking its bounded size.
    ///
    /// # Errors
    ///
    /// Returns an error when the canonical payload exceeds the persistence limit.
    pub fn new(
        kind: ContentBlobKind,
        schema: u16,
        bytes: impl Into<Vec<u8>>,
    ) -> Result<Self, CanonicalEncodingError> {
        let bytes = bytes.into();
        if bytes.len() > MAX_CANONICAL_BYTES {
            return Err(CanonicalEncodingError::TooLarge {
                actual: bytes.len(),
            });
        }
        Ok(Self {
            id: ContentBlobId::new(kind, schema, &bytes),
            bytes,
        })
    }

    #[must_use]
    pub const fn id(&self) -> ContentBlobId {
        self.id
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Verifies a stored blob without substituting bytes from another revision.
    #[must_use]
    pub fn verifies(&self) -> bool {
        self.id == ContentBlobId::new(self.id.kind, self.id.schema, &self.bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalPayload {
    edit: CanonicalBlob,
    mask_blend: CanonicalBlob,
    pipeline: CanonicalBlob,
}

impl CanonicalPayload {
    /// Encodes the three immutable payload components used by a history revision.
    ///
    /// # Errors
    ///
    /// Returns an encoding or size error before any history mutation occurs.
    pub fn from_history(payload: &HistoryPayload) -> Result<Self, CanonicalEncodingError> {
        Ok(Self {
            edit: CanonicalBlob::new(
                ContentBlobKind::Edit,
                EDIT_BLOB_SCHEMA,
                canonical_edit_bytes(payload.edit())?,
            )?,
            mask_blend: CanonicalBlob::new(
                ContentBlobKind::MaskBlend,
                MASK_BLEND_BLOB_SCHEMA,
                canonical_mask_blend_bytes(payload.mask_bytes(), payload.pipeline_bytes())?,
            )?,
            pipeline: CanonicalBlob::new(
                ContentBlobKind::Pipeline,
                PIPELINE_BLOB_SCHEMA,
                canonical_pipeline_bytes(payload.pipeline_bytes())?,
            )?,
        })
    }

    #[must_use]
    pub const fn edit(&self) -> &CanonicalBlob {
        &self.edit
    }

    #[must_use]
    pub const fn mask_blend(&self) -> &CanonicalBlob {
        &self.mask_blend
    }

    #[must_use]
    pub const fn pipeline(&self) -> &CanonicalBlob {
        &self.pipeline
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalHistoryCommand {
    Parameter {
        operation_id: OperationId,
        name: ParameterName,
        value: ParameterValue,
    },
    Order {
        operation_ids: Vec<OperationId>,
    },
    Enable {
        operation_id: OperationId,
        enabled: bool,
    },
    Mask {
        bytes: Vec<u8>,
    },
    Blend {
        bytes: Vec<u8>,
    },
    Style {
        style_id: [u8; 16],
        bytes: Vec<u8>,
    },
    Copy {
        source: ContentBlobId,
    },
    Paste {
        source: ContentBlobId,
    },
    Reset,
}

impl CanonicalHistoryCommand {
    /// Returns a stable command envelope independent of Rust enum layout.
    ///
    /// # Errors
    ///
    /// Returns an error for oversized command payloads or invalid lengths.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, CanonicalEncodingError> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RTHCMD\0\x01");
        match self {
            Self::Parameter {
                operation_id,
                name,
                value,
            } => {
                bytes.push(1);
                bytes.extend_from_slice(&operation_id.get().to_le_bytes());
                put_text(&mut bytes, name.as_str())?;
                put_parameter(&mut bytes, value)?;
            }
            Self::Order { operation_ids } => {
                bytes.push(2);
                put_count(&mut bytes, operation_ids.len())?;
                for id in operation_ids {
                    bytes.extend_from_slice(&id.get().to_le_bytes());
                }
            }
            Self::Enable {
                operation_id,
                enabled,
            } => {
                bytes.push(3);
                bytes.extend_from_slice(&operation_id.get().to_le_bytes());
                bytes.push(u8::from(*enabled));
            }
            Self::Mask { bytes: payload } => put_raw(&mut bytes, 4, payload)?,
            Self::Blend { bytes: payload } => put_raw(&mut bytes, 5, payload)?,
            Self::Style {
                style_id,
                bytes: payload,
            } => {
                bytes.push(6);
                bytes.extend_from_slice(style_id);
                put_bytes(&mut bytes, payload)?;
            }
            Self::Copy { source } => put_blob_id(&mut bytes, 7, *source),
            Self::Paste { source } => put_blob_id(&mut bytes, 8, *source),
            Self::Reset => bytes.push(9),
        }
        checked(bytes)
    }

    #[must_use]
    pub const fn kind(&self) -> HistoryOperationKind {
        match self {
            Self::Parameter { .. } => HistoryOperationKind::Parameter,
            Self::Order { .. } => HistoryOperationKind::Order,
            Self::Enable { .. } => HistoryOperationKind::Enable,
            Self::Mask { .. } => HistoryOperationKind::Mask,
            Self::Blend { .. } => HistoryOperationKind::Blend,
            Self::Style { .. } => HistoryOperationKind::Style,
            Self::Copy { .. } => HistoryOperationKind::Copy,
            Self::Paste { .. } => HistoryOperationKind::Paste,
            Self::Reset => HistoryOperationKind::Reset,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalEncodingError {
    TooLarge { actual: usize },
    LengthOverflow,
}

impl fmt::Display for CanonicalEncodingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { actual } => {
                write!(formatter, "canonical history payload is {actual} bytes")
            }
            Self::LengthOverflow => {
                formatter.write_str("canonical history length overflows wire width")
            }
        }
    }
}

impl std::error::Error for CanonicalEncodingError {}

///
/// # Errors
///
/// Returns a bounded-size or length-overflow error.
pub fn canonical_edit_bytes(edit: &Edit) -> Result<Vec<u8>, CanonicalEncodingError> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RTEDIT\0\x01");
    bytes.extend_from_slice(&edit.id().get().to_le_bytes());
    bytes.extend_from_slice(&edit.photo_id().get().to_le_bytes());
    bytes.extend_from_slice(&edit.base_photo_revision().get().to_le_bytes());
    bytes.extend_from_slice(&edit.revision().get().to_le_bytes());
    let operations = edit.operations().collect::<Vec<_>>();
    put_count(&mut bytes, operations.len())?;
    for operation in operations {
        bytes.extend_from_slice(&operation.id().get().to_le_bytes());
        put_text(&mut bytes, operation.key().as_str())?;
        bytes.push(u8::from(operation.is_enabled()));
        bytes.extend_from_slice(&operation.opacity().get().to_bits().to_le_bytes());
        let parameters = operation.parameters().collect::<Vec<_>>();
        put_count(&mut bytes, parameters.len())?;
        for (name, value) in parameters {
            put_text(&mut bytes, name.as_str())?;
            put_parameter(&mut bytes, value)?;
        }
    }
    checked(bytes)
}

///
/// # Errors
///
/// Returns a bounded-size or length-overflow error.
pub fn canonical_mask_blend_bytes(
    mask: &[u8],
    blend: &[u8],
) -> Result<Vec<u8>, CanonicalEncodingError> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RTMASKBLEND\0\x01");
    put_bytes(&mut bytes, mask)?;
    put_bytes(&mut bytes, blend)?;
    checked(bytes)
}

///
/// # Errors
///
/// Returns a bounded-size or length-overflow error.
pub fn canonical_pipeline_bytes(pipeline: &[u8]) -> Result<Vec<u8>, CanonicalEncodingError> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RTPIPE\0\x01");
    put_bytes(&mut bytes, pipeline)?;
    checked(bytes)
}

fn put_parameter(
    bytes: &mut Vec<u8>,
    value: &ParameterValue,
) -> Result<(), CanonicalEncodingError> {
    match value {
        ParameterValue::Bool(value) => {
            bytes.push(1);
            bytes.push(u8::from(*value));
        }
        ParameterValue::Integer(value) => {
            bytes.push(2);
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        ParameterValue::Scalar(value) => {
            bytes.push(3);
            bytes.extend_from_slice(&value.get().to_bits().to_le_bytes());
        }
        ParameterValue::Text(value) => {
            bytes.push(4);
            put_text(bytes, value.as_str())?;
        }
    }
    Ok(())
}

fn put_raw(bytes: &mut Vec<u8>, tag: u8, payload: &[u8]) -> Result<(), CanonicalEncodingError> {
    bytes.push(tag);
    put_bytes(bytes, payload)
}

fn put_blob_id(bytes: &mut Vec<u8>, tag: u8, id: ContentBlobId) {
    bytes.push(tag);
    bytes.push(id.kind.tag());
    bytes.extend_from_slice(&id.schema.to_le_bytes());
    bytes.extend_from_slice(&id.length.to_le_bytes());
    bytes.extend_from_slice(&id.digest);
}

fn put_text(bytes: &mut Vec<u8>, value: &str) -> Result<(), CanonicalEncodingError> {
    put_bytes(bytes, value.as_bytes())
}

fn put_bytes(bytes: &mut Vec<u8>, value: &[u8]) -> Result<(), CanonicalEncodingError> {
    let length = u32::try_from(value.len()).map_err(|_| CanonicalEncodingError::LengthOverflow)?;
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(value);
    Ok(())
}

fn put_count(bytes: &mut Vec<u8>, count: usize) -> Result<(), CanonicalEncodingError> {
    let count = u32::try_from(count).map_err(|_| CanonicalEncodingError::LengthOverflow)?;
    bytes.extend_from_slice(&count.to_le_bytes());
    Ok(())
}

fn checked(bytes: Vec<u8>) -> Result<Vec<u8>, CanonicalEncodingError> {
    if bytes.len() > MAX_CANONICAL_BYTES {
        Err(CanonicalEncodingError::TooLarge {
            actual: bytes.len(),
        })
    } else {
        Ok(bytes)
    }
}
