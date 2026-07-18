#![forbid(unsafe_code)]
#![doc = "Bounded, canonical EXIF extraction for supported image containers."]

mod container;
mod error;
mod extract;
mod image_output;
mod limits;
mod output;

pub use error::{
    MetadataInputError, MetadataLimitsError, MetadataOutputError, MetadataOutputLimit,
    MetadataOutputLimitsError,
};
pub use extract::{ExifMetadataInput, MetadataInput};
pub use image_output::{MetadataImageOutput, MetadataImageOutputError};
pub use limits::{MetadataLimits, MetadataOutputLimits};
pub use output::{CanonicalExifOutput, EncodedExif, MetadataOutput};
pub use rusttable_core::{
    ImageMetadata, MetadataEntry, MetadataText, Orientation, PositiveRational,
};
