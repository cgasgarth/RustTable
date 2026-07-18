use std::num::NonZeroU32;

use postcard::{from_bytes, to_allocvec};
use serde::{Deserialize, Serialize};

use rusttable_catalog::{ImportCandidate, ImportRecord, SourcePath};
use rusttable_core::{
    ALL_FIELDS, Asset, AssetId, AssetRole, ByteLength, ContentHash, ImageMetadata, MetadataEntry,
    MetadataField, MetadataText, Orientation, Photo, PhotoId, PositiveRational, Revision,
};
use rusttable_image::{ImageDimensions, ImageProbe, InputFormat};

use crate::schema::CURRENT_SCHEMA_VERSION;

const RECORD_FORMAT_VERSION: u8 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct StoredRecord {
    version: u8,
    source: Vec<u8>,
    photo: StoredPhoto,
    probe: StoredProbe,
    metadata: Vec<StoredMetadata>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredPhoto {
    id: [u8; 16],
    revision: [u8; 8],
    asset: StoredAsset,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredAsset {
    id: [u8; 16],
    role: u8,
    hash_algorithm: u8,
    hash: [u8; 32],
    byte_length: [u8; 8],
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredProbe {
    format: u8,
    width: [u8; 4],
    height: [u8; 4],
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredMetadata {
    field: u8,
    kind: u8,
    value: Vec<u8>,
}

pub(crate) fn encode(record: &ImportRecord) -> Result<Vec<u8>, ()> {
    let photo = record.photo();
    if photo.assets().count() != 1 || photo.primary_asset().role() != AssetRole::Primary {
        return Err(());
    }
    let asset = photo.primary_asset();
    let (hash_algorithm, hash) = match asset.content_hash() {
        ContentHash::Sha256(bytes) => (1, bytes),
    };
    let metadata = record
        .metadata()
        .iter()
        .map(encode_metadata)
        .collect::<Result<Vec<_>, _>>()?;
    let stored = StoredRecord {
        version: RECORD_FORMAT_VERSION,
        source: record.source().as_str().as_bytes().to_vec(),
        photo: StoredPhoto {
            id: photo.id().get().to_be_bytes(),
            revision: photo.revision().get().to_be_bytes(),
            asset: StoredAsset {
                id: asset.id().get().to_be_bytes(),
                role: 1,
                hash_algorithm,
                hash,
                byte_length: asset.byte_length().get().to_be_bytes(),
            },
        },
        probe: StoredProbe {
            format: match record.probe().format() {
                InputFormat::Jpeg => 1,
                InputFormat::Png => 2,
            },
            width: record.probe().dimensions().width().to_be_bytes(),
            height: record.probe().dimensions().height().to_be_bytes(),
        },
        metadata,
    };
    to_allocvec(&stored).map_err(|_| ())
}

pub(crate) fn decode(bytes: &[u8]) -> Result<ImportRecord, ()> {
    let stored: StoredRecord = from_bytes(bytes).map_err(|_| ())?;
    if stored.version != RECORD_FORMAT_VERSION || CURRENT_SCHEMA_VERSION != 1 {
        return Err(());
    }
    let source =
        SourcePath::new(std::str::from_utf8(&stored.source).map_err(|_| ())?).map_err(|_| ())?;
    let photo_id = id::<PhotoId>(stored.photo.id)?;
    let asset_id = id::<AssetId>(stored.photo.asset.id)?;
    let asset_role = match stored.photo.asset.role {
        1 => AssetRole::Primary,
        _ => return Err(()),
    };
    let content_hash = match stored.photo.asset.hash_algorithm {
        1 => ContentHash::Sha256(stored.photo.asset.hash),
        _ => return Err(()),
    };
    let asset = Asset::new(
        asset_id,
        asset_role,
        content_hash,
        ByteLength::from_bytes(u64::from_be_bytes(stored.photo.asset.byte_length)),
    );
    let photo = Photo::from_parts(
        photo_id,
        Revision::from_u64(u64::from_be_bytes(stored.photo.revision)),
        [asset],
    )
    .map_err(|_| ())?;
    let format = match stored.probe.format {
        1 => InputFormat::Jpeg,
        2 => InputFormat::Png,
        _ => return Err(()),
    };
    let dimensions = ImageDimensions::new(
        u32::from_be_bytes(stored.probe.width),
        u32::from_be_bytes(stored.probe.height),
    )
    .map_err(|_| ())?;
    let metadata = ImageMetadata::from_entries(
        stored
            .metadata
            .iter()
            .map(decode_metadata)
            .collect::<Result<Vec<_>, _>>()?,
    )
    .map_err(|_| ())?;
    let candidate = ImportCandidate::new(
        photo_id,
        asset_id,
        source,
        content_hash,
        asset.byte_length(),
        ImageProbe::new(format, dimensions),
        metadata,
    )
    .map_err(|_| ())?;
    ImportRecord::new(&candidate, photo).map_err(|_| ())
}

fn id<T>(bytes: [u8; 16]) -> Result<T, ()>
where
    T: From<u128>,
{
    T::from(u128::from_be_bytes(bytes)).ok_or(())
}

trait From<T>: Sized {
    fn from(value: T) -> Option<Self>;
}

impl From<u128> for PhotoId {
    fn from(value: u128) -> Option<Self> {
        Self::new(value)
    }
}
impl From<u128> for AssetId {
    fn from(value: u128) -> Option<Self> {
        Self::new(value)
    }
}

fn encode_metadata((field, entry): (&MetadataField, &MetadataEntry)) -> Result<StoredMetadata, ()> {
    let (kind, value) = match entry {
        MetadataEntry::CameraMake(value)
        | MetadataEntry::CameraModel(value)
        | MetadataEntry::LensModel(value)
        | MetadataEntry::CaptureDateTimeOriginal(value) => (1, value.as_str().as_bytes().to_vec()),
        MetadataEntry::Orientation(value) => (2, vec![value.code()]),
        MetadataEntry::ExposureTime(value)
        | MetadataEntry::FNumber(value)
        | MetadataEntry::FocalLength(value) => {
            let mut bytes = Vec::with_capacity(16);
            bytes.extend_from_slice(&value.numerator().to_be_bytes());
            bytes.extend_from_slice(&value.denominator().to_be_bytes());
            (3, bytes)
        }
        MetadataEntry::IsoSpeed(value) => (4, value.get().to_be_bytes().to_vec()),
    };
    if entry.field() != *field || usize::from(entry.field().rank()) >= ALL_FIELDS.len() {
        return Err(());
    }
    Ok(StoredMetadata {
        field: field.rank(),
        kind,
        value,
    })
}

fn decode_metadata(stored: &StoredMetadata) -> Result<MetadataEntry, ()> {
    let field = *ALL_FIELDS.get(stored.field as usize).ok_or(())?;
    match (field, stored.kind) {
        (MetadataField::CameraMake, 1) => Ok(MetadataEntry::CameraMake(text(&stored.value)?)),
        (MetadataField::CameraModel, 1) => Ok(MetadataEntry::CameraModel(text(&stored.value)?)),
        (MetadataField::LensModel, 1) => Ok(MetadataEntry::LensModel(text(&stored.value)?)),
        (MetadataField::CaptureDateTimeOriginal, 1) => {
            Ok(MetadataEntry::CaptureDateTimeOriginal(text(&stored.value)?))
        }
        (MetadataField::Orientation, 2) => Ok(MetadataEntry::Orientation(
            Orientation::from_u8(*stored.value.first().ok_or(())?).map_err(|_| ())?,
        )),
        (MetadataField::ExposureTime, 3) => {
            Ok(MetadataEntry::ExposureTime(rational(&stored.value)?))
        }
        (MetadataField::FNumber, 3) => Ok(MetadataEntry::FNumber(rational(&stored.value)?)),
        (MetadataField::IsoSpeed, 4) => Ok(MetadataEntry::IsoSpeed(
            NonZeroU32::new(u32::from_be_bytes(array::<4>(&stored.value)?)).ok_or(())?,
        )),
        (MetadataField::FocalLength, 3) => Ok(MetadataEntry::FocalLength(rational(&stored.value)?)),
        _ => Err(()),
    }
}

fn text(bytes: &[u8]) -> Result<MetadataText, ()> {
    MetadataText::from_bytes(bytes.to_vec()).map_err(|_| ())
}

fn rational(bytes: &[u8]) -> Result<PositiveRational, ()> {
    if bytes.len() != 16 {
        return Err(());
    }
    PositiveRational::new(
        u64::from_be_bytes(array::<8>(&bytes[..8])?),
        u64::from_be_bytes(array::<8>(&bytes[8..])?),
    )
    .map_err(|_| ())
}

fn array<const N: usize>(bytes: &[u8]) -> Result<[u8; N], ()> {
    bytes.try_into().map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_use_fixed_width_big_endian_bytes() {
        let record = test_record();
        let bytes = encode(&record).expect("encode");
        let mut expected = vec![
            1, 11, b'f', b'i', b'x', b't', b'u', b'r', b'e', b'.', b'r', b'a', b'w',
        ];
        expected.extend_from_slice(&[0; 15]);
        expected.push(1);
        expected.extend_from_slice(&[0; 8]);
        expected.extend_from_slice(&[0; 15]);
        expected.push(2);
        expected.extend_from_slice(&[1, 1, 3]);
        expected.extend_from_slice(&[3; 31]);
        expected.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 4, 2, 0, 0, 0, 1, 0, 0, 0, 1, 0]);
        assert_eq!(bytes, expected);
        let decoded = decode(&bytes).expect("decode");
        assert_eq!(decoded, record);
    }

    #[test]
    fn every_metadata_entry_kind_round_trips_without_parser_types() {
        let metadata = ImageMetadata::from_entries([
            MetadataEntry::CameraMake(MetadataText::new("Make").unwrap()),
            MetadataEntry::CameraModel(MetadataText::new("Model").unwrap()),
            MetadataEntry::LensModel(MetadataText::new("Lens").unwrap()),
            MetadataEntry::CaptureDateTimeOriginal(MetadataText::new("source text").unwrap()),
            MetadataEntry::Orientation(Orientation::from_u8(6).unwrap()),
            MetadataEntry::ExposureTime(PositiveRational::new(1, 125).unwrap()),
            MetadataEntry::FNumber(PositiveRational::new(28, 10).unwrap()),
            MetadataEntry::IsoSpeed(NonZeroU32::new(400).unwrap()),
            MetadataEntry::FocalLength(PositiveRational::new(50, 1).unwrap()),
        ])
        .unwrap();
        let record = test_record_with_metadata(metadata);
        assert_eq!(decode(&encode(&record).unwrap()).unwrap(), record);
    }

    #[test]
    fn malformed_bytes_never_yield_a_partial_record() {
        assert!(decode(&[1, 2, 3, 4]).is_err());
    }

    fn test_record() -> ImportRecord {
        test_record_with_metadata(ImageMetadata::empty())
    }

    fn test_record_with_metadata(metadata: ImageMetadata) -> ImportRecord {
        let source = SourcePath::new("fixture.raw").expect("source");
        let candidate = ImportCandidate::new(
            PhotoId::new(1).unwrap(),
            AssetId::new(2).unwrap(),
            source,
            ContentHash::Sha256([3; 32]),
            ByteLength::from_bytes(4),
            ImageProbe::new(InputFormat::Png, ImageDimensions::new(1, 1).unwrap()),
            metadata,
        )
        .unwrap();
        let photo = Photo::new(
            candidate.photo_id(),
            [Asset::new(
                candidate.asset_id(),
                AssetRole::Primary,
                candidate.content_hash(),
                candidate.byte_length(),
            )],
        )
        .unwrap();
        ImportRecord::new(&candidate, photo).unwrap()
    }
}
