use rusttable_image_io::{ManagedSvgAsset, SvgLimits};
use rusttable_processing::{
    FiniteF32, LinearRgb, RasterDimensions, WatermarkAnchor, WatermarkContext, WatermarkHistory,
    WatermarkParametersV7, WatermarkPlan, WatermarkScaleMode, decode_watermark_history,
    migrate_watermark_history, watermark_descriptor,
};

const RECT: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4" fill="#ff0000"/></svg>"##;

fn black_pixels(count: usize) -> Vec<LinearRgb> {
    let zero = FiniteF32::new(0.0).expect("finite");
    (0..count)
        .map(|_| LinearRgb::new(zero, zero, zero))
        .collect()
}

#[test]
fn watermark_plan_composites_in_linear_light_at_anchor() {
    let asset = ManagedSvgAsset::parse(RECT, SvgLimits::default()).expect("asset");
    let dimensions = RasterDimensions::new(4, 4).expect("dimensions");
    let parameters = WatermarkParametersV7::new(
        asset.source_hash(),
        1.0,
        0.5,
        WatermarkScaleMode::Width,
        WatermarkAnchor::BottomRight,
        0.0,
        0.0,
        0.0,
        [1.0; 4],
        true,
    )
    .expect("parameters");
    let context = WatermarkContext::new("image.raw", 4, 4).expect("context");
    let plan = WatermarkPlan::new(&asset, parameters, &context, dimensions).expect("plan");
    let mut pixels = black_pixels(16);
    plan.execute(&mut pixels, dimensions).expect("execute");
    assert_eq!(plan.receipt().raster_dimensions(), (2, 2));
    assert!(pixels[15].red().get() > 0.99);
    assert_eq!(pixels[0].red().get().to_bits(), 0.0_f32.to_bits());
}

#[test]
fn watermark_context_expansion_is_escaped_and_deterministic() {
    let context = WatermarkContext::new("a&b.raw", 16, 8)
        .expect("context")
        .with_sequence(7, 2)
        .with_variable("author", "A<&B")
        .expect("variable");
    let first = context
        .expand(br"<svg>{{filename}}/{{author}}/{{missing}}</svg>")
        .expect("expansion");
    let second = context
        .expand(br"<svg>{{filename}}/{{author}}/{{missing}}</svg>")
        .expect("expansion");
    assert_eq!(first, second);
    assert_eq!(first.bytes(), br"<svg>a&amp;b.raw/A&lt;&amp;B/</svg>");
    assert_eq!(first.findings(), &["missing".to_owned()]);
}

#[test]
fn watermark_history_round_trips_and_preserves_legacy_opaque_bytes() {
    let parameters = WatermarkParametersV7::default();
    let history = decode_watermark_history(7, &parameters.history_bytes()).expect("decode");
    assert_eq!(
        migrate_watermark_history(history).expect("migrate"),
        parameters
    );
    let opaque = decode_watermark_history(3, &[1, 2, 3]).expect("opaque");
    assert!(matches!(
        opaque,
        WatermarkHistory::Opaque { version: 3, .. }
    ));
}

#[test]
fn watermark_registry_descriptor_exposes_the_blocking_payload_seam() {
    let descriptor = watermark_descriptor();
    assert_eq!(descriptor.id.compatibility_name, "watermark");
    assert_eq!(
        descriptor.migration.source_versions,
        (1..=7).collect::<Vec<_>>()
    );
    let definition = rusttable_processing::builtin_registry()
        .definitions()
        .iter()
        .find(|definition| definition.descriptor().id.compatibility_name == "watermark")
        .expect("registry definition");
    assert!(!definition.availability().is_available());
}
