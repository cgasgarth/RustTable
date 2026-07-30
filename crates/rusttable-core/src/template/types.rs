use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VariableId {
    SourceBasename,
    SourceStem,
    SourceExtension,
    SourceRelativeImportPath,
    Film,
    ImportSession,
    PhotoId,
    ImageId,
    VirtualCopy,
    EditRevision,
    Sequence,
    Index,
    Count,
    Rating,
    ColorLabel,
    PickReject,
    Title,
    Creator,
    Copyright,
    MetadataArtist,
    MetadataDescription,
    MetadataLocation,
    MetadataKeywords,
    Camera,
    Lens,
    Exposure,
    Aperture,
    FocalLength,
    Iso,
    Tags,
    CaptureDate,
    CaptureTime,
    CaptureYear,
    CaptureMonth,
    CaptureDay,
    CaptureHour,
    CaptureMinute,
    CaptureSecond,
    ExportDate,
    ExportTime,
    ExportYear,
    ExportMonth,
    ExportDay,
    ExportHour,
    ExportMinute,
    ExportSecond,
    Width,
    Height,
    Aspect,
    Orientation,
    Profile,
    Recipe,
    Style,
    Encoder,
    Extension,
    DestinationCollection,
    DestinationAlbum,
    ContentHash,
    RequestHash,
    Custom(String),
}

