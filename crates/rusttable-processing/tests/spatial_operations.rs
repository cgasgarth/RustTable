use rusttable_processing::operations::{
    graduatednd::{
        GraduatedNdConfig, GraduatedNdHistory, GraduatedNdParametersV1, GraduatedNdPlan,
    },
    vignette::{
        VignetteConfig, VignetteDither, VignetteHistory, VignetteParametersV4, VignettePlan,
    },
};
use rusttable_processing::{FiniteF32, LinearRgb, RasterDimensions};

fn pixel(red: f32, green: f32, blue: f32) -> LinearRgb {
    LinearRgb::new(
        FiniteF32::new(red).expect("finite"),
        FiniteF32::new(green).expect("finite"),
        FiniteF32::new(blue).expect("finite"),
    )
}

fn image(count: usize) -> Vec<LinearRgb> {
    (0..count)
        .map(|index| {
            pixel(
                f32::from(u16::try_from(index).expect("small test image")) + 1.0,
                2.0,
                -0.5,
            )
        })
        .collect()
}

#[test]
fn vignette_v4_roundtrips_and_legacy_payloads_stay_opaque() {
    let parameters = VignetteParametersV4::defaults();
    let history = VignetteHistory::decode(4, &parameters.to_bytes()).expect("v4");
    assert_eq!(history.payload(), parameters.to_bytes());

    let legacy = vec![0x5a; 320];
    let history = VignetteHistory::decode(1, &legacy).expect("legacy");
    assert_eq!(history.payload(), legacy);
    assert!(history.current().is_err());
}

#[test]
fn vignette_uses_full_image_coordinates_for_windows_and_preserves_hdr() {
    let dimensions = RasterDimensions::new(4, 4).expect("dimensions");
    let parameters = VignetteParametersV4::new(
        0.0,
        50.0,
        0.0,
        0.0,
        [0.0, 0.0],
        true,
        1.0,
        1.0,
        VignetteDither::Off,
        true,
    );
    let config = VignetteConfig::new(parameters).expect("config");
    let plan = VignettePlan::new(config, dimensions).expect("plan");
    let source = image(16);
    let full = plan.execute(&source).expect("full");
    let first = plan.execute_window(&source[..8], 0).expect("first window");
    let second = plan.execute_window(&source[8..], 8).expect("second window");
    assert_eq!(&full[..8], &first);
    assert_eq!(&full[8..], &second);
    assert_eq!(full[0].blue().get().to_bits(), (-0.5_f32).to_bits());
}

#[test]
fn vignette_dither_is_repeatable_and_validation_is_closed() {
    let dimensions = RasterDimensions::new(3, 2).expect("dimensions");
    let parameters = VignetteParametersV4::new(
        0.0,
        100.0,
        0.0,
        0.0,
        [0.0, 0.0],
        true,
        1.0,
        1.0,
        VignetteDither::EightBit,
        true,
    );
    let config = VignetteConfig::new(parameters).expect("config");
    let plan = VignettePlan::new(config, dimensions)
        .expect("plan")
        .with_seed(42);
    assert_eq!(
        plan.execute(&image(6)).expect("first"),
        plan.execute(&image(6)).expect("second")
    );
    assert!(VignetteDither::from_id(99).is_err());
}

#[test]
fn graduatednd_roundtrips_presets_and_keeps_tiles_consistent() {
    let parameters = GraduatedNdParametersV1::defaults();
    let history = GraduatedNdHistory::decode(1, &parameters.to_bytes()).expect("v1");
    assert_eq!(history.payload(), parameters.to_bytes());
    let opaque = GraduatedNdHistory::decode(7, &[1, 2, 3]).expect("opaque");
    assert_eq!(opaque.payload(), vec![1, 2, 3]);

    let dimensions = RasterDimensions::new(5, 4).expect("dimensions");
    let config = GraduatedNdConfig::new(GraduatedNdParametersV1::new(
        2.0, 75.0, 37.0, 30.0, 0.15, 0.5,
    ))
    .expect("config");
    let plan = GraduatedNdPlan::new(config, dimensions).expect("plan");
    let source = image(20);
    let full = plan.execute(&source).expect("full");
    let first = plan.execute_window(&source[..10], 0).expect("first window");
    let second = plan
        .execute_window(&source[10..], 10)
        .expect("second window");
    assert_eq!(&full[..10], &first);
    assert_eq!(&full[10..], &second);
}

#[test]
fn graduatednd_zero_density_is_identity_and_negative_density_is_finite() {
    let dimensions = RasterDimensions::new(2, 2).expect("dimensions");
    let source = vec![pixel(2.0, -1.0, 0.25); 4];
    let identity = GraduatedNdPlan::new(
        GraduatedNdConfig::new(GraduatedNdParametersV1::new(0.0, 0.0, 0.0, 50.0, 0.0, 0.0))
            .expect("config"),
        dimensions,
    )
    .expect("plan")
    .execute(&source)
    .expect("identity");
    assert_eq!(identity, source);

    let negative = GraduatedNdPlan::new(
        GraduatedNdConfig::new(GraduatedNdParametersV1::new(
            -8.0, 100.0, 180.0, 50.0, 0.9, 1.0,
        ))
        .expect("config"),
        dimensions,
    )
    .expect("plan")
    .execute(&source)
    .expect("negative density");
    assert!(negative.iter().all(|value| {
        value.red().get().is_finite()
            && value.green().get().is_finite()
            && value.blue().get().is_finite()
    }));
}
