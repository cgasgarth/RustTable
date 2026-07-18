use std::fmt;

use rusttable_image::InputFormat;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataLimitsError {
    ZeroLimit,
}

impl fmt::Display for MetadataLimitsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("metadata limits must all be nonzero")
    }
}

impl std::error::Error for MetadataLimitsError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataInputError {
    SourceTooLarge {
        limit: u64,
        actual: u64,
    },
    FormatMismatch {
        format: InputFormat,
    },
    MalformedContainer {
        format: InputFormat,
        reason: &'static str,
    },
    DuplicateExifPayload {
        format: InputFormat,
    },
    ExifPayloadTooLarge {
        limit: u64,
        actual: u64,
    },
    JpegSegmentLimit {
        limit: u32,
    },
    PngChunkLimit {
        limit: u32,
    },
    IfdEntryLimit {
        limit: u32,
    },
    IfdNestingLimit {
        limit: u32,
    },
    ValueTooLarge {
        limit: u64,
        actual: u64,
    },
    MalformedExif,
    InvalidField {
        field: &'static str,
    },
    DuplicateField {
        field: &'static str,
    },
    ArithmeticOverflow,
}

impl fmt::Display for MetadataInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for MetadataInputError {}
