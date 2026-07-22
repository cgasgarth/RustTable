use super::*;

fn descriptor(mode: &str) -> RawCapabilityDescriptor {
    RawCapabilityDescriptor {
        key: RawCapabilityKey {
            maker: "ACME".to_owned(),
            model: "Alias One".to_owned(),
            mode: mode.to_owned(),
            container: RawContainerKind::Dng,
            compression: RawCompression::BackendDefined,
            cfa: "RGGB".to_owned(),
        },
        family: RawVendorFamily::Canon,
        profile_id: "test".to_owned(),
        backend_format: "DNG".to_owned(),
        decoder_path: "test".to_owned(),
        layout: RawCapabilityLayout::BayerOrLinear,
        quirk_ids: Vec::new(),
        reference_evidence: Vec::new(),
        reviewed: true,
        normalized_maker: "Acme".to_owned(),
        normalized_model: "Camera".to_owned(),
        bit_depth: Some(14),
        corpus_fixtures: Vec::new(),
    }
}

#[test]
fn ambiguous_camera_aliases_never_select_by_collection_order() {
    let first = RawCapabilityManifest::generated(
        vec![descriptor("compressed"), descriptor("uncompressed")],
        [1; 32],
    );
    let second = RawCapabilityManifest::generated(
        vec![descriptor("uncompressed"), descriptor("compressed")],
        [2; 32],
    );
    for manifest in [first, second] {
        assert_eq!(
            manifest.resolve("ACME", "Alias One", RawContainerKind::Dng),
            Err(RawCapabilityResolveError::Ambiguous { candidates: 2 })
        );
    }
}

#[test]
fn nonfinite_matrix_is_rejected_before_frame_publication() {
    let dimensions = RawDimensions::new(1, 1).expect("dimensions");
    let area = RawRect::new(0, 0, 1, 1).expect("area");
    let parts = RawFrameParts {
        planes: vec![RawPlane {
            dimensions,
            channels_per_pixel: 1,
            layout: RawPlaneLayout::Mosaic(RawCfa {
                width: 2,
                height: 2,
                phase_x: 0,
                phase_y: 0,
                pattern: vec![
                    RawChannel::Red,
                    RawChannel::Green,
                    RawChannel::Green,
                    RawChannel::Blue,
                ],
            }),
            samples: vec![1],
        }],
        active_area: area,
        crop_area: area,
        masked_areas: Vec::new(),
        black_levels: RawLevelPattern {
            repeat_width: 1,
            repeat_height: 1,
            channels: 1,
            values: vec![0.0],
        },
        white_levels: vec![4_095.0],
        orientation: RawOrientation::Normal,
        camera: RawCameraIdentity {
            maker: "ACME".to_owned(),
            model: "Camera".to_owned(),
            normalized_maker: "Acme".to_owned(),
            normalized_model: "Camera".to_owned(),
            mode: String::new(),
        },
        color_matrices: vec![RawColorMatrix {
            illuminant: RawIlluminant::D65,
            rows: 3,
            columns: 3,
            coefficients: vec![f32::NAN; 9],
        }],
        white_balance: vec![Some(1.0), Some(1.0), Some(1.0), None],
        compression: RawCompressionEvidence {
            compression: RawCompression::Uncompressed,
            container_code: Some(1),
        },
        bit_depth: 12,
        opcodes: Vec::new(),
        previews: Vec::new(),
    };
    assert_eq!(
        RawFrame::try_new(parts, RawDecodeLimits::standard()),
        Err(RawFrameValidationError::InvalidMatrix)
    );
}
