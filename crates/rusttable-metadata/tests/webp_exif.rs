mod support;

use rusttable_core::{MetadataEntry, MetadataField};
use rusttable_image::InputFormat;
use rusttable_metadata::{ExifMetadataInput, MetadataInput, MetadataInputError, MetadataLimits};

fn input(max_source_bytes: u64, max_exif_bytes: u64) -> ExifMetadataInput {
    ExifMetadataInput::new(
        MetadataLimits::new(max_source_bytes, max_exif_bytes, 16, 16, 4, 32, 128).unwrap(),
    )
}

fn chunk(kind: [u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&kind);
    bytes.extend_from_slice(
        &u32::try_from(payload.len())
            .expect("fixture chunk fits")
            .to_le_bytes(),
    );
    bytes.extend_from_slice(payload);
    if !payload.len().is_multiple_of(2) {
        bytes.push(0);
    }
    bytes
}

fn webp(chunks: &[Vec<u8>]) -> Vec<u8> {
    let mut body = b"WEBP".to_vec();
    for item in chunks {
        body.extend_from_slice(item);
    }
    let mut bytes = b"RIFF".to_vec();
    bytes.extend_from_slice(
        &u32::try_from(body.len())
            .expect("fixture RIFF fits")
            .to_le_bytes(),
    );
    bytes.extend_from_slice(&body);
    bytes
}

fn rewrite_riff_size(bytes: &mut [u8]) {
    let size = u32::try_from(bytes.len() - 8).expect("fixture RIFF fits");
    bytes[4..8].copy_from_slice(&size.to_le_bytes());
}

#[test]
fn extracts_raw_and_exactly_prefixed_exif_after_a_padded_chunk() {
    let tiff = support::tiff_with_metadata();
    let mut prefixed = b"Exif\0\0".to_vec();
    prefixed.extend_from_slice(&tiff);

    for payload in [&tiff[..], &prefixed] {
        let source = webp(&[chunk(*b"JUNK", b"x"), chunk(*b"EXIF", payload)]);
        let metadata = input(4096, 2048)
            .read_bytes(InputFormat::Webp, &source)
            .expect("WebP EXIF parses");
        assert!(
            matches!(metadata.get(MetadataField::CameraMake), Some(MetadataEntry::CameraMake(value)) if value.as_str() == "Canon")
        );
    }
}

#[test]
fn only_the_exact_exif_prefix_is_stripped() {
    let tiff = support::tiff_with_metadata();
    let mut payload = b"Exif\0x".to_vec();
    payload.extend_from_slice(&tiff);
    let source = webp(&[chunk(*b"EXIF", &payload)]);

    assert!(matches!(
        input(4096, 2048).read_bytes(InputFormat::Webp, &source),
        Err(MetadataInputError::MalformedExif)
    ));
}

#[test]
fn duplicate_exif_chunks_are_rejected() {
    let tiff = support::tiff_with_metadata();
    let source = webp(&[chunk(*b"EXIF", &tiff), chunk(*b"EXIF", &tiff)]);

    assert!(matches!(
        input(4096, 2048).read_bytes(InputFormat::Webp, &source),
        Err(MetadataInputError::DuplicateExifPayload {
            format: InputFormat::Webp
        })
    ));
}

#[test]
fn truncated_chunk_payload_and_padding_are_rejected() {
    let tiff = support::tiff_with_metadata();
    let mut truncated_payload = webp(&[chunk(*b"EXIF", &tiff)]);
    truncated_payload.pop();
    rewrite_riff_size(&mut truncated_payload);
    assert!(matches!(
        input(4096, 2048).read_bytes(InputFormat::Webp, &truncated_payload),
        Err(MetadataInputError::MalformedContainer {
            format: InputFormat::Webp,
            ..
        })
    ));

    let mut missing_padding = webp(&[chunk(*b"JUNK", b"x")]);
    missing_padding.pop();
    rewrite_riff_size(&mut missing_padding);
    assert!(matches!(
        input(4096, 2048).read_bytes(InputFormat::Webp, &missing_padding),
        Err(MetadataInputError::MalformedContainer {
            format: InputFormat::Webp,
            ..
        })
    ));

    let mut nonzero_padding = webp(&[chunk(*b"JUNK", b"x")]);
    *nonzero_padding.last_mut().unwrap() = 1;
    assert!(matches!(
        input(4096, 2048).read_bytes(InputFormat::Webp, &nonzero_padding),
        Err(MetadataInputError::MalformedContainer {
            format: InputFormat::Webp,
            ..
        })
    ));
}

#[test]
fn source_and_exif_payload_limits_are_enforced() {
    let tiff = support::tiff_with_metadata();
    let mut prefixed = b"Exif\0\0".to_vec();
    prefixed.extend_from_slice(&tiff);
    let source = webp(&[chunk(*b"EXIF", &prefixed)]);
    let source_bytes = u64::try_from(source.len()).unwrap();
    let tiff_bytes = u64::try_from(tiff.len()).unwrap();

    assert!(matches!(
        input(source_bytes - 1, 2048).read_bytes(InputFormat::Webp, &source),
        Err(MetadataInputError::SourceTooLarge { .. })
    ));
    assert!(matches!(
        input(4096, tiff_bytes - 1).read_bytes(InputFormat::Webp, &source),
        Err(MetadataInputError::ExifPayloadTooLarge {
            actual,
            ..
        }) if actual == tiff_bytes
    ));
}

#[test]
fn signature_and_declared_riff_size_mismatches_are_rejected() {
    let mut wrong_form = webp(&[]);
    wrong_form[8..12].copy_from_slice(b"NOPE");
    assert!(matches!(
        input(4096, 2048).read_bytes(InputFormat::Webp, &wrong_form),
        Err(MetadataInputError::FormatMismatch {
            format: InputFormat::Webp
        })
    ));

    let mut trailing = webp(&[]);
    trailing.push(0);
    assert!(matches!(
        input(4096, 2048).read_bytes(InputFormat::Webp, &trailing),
        Err(MetadataInputError::MalformedContainer {
            format: InputFormat::Webp,
            ..
        })
    ));
}
