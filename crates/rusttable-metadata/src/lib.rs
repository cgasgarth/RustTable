#![forbid(unsafe_code)]
#![doc = "Bounded, canonical EXIF extraction for supported image containers."]

mod container;
mod error;
mod extract;
mod image_output;
mod limits;
mod output;
mod packet;
mod policy;
mod views;

pub use error::{
    MetadataInputError, MetadataLimitsError, MetadataOutputError, MetadataOutputLimit,
    MetadataOutputLimitsError,
};
pub use extract::{ExifMetadataInput, MetadataInput, MetadataReadResult, MetadataReadStatus};
pub use image_output::{MetadataImageOutput, MetadataImageOutputError};
pub use limits::{MetadataLimits, MetadataOutputLimits};
pub use output::{CanonicalExifOutput, EncodedExif, MetadataOutput};
pub use packet::{MetadataPacket, MetadataPacketBuilder};
pub use policy::{
    CanonicalMetadataPolicy, MetadataAction, MetadataBuildError, MetadataCategory,
    MetadataProperty, MetadataSource, MetadataValue,
};
pub use rusttable_core::{
    ImageMetadata, MetadataEntry, MetadataText, Orientation, PositiveRational,
};
pub use views::{FormatView, FormatViewKind};
