use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rusttable_export::{
    CollisionPolicy, ExportMetadata, MetadataAction, MetadataPolicy, PngCollisionResult,
    PngExportLimits, PngMetadataReceipt, PngPublishCompletion, PngPublishControl, PngPublishError,
    PngPublishProgress, PngPublishStage, PngPublisher,
};
use rusttable_image::{ColorEncoding, DecodeLimits, DecodedImage, ImageDimensions, ImageInput};
use rusttable_image_io::FileImageInput;
use rusttable_metadata::{
    MetadataCategory, MetadataPacketBuilder, MetadataProperty, MetadataSource, MetadataValue,
};

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn image() -> DecodedImage {
    DecodedImage::new_with_color_encoding(
        ImageDimensions::new(2, 1).expect("dimensions"),
        vec![255, 0, 0, 255, 0, 255, 0, 128],
        ColorEncoding::SrgbD65,
    )
    .expect("pixels match dimensions")
}

fn test_directory(name: &str) -> PathBuf {
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "rusttable-export-{name}-{}-{sequence}",
        std::process::id()
    ));
    fs::create_dir(&path).expect("test directory should be unused");
    path
}

fn publisher(max_width: u32, max_height: u32, max_bytes: u64) -> PngPublisher {
    PngPublisher::new(PngExportLimits::new(max_width, max_height, max_bytes).expect("valid limits"))
}

fn decode(path: &Path) -> DecodedImage {
    let limits = DecodeLimits::new(1_000_000, 2, 1, 2, 8).expect("decode limits");
    FileImageInput::new(limits)
        .decode_path(path)
        .expect("PNG should decode")
}

fn metadata_policy() -> MetadataPolicy {
    MetadataPolicy {
        exif: MetadataAction::Include,
        iptc: MetadataAction::Include,
        xmp: MetadataAction::Include,
        gps: MetadataAction::Exclude,
        faces_and_regions: MetadataAction::Exclude,
        ratings_labels_tags: MetadataAction::Include,
        history: MetadataAction::Exclude,
        thumbnail: MetadataAction::Exclude,
        icc_and_cicp: MetadataAction::Include,
        software_and_version: MetadataAction::Include,
        user_fields: MetadataAction::Exclude,
    }
}

fn metadata_packet(policy: MetadataPolicy) -> rusttable_export::MetadataPacket {
    MetadataPacketBuilder::new(policy.canonical())
        .add_property(MetadataProperty::new(
            "exif",
            "CameraMake",
            MetadataCategory::CameraExposureLens,
            MetadataSource::Imported,
            MetadataValue::Text("RustTable Camera".into()),
        ))
        .add_property(MetadataProperty::new(
            "xmp",
            "Description",
            MetadataCategory::DescriptionRights,
            MetadataSource::Imported,
            MetadataValue::Text("round-trip description".into()),
        ))
        .add_property(MetadataProperty::new(
            "xmp",
            "Rating",
            MetadataCategory::KeywordsRating,
            MetadataSource::CatalogEdit,
            MetadataValue::Integer(5),
        ))
        .add_property(MetadataProperty::new(
            "exif",
            "GPSLatitude",
            MetadataCategory::GpsLocation,
            MetadataSource::Imported,
            MetadataValue::Text("41.9".into()),
        ))
        .add_property(MetadataProperty::new(
            "rusttable",
            "EditRevision",
            MetadataCategory::EditHistory,
            MetadataSource::CatalogEdit,
            MetadataValue::Integer(7),
        ))
        .add_property(MetadataProperty::new(
            "rusttable",
            "WorkingProfile",
            MetadataCategory::Technical,
            MetadataSource::GeneratedTechnical,
            MetadataValue::Text("sRGB".into()),
        ))
        .build()
        .expect("metadata packet")
}

