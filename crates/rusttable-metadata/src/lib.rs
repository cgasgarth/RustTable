#![forbid(unsafe_code)]
#![doc = "Bounded, canonical EXIF extraction for supported image containers."]

mod container;
mod error;
mod extract;
mod limits;

pub use error::{MetadataInputError, MetadataLimitsError};
pub use extract::{ExifMetadataInput, MetadataInput};
pub use limits::MetadataLimits;
