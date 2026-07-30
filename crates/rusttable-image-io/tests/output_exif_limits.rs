mod support;

use std::fs;

use rusttable_image::{JpegQuality, OutputLimits, OutputOptions};
use rusttable_image_io::FileImageOutput;
use rusttable_metadata::{
    ImageMetadata, MetadataEntry, MetadataImageOutput, MetadataImageOutputError,
};

#[test]
fn exact_final_limit_succeeds_and_first_byte_beyond_it_fails_atomically() {
    let baseline_path = support::destination("limit-baseline.png");
    FileImageOutput::new(support::output_limits())
        .write_new_with_metadata(
            &support::image(),
            &support::metadata(),
            &baseline_path,
            OutputOptions::Png,
            support::metadata_limits(),
        )
        .expect("baseline output");
    let baseline = fs::read(&baseline_path).expect("baseline bytes");
    fs::remove_file(&baseline_path).expect("cleanup");

    let exact_path = support::destination("limit-exact.png");
    FileImageOutput::new(OutputLimits::new(baseline.len() as u64).expect("exact limit"))
        .write_new_with_metadata(
            &support::image(),
            &support::metadata(),
            &exact_path,
            OutputOptions::Png,
            support::metadata_limits(),
        )
        .expect("exact final limit");
    assert_eq!(
        fs::read(&exact_path).expect("exact bytes").len(),
        baseline.len()
    );
    fs::remove_file(&exact_path).expect("cleanup");

    let rejected_path = support::destination("limit-rejected.png");
    let result = FileImageOutput::new(
        OutputLimits::new((baseline.len() - 1) as u64).expect("below exact limit"),
    )
    .write_new_with_metadata(
        &support::image(),
        &support::metadata(),
        &rejected_path,
        OutputOptions::Png,
        support::metadata_limits(),
    );
    assert!(matches!(
        result,
        Err(MetadataImageOutputError::BeforePublication {
            source: rusttable_image::ImageOutputError::EncodedOutputTooLarge { .. }
        })
    ));
    assert!(!rejected_path.exists());
}

#[test]
fn serializer_failure_and_tiff_are_rejected_before_publication() {
    let destination = support::destination("serializer-failure.jpg");
    let unrepresentable =
        ImageMetadata::from_entries([MetadataEntry::CameraMake(support::text("Café"))])
            .expect("metadata");
    let result = FileImageOutput::new(support::output_limits()).write_new_with_metadata(
        &support::image(),
        &unrepresentable,
        &destination,
        OutputOptions::Jpeg {
            quality: JpegQuality::new(90).expect("quality"),
        },
        support::metadata_limits(),
    );
    assert!(matches!(
        result,
        Err(MetadataImageOutputError::MetadataSerializationFailure { .. })
    ));
    assert!(!destination.exists());

    let tiff = support::destination("metadata.tiff");
    let result = FileImageOutput::new(support::output_limits()).write_new_with_metadata(
        &support::image(),
        &support::metadata(),
        &tiff,
        OutputOptions::Tiff,
        support::metadata_limits(),
    );
    assert!(matches!(
        result,
        Err(MetadataImageOutputError::UnsupportedMetadataOutputFormat {
            format: rusttable_image::OutputFormat::Tiff
        })
    ));
    assert!(!tiff.exists());
}

#[test]
fn existing_destination_is_not_clobbered_by_metadata_output() {
    let destination = support::destination("collision.png");
    fs::write(&destination, b"keep").expect("destination");
    let result = FileImageOutput::new(support::output_limits()).write_new_with_metadata(
        &support::image(),
        &support::metadata(),
        &destination,
        OutputOptions::Png,
        support::metadata_limits(),
    );
    assert!(matches!(
        result,
        Err(MetadataImageOutputError::BeforePublication {
            source: rusttable_image::ImageOutputError::DestinationExists { .. }
        })
    ));
    assert_eq!(fs::read(&destination).expect("destination bytes"), b"keep");
    fs::remove_file(destination).expect("cleanup");
}
