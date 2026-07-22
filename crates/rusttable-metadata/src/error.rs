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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataOutputLimit {
    PayloadBytes,
    IfdEntries,
    ValueBytes,
    AllocationBytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataOutputLimitsError {
    ZeroLimit {
        limit: MetadataOutputLimit,
    },
    Inconsistent {
        smaller: MetadataOutputLimit,
        larger: MetadataOutputLimit,
    },
    NotRepresentable {
        limit: MetadataOutputLimit,
    },
}

impl fmt::Display for MetadataOutputLimitsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for MetadataOutputLimitsError {}

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
    TiffValueSizeOverflow {
        kind: u16,
        count: u64,
        element_size: u8,
    },
    TiffOffsetOverflow {
        offset: u64,
        length: u64,
    },
    TiffValueTruncated {
        kind: u16,
        count: u64,
        offset: u64,
        required: u64,
        available: u64,
    },
    TiffStructureTruncated {
        offset: u64,
        required: u64,
        available: u64,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataOutputError {
    InvalidLimits {
        reason: MetadataOutputLimitsError,
    },
    UnrepresentableText {
        field: rusttable_core::MetadataField,
    },
    UnrepresentableRational {
        field: rusttable_core::MetadataField,
        numerator: u64,
        denominator: u64,
    },
    IfdEntryLimit {
        limit: u32,
        actual: u32,
    },
    ValueLimit {
        field: rusttable_core::MetadataField,
        limit: u64,
        actual: u64,
    },
    PayloadLimit {
        limit: u64,
        actual: u64,
    },
    AllocationLimit {
        limit: u64,
        actual: u64,
    },
    ArithmeticOverflow {
        context: &'static str,
    },
    AllocationFailure {
        requested: u64,
    },
    InternalInvariant {
        context: &'static str,
    },
}

impl fmt::Display for MetadataOutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for MetadataOutputError {}
