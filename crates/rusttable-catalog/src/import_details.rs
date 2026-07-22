use rusttable_core::{
    AssetId, ByteLength, ContentHash, Edit, EditId, MetadataEntry, MetadataField, Orientation,
    PhotoId,
};
use rusttable_image::{ImageDimensions, InputFormat};

use crate::ImportRecord;

pub const IMPORT_DETAILS_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMetadataStatus {
    Available,
    Unavailable,
}

/// Opaque, deterministic identity for an accepted reference path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReferencePathIdentity([u8; 32]);

impl ReferencePathIdentity {
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl From<[u8; 32]> for ReferencePathIdentity {
    fn from(bytes: [u8; 32]) -> Self {
        Self::new(bytes)
    }
}

/// Non-sensitive facts extracted while an image is registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportMetadataSummary {
    version: u8,
    format: InputFormat,
    dimensions: ImageDimensions,
    orientation: Option<Orientation>,
    camera_make_available: bool,
    camera_model_available: bool,
    capture_datetime_available: bool,
    metadata_status: ImportMetadataStatus,
}

impl ImportMetadataSummary {
    #[must_use]
    pub fn from_record(record: &ImportRecord) -> Self {
        Self::from_record_with_status(record, ImportMetadataStatus::Available)
    }

    #[must_use]
    pub fn from_record_with_status(
        record: &ImportRecord,
        metadata_status: ImportMetadataStatus,
    ) -> Self {
        let metadata = record.metadata();
        let orientation = match metadata.get(MetadataField::Orientation) {
            Some(MetadataEntry::Orientation(value)) => Some(*value),
            _ => None,
        };
        Self {
            version: IMPORT_DETAILS_VERSION,
            format: record.probe().format(),
            dimensions: record.probe().dimensions(),
            orientation,
            camera_make_available: metadata.get(MetadataField::CameraMake).is_some(),
            camera_model_available: metadata.get(MetadataField::CameraModel).is_some(),
            capture_datetime_available: metadata
                .get(MetadataField::CaptureDateTimeOriginal)
                .is_some(),
            metadata_status,
        }
    }

    #[must_use]
    pub const fn new(
        format: InputFormat,
        dimensions: ImageDimensions,
        orientation: Option<Orientation>,
        camera_make_available: bool,
        camera_model_available: bool,
        capture_datetime_available: bool,
    ) -> Self {
        Self {
            version: IMPORT_DETAILS_VERSION,
            format,
            dimensions,
            orientation,
            camera_make_available,
            camera_model_available,
            capture_datetime_available,
            metadata_status: ImportMetadataStatus::Available,
        }
    }

    #[must_use]
    pub const fn new_with_status(
        format: InputFormat,
        dimensions: ImageDimensions,
        orientation: Option<Orientation>,
        camera_make_available: bool,
        camera_model_available: bool,
        capture_datetime_available: bool,
        metadata_status: ImportMetadataStatus,
    ) -> Self {
        Self {
            version: IMPORT_DETAILS_VERSION,
            format,
            dimensions,
            orientation,
            camera_make_available,
            camera_model_available,
            capture_datetime_available,
            metadata_status,
        }
    }

    #[must_use]
    pub const fn version(self) -> u8 {
        self.version
    }

    #[must_use]
    pub const fn format(self) -> InputFormat {
        self.format
    }

    #[must_use]
    pub const fn dimensions(self) -> ImageDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn orientation(self) -> Option<Orientation> {
        self.orientation
    }

    #[must_use]
    pub const fn camera_make_available(self) -> bool {
        self.camera_make_available
    }

    #[must_use]
    pub const fn camera_model_available(self) -> bool {
        self.camera_model_available
    }

    #[must_use]
    pub const fn capture_datetime_available(self) -> bool {
        self.capture_datetime_available
    }

    #[must_use]
    pub const fn metadata_status(self) -> ImportMetadataStatus {
        self.metadata_status
    }

    #[must_use]
    pub fn same_record_facts(self, other: Self) -> bool {
        self.version == other.version
            && self.format == other.format
            && self.dimensions == other.dimensions
            && self.orientation == other.orientation
            && self.camera_make_available == other.camera_make_available
            && self.camera_model_available == other.camera_model_available
            && self.capture_datetime_available == other.capture_datetime_available
    }
}

