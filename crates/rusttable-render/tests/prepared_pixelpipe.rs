use rusttable_core::{EditId, PhotoId, Revision};
use rusttable_image::ImageDimensions;
use rusttable_processing::{FiniteF32, LinearRgb, RasterDimensions, WorkingRgbImage};
use rusttable_render::{
    PreparedCpuPixelpipeResult, PreparedCpuPixelpipeResultError, PreviewBounds, RenderProvenance,
    RenderSampling, RenderTarget, SCENE_REFERRED_RAW_EXPOSURE_STOPS,
    SCENE_REFERRED_RAW_LINEAR_GAIN, SourceColorDecision, SrgbFallbackContract,
    render_prepared_cpu_pixelpipe,
};

fn pixels(values: &[(f32, f32, f32)], width: u32, height: u32) -> WorkingRgbImage {
    WorkingRgbImage::new(
        RasterDimensions::new(width, height).expect("nonzero dimensions"),
        values
            .iter()
            .map(|&(red, green, blue)| {
                LinearRgb::new(
                    FiniteF32::new(red).expect("finite red"),
                    FiniteF32::new(green).expect("finite green"),
                    FiniteF32::new(blue).expect("finite blue"),
                )
            })
            .collect(),
    )
    .expect("matching pixels")
}

fn provenance() -> RenderProvenance {
    RenderProvenance::new(
        EditId::new(7).expect("nonzero edit ID"),
        PhotoId::new(8).expect("nonzero photo ID"),
        Revision::from_u64(9),
        Revision::from_u64(10),
    )
}

#[test]
fn full_render_encodes_the_completed_pixelpipe_result_without_re_evaluation() {
    let prepared = PreparedCpuPixelpipeResult::new(
        pixels(&[(0.25, 0.5, 2.0)], 1, 1),
        vec![71.0 / 255.0],
        SourceColorDecision::DeclaredSrgb,
        provenance(),
    )
    .expect("matching alpha");

    let output = render_prepared_cpu_pixelpipe(&prepared, RenderTarget::FullResolution)
        .expect("rendered prepared result");

    assert_eq!(
        output.image().dimensions(),
        ImageDimensions::new(1, 1).unwrap()
    );
    assert_eq!(output.image().pixels(), &[137, 188, 255, 71]);
    assert_eq!(output.plan().sampling(), RenderSampling::Identity);
    assert_eq!(
        output.source_color_decision(),
        SourceColorDecision::DeclaredSrgb
    );
    assert_eq!(output.provenance(), provenance());
    assert_eq!(output.clipping().above_one().blue(), 1);
}

#[test]
fn scene_referred_raw_fallback_applies_the_darktable_baseline_before_srgb_transfer() {
    let prepared = PreparedCpuPixelpipeResult::new(
        pixels(
            &[(0.05, 0.05, 0.05), (0.18, 0.18, 0.18), (0.5, 0.5, 0.5)],
            3,
            1,
        ),
        vec![1.0; 3],
        SourceColorDecision::EmbeddedChromaticities,
        provenance(),
    )
    .expect("matching alpha");
    let colorimetric = render_prepared_cpu_pixelpipe(&prepared, RenderTarget::FullResolution)
        .expect("colorimetric render");
    let raw = render_prepared_cpu_pixelpipe(
        &prepared.with_presentation(SrgbFallbackContract::SceneReferredRawV1),
        RenderTarget::FullResolution,
    )
    .expect("scene-referred RAW render");

    assert_eq!(
        SCENE_REFERRED_RAW_EXPOSURE_STOPS.to_bits(),
        0.7_f32.to_bits()
    );
    assert!((SCENE_REFERRED_RAW_LINEAR_GAIN - 0.7_f32.exp2()).abs() < 0.000_001);
    assert_eq!(
        colorimetric.image().pixels(),
        &[63, 63, 63, 255, 118, 118, 118, 255, 188, 188, 188, 255]
    );
    assert_eq!(
        raw.image().pixels(),
        &[80, 80, 80, 255, 147, 147, 147, 255, 233, 233, 233, 255]
    );
    assert_eq!(
        colorimetric.presentation(),
        SrgbFallbackContract::Colorimetric
    );
    assert_eq!(raw.presentation(), SrgbFallbackContract::SceneReferredRawV1);
}

#[test]
fn preview_sampling_routes_the_completed_pixelpipe_result() {
    let prepared = PreparedCpuPixelpipeResult::new(
        pixels(
            &[
                (0.0, 0.0, 0.0),
                (1.0, 0.0, 0.0),
                (0.0, 1.0, 0.0),
                (0.0, 0.0, 1.0),
            ],
            4,
            1,
        ),
        vec![1.0 / 255.0, 2.0 / 255.0, 3.0 / 255.0, 4.0 / 255.0],
        SourceColorDecision::AssumedSrgb,
        provenance(),
    )
    .expect("matching alpha");

    let output = render_prepared_cpu_pixelpipe(
        &prepared,
        RenderTarget::PreviewFit(PreviewBounds::new(2, 1).unwrap()),
    )
    .expect("rendered prepared preview");

    assert_eq!(
        output.image().dimensions(),
        ImageDimensions::new(2, 1).unwrap()
    );
    assert_eq!(output.image().pixels(), &[219, 101, 0, 2, 55, 183, 188, 3]);
    assert_eq!(output.plan().sampling(), RenderSampling::Filtered);
    assert_eq!(
        output.source_color_decision(),
        SourceColorDecision::AssumedSrgb
    );
}

#[test]
fn prepared_result_rejects_incomplete_alpha() {
    let error = PreparedCpuPixelpipeResult::new(
        pixels(&[(0.0, 0.0, 0.0), (0.0, 0.0, 0.0)], 2, 1),
        vec![1.0 / 255.0],
        SourceColorDecision::DeclaredSrgb,
        provenance(),
    )
    .expect_err("alpha must cover every pixel");

    assert_eq!(
        error,
        PreparedCpuPixelpipeResultError::AlphaLength {
            expected: 2,
            actual: 1,
        }
    );
}
