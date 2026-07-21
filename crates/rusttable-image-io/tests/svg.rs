use rusttable_image_io::{ManagedSvgAsset, SvgError, SvgLimits};

const RECT: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4" fill="#ff0000"/></svg>"##;

#[test]
fn managed_svg_is_bounded_and_rasterizes_deterministically() {
    let asset = ManagedSvgAsset::parse(RECT.to_vec(), SvgLimits::default()).expect("managed SVG");
    let first = asset.rasterize(4, 4).expect("raster");
    let second = asset.rasterize(4, 4).expect("raster");
    assert_eq!(first, second);
    assert_eq!(
        asset.source_hash(),
        ManagedSvgAsset::parse(RECT.to_vec(), SvgLimits::default())
            .unwrap()
            .source_hash()
    );
    assert_eq!(&first.pixels()[..4], &[255, 0, 0, 255]);
}

#[test]
fn managed_svg_rejects_external_and_active_content() {
    for source in [
        br#"<svg xmlns="http://www.w3.org/2000/svg"><image href="file:///tmp/private.png" width="1" height="1"/></svg>"#.as_slice(),
        br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#.as_slice(),
    ] {
        assert!(ManagedSvgAsset::parse(source.to_vec(), SvgLimits::default()).is_err());
    }
}

#[test]
fn managed_svg_source_limit_is_checked_before_parsing() {
    let limits = SvgLimits {
        max_source_bytes: 8,
        ..SvgLimits::default()
    };
    assert!(matches!(
        ManagedSvgAsset::parse(RECT.to_vec(), limits),
        Err(SvgError::SourceTooLarge { .. })
    ));
}