/// Immutable proof that one source registration reached durable storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRegistrationReceipt {
    version: u8,
    source_alias: String,
    content_sha256: [u8; 32],
    byte_length: ByteLength,
    photo_id: PhotoId,
    asset_id: AssetId,
    edit_id: EditId,
    replaces_photo_id: Option<PhotoId>,
    status: ImportRegistrationStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportRegistrationReceiptError {
    EmptyAlias,
    AliasTooLong,
    UnsafeAlias,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportRegistrationStatus {
    Registered,
}

impl ImportRegistrationReceipt {
    /// Creates a receipt without a path-reuse predecessor.
    ///
    /// # Errors
    ///
    /// Returns an error when the presentation alias is empty, oversized, or unsafe to display.
    pub fn new(
        source_alias: String,
        content_sha256: [u8; 32],
        byte_length: ByteLength,
        photo_id: PhotoId,
        asset_id: AssetId,
        edit_id: EditId,
    ) -> Result<Self, ImportRegistrationReceiptError> {
        validate_alias(&source_alias)?;
        Ok(Self {
            version: IMPORT_DETAILS_VERSION,
            source_alias,
            content_sha256,
            byte_length,
            photo_id,
            asset_id,
            edit_id,
            replaces_photo_id: None,
            status: ImportRegistrationStatus::Registered,
        })
    }

    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    #[must_use]
    pub fn source_alias(&self) -> &str {
        &self.source_alias
    }

    #[must_use]
    pub const fn content_sha256(&self) -> [u8; 32] {
        self.content_sha256
    }

    #[must_use]
    pub const fn byte_length(&self) -> ByteLength {
        self.byte_length
    }

    #[must_use]
    pub const fn photo_id(&self) -> PhotoId {
        self.photo_id
    }

    #[must_use]
    pub const fn asset_id(&self) -> AssetId {
        self.asset_id
    }

    #[must_use]
    pub const fn edit_id(&self) -> EditId {
        self.edit_id
    }

    #[must_use]
    pub const fn replaces_photo_id(&self) -> Option<PhotoId> {
        self.replaces_photo_id
    }

    #[must_use]
    pub const fn status(&self) -> ImportRegistrationStatus {
        self.status
    }

    #[must_use]
    pub fn with_replaces_photo_id(mut self, photo_id: Option<PhotoId>) -> Self {
        self.replaces_photo_id = photo_id;
        self
    }
}

/// Versioned, privacy-safe details persisted with a registered import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDetails {
    summary: ImportMetadataSummary,
    receipt: ImportRegistrationReceipt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportDetailsValidationError {
    MismatchedRecordOrEdit,
}

impl ImportDetails {
    #[must_use]
    pub const fn new(summary: ImportMetadataSummary, receipt: ImportRegistrationReceipt) -> Self {
        Self { summary, receipt }
    }

    #[must_use]
    pub const fn summary(&self) -> ImportMetadataSummary {
        self.summary
    }

    #[must_use]
    pub const fn receipt(&self) -> &ImportRegistrationReceipt {
        &self.receipt
    }

    #[must_use]
    pub fn with_replaces_photo_id(mut self, photo_id: Option<PhotoId>) -> Self {
        self.receipt = self.receipt.with_replaces_photo_id(photo_id);
        self
    }

    #[must_use]
    pub fn with_metadata_status(mut self, status: ImportMetadataStatus) -> Self {
        self.summary.metadata_status = status;
        self
    }

    /// Checks that the durable facts exactly describe one record and its edit.
    ///
    /// # Errors
    ///
    /// Returns an error when an adapter attempts to persist mismatched evidence.
    pub fn validate(
        &self,
        record: &ImportRecord,
        edit: &Edit,
    ) -> Result<(), ImportDetailsValidationError> {
        let ContentHash::Sha256(content_sha256) = record.photo().primary_asset().content_hash();
        if !self
            .summary
            .same_record_facts(ImportMetadataSummary::from_record(record))
            || self.receipt.content_sha256 != content_sha256
            || self.receipt.byte_length != record.photo().primary_asset().byte_length()
            || self.receipt.photo_id != record.photo().id()
            || self.receipt.asset_id != record.photo().primary_asset_id()
            || self.receipt.edit_id != edit.id()
            || edit.photo_id() != record.photo().id()
        {
            return Err(ImportDetailsValidationError::MismatchedRecordOrEdit);
        }
        Ok(())
    }
}

/// Input to one atomic registration write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRegistration {
    details: ImportDetails,
    reference_path_identity: ReferencePathIdentity,
}

impl ImportRegistration {
    #[must_use]
    pub const fn new(
        details: ImportDetails,
        reference_path_identity: ReferencePathIdentity,
    ) -> Self {
        Self {
            details,
            reference_path_identity,
        }
    }

    #[must_use]
    pub const fn details(&self) -> &ImportDetails {
        &self.details
    }

    #[must_use]
    pub const fn reference_path_identity(&self) -> ReferencePathIdentity {
        self.reference_path_identity
    }
}

fn validate_alias(alias: &str) -> Result<(), ImportRegistrationReceiptError> {
    if alias.is_empty() {
        return Err(ImportRegistrationReceiptError::EmptyAlias);
    }
    if alias.chars().count() > 128 {
        return Err(ImportRegistrationReceiptError::AliasTooLong);
    }
    if alias
        .chars()
        .any(|character| character.is_control() || matches!(character, '/' | '\\'))
    {
        return Err(ImportRegistrationReceiptError::UnsafeAlias);
    }
    Ok(())
}
