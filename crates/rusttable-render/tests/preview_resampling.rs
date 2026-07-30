#![forbid(unsafe_code)]

use rusttable_core::{EditId, PhotoId, Revision};
use rusttable_image::ImageDimensions;
use rusttable_image::Roi;
use rusttable_processing::operations::finalscale::{
    FinalScaleConfig, FinalScaleKernel, FinalScalePlan, RenderQuality, RenderSizeRequest,
};
use rusttable_processing::{
    FiniteF32, LinearRgb, RasterDimensions, WorkingRgbImage, encode_working_to_srgb,
};
use rusttable_render::{
    PreparedCpuPixelpipeResult, PreviewBounds, RenderAlphaPolicy, RenderBorderPolicy, RenderPlan,
    RenderSampling, RenderTarget, render_prepared_cpu_pixelpipe,
};

fn provenance() -> rusttable_render::RenderProvenance {
    rusttable_render::RenderProvenance::new(
        EditId::new(7).expect("edit"),
        PhotoId::new(8).expect("photo"),
        Revision::from_u64(9),
        Revision::from_u64(10),
    )
}

fn prepared(
    width: u32,
    height: u32,
    values: Vec<LinearRgb>,
    alpha: Vec<f32>,
) -> PreparedCpuPixelpipeResult {
    let image = WorkingRgbImage::new(
        RasterDimensions::new(width, height).expect("dimensions"),
        values,
    )
    .expect("pixels match dimensions");
    PreparedCpuPixelpipeResult::new(
        image,
        alpha,
        rusttable_render::SourceColorDecision::DeclaredSrgb,
        provenance(),
    )
    .expect("prepared image")
}

fn scalar(value: f32) -> LinearRgb {
    let value = FiniteF32::new(value).expect("finite value");
    LinearRgb::new(value, value, value)
}

fn quantize(value: f32) -> u8 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the encoded fixture channel is bounded to the u8 presentation range"
    )]
    {
        (value * 255.0 + 0.5).floor() as u8
    }
}

fn preview(prepared: &PreparedCpuPixelpipeResult, width: u32, height: u32) -> Vec<u8> {
    render_prepared_cpu_pixelpipe(
        prepared,
        RenderTarget::PreviewFit(PreviewBounds::new(width, height).expect("bounds")),
    )
    .expect("preview")
    .image()
    .pixels()
    .to_vec()
}

#[test]
fn checkerboard_is_low_pass_filtered_instead_of_point_sampled() {
    let values = (0..64)
        .map(|index| {
            if (index % 8 + index / 8) % 2 == 0 {
                0.0
            } else {
                1.0
            }
        })
        .map(scalar)
        .collect();
    let output = preview(&prepared(8, 8, values, vec![1.0; 64]), 4, 4);

    let gray = output
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| pixel[0])
        .collect::<Vec<_>>();
    assert!(gray.iter().all(|&value| (80..=230).contains(&value)));
    assert!(gray.iter().any(|&value| value > 150));
    assert!(gray.iter().any(|&value| value < 220));
}

#[test]
fn diagonal_edge_and_impulse_produce_repeatable_filtered_energy() {
    let diagonal = (0..35)
        .map(|index| if index % 7 >= index / 7 { 1.0 } else { 0.0 })
        .map(scalar)
        .collect();
    let diagonal_output = preview(&prepared(7, 5, diagonal, vec![1.0; 35]), 3, 2);
    assert!(
        diagonal_output
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[0] > 0 && pixel[0] < 255)
    );

    let mut impulse = vec![scalar(0.0); 63];
    impulse[3 * 9 + 4] = scalar(1.0);
    let alpha = vec![1.0; 63];
    let first = preview(&prepared(9, 7, impulse.clone(), alpha.clone()), 4, 3);
    let second = preview(&prepared(9, 7, impulse, alpha), 4, 3);
    assert_eq!(first, second);
    assert!(
        first
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[0] > 0)
            .count()
            > 1
    );
}

#[test]
fn odd_dimensions_non_integer_downscale_preserves_a_smooth_gradient() {
    let values = (0_u16..35)
        .map(|index| {
            let x = f32::from(index % 7) / 6.0;
            let y = f32::from(index / 7) / 4.0;
            let value = x.midpoint(y);
            scalar(value)
        })
        .collect();
    let output = preview(&prepared(7, 5, values, vec![1.0; 35]), 3, 2);
    assert_eq!(output.len(), 2 * 2 * 4);
    let gray = output
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| pixel[0])
        .collect::<Vec<_>>();
    assert!(gray.windows(2).any(|pair| pair[0] != pair[1]));
}

