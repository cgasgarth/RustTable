#[test]
fn darktable_image_contract_sources_are_pinned_and_accounted() {
    let manifest = include_str!("../../../architecture/darktable-image-contracts.toml");
    assert!(manifest.contains("commit = \"cfe57f3bbf5269bfacf31e832267279caa6938ad\""));
    for source in [
        "src/common/image.h",
        "src/develop/imageop.h",
        "src/common/mipmap_cache.h",
        "src/imageio/imageio_common.h",
        "src/imageio/imageio_module.h",
        "src/develop/pixelpipe.h",
        "src/common/imagebuf.c",
        "src/common/mipmap_cache.c",
    ] {
        assert!(
            manifest.contains(source),
            "unaccounted source anchor: {source}"
        );
    }
}
