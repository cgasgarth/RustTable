use std::fmt;
use std::path::Path;

use rusttable_image::{DecodedImage, ImageOutputError, OutputFormat, OutputOptions, OutputReceipt};

use crate::{ImageMetadata, MetadataOutputError, MetadataOutputLimits};

pub trait MetadataImageOutput: Send + Sync {
    fn write_new_with_metadata(
        &self,
        image: &DecodedImage,
        metadata: &ImageMetadata,
        destination: &Path,
        options: OutputOptions,
        metadata_limits: MetadataOutputLimits,
    ) -> Result<OutputReceipt, MetadataImageOutputError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataImageOutputError {
    UnsupportedMetadataOutputFormat {
        format: OutputFormat,
    },
    MetadataSerializationFailure {
        source: MetadataOutputError,
    },
    ExifFramingLimit {
        format: OutputFormat,
        limit: u64,
        actual: u64,
    },
    MalformedEncodedContainer {
        format: OutputFormat,
        reason: &'static str,
    },
    BeforePublication {
        source: ImageOutputError,
    },
}

impl fmt::Display for MetadataImageOutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "metadata image output failed: {self:?}")
    }
}

impl std::error::Error for MetadataImageOutputError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::MetadataSerializationFailure { source } => Some(source),
            Self::BeforePublication { source } => Some(source),
            Self::UnsupportedMetadataOutputFormat { .. }
            | Self::ExifFramingLimit { .. }
            | Self::MalformedEncodedContainer { .. } => None,
        }
    }
}
