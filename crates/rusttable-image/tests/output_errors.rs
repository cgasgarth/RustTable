use std::path::PathBuf;

use rusttable_image::{ImageOutputError, OutputFormat};

#[test]
fn output_failures_are_matchable_without_opaque_errors() {
    let errors = [
        ImageOutputError::InvalidDestination {
            path: PathBuf::from("bad"),
        },
        ImageOutputError::MissingDestinationParent {
            path: PathBuf::from("missing/out"),
        },
        ImageOutputError::DestinationExists {
            path: PathBuf::from("exists"),
        },
        ImageOutputError::NonOpaqueJpegInput { pixel_index: 4 },
        ImageOutputError::EncodedOutputTooLarge {
            limit: 10,
            actual: 11,
        },
        ImageOutputError::AllocationFailure,
        ImageOutputError::EncodeFailure {
            format: OutputFormat::Png,
        },
        ImageOutputError::TemporaryFileCreationFailure,
        ImageOutputError::WriteFailure,
        ImageOutputError::SyncFailure,
        ImageOutputError::PublishFailure,
    ];
    assert!(errors.iter().all(|error| !error.to_string().is_empty()));
}
