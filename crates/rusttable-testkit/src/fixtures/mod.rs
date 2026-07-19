mod compression;
mod manifest;
mod privacy;
mod qualification;
mod verify;

pub use compression::{CompressionError, DecompressionReport};
pub use manifest::{
    ArtifactClass, Compression, FixtureDimensions, FixtureEntry, FixtureExpectation,
    FixtureManifest, FixtureManifestLimits, ManifestError, PrivacyClass,
};
pub use privacy::{
    PrivacyFinding, PrivacyFindingKind, PrivacyReport, PrivacyScanner, PrivacyScannerLimits,
};
pub use qualification::{QualificationError, qualify_binary};
pub use verify::{
    FixtureRepository, VerificationError, VerificationReport, VerifiedFixture, sha256_hex,
};
