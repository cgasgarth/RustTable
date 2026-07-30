use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rusttable_testkit::fixtures::{
    Compression, FixtureManifest, FixtureRepository, PrivacyFindingKind, VerificationError,
};

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        let number = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rusttable-testkit-fixtures-{}-{number}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temporary fixture directory should be created");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn manifest(path: &str, size: usize, checksum: &str) -> FixtureManifest {
    FixtureManifest::parse(&format!(
        r#"
version = 1
governed_roots = ["fixtures"]

[limits]
max_file_bytes = 16
max_total_bytes = 64
max_decompressed_bytes = 16
max_compression_ratio = 4

[[fixtures]]
id = "fixture.one"
path = "{path}"
size = {size}
sha256 = "{checksum}"
media_type = "application/octet-stream"
compression = "none"
privacy = "synthetic"
artifact_class = "valid-binary"
format = "binary"
source = "rusttable-test"
generator = "rusttable-test"
parser = "rusttable-testkit"
consumers = ["test"]
"#
    ))
    .expect("manifest should parse")
}

fn repository(directory: &TempDirectory, manifest: FixtureManifest) -> FixtureRepository {
    FixtureRepository::new(directory.path(), manifest).expect("repository should open")
}

#[test]
fn parses_typed_manifest_and_resolves_canonical_fixture_path() {
    let directory = TempDirectory::new();
    fs::create_dir_all(directory.path().join("fixtures/nested")).expect("fixtures");
    fs::write(directory.path().join("fixtures/nested/data.bin"), b"abc").expect("fixture");
    let repository = repository(
        &directory,
        manifest(
            "fixtures/nested/data.bin",
            3,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
    );

    let report = repository.verify().expect("fixture should verify");
    assert_eq!(report.fixtures().len(), 1);
    assert!(report.fixtures()[0].path().is_absolute());
    assert_eq!(report.fixtures()[0].size(), 3);
}

#[test]
fn rejects_manifest_path_traversal_before_filesystem_access() {
    let result = FixtureManifest::parse(
        r#"
version = 1
governed_roots = ["fixtures"]
[[fixtures]]
id = "escape"
path = "fixtures/../outside.bin"
size = 1
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
media_type = "application/octet-stream"
privacy = "synthetic"
artifact_class = "valid-binary"
format = "binary"
source = "rusttable-test"
generator = "rusttable-test"
parser = "rusttable-testkit"
consumers = []
"#,
    );

    assert!(matches!(result, Err(error) if error.to_string().contains("path traversal")));
}

#[test]
fn reports_checksum_drift_without_exposing_fixture_bytes() {
    let directory = TempDirectory::new();
    fs::create_dir_all(directory.path().join("fixtures")).expect("fixtures");
    fs::write(directory.path().join("fixtures/data.bin"), b"abc").expect("fixture");
    let repository = repository(
        &directory,
        manifest(
            "fixtures/data.bin",
            3,
            "0000000000000000000000000000000000000000000000000000000000000000",
        ),
    );

    let error = repository.verify().expect_err("checksum drift should fail");
    assert!(matches!(error, VerificationError::ChecksumDrift { .. }));
    assert!(!error.to_string().contains("abc"));
}

#[test]
fn rejects_oversized_files_and_duplicate_content() {
    let directory = TempDirectory::new();
    fs::create_dir_all(directory.path().join("fixtures")).expect("fixtures");
    fs::write(
        directory.path().join("fixtures/large.bin"),
        b"0123456789abcdefg",
    )
    .expect("large");
    assert!(matches!(
        FixtureManifest::parse(
            r#"
version = 1
governed_roots = ["fixtures"]
[limits]
max_file_bytes = 16
[[fixtures]]
id = "large"
path = "fixtures/large.bin"
size = 17
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
media_type = "application/octet-stream"
privacy = "synthetic"
artifact_class = "valid-binary"
format = "binary"
source = "rusttable-test"
generator = "rusttable-test"
parser = "rusttable-testkit"
consumers = []
"#,
        ),
        Err(rusttable_testkit::fixtures::ManifestError::EntrySizeLimit { .. })
    ));
    fs::remove_file(directory.path().join("fixtures/large.bin")).expect("large cleanup");

    fs::write(directory.path().join("fixtures/first.bin"), b"same").expect("first");
    fs::write(directory.path().join("fixtures/second.bin"), b"same").expect("second");
    let checksum = "0967115f2813a3541eaef77de9d9d5773f1c0c04314b0bbfe4ff3b3b1c55b5d5";
    let duplicate_manifest = FixtureManifest::parse(&format!(
        r#"
version = 1
governed_roots = ["fixtures"]
[[fixtures]]
id = "first"
path = "fixtures/first.bin"
size = 4
sha256 = "{checksum}"
media_type = "application/octet-stream"
privacy = "synthetic"
artifact_class = "valid-binary"
format = "binary"
source = "rusttable-test"
generator = "rusttable-test"
parser = "rusttable-testkit"
consumers = []
[[fixtures]]
id = "second"
path = "fixtures/second.bin"
size = 4
sha256 = "{checksum}"
media_type = "application/octet-stream"
privacy = "synthetic"
artifact_class = "valid-binary"
format = "binary"
source = "rusttable-test"
generator = "rusttable-test"
parser = "rusttable-testkit"
consumers = []
"#
    ))
    .expect("duplicate manifest should parse");
    let duplicate = repository(&directory, duplicate_manifest);
    assert!(matches!(
        duplicate.verify(),
        Err(VerificationError::DuplicateContent { .. })
    ));
}

#[test]
fn rejects_unregistered_and_hidden_governed_files() {
    let directory = TempDirectory::new();
    fs::create_dir_all(directory.path().join("fixtures")).expect("fixtures");
    fs::write(directory.path().join("fixtures/data.bin"), b"abc").expect("fixture");
    fs::write(
        directory.path().join("fixtures/unregistered.bin"),
        b"unregistered",
    )
    .expect("unregistered");
    fs::write(directory.path().join("fixtures/.hidden"), b"hidden").expect("hidden");
    let repository = repository(
        &directory,
        manifest(
            "fixtures/data.bin",
            3,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
    );

    let error = repository
        .verify()
        .expect_err("governed extras should fail");
    assert!(matches!(
        error,
        VerificationError::HiddenFile { .. } | VerificationError::UnregisteredFile { .. }
    ));
}

#[test]
fn privacy_scanner_reports_fields_but_never_sensitive_values() {
    let directory = TempDirectory::new();
    fs::create_dir_all(directory.path().join("fixtures")).expect("fixtures");
    let bytes = jpeg_with_comment_and_xmp();
    fs::write(directory.path().join("fixtures/photo.jpg"), &bytes).expect("photo");
    let checksum = rusttable_testkit::fixtures::sha256_hex(&bytes);
    let text = format!(
        r#"
version = 1
governed_roots = ["fixtures"]
[[fixtures]]
id = "private.photo"
path = "fixtures/photo.jpg"
size = {}
sha256 = "{checksum}"
media_type = "image/jpeg"
privacy = "scrubbed"
artifact_class = "valid-binary"
format = "binary"
source = "rusttable-test"
generator = "rusttable-test"
parser = "rusttable-testkit"
consumers = []
"#,
        bytes.len()
    );
    let repository = repository(
        &directory,
        FixtureManifest::parse(&text).expect("manifest should parse"),
    );

    let error = repository.verify().expect_err("privacy leak should fail");
    let VerificationError::PrivacyLeak { findings, .. } = error else {
        panic!("expected privacy failure");
    };
    assert!(findings.iter().any(|finding| {
        finding.kind() == PrivacyFindingKind::JpegComment && finding.field() == "jpeg.comment[0]"
    }));
    assert!(
        findings
            .iter()
            .any(|finding| finding.field().contains("xmp"))
    );
    let rendered = findings.iter().map(ToString::to_string).collect::<String>();
    assert!(!rendered.contains("private-camera-note"));
}

#[test]
fn compression_is_bounded_before_content_is_consumed() {
    let directory = TempDirectory::new();
    fs::create_dir_all(directory.path().join("fixtures")).expect("fixtures");
    let compressed = gzip(&[b'x'; 64]);
    fs::write(directory.path().join("fixtures/bomb.gz"), &compressed).expect("bomb");
    let checksum = rusttable_testkit::fixtures::sha256_hex(&compressed);
    let manifest = FixtureManifest::parse(&format!(
        r#"
version = 1
governed_roots = ["fixtures"]
[limits]
max_file_bytes = 1024
max_total_bytes = 1024
max_decompressed_bytes = 16
max_compression_ratio = 4
[[fixtures]]
id = "bomb"
path = "fixtures/bomb.gz"
size = {}
sha256 = "{checksum}"
media_type = "application/octet-stream"
compression = "gzip"
privacy = "synthetic"
artifact_class = "valid-binary"
format = "binary"
source = "rusttable-test"
generator = "rusttable-test"
parser = "rusttable-testkit"
consumers = []
"#,
        compressed.len()
    ))
    .expect("manifest should parse");

    assert!(matches!(
        repository(&directory, manifest).verify(),
        Err(VerificationError::DecompressedSizeLimit { .. }
            | VerificationError::CompressionRatioLimit { .. },)
    ));
}

#[test]
fn committed_fixture_manifest_is_registered_and_checksum_valid() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest = FixtureManifest::parse(include_str!("../../../fixtures/manifest.toml"))
        .expect("committed manifest should parse");
    let report = FixtureRepository::new(root, manifest)
        .expect("committed repository should open")
        .verify()
        .expect("committed fixtures should verify");
    assert!(report.fixtures().len() >= 50);
}

#[test]
fn scanner_covers_iptc_png_tiff_and_filename_path_fields() {
    use rusttable_testkit::fixtures::PrivacyScanner;

    let scanner = PrivacyScanner::default();
    let iptc_report = scanner.scan(Path::new("fixtures/photo.jpg"), &jpeg_with_iptc());
    assert!(
        iptc_report
            .findings()
            .iter()
            .any(|finding| finding.kind() == PrivacyFindingKind::Iptc)
    );

    let png_report = scanner.scan(Path::new("fixtures/photo.png"), &png_with_text());
    assert!(
        png_report
            .findings()
            .iter()
            .any(|finding| finding.kind() == PrivacyFindingKind::PngText)
    );

    let tiff_report = scanner.scan(Path::new("fixtures/photo.tiff"), &tiff_with_string());
    assert!(
        tiff_report
            .findings()
            .iter()
            .any(|finding| finding.kind() == PrivacyFindingKind::TiffString)
    );

    let path_report = scanner.scan_path(Path::new("/Users/alice/Documents/private.jpg"));
    assert!(
        path_report
            .findings()
            .iter()
            .any(|finding| finding.kind() == PrivacyFindingKind::Path)
    );
}

#[test]
fn verifies_external_cache_references_without_network_access() {
    let directory = TempDirectory::new();
    fs::create_dir_all(directory.path().join("fixtures/external-cache")).expect("external cache");
    let bytes = b"cached-large-corpus-reference";
    fs::write(
        directory.path().join("fixtures/external-cache/corpus.bin"),
        bytes,
    )
    .expect("cached fixture");
    let checksum = rusttable_testkit::fixtures::sha256_hex(bytes);
    let manifest = FixtureManifest::parse(&format!(
        r#"
version = 1
governed_roots = ["fixtures/external-cache"]
[[fixtures]]
id = "external.corpus"
path = "fixtures/external-cache/corpus.bin"
size = {}
sha256 = "{checksum}"
media_type = "application/octet-stream"
compression = "none"
privacy = "external"
artifact_class = "valid-binary"
format = "binary"
source = "rusttable-test"
generator = "rusttable-test"
parser = "rusttable-testkit"
consumers = ["integration"]
"#,
        bytes.len()
    ))
    .expect("external manifest should parse");

    let report = repository(&directory, manifest)
        .verify()
        .expect("local cache should verify without a network fetch");
    assert_eq!(report.fixtures()[0].sha256(), checksum);
}

fn jpeg_with_comment_and_xmp() -> Vec<u8> {
    let xmp = b"http://ns.adobe.com/xap/1.0/\0<dc:creator>private-camera-note</dc:creator>";
    let mut bytes = vec![0xff, 0xd8, 0xff, 0xfe, 0, 21];
    bytes.extend_from_slice(b"private-camera-note");
    bytes.extend_from_slice(&[0xff, 0xe1]);
    let length = u16::try_from(xmp.len() + 2).expect("xmp segment length");
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(xmp);
    bytes.extend_from_slice(&[0xff, 0xd9]);
    bytes
}

fn jpeg_with_iptc() -> Vec<u8> {
    let payload = b"Photoshop 3.0\08BIM\x04\x04\0\0\0\0\0\0";
    jpeg_segment(0xed, payload)
}

fn jpeg_segment(marker: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0xff, 0xd8, 0xff, marker];
    let length = u16::try_from(payload.len() + 2).expect("JPEG segment length");
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes.extend_from_slice(&[0xff, 0xd9]);
    bytes
}

fn png_with_text() -> Vec<u8> {
    let data = b"Author\0private-author";
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(
        &u32::try_from(data.len())
            .expect("PNG text length")
            .to_be_bytes(),
    );
    bytes.extend_from_slice(b"tEXt");
    bytes.extend_from_slice(data);
    bytes.extend_from_slice(&[0; 4]);
    bytes
}

fn tiff_with_string() -> Vec<u8> {
    let mut bytes = vec![b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0];
    bytes.extend_from_slice(&0x010e_u16.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&7_u32.to_le_bytes());
    bytes.extend_from_slice(&26_u32.to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(b"secret\0");
    bytes
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    use flate2::Compression as GzipCompression;
    use flate2::write::GzEncoder;
    use std::io::Write;

    let mut encoder = GzEncoder::new(Vec::new(), GzipCompression::fast());
    encoder.write_all(bytes).expect("gzip input");
    encoder.finish().expect("gzip output")
}

#[allow(dead_code)]
fn _compression_is_typed() {
    assert_eq!(Compression::None, Compression::default());
}
