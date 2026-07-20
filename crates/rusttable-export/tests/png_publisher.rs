use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rusttable_export::{CollisionPolicy, PngExportLimits, PngPublishError, PngPublisher};
use rusttable_image::{DecodeLimits, DecodedImage, ImageDimensions, ImageInput};
use rusttable_image_io::FileImageInput;

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn image() -> DecodedImage {
    DecodedImage::new(
        ImageDimensions::new(2, 1).expect("dimensions"),
        vec![255, 0, 0, 255, 0, 255, 0, 128],
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

    publisher(2, 1, 1_000_000)
        .publish(&image(), &destination, CollisionPolicy::ReplaceExisting)
        .expect("replacement publication");

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
