use std::fmt;
use std::path::Path;

use rusttable_catalog::{EditRepository, ImportRepository};
use rusttable_catalog_store::RedbCatalogRepository;
use rusttable_core::{Edit, ImageMetadata, MetadataEntry, PhotoId};
use rusttable_export::{
    ExportMetadata, MetadataPacketBuilder, MetadataPolicy, MetadataProperty, MetadataSource,
    MetadataValue, PngMetadataReceipt,
};
use rusttable_metadata::MetadataCategory;
use rusttable_render::RenderOutput;

#[derive(Debug)]
pub(crate) enum ExportMetadataError {
    Catalog(String),
    MissingPhoto(PhotoId),
    MissingEdit(rusttable_core::EditId),
    Packet(rusttable_metadata::MetadataBuildError),
}

impl fmt::Display for ExportMetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Catalog(error) => write!(formatter, "metadata catalog snapshot failed: {error}"),
            Self::MissingPhoto(photo_id) => write!(formatter, "photo {photo_id} is missing"),
            Self::MissingEdit(edit_id) => write!(formatter, "edit {edit_id} is missing"),
            Self::Packet(error) => write!(formatter, "metadata packet failed: {error}"),
        }
    }
}

impl std::error::Error for ExportMetadataError {}

pub(crate) struct ResolvedExportMetadata {
    pub(crate) metadata: ExportMetadata,
    pub(crate) receipt: PngMetadataReceipt,
}

/// Resolves one owned packet from the imported source, catalog organization,
/// selected edit, and rendered working/profile evidence. The policy is applied
/// by `MetadataPacketBuilder` before any format-native serialization occurs.
pub(crate) fn resolve(
    catalog_path: &Path,
    photo_id: PhotoId,
    edit_id: rusttable_core::EditId,
    render: &RenderOutput,
    policy: MetadataPolicy,
) -> Result<ResolvedExportMetadata, ExportMetadataError> {
    let repository = RedbCatalogRepository::open(catalog_path)
        .map_err(|error| ExportMetadataError::Catalog(format!("{error:?}")))?;
    let record = repository
        .find_by_photo_id(photo_id)
        .map_err(|error| ExportMetadataError::Catalog(format!("{error:?}")))?
        .ok_or(ExportMetadataError::MissingPhoto(photo_id))?;
    let edit = repository
        .find_by_edit_id(edit_id)
        .map_err(|error| ExportMetadataError::Catalog(format!("{error:?}")))?
        .ok_or(ExportMetadataError::MissingEdit(edit_id))?;

    let organization = repository
        .organization_states()
        .map_err(|error| ExportMetadataError::Catalog(format!("{error:?}")))?
        .remove(&photo_id);
    let mut builder = MetadataPacketBuilder::new(policy.canonical());
    builder = add_imported_metadata(builder, record.metadata());
    builder = add_catalog_metadata(builder, organization.as_ref());
    builder = add_edit_metadata(builder, &edit);
    builder = add_profile_metadata(builder, render);
    let packet = builder.build().map_err(ExportMetadataError::Packet)?;
    let receipt = PngMetadataReceipt::from_packet(&packet, policy);
    Ok(ResolvedExportMetadata {
        metadata: ExportMetadata::from_packet(packet),
        receipt,
    })
}

fn add_imported_metadata(
    mut builder: MetadataPacketBuilder,
    metadata: &ImageMetadata,
) -> MetadataPacketBuilder {
    for (_, entry) in metadata.iter() {
        let (name, category, value) = match entry {
            MetadataEntry::CameraMake(value) => (
                "CameraMake",
                MetadataCategory::CameraExposureLens,
                MetadataValue::Text(value.as_str().to_owned()),
            ),
            MetadataEntry::CameraModel(value) => (
                "CameraModel",
                MetadataCategory::CameraExposureLens,
                MetadataValue::Text(value.as_str().to_owned()),
            ),
            MetadataEntry::LensModel(value) => (
                "LensModel",
                MetadataCategory::CameraExposureLens,
                MetadataValue::Text(value.as_str().to_owned()),
            ),
            MetadataEntry::CaptureDateTimeOriginal(value) => (
                "CaptureDateTimeOriginal",
                MetadataCategory::CaptureDateTime,
                MetadataValue::DateTime(value.as_str().to_owned()),
            ),
            MetadataEntry::Orientation(value) => (
                "Orientation",
                MetadataCategory::Technical,
                MetadataValue::Integer(i64::from(value.code())),
            ),
            MetadataEntry::ExposureTime(value) => (
                "ExposureTime",
                MetadataCategory::CameraExposureLens,
                MetadataValue::Rational {
                    numerator: i64::try_from(value.numerator()).unwrap_or(i64::MAX),
                    denominator: value.denominator(),
                },
            ),
            MetadataEntry::FNumber(value) => (
                "FNumber",
                MetadataCategory::CameraExposureLens,
                MetadataValue::Rational {
                    numerator: i64::try_from(value.numerator()).unwrap_or(i64::MAX),
                    denominator: value.denominator(),
                },
            ),
            MetadataEntry::IsoSpeed(value) => (
                "IsoSpeed",
                MetadataCategory::CameraExposureLens,
                MetadataValue::Integer(i64::from(value.get())),
            ),
            MetadataEntry::FocalLength(value) => (
                "FocalLength",
                MetadataCategory::CameraExposureLens,
                MetadataValue::Rational {
                    numerator: i64::try_from(value.numerator()).unwrap_or(i64::MAX),
                    denominator: value.denominator(),
                },
            ),
        };
        builder = builder.add_property(MetadataProperty::new(
            "exif",
            name,
            category,
            MetadataSource::Imported,
            value,
        ));
    }
    builder
}

