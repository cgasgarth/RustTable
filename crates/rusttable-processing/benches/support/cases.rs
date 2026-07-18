use rusttable_core::{
    Asset, AssetId, AssetRole, ByteLength, ContentHash, Edit, EditId, FiniteF64, Operation,
    OperationId, OperationKey, ParameterName, ParameterValue, Photo, PhotoId, Revision,
};
use rusttable_processing::{
    CompiledPipeline, RasterDimensions, SourceRgb, SourceRgbImage, SrgbChannel, WorkingRgbImage,
    evaluate, to_linear_srgb,
};
use std::hint::black_box;

pub fn prepared_photo_assets() -> Vec<Asset> {
    (1..=128)
        .map(|id| {
            Asset::new(
                AssetId::new(id).expect("asset"),
                if id == 1 {
                    AssetRole::Primary
                } else {
                    AssetRole::Sidecar
                },
                ContentHash::Sha256([u8::try_from(id).expect("bounded asset ID"); 32]),
                ByteLength::from_bytes(4096),
            )
        })
        .collect()
}

pub fn consume_photo(assets: &[Asset]) -> u64 {
    let photo = Photo::new(PhotoId::new(1).expect("photo"), assets.iter().copied()).expect("photo");
    black_box(
        photo
            .assets()
            .map(|asset| {
                u64::try_from(asset.id().get())
                    .expect("bounded asset ID")
                    .wrapping_add(asset.byte_length().get())
            })
            .sum(),
    )
}

pub fn prepared_render() -> (WorkingRgbImage, CompiledPipeline) {
    let dimensions = RasterDimensions::new(256, 256).expect("dimensions");
    let channel = SrgbChannel::new(0.25).expect("channel");
    let source = SourceRgbImage::new(
        dimensions,
        vec![SourceRgb::new(channel, channel, channel); 256 * 256],
    )
    .expect("source");
    let edit = Edit::new(
        EditId::new(1).expect("edit"),
        PhotoId::new(1).expect("photo"),
        Revision::ZERO,
        [
            operation(1, "rusttable.exposure", "stops", 1.0),
            operation(2, "rusttable.linear_offset", "value", 0.1),
        ],
    )
    .expect("edit");
    (
        to_linear_srgb(&source),
        CompiledPipeline::compile(&edit).expect("pipeline"),
    )
}

pub fn consume_render(image: &WorkingRgbImage, pipeline: &CompiledPipeline) -> u64 {
    let output = evaluate(pipeline, image).expect("render");
    black_box(
        output
            .pixel_slice()
            .iter()
            .map(|pixel| {
                u64::from(pixel.red().get().to_bits())
                    + u64::from(pixel.green().get().to_bits())
                    + u64::from(pixel.blue().get().to_bits())
            })
            .sum(),
    )
}

fn operation(id: u128, key: &str, parameter: &str, value: f64) -> Operation {
    Operation::new(
        OperationId::new(id).expect("operation"),
        OperationKey::new(key).expect("key"),
        true,
        [(
            ParameterName::new(parameter).expect("parameter"),
            ParameterValue::Scalar(FiniteF64::new(value).expect("value")),
        )],
    )
    .expect("operation")
}
