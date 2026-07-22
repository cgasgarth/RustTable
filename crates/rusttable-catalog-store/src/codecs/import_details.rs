use postcard::{from_bytes, to_allocvec};
use serde::{Deserialize, Serialize};

use rusttable_catalog::{
    IMPORT_DETAILS_VERSION, ImportDetails, ImportMetadataSummary, ImportRegistrationReceipt,
    ImportRegistrationStatus,
};
use rusttable_core::{AssetId, ByteLength, EditId, Orientation, PhotoId};
use rusttable_image::{ImageDimensions, InputFormat};

const DETAILS_FORMAT_VERSION: u8 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct StoredDetails {
    version: u8,
    summary: StoredSummary,
    receipt: StoredReceipt,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredSummary {
    version: u8,
    format: u8,
    width: [u8; 4],
    height: [u8; 4],
    orientation: Option<u8>,
    camera_make_available: bool,
    camera_model_available: bool,
    capture_datetime_available: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredReceipt {
    version: u8,
    source_alias: String,
    content_sha256: [u8; 32],
    byte_length: [u8; 8],
    photo_id: [u8; 16],
    asset_id: [u8; 16],
    edit_id: [u8; 16],
    replaces_photo_id: Option<[u8; 16]>,
    status: u8,
}

pub(crate) fn encode(details: &ImportDetails) -> Result<Vec<u8>, ()> {
    let summary = details.summary();
    let receipt = details.receipt();
    if summary.version() != IMPORT_DETAILS_VERSION || receipt.version() != IMPORT_DETAILS_VERSION {
        return Err(());
    }
    let dimensions = summary.dimensions();
    let stored = StoredDetails {
        version: DETAILS_FORMAT_VERSION,
        summary: StoredSummary {
            version: summary.version(),
            format: encode_format(summary.format()),
            width: dimensions.width().to_be_bytes(),
            height: dimensions.height().to_be_bytes(),
            orientation: summary.orientation().map(Orientation::code),
            camera_make_available: summary.camera_make_available(),
            camera_model_available: summary.camera_model_available(),
            capture_datetime_available: summary.capture_datetime_available(),
        },
        receipt: StoredReceipt {
            version: receipt.version(),
            source_alias: receipt.source_alias().to_owned(),
            content_sha256: receipt.content_sha256(),
            byte_length: receipt.byte_length().get().to_be_bytes(),
            photo_id: receipt.photo_id().get().to_be_bytes(),
            asset_id: receipt.asset_id().get().to_be_bytes(),
            edit_id: receipt.edit_id().get().to_be_bytes(),
            replaces_photo_id: receipt
                .replaces_photo_id()
                .map(|photo_id| photo_id.get().to_be_bytes()),
            status: encode_status(receipt.status()),
        },
    };
    to_allocvec(&stored).map_err(|_| ())
}

pub(crate) fn decode(bytes: &[u8]) -> Result<ImportDetails, ()> {
    let stored: StoredDetails = from_bytes(bytes).map_err(|_| ())?;
    if stored.version != DETAILS_FORMAT_VERSION
        || stored.summary.version != IMPORT_DETAILS_VERSION
        || stored.receipt.version != IMPORT_DETAILS_VERSION
        || decode_status(stored.receipt.status).is_err()
    {
        return Err(());
    }
    let summary = ImportMetadataSummary::new(
        decode_format(stored.summary.format)?,
        ImageDimensions::new(
            u32::from_be_bytes(stored.summary.width),
            u32::from_be_bytes(stored.summary.height),
        )
        .map_err(|_| ())?,
        stored
            .summary
            .orientation
            .map(Orientation::from_u8)
            .transpose()
            .map_err(|_| ())?,
        stored.summary.camera_make_available,
        stored.summary.camera_model_available,
        stored.summary.capture_datetime_available,
    );
    let receipt = ImportRegistrationReceipt::new(
        stored.receipt.source_alias,
        stored.receipt.content_sha256,
        ByteLength::from_bytes(u64::from_be_bytes(stored.receipt.byte_length)),
        PhotoId::new(u128::from_be_bytes(stored.receipt.photo_id)).ok_or(())?,
        AssetId::new(u128::from_be_bytes(stored.receipt.asset_id)).ok_or(())?,
        EditId::new(u128::from_be_bytes(stored.receipt.edit_id)).ok_or(())?,
    )
    .map_err(|_| ())?
    .with_replaces_photo_id(
        stored
            .receipt
            .replaces_photo_id
            .map(u128::from_be_bytes)
            .map(|value| PhotoId::new(value).ok_or(()))
            .transpose()?,
    );
    Ok(ImportDetails::new(summary, receipt))
}

const fn encode_format(format: InputFormat) -> u8 {
    match format {
        InputFormat::Jpeg => 1,
        InputFormat::Png => 2,
        InputFormat::Tiff => 3,
        InputFormat::Raw => 4,
        InputFormat::OpenExr => 5,
    }
}

fn decode_format(value: u8) -> Result<InputFormat, ()> {
    match value {
        1 => Ok(InputFormat::Jpeg),
        2 => Ok(InputFormat::Png),
        3 => Ok(InputFormat::Tiff),
        4 => Ok(InputFormat::Raw),
        5 => Ok(InputFormat::OpenExr),
        _ => Err(()),
    }
}

const fn encode_status(status: ImportRegistrationStatus) -> u8 {
    match status {
        ImportRegistrationStatus::Registered => 1,
    }
}

fn decode_status(value: u8) -> Result<ImportRegistrationStatus, ()> {
    match value {
        1 => Ok(ImportRegistrationStatus::Registered),
        _ => Err(()),
    }
}
