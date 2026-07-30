use std::collections::BTreeMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::num::NonZeroU32;

use super::{MetadataText, Orientation, PositiveRational};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetadataField {
    CameraMake,
    CameraModel,
    LensModel,
    CaptureDateTimeOriginal,
    Orientation,
    ExposureTime,
    FNumber,
    IsoSpeed,
    FocalLength,
}

pub const ALL_FIELDS: [MetadataField; 9] = [
    MetadataField::CameraMake,
    MetadataField::CameraModel,
    MetadataField::LensModel,
    MetadataField::CaptureDateTimeOriginal,
    MetadataField::Orientation,
    MetadataField::ExposureTime,
    MetadataField::FNumber,
    MetadataField::IsoSpeed,
    MetadataField::FocalLength,
];

impl MetadataField {
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::CameraMake => 0,
            Self::CameraModel => 1,
            Self::LensModel => 2,
            Self::CaptureDateTimeOriginal => 3,
            Self::Orientation => 4,
            Self::ExposureTime => 5,
            Self::FNumber => 6,
            Self::IsoSpeed => 7,
            Self::FocalLength => 8,
        }
    }
}

impl PartialOrd for MetadataField {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MetadataField {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank().cmp(&other.rank())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataEntry {
    CameraMake(MetadataText),
    CameraModel(MetadataText),
    LensModel(MetadataText),
    CaptureDateTimeOriginal(MetadataText),
    Orientation(Orientation),
    ExposureTime(PositiveRational),
    FNumber(PositiveRational),
    IsoSpeed(NonZeroU32),
    FocalLength(PositiveRational),
}

impl MetadataEntry {
    #[must_use]
    pub const fn field(&self) -> MetadataField {
        match self {
            Self::CameraMake(_) => MetadataField::CameraMake,
            Self::CameraModel(_) => MetadataField::CameraModel,
            Self::LensModel(_) => MetadataField::LensModel,
            Self::CaptureDateTimeOriginal(_) => MetadataField::CaptureDateTimeOriginal,
            Self::Orientation(_) => MetadataField::Orientation,
            Self::ExposureTime(_) => MetadataField::ExposureTime,
            Self::FNumber(_) => MetadataField::FNumber,
            Self::IsoSpeed(_) => MetadataField::IsoSpeed,
            Self::FocalLength(_) => MetadataField::FocalLength,
        }
    }
}

impl Hash for MetadataEntry {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::CameraMake(value) => {
                0u8.hash(state);
                value.hash(state);
            }
            Self::CameraModel(value) => {
                1u8.hash(state);
                value.hash(state);
            }
            Self::LensModel(value) => {
                2u8.hash(state);
                value.hash(state);
            }
            Self::CaptureDateTimeOriginal(value) => {
                3u8.hash(state);
                value.hash(state);
            }
            Self::Orientation(value) => {
                4u8.hash(state);
                value.code().hash(state);
            }
            Self::ExposureTime(value) => {
                5u8.hash(state);
                value.hash(state);
            }
            Self::FNumber(value) => {
                6u8.hash(state);
                value.hash(state);
            }
            Self::IsoSpeed(value) => {
                7u8.hash(state);
                value.get().hash(state);
            }
            Self::FocalLength(value) => {
                8u8.hash(state);
                value.hash(state);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataModelError {
    DuplicateField(MetadataField),
}

impl fmt::Display for MetadataModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateField(field) => {
                write!(formatter, "metadata field {field:?} is duplicated")
            }
        }
    }
}

impl std::error::Error for MetadataModelError {}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImageMetadata {
    entries: BTreeMap<MetadataField, MetadataEntry>,
}

impl ImageMetadata {
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Builds immutable metadata in canonical field-rank order.
    ///
    /// # Errors
    ///
    /// Returns [`MetadataModelError::DuplicateField`] when two entries target
    /// the same field.
    pub fn from_entries<I>(entries: I) -> Result<Self, MetadataModelError>
    where
        I: IntoIterator<Item = MetadataEntry>,
    {
        let mut values = BTreeMap::new();
        for entry in entries {
            let field = entry.field();
            if values.insert(field, entry).is_some() {
                return Err(MetadataModelError::DuplicateField(field));
            }
        }
        Ok(Self { entries: values })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn get(&self, field: MetadataField) -> Option<&MetadataEntry> {
        self.entries.get(&field)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&MetadataField, &MetadataEntry)> {
        self.entries.iter()
    }
}

impl Hash for ImageMetadata {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.entries.len().hash(state);
        for (field, entry) in &self.entries {
            field.rank().hash(state);
            entry.hash(state);
        }
    }
}
