use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetadataCategory {
    CameraExposureLens,
    CaptureDateTime,
    DescriptionRights,
    KeywordsRating,
    GpsLocation,
    PeopleRegions,
    OtherRegions,
    SoftwareVersion,
    EditHistory,
    SourceIdentity,
    Technical,
    Thumbnail,
    UnknownImported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetadataAction {
    Include,
    Exclude,
    Redact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalMetadataPolicy {
    pub camera_exposure_lens: MetadataAction,
    pub capture_date_time: MetadataAction,
    pub description_rights: MetadataAction,
    pub keywords_rating: MetadataAction,
    pub gps_location: MetadataAction,
    pub people_regions: MetadataAction,
    pub other_regions: MetadataAction,
    pub software_version: MetadataAction,
    pub edit_history: MetadataAction,
    pub source_identity: MetadataAction,
    pub technical: MetadataAction,
    pub thumbnail: MetadataAction,
    pub unknown_imported: MetadataAction,
}

impl Default for CanonicalMetadataPolicy {
    fn default() -> Self {
        Self {
            camera_exposure_lens: MetadataAction::Include,
            capture_date_time: MetadataAction::Include,
            description_rights: MetadataAction::Include,
            keywords_rating: MetadataAction::Include,
            gps_location: MetadataAction::Exclude,
            people_regions: MetadataAction::Exclude,
            other_regions: MetadataAction::Exclude,
            software_version: MetadataAction::Include,
            edit_history: MetadataAction::Exclude,
            source_identity: MetadataAction::Exclude,
            technical: MetadataAction::Include,
            thumbnail: MetadataAction::Exclude,
            unknown_imported: MetadataAction::Exclude,
        }
    }
}

impl CanonicalMetadataPolicy {
    #[must_use]
    pub const fn action(self, category: MetadataCategory) -> MetadataAction {
        match category {
            MetadataCategory::CameraExposureLens => self.camera_exposure_lens,
            MetadataCategory::CaptureDateTime => self.capture_date_time,
            MetadataCategory::DescriptionRights => self.description_rights,
            MetadataCategory::KeywordsRating => self.keywords_rating,
            MetadataCategory::GpsLocation => self.gps_location,
            MetadataCategory::PeopleRegions => self.people_regions,
            MetadataCategory::OtherRegions => self.other_regions,
            MetadataCategory::SoftwareVersion => self.software_version,
            MetadataCategory::EditHistory => self.edit_history,
            MetadataCategory::SourceIdentity => self.source_identity,
            MetadataCategory::Technical => self.technical,
            MetadataCategory::Thumbnail => self.thumbnail,
            MetadataCategory::UnknownImported => self.unknown_imported,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetadataSource {
    Exif,
    Iptc,
    /// Legacy XMP source when the caller cannot distinguish embedded and sidecar packets.
    Xmp,
    /// Legacy import-default source.
    Imported,
    /// Legacy catalog/user-edit source.
    CatalogEdit,
    /// Legacy export-only override source.
    RecipeOverride,
    GeneratedTechnical,
    Container,
    MakerNote,
    EmbeddedXmp,
    SidecarXmp,
    ImportDefault,
    CatalogValue,
    UserOverride,
    ExportOverride,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetadataSourceClass {
    Extracted,
    ImportDefault,
    Catalog,
    UserOverride,
    ExportOnly,
    Generated,
}

impl MetadataSource {
    #[must_use]
    pub const fn precedence(self) -> u16 {
        match self {
            Self::Imported | Self::ImportDefault => 0,
            Self::Container => 10,
            Self::Exif => 20,
            Self::MakerNote => 30,
            Self::Iptc => 40,
            Self::Xmp | Self::EmbeddedXmp => 50,
            Self::SidecarXmp => 60,
            Self::CatalogValue => 70,
            Self::CatalogEdit | Self::UserOverride => 80,
            Self::RecipeOverride | Self::ExportOverride => 90,
            Self::GeneratedTechnical => 100,
        }
    }

    #[must_use]
    pub const fn class(self) -> MetadataSourceClass {
        match self {
            Self::Container
            | Self::Exif
            | Self::MakerNote
            | Self::Iptc
            | Self::Xmp
            | Self::EmbeddedXmp
            | Self::SidecarXmp => MetadataSourceClass::Extracted,
            Self::Imported | Self::ImportDefault => MetadataSourceClass::ImportDefault,
            Self::CatalogValue => MetadataSourceClass::Catalog,
            Self::CatalogEdit | Self::UserOverride => MetadataSourceClass::UserOverride,
            Self::RecipeOverride | Self::ExportOverride => MetadataSourceClass::ExportOnly,
            Self::GeneratedTechnical => MetadataSourceClass::Generated,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exif => "exif",
            Self::Iptc => "iptc",
            Self::Xmp => "xmp",
            Self::Imported => "imported",
            Self::CatalogEdit => "catalog-edit",
            Self::RecipeOverride => "recipe-override",
            Self::GeneratedTechnical => "generated-technical",
            Self::Container => "container",
            Self::MakerNote => "maker-note",
            Self::EmbeddedXmp => "embedded-xmp",
            Self::SidecarXmp => "sidecar-xmp",
            Self::ImportDefault => "import-default",
            Self::CatalogValue => "catalog-value",
            Self::UserOverride => "user-override",
            Self::ExportOverride => "export-override",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataValue {
    Text(String),
    Integer(i64),
    Rational {
        numerator: i64,
        denominator: u64,
    },
    SignedRational {
        numerator: i64,
        denominator: i64,
    },
    Float(u64),
    DateTime(String),
    Binary(Vec<u8>),
    Array(Vec<MetadataValue>),
    Region {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataProperty {
    pub namespace: String,
    pub name: String,
    pub category: MetadataCategory,
    pub source: MetadataSource,
    pub value: MetadataValue,
}

impl MetadataProperty {
    #[must_use]
    pub fn new(
        namespace: impl Into<String>,
        name: impl Into<String>,
        category: MetadataCategory,
        source: MetadataSource,
        value: MetadataValue,
    ) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
            category,
            source,
            value,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataBuildError {
    EmptyName,
    ValueLimit { property: String },
    InvalidRational { property: String },
    NonFinite { property: String },
    TooManyProperties,
    TooManyBytes,
}

impl fmt::Display for MetadataBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "metadata packet build failed: {self:?}")
    }
}
impl std::error::Error for MetadataBuildError {}
