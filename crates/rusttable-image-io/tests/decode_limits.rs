use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rusttable_image::{DecodeLimits, ImageInput, ImageInputError};
use rusttable_image_io::FileImageInput;

static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn png() -> Vec<u8> {
    decode(include_str!("fixtures/rgba-2x1.png.b64"))
}

fn png_1x2() -> Vec<u8> {
    decode(include_str!("fixtures/rgba-1x2.png.b64"))
}

fn decode(encoded: &str) -> Vec<u8> {
    let mut output = Vec::new();
    let mut quartet = [0u8; 4];
    let mut count = 0;
    for byte in encoded.bytes().filter(|byte| !byte.is_ascii_whitespace()) {
        quartet[count] = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => 64,
            _ => panic!("fixture contains invalid base64"),
        };
        count += 1;
        if count == 4 {
            output.push((quartet[0] << 2) | (quartet[1] >> 4));
            if quartet[2] != 64 {
                output.push((quartet[1] << 4) | (quartet[2] >> 2));
            }
            if quartet[3] != 64 {
                output.push((quartet[2] << 6) | quartet[3]);
            }
            count = 0;
        }
    }
    output
}

fn input(limits: DecodeLimits) -> FileImageInput {
    FileImageInput::new(limits)
}

fn with_png<T>(name: &str, operation: impl FnOnce(&Path) -> T) -> T {
    let bytes = png();
    with_bytes(name, &bytes, operation)
}

fn with_png_1x2<T>(name: &str, operation: impl FnOnce(&Path) -> T) -> T {
    let bytes = png_1x2();
    with_bytes(name, &bytes, operation)
}

fn with_bytes<T>(name: &str, bytes: &[u8], operation: impl FnOnce(&Path) -> T) -> T {
    let fixture = TestFixture::create(name, bytes);
    operation(fixture.path())
}

struct TestFixture {
    path: PathBuf,
}

impl TestFixture {
    fn create(name: &str, bytes: &[u8]) -> Self {
        let (path, mut file) = unique_file(name);
        file.write_all(bytes).expect("fixture should be writable");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestFixture {
    fn drop(&mut self) {
        match fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => panic!("owned fixture should be removable: {error}"),
        }
    }
}

fn unique_file(name: &str) -> (PathBuf, File) {
    loop {
        let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rusttable-image-io-limit-{name}-{}-{sequence}.fixture",
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return (path, file),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => panic!("unique fixture should be creatable: {error}"),
        }
    }
}

fn limits(
    max_source_bytes: u64,
    max_width: u32,
    max_height: u32,
    max_pixels: u64,
    max_bytes: u64,
) -> DecodeLimits {
    DecodeLimits::new(
        max_source_bytes,
        max_width,
        max_height,
        max_pixels,
        max_bytes,
    )
    .expect("valid test limits")
}

#[test]
fn width_limit_is_checked_before_decode() {
    let result = with_png("width", |path| {
        input(limits(1_000_000, 1, 1, 1, 4)).probe_path(path)
    });
    assert_eq!(
        result,
        Err(ImageInputError::WidthLimit {
            actual: 2,
            limit: 1
        })
    );
}

#[test]
fn height_limit_is_checked_before_decode() {
    let result = with_png_1x2("height", |path| {
        input(limits(1_000_000, 1, 1, 1, 4)).probe_path(path)
    });
    assert_eq!(
        result,
        Err(ImageInputError::HeightLimit {
            actual: 2,
            limit: 1
        })
    );
}

#[test]
fn pixel_limit_is_checked_before_decode() {
    let result = with_png("pixels", |path| {
        input(limits(1_000_000, 2, 1, 1, 4)).probe_path(path)
    });
    assert_eq!(
        result,
        Err(ImageInputError::PixelLimit {
            actual: 2,
            limit: 1
        })
    );
}

#[test]
fn decoded_byte_limit_is_checked_before_decode() {
    let result = with_png("bytes", |path| {
        input(limits(1_000_000, 2, 1, 2, 4)).probe_path(path)
    });
    assert_eq!(
        result,
        Err(ImageInputError::DecodedByteLimit {
            actual: 8,
            limit: 4
        })
    );
}