impl VariableId {
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let id = match name {
            "source_basename" => Self::SourceBasename,
            "source_stem" => Self::SourceStem,
            "source_extension" => Self::SourceExtension,
            "source_relative_import_path" => Self::SourceRelativeImportPath,
            "film" => Self::Film,
            "import_session" => Self::ImportSession,
            "photo_id" => Self::PhotoId,
            "image_id" => Self::ImageId,
            "virtual_copy" => Self::VirtualCopy,
            "edit_revision" => Self::EditRevision,
            "sequence" => Self::Sequence,
            "index" => Self::Index,
            "count" => Self::Count,
            "rating" => Self::Rating,
            "color_label" => Self::ColorLabel,
            "pick_reject" => Self::PickReject,
            "title" => Self::Title,
            "creator" => Self::Creator,
            "copyright" => Self::Copyright,
            "metadata_artist" => Self::MetadataArtist,
            "metadata_description" => Self::MetadataDescription,
            "metadata_location" => Self::MetadataLocation,
            "metadata_keywords" => Self::MetadataKeywords,
            "camera" => Self::Camera,
            "lens" => Self::Lens,
            "exposure" => Self::Exposure,
            "aperture" => Self::Aperture,
            "focal_length" => Self::FocalLength,
            "iso" => Self::Iso,
            "tags" => Self::Tags,
            "capture_date" => Self::CaptureDate,
            "capture_time" => Self::CaptureTime,
            "capture_year" => Self::CaptureYear,
            "capture_month" => Self::CaptureMonth,
            "capture_day" => Self::CaptureDay,
            "capture_hour" => Self::CaptureHour,
            "capture_minute" => Self::CaptureMinute,
            "capture_second" => Self::CaptureSecond,
            "export_date" => Self::ExportDate,
            "export_time" => Self::ExportTime,
            "export_year" => Self::ExportYear,
            "export_month" => Self::ExportMonth,
            "export_day" => Self::ExportDay,
            "export_hour" => Self::ExportHour,
            "export_minute" => Self::ExportMinute,
            "export_second" => Self::ExportSecond,
            "width" => Self::Width,
            "height" => Self::Height,
            "aspect" => Self::Aspect,
            "orientation" => Self::Orientation,
            "profile" => Self::Profile,
            "recipe" => Self::Recipe,
            "style" => Self::Style,
            "encoder" => Self::Encoder,
            "extension" => Self::Extension,
            "destination_collection" => Self::DestinationCollection,
            "destination_album" => Self::DestinationAlbum,
            "content_hash" => Self::ContentHash,
            "request_hash" => Self::RequestHash,
            _ => return None,
        };
        Some(id)
    }

    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::SourceBasename => "source_basename",
            Self::SourceStem => "source_stem",
            Self::SourceExtension => "source_extension",
            Self::SourceRelativeImportPath => "source_relative_import_path",
            Self::Film => "film",
            Self::ImportSession => "import_session",
            Self::PhotoId => "photo_id",
            Self::ImageId => "image_id",
            Self::VirtualCopy => "virtual_copy",
            Self::EditRevision => "edit_revision",
            Self::Sequence => "sequence",
            Self::Index => "index",
            Self::Count => "count",
            Self::Rating => "rating",
            Self::ColorLabel => "color_label",
            Self::PickReject => "pick_reject",
            Self::Title => "title",
            Self::Creator => "creator",
            Self::Copyright => "copyright",
            Self::MetadataArtist => "metadata_artist",
            Self::MetadataDescription => "metadata_description",
            Self::MetadataLocation => "metadata_location",
            Self::MetadataKeywords => "metadata_keywords",
            Self::Camera => "camera",
            Self::Lens => "lens",
            Self::Exposure => "exposure",
            Self::Aperture => "aperture",
            Self::FocalLength => "focal_length",
            Self::Iso => "iso",
            Self::Tags => "tags",
            Self::CaptureDate => "capture_date",
            Self::CaptureTime => "capture_time",
            Self::CaptureYear => "capture_year",
            Self::CaptureMonth => "capture_month",
            Self::CaptureDay => "capture_day",
            Self::CaptureHour => "capture_hour",
            Self::CaptureMinute => "capture_minute",
            Self::CaptureSecond => "capture_second",
            Self::ExportDate => "export_date",
            Self::ExportTime => "export_time",
            Self::ExportYear => "export_year",
            Self::ExportMonth => "export_month",
            Self::ExportDay => "export_day",
            Self::ExportHour => "export_hour",
            Self::ExportMinute => "export_minute",
            Self::ExportSecond => "export_second",
            Self::Width => "width",
            Self::Height => "height",
            Self::Aspect => "aspect",
            Self::Orientation => "orientation",
            Self::Profile => "profile",
            Self::Recipe => "recipe",
            Self::Style => "style",
            Self::Encoder => "encoder",
            Self::Extension => "extension",
            Self::DestinationCollection => "destination_collection",
            Self::DestinationAlbum => "destination_album",
            Self::ContentHash => "content_hash",
            Self::RequestHash => "request_hash",
            Self::Custom(name) => name,
        }
    }

    #[must_use]
    pub(crate) fn value_type(&self) -> VariableType {
        descriptor_for(self.clone()).value_type
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariableType {
    Text,
    Integer,
    Rational,
    DateTime,
    Dimensions,
    Tags,
    Enum,
    Hash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyClass {
    Public,
    Private,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableDescriptor {
    pub id: VariableId,
    pub value_type: VariableType,
    pub privacy: PrivacyClass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateDateTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub offset_minutes: i16,
}

impl TemplateDateTime {
    /// Creates a locale-independent date-time value with an explicit UTC offset.
    ///
    /// # Errors
    ///
    /// Returns an error when any component is outside the bounded representation.
    pub fn new(
        year: i32,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
        offset_minutes: i16,
    ) -> Result<Self, &'static str> {
        if !(1..=12).contains(&month)
            || !(1..=31).contains(&day)
            || hour > 23
            || minute > 59
            || second > 60
            || !(-1_440..=1_440).contains(&offset_minutes)
        {
            return Err("invalid date-time component");
        }
        Ok(Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
            offset_minutes,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rational {
    pub numerator: i32,
    pub denominator: u32,
}

impl Rational {
    /// Creates a nonzero-denominator rational value.
    ///
    /// # Errors
    ///
    /// Returns an error when the denominator is zero.
    pub fn new(numerator: i32, denominator: u32) -> Result<Self, &'static str> {
        if denominator == 0 {
            return Err("rational denominator cannot be zero");
        }
        Ok(Self {
            numerator,
            denominator,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

impl Dimensions {
    /// Creates nonzero image dimensions.
    ///
    /// # Errors
    ///
    /// Returns an error when either dimension is zero.
    pub fn new(width: u32, height: u32) -> Result<Self, &'static str> {
        if width == 0 || height == 0 {
            return Err("dimensions must be nonzero");
        }
        Ok(Self { width, height })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateValue {
    Text(String),
    Integer(i64),
    Rational(Rational),
    DateTime(TemplateDateTime),
    Dimensions(Dimensions),
    Tags(Vec<String>),
    Enum(String),
    Hash(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Available,
    Missing,
    Redacted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableValue {
    pub availability: Availability,
    pub value: Option<TemplateValue>,
}

impl VariableValue {
    #[must_use]
    pub const fn available(value: TemplateValue) -> Self {
        Self {
            availability: Availability::Available,
            value: Some(value),
        }
    }

    #[must_use]
    pub const fn missing() -> Self {
        Self {
            availability: Availability::Missing,
            value: None,
        }
    }

    #[must_use]
    pub const fn redacted() -> Self {
        Self {
            availability: Availability::Redacted,
            value: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct VariableRegistry {
    descriptors: BTreeMap<VariableId, VariableDescriptor>,
}

impl Default for VariableRegistry {
    fn default() -> Self {
        Self::standard()
    }
}

impl VariableRegistry {
    #[must_use]
    pub fn standard() -> Self {
        let mut registry = Self {
            descriptors: BTreeMap::new(),
        };
        for id in standard_ids() {
            registry.insert_descriptor(id.clone(), descriptor_for(id));
        }
        registry
    }

    /// Registers one recipe-owned typed variable without enabling dynamic lookup.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid or duplicate names.
    pub fn register_custom(
        &mut self,
        name: impl Into<String>,
        value_type: VariableType,
        privacy: PrivacyClass,
    ) -> Result<VariableId, &'static str> {
        let name = name.into();
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        {
            return Err("custom variable names must be lowercase ASCII identifiers");
        }
        let id = VariableId::Custom(name);
        if self.descriptors.contains_key(&id) {
            return Err("custom variable is already registered");
        }
        self.insert_descriptor(
            id.clone(),
            VariableDescriptor {
                id: id.clone(),
                value_type,
                privacy,
            },
        );
        Ok(id)
    }

    fn insert_descriptor(&mut self, id: VariableId, descriptor: VariableDescriptor) {
        self.descriptors.insert(id, descriptor);
    }

    #[must_use]
    pub fn resolve(&self, name: &str) -> Option<VariableDescriptor> {
        let builtin = VariableId::from_name(name);
        builtin
            .or_else(|| {
                self.descriptors
                    .keys()
                    .find(|id| id.name() == name)
                    .cloned()
            })
            .and_then(|id| self.descriptors.get(&id).cloned())
    }

    #[must_use]
    pub fn descriptor(&self, id: &VariableId) -> Option<VariableDescriptor> {
        self.descriptors.get(id).cloned()
    }
}

#[derive(Debug, Clone, Default)]
pub struct TemplateContext {
    values: BTreeMap<VariableId, VariableValue>,
}

impl TemplateContext {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, id: VariableId, value: VariableValue) {
        self.values.insert(id, value);
    }

    pub fn set_text(&mut self, id: VariableId, value: impl Into<String>) {
        self.set(
            id,
            VariableValue::available(TemplateValue::Text(value.into())),
        );
    }

    pub fn set_integer(&mut self, id: VariableId, value: i64) {
        self.set(id, VariableValue::available(TemplateValue::Integer(value)));
    }

    pub fn set_tags(&mut self, values: Vec<String>) {
        self.set(
            VariableId::Tags,
            VariableValue::available(TemplateValue::Tags(values)),
        );
    }

    #[must_use]
    pub fn get(&self, id: &VariableId) -> VariableValue {
        self.values
            .get(id)
            .cloned()
            .unwrap_or_else(VariableValue::missing)
    }

    #[must_use]
    pub fn values(&self) -> &BTreeMap<VariableId, VariableValue> {
        &self.values
    }
}

fn standard_ids() -> [VariableId; 59] {
    [
        VariableId::SourceBasename,
        VariableId::SourceStem,
        VariableId::SourceExtension,
        VariableId::SourceRelativeImportPath,
        VariableId::Film,
        VariableId::ImportSession,
        VariableId::PhotoId,
        VariableId::ImageId,
        VariableId::VirtualCopy,
        VariableId::EditRevision,
        VariableId::Sequence,
        VariableId::Index,
        VariableId::Count,
        VariableId::Rating,
        VariableId::ColorLabel,
        VariableId::PickReject,
        VariableId::Title,
        VariableId::Creator,
        VariableId::Copyright,
        VariableId::MetadataArtist,
        VariableId::MetadataDescription,
        VariableId::MetadataLocation,
        VariableId::MetadataKeywords,
        VariableId::Camera,
        VariableId::Lens,
        VariableId::Exposure,
        VariableId::Aperture,
        VariableId::FocalLength,
        VariableId::Iso,
        VariableId::Tags,
        VariableId::CaptureDate,
        VariableId::CaptureTime,
        VariableId::CaptureYear,
        VariableId::CaptureMonth,
        VariableId::CaptureDay,
        VariableId::CaptureHour,
        VariableId::CaptureMinute,
        VariableId::CaptureSecond,
        VariableId::ExportDate,
        VariableId::ExportTime,
        VariableId::ExportYear,
        VariableId::ExportMonth,
        VariableId::ExportDay,
        VariableId::ExportHour,
        VariableId::ExportMinute,
        VariableId::ExportSecond,
        VariableId::Width,
        VariableId::Height,
        VariableId::Aspect,
        VariableId::Orientation,
        VariableId::Profile,
        VariableId::Recipe,
        VariableId::Style,
        VariableId::Encoder,
        VariableId::Extension,
        VariableId::DestinationCollection,
        VariableId::DestinationAlbum,
        VariableId::ContentHash,
        VariableId::RequestHash,
    ]
}

fn descriptor_for(id: VariableId) -> VariableDescriptor {
    let value_type = match id {
        VariableId::PhotoId
        | VariableId::ImageId
        | VariableId::VirtualCopy
        | VariableId::EditRevision
        | VariableId::Sequence
        | VariableId::Index
        | VariableId::Count
        | VariableId::Rating
        | VariableId::CaptureYear
        | VariableId::CaptureMonth
        | VariableId::CaptureDay
        | VariableId::CaptureHour
        | VariableId::CaptureMinute
        | VariableId::CaptureSecond
        | VariableId::ExportYear
        | VariableId::ExportMonth
        | VariableId::ExportDay
        | VariableId::ExportHour
        | VariableId::ExportMinute
        | VariableId::ExportSecond
        | VariableId::Width
        | VariableId::Height
        | VariableId::Iso => VariableType::Integer,
        VariableId::Exposure | VariableId::Aperture | VariableId::FocalLength => {
            VariableType::Rational
        }
        VariableId::CaptureDate
        | VariableId::CaptureTime
        | VariableId::ExportDate
        | VariableId::ExportTime => VariableType::DateTime,
        VariableId::Tags => VariableType::Tags,
        VariableId::ContentHash | VariableId::RequestHash => VariableType::Hash,
        VariableId::Aspect => VariableType::Dimensions,
        VariableId::ColorLabel
        | VariableId::PickReject
        | VariableId::Orientation
        | VariableId::Encoder => VariableType::Enum,
        _ => VariableType::Text,
    };
    let privacy = match id {
        VariableId::SourceRelativeImportPath
        | VariableId::Title
        | VariableId::Creator
        | VariableId::Copyright
        | VariableId::MetadataArtist
        | VariableId::MetadataDescription
        | VariableId::MetadataLocation
        | VariableId::MetadataKeywords
        | VariableId::Camera
        | VariableId::Lens
        | VariableId::Tags => PrivacyClass::Private,
        _ => PrivacyClass::Public,
    };
    VariableDescriptor {
        id,
        value_type,
        privacy,
    }
}
