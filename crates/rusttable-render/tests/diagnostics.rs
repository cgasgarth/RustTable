use rusttable_color::ColorEncoding;
use rusttable_image::{
    BlackWhiteLevels, CancellationToken, CfaColor, CfaPattern, CfaPhase, ImageDimensions,
    Orientation, RawMosaic,
};
use rusttable_render::{
    DiagnosticBackend, DiagnosticFinding, DiagnosticFrame, DiagnosticGeometry,
    OverexposedColorScheme, OverexposedMode, OverexposedPlan, OverexposedState, RawOverexposedPlan,
    RawOverexposedState, RawOverlayMode, RawSolidColor,
};

fn dimensions(width: u32, height: u32) -> ImageDimensions {
    ImageDimensions::new(width, height).expect("dimensions")
}

fn frame(pixels: Vec<[f32; 4]>) -> DiagnosticFrame {
    DiagnosticFrame::new(
        dimensions(u32::try_from(pixels.len()).expect("one row"), 1),
        pixels,
    )
    .expect("frame")
}

fn assert_pixel(actual: [f32; 4], expected: [f32; 4]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!((actual - expected).abs() <= 1.0e-6);
    }
}

fn raw(
    pattern: CfaPattern,
    samples: Vec<u16>,
    width: u32,
    height: u32,
) -> rusttable_image::RawMosaic {
    RawMosaic::new(
        dimensions(width, height),
        usize::try_from(width).expect("stride"),
        samples,
        pattern,
        CfaPhase::new(0, 0, pattern),
        BlackWhiteLevels::new(100, 1000).expect("levels"),
        Orientation::Normal,
    )
    .expect("raw")
}

#[test]
fn raw_overlay_marks_exact_boundary_and_preserves_alpha_in_all_modes() {
    let raw = raw(CfaPattern::bayer_rggb(), vec![550, 100, 100, 100], 2, 2);
    let input =
        DiagnosticFrame::new(dimensions(2, 2), vec![[0.2, 0.3, 0.4, 0.25]; 4]).expect("frame");
    let token = CancellationToken::new();
    let mark_cfa = RawOverexposedPlan::new(
        raw.clone(),
        dimensions(2, 2),
        RawOverexposedState::new(RawOverlayMode::MarkCfa, 0.5, RawSolidColor::Black),
        DiagnosticGeometry::identity(),
    )
    .expect("plan")
    .execute(&input, DiagnosticBackend::Cpu, &token);
    assert!(mark_cfa.applied());
    assert_pixel(mark_cfa.frame().pixels()[0], [1.0, 0.0, 0.0, 0.25]);
    assert_eq!(mark_cfa.receipt().clipped, [1, 0, 0, 0]);

    let solid = RawOverexposedPlan::new(
        raw.clone(),
        dimensions(2, 2),
        RawOverexposedState::new(RawOverlayMode::MarkSolid, 0.5, RawSolidColor::Blue),
        DiagnosticGeometry::identity(),
    )
    .expect("plan")
    .execute(&input, DiagnosticBackend::Cpu, &token);
    assert_pixel(solid.frame().pixels()[0], [0.0, 0.0, 1.0, 0.25]);
    let false_color = RawOverexposedPlan::new(
        raw,
        dimensions(2, 2),
        RawOverexposedState::new(RawOverlayMode::FalseColor, 0.5, RawSolidColor::Black),
        DiagnosticGeometry::identity(),
    )
    .expect("plan")
    .execute(&input, DiagnosticBackend::Cpu, &token);
    assert_pixel(false_color.frame().pixels()[0], [0.0, 0.3, 0.4, 0.25]);
}

#[test]
fn raw_overlay_follows_reverse_geometry_and_xtrans_phase() {
    let pattern = CfaPattern::XTrans([
        [
            CfaColor::Red,
            CfaColor::Green,
            CfaColor::Blue,
            CfaColor::Green,
            CfaColor::Red,
            CfaColor::Green,
        ],
        [
            CfaColor::Green,
            CfaColor::Blue,
            CfaColor::Green,
            CfaColor::Red,
            CfaColor::Green,
            CfaColor::Blue,
        ],
        [
            CfaColor::Blue,
            CfaColor::Green,
            CfaColor::Red,
            CfaColor::Green,
            CfaColor::Blue,
            CfaColor::Green,
        ],
        [
            CfaColor::Green,
            CfaColor::Red,
            CfaColor::Green,
            CfaColor::Blue,
            CfaColor::Green,
            CfaColor::Red,
        ],
        [
            CfaColor::Red,
            CfaColor::Green,
            CfaColor::Blue,
            CfaColor::Green,
            CfaColor::Red,
            CfaColor::Green,
        ],
        [
            CfaColor::Green,
            CfaColor::Blue,
            CfaColor::Green,
            CfaColor::Red,
            CfaColor::Green,
            CfaColor::Blue,
        ],
    ]);
    let mut samples = vec![100_u16; 36];
    samples[1] = 1000;
    let geometry =
        DiagnosticGeometry::new([0.0, 1.0, 1.0, 1.0, 0.0, 0.0]).expect("reverse transform");
    let plan = RawOverexposedPlan::new(
        raw(pattern, samples, 6, 6),
        dimensions(1, 1),
        RawOverexposedState::new(RawOverlayMode::MarkCfa, 1.0, RawSolidColor::Black),
        geometry,
    )
    .expect("plan");
    let result = plan.execute(
        &frame(vec![[0.2, 0.3, 0.4, 1.0]]),
        DiagnosticBackend::Cpu,
        &CancellationToken::new(),
    );
    assert_pixel(result.frame().pixels()[0], [0.0, 1.0, 0.0, 1.0]);
    assert_eq!(result.receipt().clipped, [0, 1, 0, 0]);
}