fn add_catalog_metadata(
    mut builder: MetadataPacketBuilder,
    organization: Option<&rusttable_catalog::PhotoOrganizationState>,
) -> MetadataPacketBuilder {
    let Some(organization) = organization else {
        return builder;
    };
    builder = builder.add_property(MetadataProperty::new(
        "xmp",
        "Rating",
        MetadataCategory::KeywordsRating,
        MetadataSource::CatalogEdit,
        MetadataValue::Integer(i64::from(organization.rating.as_u8())),
    ));
    builder = builder.add_property(MetadataProperty::new(
        "xmp",
        "Rejected",
        MetadataCategory::KeywordsRating,
        MetadataSource::CatalogEdit,
        MetadataValue::Integer(i64::from(u8::from(organization.rejected))),
    ));
    let labels = organization
        .color_labels
        .iter()
        .map(|label| MetadataValue::Text(format!("{label:?}")))
        .collect();
    builder.add_property(MetadataProperty::new(
        "xmp",
        "ColorLabels",
        MetadataCategory::KeywordsRating,
        MetadataSource::CatalogEdit,
        MetadataValue::Array(labels),
    ))
}

fn add_edit_metadata(mut builder: MetadataPacketBuilder, edit: &Edit) -> MetadataPacketBuilder {
    builder = builder.add_property(MetadataProperty::new(
        "rusttable",
        "EditId",
        MetadataCategory::EditHistory,
        MetadataSource::CatalogEdit,
        MetadataValue::Integer(i64::try_from(edit.id().get()).unwrap_or(i64::MAX)),
    ));
    builder = builder.add_property(MetadataProperty::new(
        "rusttable",
        "EditRevision",
        MetadataCategory::EditHistory,
        MetadataSource::CatalogEdit,
        MetadataValue::Integer(i64::try_from(edit.revision().get()).unwrap_or(i64::MAX)),
    ));
    builder.add_property(MetadataProperty::new(
        "rusttable",
        "OperationCount",
        MetadataCategory::EditHistory,
        MetadataSource::CatalogEdit,
        MetadataValue::Integer(i64::try_from(edit.operations().count()).unwrap_or(i64::MAX)),
    ))
}

fn add_profile_metadata(
    mut builder: MetadataPacketBuilder,
    render: &RenderOutput,
) -> MetadataPacketBuilder {
    let profile = render.working_profile();
    builder = builder.add_property(MetadataProperty::new(
        "rusttable",
        "WorkingProfile",
        MetadataCategory::Technical,
        MetadataSource::GeneratedTechnical,
        MetadataValue::Text(format!("{:?}", profile.encoding())),
    ));
    builder = builder.add_property(MetadataProperty::new(
        "rusttable",
        "WorkingProfileProvenance",
        MetadataCategory::Technical,
        MetadataSource::GeneratedTechnical,
        MetadataValue::Text(format!("{:?}", profile.provenance())),
    ));
    if let Some(profile_id) = profile.profile_id() {
        builder = builder.add_property(MetadataProperty::new(
            "rusttable",
            "WorkingProfileId",
            MetadataCategory::Technical,
            MetadataSource::GeneratedTechnical,
            MetadataValue::Text(profile_id.to_string()),
        ));
    }
    builder.add_property(MetadataProperty::new(
        "rusttable",
        "RenderProfileIdentity",
        MetadataCategory::Technical,
        MetadataSource::GeneratedTechnical,
        MetadataValue::Binary(profile.identity().to_vec()),
    ))
}