#[test]
fn premultiplied_alpha_edges_do_not_bleed_transparent_color() {
    let values = vec![
        LinearRgb::new(
            FiniteF32::new(1.0).expect("red"),
            FiniteF32::new(0.0).expect("green"),
            FiniteF32::new(0.0).expect("blue"),
        ),
        LinearRgb::new(
            FiniteF32::new(0.0).expect("red"),
            FiniteF32::new(0.0).expect("green"),
            FiniteF32::new(1.0).expect("blue"),
        ),
    ];
    let output = preview(&prepared(2, 1, values, vec![0.0, 1.0]), 1, 1);
    assert!(output[0] <= 1);
    assert!(output[1] <= 1);
    assert!(output[2] >= 250);
    assert!((120..=140).contains(&output[3]));
}

#[test]
fn preview_matches_a_full_output_resampled_with_the_same_linear_plan() {
    let values = (0_u16..77)
        .map(|index| {
            let value = f32::from((index * 13) % 29) / 28.0;
            scalar(value)
        })
        .collect::<Vec<_>>();
    let alpha = vec![1.0; 77];
    let prepared = prepared(11, 7, values.clone(), alpha);
    let output = preview(&prepared, 5, 4);
    let full = render_prepared_cpu_pixelpipe(&prepared, RenderTarget::FullResolution)
        .expect("full output");
    assert_eq!(full.plan().sampling(), RenderSampling::Identity);

    let scale = FinalScalePlan::from_config(
        RasterDimensions::new(11, 7).expect("source"),
        FinalScaleConfig::new(RenderSizeRequest::exact(5, 3))
            .with_quality(RenderQuality::preview(FinalScaleKernel::Bicubic)),
    )
    .expect("scale plan");
    let expected_linear = scale.execute(&values).expect("resampled full output");
    let expected_image = WorkingRgbImage::new(
        RasterDimensions::new(5, 3).expect("output"),
        expected_linear.pixels().to_vec(),
    )
    .expect("expected image");
    let expected_encoded = encode_working_to_srgb(&expected_image);
    let expected = expected_encoded
        .image()
        .pixels()
        .flat_map(|pixel| {
            [
                quantize(pixel.red().get()),
                quantize(pixel.green().get()),
                quantize(pixel.blue().get()),
                255,
            ]
        })
        .collect::<Vec<_>>();
    assert_eq!(output, expected);
}

#[test]
fn shared_resampler_is_identical_when_output_is_rendered_in_tiles() {
    let source = RasterDimensions::new(7, 5).expect("source");
    let plan = FinalScalePlan::from_config(
        source,
        FinalScaleConfig::new(RenderSizeRequest::exact(3, 2))
            .with_quality(RenderQuality::preview(FinalScaleKernel::Bicubic)),
    )
    .expect("scale plan");
    let input = (0_u16..35)
        .map(|value| f32::from(value) / 34.0)
        .collect::<Vec<_>>();
    let full = plan
        .execute_interleaved(&input, 1, 7)
        .expect("full resample");
    let source_roi = Roi::new(0, 0, 7, 5).expect("source roi");
    let first = plan
        .execute_roi(
            &input,
            source_roi,
            Roi::new(0, 0, 3, 1).expect("first tile"),
            1,
            7,
        )
        .expect("first tile");
    let second = plan
        .execute_roi(
            &input,
            source_roi,
            Roi::new(0, 1, 3, 1).expect("second tile"),
            1,
            7,
        )
        .expect("second tile");
    assert_eq!([first, second].concat(), full);
}

#[test]
fn plan_exposes_filter_support_border_alpha_and_scale() {
    let plan = RenderPlan::for_source(
        ImageDimensions::new(11, 7).expect("source"),
        RenderTarget::PreviewFit(PreviewBounds::new(5, 5).expect("bounds")),
    );
    let policy = plan.resampling().expect("filtered policy");
    assert_eq!(policy.filter(), FinalScaleKernel::Bicubic);
    assert_eq!(policy.support(), 2);
    assert_eq!(policy.border(), RenderBorderPolicy::Reflect);
    assert_eq!(policy.alpha(), RenderAlphaPolicy::Premultiplied);
    assert!((plan.scale_x() - 5.0 / 11.0).abs() < f64::EPSILON);
    assert!((plan.scale_y() - 3.0 / 7.0).abs() < f64::EPSILON);
}
