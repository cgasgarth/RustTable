use rusttable_catalog::{ImportCandidateError, ImportError};
use rusttable_image::ImageInputError;
use rusttable_import::{SourceImportError, SourceReadStage, SourceSnapshotError};
use rusttable_metadata::MetadataInputError;

#[test]
fn nested_source_errors_remain_matchable_and_expose_their_cause() {
    let snapshot = SourceImportError::Snapshot(SourceSnapshotError::Io {
        stage: SourceReadStage::Read,
        path: "source.raw".into(),
    });
    assert!(std::error::Error::source(&snapshot).is_some());

    let image = SourceImportError::Image(ImageInputError::ArithmeticOverflow);
    let metadata = SourceImportError::Metadata(MetadataInputError::MalformedExif);
    let candidate = SourceImportError::Candidate(ImportCandidateError::ZeroByteLength);
    let import =
        SourceImportError::Import(ImportError::Candidate(ImportCandidateError::ZeroByteLength));
    assert!(std::error::Error::source(&image).is_some());
    assert!(std::error::Error::source(&metadata).is_some());
    assert!(std::error::Error::source(&candidate).is_some());
    assert!(std::error::Error::source(&import).is_some());
}
