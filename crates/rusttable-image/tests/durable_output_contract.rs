use std::path::Path;

use rusttable_image::{
    DecodedImage, DurableImageOutput, DurableImageOutputError, DurableOutputReceipt,
    DurableOutputTag, ImageDimensions, ImageOutputError, OutputOptions,
};

struct FakeDurable;

impl DurableImageOutput for FakeDurable {
    fn write_new_durable(
        &self,
        _image: &DecodedImage,
        _destination: &Path,
        _options: OutputOptions,
    ) -> Result<DurableOutputReceipt, DurableImageOutputError> {
        Err(DurableImageOutputError::DurabilityUnsupported {
            destination: Path::new("unsupported").to_owned(),
        })
    }
}

#[test]
fn durable_output_is_object_safe_and_has_a_closed_success_tag() {
    let output: Box<dyn DurableImageOutput> = Box::new(FakeDurable);
    let image = DecodedImage::new(
        ImageDimensions::new(1, 1).expect("dimensions"),
        vec![1, 2, 3, 255],
    )
    .expect("image");
    let result = output.write_new_durable(&image, Path::new("ignored"), OutputOptions::Png);
    assert!(matches!(
        result,
        Err(DurableImageOutputError::DurabilityUnsupported { .. })
    ));
    assert_eq!(
        DurableOutputTag::FileAndParentDirectorySynchronized,
        DurableOutputTag::FileAndParentDirectorySynchronized
    );
}

#[test]
fn prepublication_error_preserves_the_completed_output_cause() {
    let error = DurableImageOutputError::BeforePublication {
        source: ImageOutputError::DestinationExists {
            path: Path::new("existing.png").to_owned(),
        },
    };
    assert!(std::error::Error::source(&error).is_some());
}
