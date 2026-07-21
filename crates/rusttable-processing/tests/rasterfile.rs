use rusttable_image::Roi;
use rusttable_processing::descriptor::{OperationFlags, RoiKind};
use rusttable_processing::operations::rasterfile::{
    RASTERFILE_WGSL, decode_history, migrate_history, wgpu_dispatch,
};
use rusttable_processing::{
    RasterFileChannelMode, RasterFileHistory, RasterFileParametersV1, RasterFilePlan,
    RasterMaskAsset, RasterMaskCache, RasterMaskFormat, RasterMaskLimits, builtin_registry,
    descriptor,
};

fn pfm(width: u32, height: u32, scale: f32, samples: &[[f32; 3]]) -> Vec<u8> {
    let mut bytes = format!("PF\n{width} {height}\n{scale}\n").into_bytes();
    for sample in samples {
        for channel in sample {
            bytes.extend(if scale.is_sign_negative() {
                channel.to_le_bytes()
            } else {
                channel.to_be_bytes()
            });
        }
    }
    bytes
}

fn asset() -> RasterMaskAsset {
    RasterMaskAsset::decode(
        pfm(
            3,
            2,
            1.0,
            &[
                [0.7, 0.0, 0.0],
                [0.0, 0.8, 0.0],
                [0.0, 0.0, 0.0],
                [0.9, 0.0, 0.0],
                [0.0, 0.0, 0.0],
                [0.6, 0.0, 0.0],
            ],
        ),
        RasterFileChannelMode::ALL,
        RasterMaskLimits::default(),
    )
    .expect("test PFM asset")
}

#[test]
fn v1_parameters_round_trip_and_unknown_history_stays_opaque() {
    let parameters = RasterFileParametersV1::new(
        RasterFileChannelMode::GREEN_BLUE,
        b"/managed/source",
        b"source.pfm",
    )
    .expect("parameters");
    assert_eq!(
        RasterFileParametersV1::from_bytes(&parameters.to_bytes()),
        Ok(parameters.clone())
    );
    assert_eq!(
        decode_history(1, &parameters.to_bytes()).expect("history"),
        RasterFileHistory::V1(Box::new(parameters.clone()))
    );
    assert_eq!(
        decode_history(9, &[1, 2, 3]).expect("opaque history"),
        RasterFileHistory::Opaque {
            version: 9,
            bytes: vec![1, 2, 3]
        }
    );
    assert!(
        migrate_history(RasterFileHistory::Opaque {
            version: 9,
            bytes: vec![]
        })
        .is_err()
    );
}

#[test]
fn descriptor_and_registry_publish_the_full_image_mask_contract() {
    let descriptor = descriptor::rasterfile_descriptor();
    descriptor.validate().expect("descriptor");
    assert_eq!(descriptor.id.compatibility_name, "rasterfile");
    assert_eq!(descriptor.id.parameter_version, 1);
    assert_eq!(descriptor.roi, RoiKind::FullImage);
    assert!(descriptor.flags.contains(OperationFlags::MASKS));
    assert!(descriptor.mask_blend.publishes_mask);
    assert!(!descriptor.mask_blend.consumes_mask);
    assert!(!descriptor.mask_blend.blend_if);
    let definition = builtin_registry()
        .definition("rusttable.rasterfile")
        .expect("registry definition");
    assert!(definition.availability().is_available());
    assert_eq!(
        definition.cpu().expect("CPU factory").roi(),
        RoiKind::FullImage
    );
    assert_eq!(
        definition.gpu().expect("GPU binding").binding_id(),
        "rusttable.rasterfile.wgpu"
    );
}

#[test]
fn plan_publishes_tiles_rejects_dimension_mismatch_and_is_path_independent() {
    let first = asset();
    let second = RasterMaskAsset::decode(
        first.original_bytes().to_vec(),
        RasterFileChannelMode::ALL,
        RasterMaskLimits::default(),
    )
    .expect("same managed bytes");
    assert_eq!(first.identity(), second.identity());
    assert_eq!(first.mask().values(), second.mask().values());

    let plan = RasterFilePlan::new(first.clone(), 42).expect("plan");
    plan.validate_input_dimensions((3, 2))
        .expect("matching dimensions");
    assert!(plan.validate_input_dimensions((2, 3)).is_err());
    let publication = plan.publish().expect("publication");
    assert_eq!(publication.raster().values(), plan.mask().values());
    let tile = plan.tile(Roi::new(1, 0, 2, 2).expect("ROI")).expect("tile");
    assert_eq!(tile.mask().values(), &[0.0, 0.6, 0.8, 0.0]);
    assert_eq!(plan.receipt().format(), RasterMaskFormat::Pfm);
}

#[test]
fn vectorization_receipt_is_deterministic_and_cache_is_bounded() {
    let asset = asset();
    let plan = RasterFilePlan::new(asset.clone(), 7).expect("plan");
    let (first_forms, first_receipt) = plan.vectorize().expect("vectorization");
    let (second_forms, second_receipt) = plan.vectorize().expect("repeat vectorization");
    assert_eq!(first_forms, second_forms);
    assert_eq!(first_receipt, second_receipt);
    assert_eq!(first_receipt.form_count(), 2);
    assert_eq!(first_receipt.geometry_hashes().len(), 2);
    assert_eq!(first_forms[0].bounds(), (0, 0, 2, 2));
    assert_eq!(first_forms[1].bounds(), (2, 0, 3, 1));

    let mut cache = RasterMaskCache::new(asset.memory_bytes());
    cache.insert(asset.clone()).expect("cache insert");
    assert_eq!(cache.resident_bytes(), asset.memory_bytes());
    assert!(cache.get(&asset.identity()).is_some());
    cache.insert(asset.clone()).expect("cache replacement");
    assert_eq!(cache.resident_bytes(), asset.memory_bytes());
}

#[test]
fn gpu_publication_metadata_has_checked_dispatch_arithmetic() {
    assert_eq!(wgpu_dispatch(3, 2).expect("dispatch"), 1);
    assert!(wgpu_dispatch(0, 2).is_err());
    assert!(RASTERFILE_WGSL.contains("src_stride"));
    assert!(RASTERFILE_WGSL.contains("clamp"));
}

#[test]
fn invalid_channel_modes_and_resource_limits_are_blocking() {
    assert!(RasterFileChannelMode::new(0).is_err());
    assert!(RasterFileChannelMode::new(8).is_err());
    assert!(
        RasterMaskAsset::decode(
            b"not an image".to_vec(),
            RasterFileChannelMode::RED,
            RasterMaskLimits::default()
        )
        .is_err()
    );
}
