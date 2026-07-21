use rusttable_processing::{
    FiniteF32, LinearRgb, OverlayAlpha, OverlayAnchor, OverlayAsset, OverlayAssetStore,
    OverlayBaseScale, OverlayConfig, OverlayEdge, OverlayImageScale, OverlayInterpolation,
    OverlayPlan, OverlayReference, RasterDimensions,
};

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("dimensions")
}

fn black() -> LinearRgb {
    LinearRgb::new(
        FiniteF32::new(0.0).expect("red"),
        FiniteF32::new(0.0).expect("green"),
        FiniteF32::new(0.0).expect("blue"),
    )
}

fn asset() -> OverlayAsset {
    OverlayAsset::from_rgba8(2, 2, [255, 0, 0, 255].repeat(4)).expect("asset")
}

fn config(asset: &OverlayAsset) -> OverlayConfig {
    let mut config = OverlayConfig::defaults(asset.identity());
    config.interpolation = OverlayInterpolation::Nearest;
    config.edge = OverlayEdge::Transparent;
    config.alpha = OverlayAlpha::Straight;
    config.anchor = OverlayAnchor::Center;
    config.base_scale = OverlayBaseScale::Image;
    config.image_scale = OverlayImageScale::Larger;
    config.reference = OverlayReference::Width;
    config
}

#[test]
fn managed_asset_identity_and_budget_are_path_independent() {
    let asset = asset();
    let identity = asset.identity();
    let mut store = OverlayAssetStore::new(asset.memory_bytes());
    assert_eq!(store.insert(asset.clone()).expect("insert"), identity);
    assert_eq!(store.insert(asset).expect("deduplicated insert"), identity);
    assert_eq!(store.bytes(), 16);
    assert!(
        OverlayAssetStore::new(15)
            .insert(OverlayAsset::from_rgba8(2, 2, [0, 0, 0, 255].repeat(4)).expect("asset"))
            .is_err()
    );
}

#[test]
fn centered_overlay_composites_and_out_of_frame_is_pass_through() {
    let asset = asset();
    let plan = OverlayPlan::new(asset.clone(), config(&asset), dimensions(4, 4)).expect("plan");
    let input = vec![black(); 16];
    let output = plan.execute(&input).expect("composite");
    assert_eq!(output.pixels().len(), 16);
    assert!(output.pixels()[5].red().get() > 0.9);
    assert!(output.receipt().sampled_pixels > 0);
    assert!(!output.receipt().pass_through);

    let mut shifted = config(&asset);
    shifted.xoffset = FiniteF32::new(1.0).expect("offset");
    let shifted_plan = OverlayPlan::new(asset, shifted, dimensions(1, 1)).expect("plan");
    let pass = shifted_plan.execute(&[black()]).expect("pass-through");
    assert_eq!(pass.pixels(), &[black()]);
    assert!(pass.receipt().pass_through);
}

#[test]
fn parameters_round_trip_and_cancellation_are_explicit() {
    let asset = asset();
    let parameters = rusttable_processing::OverlayParametersV1::new(
        OverlayConfig::defaults([0; 32]),
        b"managed/overlay.png",
    )
    .expect("parameters");
    let decoded = rusttable_processing::OverlayParametersV1::from_bytes(&parameters.to_bytes())
        .expect("decode");
    assert_eq!(decoded.config.opacity, parameters.config.opacity);
    assert_eq!(decoded.config.scale, parameters.config.scale);
    assert_eq!(decoded.config.anchor, parameters.config.anchor);
    assert_eq!(decoded.filename, parameters.filename);
    let plan = OverlayPlan::new(asset.clone(), config(&asset), dimensions(4, 4)).expect("plan");
    assert!(matches!(
        plan.execute_with_cancel(&[black(); 16], || true),
        Err(rusttable_processing::OverlayExecutionError::Cancelled)
    ));
}
