#![forbid(unsafe_code)]
#![doc = "Bounded JPEG, PNG, and classic-TIFF file input/output for `RustTable`."]

mod input;
mod output;
mod source;

pub use input::FileImageInput;
pub use output::FileImageOutput;
pub use source::{
    ContentEvidence, FileIdentityClass, HashStatus, PositionedSourceReader, ReadCancellation,
    SequentialReader, SequentialSourceReader, SnapshotPolicy, SnapshotPolicyError, SnapshotReceipt,
    SourceAlias, SourceAliasError, SourceChanged, SourceIdentity, SourceReadError, SourceSnapshot,
    SourceSnapshotError, SourceSnapshotMode,
};
