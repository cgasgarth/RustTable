use std::cell::Cell;

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
fn vignette_native_buffer_centered_coordinate_has_zero_at_top_left() {
    let dimensions = RasterDimensions::new(4, 4).expect("dimensions");
    let parameters = VignetteParametersV4::new(
        80.0,
        50.0,
        -1.0,
        0.0,
        [0.0, 0.0],
        false,
        1.0,
        1.0,
        VignetteDither::Off,
        true,
    );
    let config = VignetteConfig::new(parameters).expect("config");
    let plan = VignettePlan::new(config, dimensions).expect("plan");
    let output = plan
        .execute(&[pixel(1.0, 1.0, 1.0); 16])
        .expect("native vignette raster");

    assert_eq!(output[0].red().get().to_bits(), 0.0_f32.to_bits());
    assert_eq!(output[0].green().get().to_bits(), 0.0_f32.to_bits());
    assert_eq!(output[0].blue().get().to_bits(), 0.0_f32.to_bits());
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
fn graduatednd_native_rotation_zero_subtracts_vertical_length() {
    let dimensions = RasterDimensions::new(4, 4).expect("dimensions");
    let config =
        GraduatedNdConfig::new(GraduatedNdParametersV1::new(1.0, 0.0, 0.0, 50.0, 0.0, 0.0))
            .expect("config");
    let plan = GraduatedNdPlan::new(config, dimensions).expect("plan");
    let source = (0..4)
        .flat_map(|row| {
            let value = 2.0_f32.powi(row);
            [pixel(value, value, value); 4]
        })
        .collect::<Vec<_>>();
    let output = plan.execute(&source).expect("native graduated ND raster");
    let expected_red = [
        0.666_666_7_f32,
        1.425_036_3_f32,
        3.313_708_3_f32,
        7.594_312_f32,
    ];

    for (row, expected) in expected_red.into_iter().enumerate() {
        let actual = output[row * 4].red().get();
        assert!(
            (actual - expected).abs() <= 1.0e-6,
            "row {row}: {actual} != {expected}"
        );
    }
    assert!(output[0].red().get() < output[12].red().get());
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

#[test]
fn vignette_native_dither_golden_carries_tea_state_across_rows() {
    let dimensions = RasterDimensions::new(4, 3).expect("dimensions");
    let source = vec![pixel(0.25, 0.35, 0.45); 12];

    // These f32-bit values include state[1] carried into later rows; a
    // stateless counter or a per-row reset produces different bits.
    for (dithering, expected_red_bits) in [
        (
            VignetteDither::EightBit,
            [
                0x3e81_a30f,
                0x3e7f_a796,
                0x3e7e_f3e2,
                0x3e80_0125,
                0x3e81_24be,
                0x3e81_44b9,
                0x3e7d_65d6,
                0x3e7e_52fd,
                0x3e81_108a,
                0x3e81_1fa5,
                0x3e7d_7bb0,
                0x3e7f_cd78,
            ],
        ),
        (
            VignetteDither::SixteenBit,
            [
                0x3e80_01a3,
                0x3e7f_ffa8,
                0x3e7f_fef4,
                0x3e80_0001,
                0x3e80_0125,
                0x3e80_0145,
                0x3e7f_fd66,
                0x3e7f_fe53,
                0x3e80_0111,
                0x3e80_0120,
                0x3e7f_fd7c,
                0x3e7f_ffcd,
            ],
        ),
    ] {
        let config = VignetteConfig::new(VignetteParametersV4::new(
            0.0,
            200.0,
            0.0,
            0.0,
            [0.0, 0.0],
            true,
            1.0,
            1.0,
            dithering,
            true,
        ))
        .expect("config");
        let plan = VignettePlan::new(config, dimensions).expect("plan");
        let output = plan.execute(&source).expect("dithered output");
        let actual = output
            .iter()
            .map(|value| value.red().get().to_bits())
            .collect::<Vec<_>>();
        assert_eq!(actual, expected_red_bits);

        let first = plan.execute_window(&source[..5], 0).expect("first window");
        let second = plan.execute_window(&source[5..], 5).expect("second window");
        assert_eq!(
            first
                .into_iter()
                .chain(second)
                .map(|value| value.red().get().to_bits())
                .collect::<Vec<_>>(),
            expected_red_bits
        );
    }
}

#[test]
fn spatial_plans_poll_cancellation_without_publishing_partial_windows() {
    let dimensions = RasterDimensions::new(4, 4).expect("dimensions");
    let source = image(16);

    let vignette = VignettePlan::new(VignetteConfig::defaults(), dimensions).expect("vignette");
    assert!(matches!(
        vignette.execute_with_cancel(&source, || true),
        Err(rusttable_processing::operations::OperationExecutionError::Cancelled)
    ));
    let vignette_polls = Cell::new(0);
    assert!(matches!(
        vignette.execute_with_cancel(&source, || {
            let polls = vignette_polls.get() + 1;
            vignette_polls.set(polls);
            polls >= 2
        }),
        Err(rusttable_processing::operations::OperationExecutionError::Cancelled)
    ));
    assert_eq!(vignette_polls.get(), 2);

    let graduated =
        GraduatedNdPlan::new(GraduatedNdConfig::defaults(), dimensions).expect("graduated ND");
    assert!(matches!(
        graduated.execute_with_cancel(&source, || true),
        Err(rusttable_processing::operations::OperationExecutionError::Cancelled)
    ));
    let graduated_polls = Cell::new(0);
    assert!(matches!(
        graduated.execute_with_cancel(&source, || {
            let polls = graduated_polls.get() + 1;
            graduated_polls.set(polls);
            polls >= 2
        }),
        Err(rusttable_processing::operations::OperationExecutionError::Cancelled)
    ));
    assert_eq!(graduated_polls.get(), 2);
}
