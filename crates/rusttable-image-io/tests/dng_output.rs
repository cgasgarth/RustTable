use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use rusttable_color::Matrix3;
use rusttable_image::{DecodeLimits, ImageDimensions, Orientation};
use rusttable_image_io::ImageDecoderRegistry;
use rusttable_image_io::dng_output::{
    DngCfaColor, DngCfaDescriptor, DngCfaPattern, DngError, DngLinearColor, DngLinearDescriptor,
    DngOutput, DngOutputRequest, DngRawLayout, DngRawLayoutKind,
};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn destination(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rusttable-dng-{label}-{}-{}.dng",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn cfa(destination: PathBuf) -> DngOutputRequest {
    let dimensions = ImageDimensions::new(4, 4).expect("dimensions");
    let descriptor = DngCfaDescriptor::new(
        dimensions,
        4,
        (0..16).map(|value| 100 + value).collect(),
        DngCfaPattern::new(
            [
                [DngCfaColor::Red, DngCfaColor::Green],
                [DngCfaColor::Green, DngCfaColor::Blue],
            ],
            (0, 0),
        ),
        Orientation::Normal,
        None,
        None,
        Vec::new(),
        [0; 4],
        [1_000; 4],
        [1.0; 4],
        Matrix3::identity(),
        [1; 32],
        [2; 32],
        [3; 32],
    )
    .expect("descriptor");
    DngOutputRequest::new(destination, DngRawLayout::CfaBayerU16(descriptor)).expect("request")
}

#[test]
fn cfa_is_deterministic_and_round_trips_through_generic_probe() {
    let path = destination("cfa");
    let request = cfa(path.clone());
    let first = DngOutput::publish(&request, || false).expect("publish");
    let probe = DngOutput::probe(&path, request.limits.max_encoded_bytes).expect("probe");
    assert_eq!(probe.layout, DngRawLayoutKind::CfaBayerU16);
    assert_eq!(
        probe.samples,
        (0..16).map(|value| 100 + value).collect::<Vec<_>>()
    );
    let generic = ImageDecoderRegistry::standard()
        .probe_bytes(
            &fs::read(&path).expect("bytes"),
            DecodeLimits::new(1_000_000, 10, 10, 100, 400).expect("limits"),
        )
        .expect("generic TIFF probe");
    assert_eq!(
        generic.dimensions(),
        ImageDimensions::new(4, 4).expect("dimensions")
    );
    let second_path = destination("cfa-repeat");
    let second = DngOutput::publish(&cfa(second_path.clone()), || false).expect("publish");
    assert_eq!(
        first.receipt.artifact_identity,
        second.receipt.artifact_identity
    );
    assert_eq!(
        fs::read(first.destination).unwrap_or_default(),
        fs::read(second.destination).unwrap_or_default()
    );
    DngOutput::discard(&path).expect("discard");
    DngOutput::discard(&second_path).expect("discard");
}

#[test]
fn linear_raw_and_cancellation_are_checked() {
    let path = destination("linear");
    let descriptor = DngLinearDescriptor::new(
        ImageDimensions::new(2, 1).expect("dimensions"),
        vec![1, 2, 3, 4, 5, 6],
        DngLinearColor::SrgbD65,
        [0; 3],
        [100; 3],
        Orientation::Normal,
        None,
        None,
        Vec::new(),
        None,
        [4; 32],
        [5; 32],
        [6; 32],
    )
    .expect("descriptor");
    let request = DngOutputRequest::new(path.clone(), DngRawLayout::LinearRawRgbU16(descriptor))
        .expect("request");
    assert_eq!(
        DngOutput::publish(&request, || true),
        Err(DngError::Cancelled)
    );
    let published = DngOutput::publish(&request, || false).expect("publish");
    assert_eq!(
        DngOutput::probe(&path, request.limits.max_encoded_bytes)
            .expect("probe")
            .layout,
        DngRawLayoutKind::LinearRawRgbU16
    );
    DngOutput::discard(&published.destination).expect("discard");
}
