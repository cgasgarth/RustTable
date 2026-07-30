//! Canonicalize bounded EXIF extraction into the parser-independent metadata domain.

use rusttable_core::{ImageMetadata, MetadataEntry};

use crate::domain::{
    DomainValue, MetadataDocument, MetadataDomainError, MetadataKey, MetadataNamespace,
    MetadataProvenance, PrivacyClass, Rational,
};
use crate::policy::MetadataSource;

/// Converts the bounded legacy EXIF projection into the canonical metadata domain.
///
/// The byte parser remains owned by [`crate::ExifMetadataInput`]. This adapter is deliberately
/// parser-independent: canonical consumers receive stable keys, exact rationals, and explicit
/// provenance regardless of the EXIF library representation.
///
/// # Errors
///
/// Returns [`MetadataDomainError`] when an EXIF value cannot satisfy the canonical domain
/// bounds or when duplicate canonical keys are present.
pub fn canonicalize_exif(
    metadata: &ImageMetadata,
) -> Result<MetadataDocument, MetadataDomainError> {
    let provenance = MetadataProvenance::new(
        MetadataSource::Exif,
        crate::Confidence::new(100)?,
        PrivacyClass::Public,
    );
    let mut records = Vec::new();
    for (_, entry) in metadata.iter() {
        let (field, value) = canonical_entry(entry)?;
        let key = MetadataKey::new(MetadataNamespace::Exif, field.as_key())?;
        records.push(crate::MetadataRecord::new(key, value, provenance.clone()));
    }
    MetadataDocument::from_records(records)
}

fn canonical_entry(
    entry: &MetadataEntry,
) -> Result<(crate::CanonicalField, DomainValue), MetadataDomainError> {
    let result = match entry {
        MetadataEntry::CameraMake(value) => (
            crate::CanonicalField::CameraMake,
            DomainValue::Text(value.as_str().trim().to_owned()),
        ),
        MetadataEntry::CameraModel(value) => (
            crate::CanonicalField::CameraModel,
            DomainValue::Text(value.as_str().trim().to_owned()),
        ),
        MetadataEntry::LensModel(value) => (
            crate::CanonicalField::LensModel,
            DomainValue::Text(value.as_str().trim().to_owned()),
        ),
        MetadataEntry::CaptureDateTimeOriginal(value) => (
            crate::CanonicalField::CaptureDateTimeOriginal,
            DomainValue::Text(value.as_str().trim().to_owned()),
        ),
        MetadataEntry::Orientation(value) => (
            crate::CanonicalField::Orientation,
            DomainValue::Orientation(*value),
        ),
        MetadataEntry::ExposureTime(value) => (
            crate::CanonicalField::ExposureTime,
            DomainValue::Rational(exact_rational(value.numerator(), value.denominator())?),
        ),
        MetadataEntry::FNumber(value) => (
            crate::CanonicalField::FNumber,
            DomainValue::Rational(exact_rational(value.numerator(), value.denominator())?),
        ),
        MetadataEntry::IsoSpeed(value) => (
            crate::CanonicalField::IsoSpeed,
            DomainValue::Unsigned(u64::from(value.get())),
        ),
        MetadataEntry::FocalLength(value) => (
            crate::CanonicalField::FocalLength,
            DomainValue::Rational(exact_rational(value.numerator(), value.denominator())?),
        ),
    };
    Ok(result)
}

fn exact_rational(numerator: u64, denominator: u64) -> Result<Rational, MetadataDomainError> {
    Rational::new(
        i64::try_from(numerator).map_err(|_| MetadataDomainError::RationalOutOfRange)?,
        denominator,
    )
}
