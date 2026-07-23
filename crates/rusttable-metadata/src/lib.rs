#![forbid(unsafe_code)]
#![doc = "Bounded, canonical EXIF extraction for supported image containers."]

mod codec;
mod container;
mod domain;
mod error;
mod extract;
mod image_output;
mod limits;
mod output;
mod packet;
mod policy;
mod views;

pub use codec::CanonicalCodec;
pub use domain::{
    CanonicalField, Confidence, DatePrecision, DomainValue, GpsCoordinate, HierarchicalKeywords,
    LanguageAlternative, LanguageTag, MAX_METADATA_CODEC_BYTES, MAX_METADATA_KEY_BYTES,
    MAX_METADATA_LIST_ITEMS, MAX_METADATA_NAMESPACE_BYTES, MAX_METADATA_RAW_BYTES,
    MAX_METADATA_RECORDS, MAX_METADATA_STRUCTURED_FIELDS, MAX_METADATA_TEXT_BYTES,
    MetadataDateTime, MetadataDocument, MetadataDomainError, MetadataKey, MetadataNamespace,
    MetadataProvenance, MetadataRecord, NormalizationWarning, PrivacyClass, Rational,
    RawRepresentation, StructuredValue,
};
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