#[test]
fn create_new_publishes_and_verifies_the_rendered_pixels() {
    let directory = test_directory("create-new");
    let destination = directory.join("render.png");
    let source = image();

    let receipt = publisher(2, 1, 1_000_000)
        .publish(&source, &destination, CollisionPolicy::CreateNew)
        .expect("PNG publication");

    assert_eq!(receipt.destination(), destination);
    assert_eq!(receipt.dimensions(), source.dimensions());
    assert_eq!(receipt.verified_dimensions(), source.dimensions());
    assert_eq!(receipt.collision(), PngCollisionResult::CreatedNew);
    assert_eq!(receipt.completion(), PngPublishCompletion::Completed);
    assert_eq!(receipt.verification().artifact_sha256().len(), 64);
    assert_eq!(receipt.verification().pixel_sha256().len(), 64);
    assert_eq!(decode(&destination), source);
    assert_eq!(fs::read_dir(&directory).expect("directory").count(), 1);
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn create_new_never_replaces_an_existing_destination() {
    let directory = test_directory("collision");
    let destination = directory.join("render.png");
    fs::write(&destination, b"existing").expect("seed destination");

    let error = publisher(2, 1, 1_000_000)
        .publish(&image(), &destination, CollisionPolicy::CreateNew)
        .expect_err("existing destination must be rejected");

    assert!(matches!(error, PngPublishError::DestinationExists { .. }));
    assert_eq!(fs::read(&destination).expect("destination"), b"existing");
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn replace_existing_atomically_publishes_the_new_png() {
    let directory = test_directory("replace");
    let destination = directory.join("render.png");
    fs::write(&destination, b"old output").expect("seed destination");

    let receipt = publisher(2, 1, 1_000_000)
        .publish(&image(), &destination, CollisionPolicy::ReplaceExisting)
        .expect("replacement publication");

    assert_eq!(receipt.collision(), PngCollisionResult::ReplacedExisting);
    assert_eq!(decode(&destination), image());
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn observer_reports_explicit_stages_and_receipt_verification_data() {
    let directory = test_directory("progress");
    let destination = directory.join("render.png");
    let mut stages = Vec::new();

    let receipt = publisher(2, 1, 1_000_000)
        .publish_with_observer(
            &image(),
            &destination,
            CollisionPolicy::CreateNew,
            |progress: PngPublishProgress| {
                stages.push(progress.stage());
                PngPublishControl::Continue
            },
        )
        .expect("PNG publication");

    assert_eq!(
        stages,
        vec![
            PngPublishStage::Preparing,
            PngPublishStage::Encoding,
            PngPublishStage::Verifying,
            PngPublishStage::Publishing,
            PngPublishStage::Completed,
        ]
    );
    assert_eq!(receipt.verification().dimensions(), image().dimensions());
    assert!(
        receipt
            .verification()
            .artifact_sha256()
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    );
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn cancellation_before_publication_removes_staging_and_preserves_destination() {
    let directory = test_directory("cancel-before-publish");
    let destination = directory.join("render.png");
    fs::write(&destination, b"previous artifact").expect("seed destination");

    let error = publisher(2, 1, 1_000_000)
        .publish_with_observer(
            &image(),
            &destination,
            CollisionPolicy::ReplaceExisting,
            |progress: PngPublishProgress| {
                if progress.stage() == PngPublishStage::Publishing {
                    PngPublishControl::Cancel
                } else {
                    PngPublishControl::Continue
                }
            },
        )
        .expect_err("cancellation must stop before replacement");

    assert!(matches!(
        error,
        PngPublishError::Cancelled {
            stage: PngPublishStage::Publishing
        }
    ));
    assert_eq!(
        fs::read(&destination).expect("destination"),
        b"previous artifact"
    );
    assert_eq!(fs::read_dir(&directory).expect("directory").count(), 1);
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn cancellation_after_publication_is_recorded_without_rollback() {
    let directory = test_directory("cancel-after-publish");
    let destination = directory.join("render.png");

    let receipt = publisher(2, 1, 1_000_000)
        .publish_with_observer(
            &image(),
            &destination,
            CollisionPolicy::CreateNew,
            |progress: PngPublishProgress| {
                if progress.stage() == PngPublishStage::Completed {
                    PngPublishControl::Cancel
                } else {
                    PngPublishControl::Continue
                }
            },
        )
        .expect("completed publication cannot be rolled back");

    assert_eq!(
        receipt.completion(),
        PngPublishCompletion::CompletedAfterCancellation
    );
    assert_eq!(decode(&destination), image());
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn dimensions_and_encoded_bytes_are_rejected_before_publication() {
    let directory = test_directory("limits");
    let destination = directory.join("render.png");

    let dimension_error = publisher(1, 1, 1_000_000)
        .publish(&image(), &destination, CollisionPolicy::CreateNew)
        .expect_err("dimensions must be bounded");
    assert!(matches!(
        dimension_error,
        PngPublishError::DimensionLimit { .. }
    ));
    assert!(!destination.exists());

    let byte_error = publisher(2, 1, 8)
        .publish(&image(), &destination, CollisionPolicy::CreateNew)
        .expect_err("encoded bytes must be bounded");
    assert!(matches!(
        byte_error,
        PngPublishError::Output(rusttable_image::ImageOutputError::EncodedOutputTooLarge { .. })
    ));
    assert!(!destination.exists());
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn metadata_policy_round_trips_supported_fields_and_excludes_private_groups() {
    let directory = test_directory("metadata-policy");
    let destination = directory.join("render.png");
    let policy = metadata_policy();
    let packet = metadata_packet(policy);
    let packet_fields = packet.property_names();
    let metadata = ExportMetadata::from_packet(packet.clone());
    let metadata_receipt = PngMetadataReceipt::from_packet(&packet, policy);

    let receipt = publisher(2, 1, 1_000_000)
        .publish_with_metadata(
            &image(),
            &destination,
            CollisionPolicy::CreateNew,
            metadata,
            metadata_receipt,
        )
        .expect("PNG with metadata publication");

    let bytes = fs::read(&destination).expect("published PNG");
    let reader = png::Decoder::new(Cursor::new(bytes))
        .read_info()
        .expect("PNG metadata round trip");
    let info = reader.info();
    assert!(
        info.exif_metadata.is_some(),
        "supported EXIF should survive"
    );
    let xmp = info
        .utf8_text
        .iter()
        .find(|chunk| chunk.keyword == "XML:com.adobe.xmp")
        .expect("XMP iTXt chunk")
        .get_text()
        .expect("XMP text");
    assert!(xmp.contains("exif:CameraMake"));
    assert!(xmp.contains("xmp:Description"));
    assert!(xmp.contains("xmp:Rating"));
    assert!(xmp.contains("WorkingProfile"));
    assert!(!xmp.contains("GPSLatitude"));
    assert!(!xmp.contains("EditRevision"));
    assert_eq!(
        receipt
            .metadata()
            .expect("metadata receipt")
            .included_fields(),
        &packet_fields.into_iter().collect::<Vec<_>>()
    );
    assert_eq!(
        receipt
            .metadata()
            .expect("metadata receipt")
            .policy_identity(),
        policy.identity()
    );
    assert!(
        receipt
            .metadata()
            .expect("metadata receipt")
            .excluded_groups()
            .iter()
            .any(|group| group == "gps")
    );
    assert_eq!(reader.output_buffer_size(), Some(8));
    fs::remove_dir_all(directory).expect("cleanup");
}
