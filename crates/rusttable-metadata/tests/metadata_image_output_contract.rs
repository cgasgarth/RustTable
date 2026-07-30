use std::path::Path;

use rusttable_core::ImageMetadata;
use rusttable_image::{DecodedImage, ImageOutputError, OutputOptions, OutputReceipt};
use rusttable_metadata::MetadataImageOutput;

fn assert_object_safe(_: &dyn MetadataImageOutput) {}

fn assert_signature<T: MetadataImageOutput>(
    output: &T,
    image: &DecodedImage,
    metadata: &ImageMetadata,
) {
    let _ = output.write_new_with_metadata(
        image,
        metadata,
        Path::new("ignored.png"),
        OutputOptions::Png,
        rusttable_metadata::MetadataOutputLimits::new(4096, 10, 4096, 4096).unwrap(),
    );
}

#[test]
fn metadata_image_output_is_object_safe_and_has_the_opt_in_signature() {
    let _ = assert_object_safe;
    let _ = assert_signature::<TestOutput>;
}

#[test]
fn metadata_image_output_errors_preserve_nested_sources() {
    let output = rusttable_metadata::MetadataImageOutputError::BeforePublication {
        source: ImageOutputError::DestinationExists {
            path: "existing.png".into(),
        },
    };
    assert!(std::error::Error::source(&output).is_some());
    let output = rusttable_metadata::MetadataImageOutputError::MetadataSerializationFailure {
        source: rusttable_metadata::MetadataOutputError::InternalInvariant { context: "test" },
    };
    assert!(std::error::Error::source(&output).is_some());
}

struct TestOutput;

impl MetadataImageOutput for TestOutput {
    fn write_new_with_metadata(
        &self,
        image: &DecodedImage,
        metadata: &ImageMetadata,
        destination: &Path,
        options: OutputOptions,
        metadata_limits: rusttable_metadata::MetadataOutputLimits,
    ) -> Result<OutputReceipt, rusttable_metadata::MetadataImageOutputError> {
        let _ = (image, metadata, destination, options, metadata_limits);
        unreachable!("signature-only test implementation")
    }
}