#[test]
fn raw_unsupported_cfa_and_cancellation_pass_through() {
    let pattern = CfaPattern::Bayer([
        [CfaColor::Red, CfaColor::Green],
        [CfaColor::Blue, CfaColor::Clear],
    ]);
    assert_eq!(
        RawOverexposedPlan::new(
            raw(pattern, vec![100; 4], 2, 2),
            dimensions(2, 2),
            RawOverexposedState::new(RawOverlayMode::MarkCfa, 0.5, RawSolidColor::Red),
            DiagnosticGeometry::identity()
        ),
        Err(DiagnosticFinding::UnsupportedCfa)
    );
    let raw = raw(CfaPattern::bayer_rggb(), vec![1000, 100, 100, 100], 2, 2);
    let plan = RawOverexposedPlan::new(
        raw,
        dimensions(2, 2),
        RawOverexposedState::new(RawOverlayMode::MarkCfa, 0.5, RawSolidColor::Red),
        DiagnosticGeometry::identity(),
    )
    .expect("plan");
    let token = CancellationToken::new();
    token.cancel();
    let input =
        DiagnosticFrame::new(dimensions(2, 2), vec![[0.2, 0.3, 0.4, 0.7]; 4]).expect("frame");
    let result = plan.execute(&input, DiagnosticBackend::Cpu, &token);
    assert_eq!(result.finding(), Some(DiagnosticFinding::Cancelled));
    assert_eq!(result.frame(), &input);
}

#[test]
fn overexposed_modes_use_exact_thresholds_and_preserve_unmarked_alpha() {
    let input = DiagnosticFrame::new(
        dimensions(4, 1),
        vec![
            [1.0, 0.1, 0.1, 0.2],
            [0.0, 0.0, 0.0, 0.3],
            [0.5, 0.5, 0.5, 0.4],
            [0.2, 0.4, 0.2, 0.5],
        ],
    )
    .expect("frame");
    let token = CancellationToken::new();
    let any = OverexposedPlan::new(
        ColorEncoding::Srgb,
        dimensions(4, 1),
        OverexposedState::new(
            OverexposedMode::AnyRgb,
            -8.0,
            100.0,
            OverexposedColorScheme::RedBlue,
            ColorEncoding::Srgb,
        ),
    )
    .expect("plan")
    .execute(&input, DiagnosticBackend::Cpu, &token);
    assert_pixel(any.frame().pixels()[0], [1.0, 0.0, 0.0, 0.2]);
    assert_pixel(any.frame().pixels()[1], [0.0, 0.0, 1.0, 0.3]);
    assert_eq!(any.receipt().upper_count, 1);
    assert_eq!(any.receipt().lower_count, 1);

    let luminance = OverexposedPlan::new(
        ColorEncoding::Srgb,
        dimensions(4, 1),
        OverexposedState::new(
            OverexposedMode::Luminance,
            -8.0,
            50.0,
            OverexposedColorScheme::BlackWhite,
            ColorEncoding::LinearSrgb,
        ),
    )
    .expect("plan")
    .execute(&input, DiagnosticBackend::Cpu, &token);
    assert_eq!(luminance.receipt().upper_count, 0);
    assert_pixel(luminance.frame().pixels()[2], input.pixels()[2]);
}

#[test]
fn overexposed_gamut_and_saturation_use_profile_transform_and_all_schemes() {
    let input = DiagnosticFrame::new(
        dimensions(2, 1),
        vec![[0.8, 0.2, 0.2, 0.6], [0.3, 0.3, 0.3, 0.7]],
    )
    .expect("frame");
    for scheme in [
        OverexposedColorScheme::BlackWhite,
        OverexposedColorScheme::RedBlue,
        OverexposedColorScheme::PurpleGreen,
    ] {
        let plan = OverexposedPlan::new(
            ColorEncoding::DisplayP3,
            dimensions(2, 1),
            OverexposedState::new(
                OverexposedMode::Gamut,
                -8.0,
                60.0,
                scheme,
                ColorEncoding::LinearSrgb,
            ),
        )
        .expect("builtin profile plan");
        let result = plan.execute(&input, DiagnosticBackend::Wgpu, &CancellationToken::new());
        assert!(result.applied());
        assert_eq!(
            result.receipt().path,
            rusttable_render::DiagnosticPath::CpuFallback
        );
        assert!((result.frame().pixels()[0][3] - 0.6).abs() <= 1.0e-6);
        assert!((result.frame().pixels()[1][3] - 0.7).abs() <= 1.0e-6);
    }
    let saturation = OverexposedPlan::new(
        ColorEncoding::Srgb,
        dimensions(2, 1),
        OverexposedState::new(
            OverexposedMode::Saturation,
            -8.0,
            60.0,
            OverexposedColorScheme::PurpleGreen,
            ColorEncoding::LinearSrgb,
        ),
    )
    .expect("plan")
    .execute(&input, DiagnosticBackend::Cpu, &CancellationToken::new());
    assert!(saturation.receipt().upper_count <= 2);
}

#[test]
fn diagnostic_descriptors_are_hidden_and_non_mutating() {
    let raw = RawOverexposedPlan::descriptor();
    assert_eq!(raw.compatibility_id(), "rawoverexposed");
    assert_eq!(raw.module_version(), 1);
    assert!(raw.hidden());
    assert!(!raw.affects_history() && !raw.affects_export() && !raw.affects_thumbnail());
    let over = OverexposedPlan::descriptor();
    assert_eq!(over.compatibility_id(), "overexposed");
    assert_eq!(over.module_version(), 3);
}
