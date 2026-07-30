use rusttable_image_io::{
    RawCalibrationEvidence, RawCalibrationMatrix, RawCalibrationMatrixKind, RawChannel,
    RawDecodeLimits, RawDecodeRequest, RawFrame, RawGeometryEvidence, RawIdentityEvidence,
    RawIlluminant, RawMetadataContext, RawMetadataEvidence, RawMetadataField,
    RawMetadataFindingCode, RawMetadataProvenance, RawMetadataReceipt, RawMetadataSourceKind,
    RawMetadataStatus, RawPixelAspect, RawPlaneLayout, RawRect, RawlerRawDecoder,
    normalize_raw_metadata, rawler_capability_manifest,
};
use rusttable_testkit::fixtures::deterministic_compressed_raf;

fn decode_fixture() -> rusttable_image_io::RawDecodeResult {
    RawlerRawDecoder::new()
        .decode_bytes(
            deterministic_compressed_raf().bytes(),
            &RawDecodeRequest::new(
                RawDecodeLimits::new(1_000_000, 2_000, 2_000, 4_000_000, 8_000_000)
                    .expect("limits"),
            ),
        )
        .expect("validated RAF")
}

fn provenance(kind: RawMetadataSourceKind, id: &str, byte: u8) -> RawMetadataProvenance {
    RawMetadataProvenance::new(kind, id, [byte; 32])
}

#[test]
fn xpro2_decode_publishes_canonical_14_bit_xtrans_metadata_without_changing_raw() {
    let first = decode_fixture();
    let second = decode_fixture();
    let receipt = &first.receipt.metadata;

    assert_eq!(first.receipt.bit_depth, 14);
    assert_eq!(receipt.normalized.geometry.sensor_precision_bits, 14);
    assert_eq!(
        receipt.normalized.camera.source.maker.as_deref(),
        Some("FUJIFILM")
    );
    assert_eq!(
        receipt.normalized.camera.source.model.as_deref(),
        Some("X-Pro2")
    );
    assert_eq!(
        receipt
            .normalized
            .camera
            .key
            .as_ref()
            .map(|key| (key.maker.as_str(), key.model.as_str())),
        Some(("FUJIFILM", "X-Pro2"))
    );
    assert!(matches!(
        receipt.normalized.geometry.layout,
        RawPlaneLayout::Mosaic(ref cfa) if (cfa.width, cfa.height) == (6, 6)
    ));
    assert_eq!(
        first.frame.parts().planes[0].samples,
        second.frame.parts().planes[0].samples
    );
    assert_eq!(
        receipt.normalized_sha256,
        second.receipt.metadata.normalized_sha256
    );

    let bytes = receipt.to_canonical_bytes().expect("canonical receipt");
    assert!(bytes.len() < 64 * 1024);
    assert_eq!(
        RawMetadataReceipt::from_canonical_bytes(&bytes).expect("round trip"),
        *receipt
    );
    let diagnostic = receipt.diagnostic();
    assert_eq!(diagnostic.normalized_sha256, receipt.normalized_sha256);
    assert!(!format!("{diagnostic:?}").contains("X-Pro2"));
}

#[test]
#[allow(clippy::too_many_lines)]
fn standardized_dng_beats_maker_note_and_invalid_override_falls_back_with_provenance() {
    let decoded = decode_fixture();
    let context = RawMetadataContext {
        primary_source: RawMetadataProvenance::new(
            RawMetadataSourceKind::BackendProfile,
            decoded.receipt.camera_profile_id.clone(),
            rawler_capability_manifest().sha256,
        ),
        backend_profile_id: decoded.receipt.camera_profile_id.clone(),
        content_sha256: decoded.receipt.source.source_sha256,
    };
    let maker_note = RawMetadataEvidence {
        provenance: provenance(RawMetadataSourceKind::MakerNote, "fuji-maker-note-v1", 1),
        identity: RawIdentityEvidence {
            unique_model: Some("maker-note-model".to_owned()),
            ..RawIdentityEvidence::default()
        },
        geometry: RawGeometryEvidence {
            pixel_aspect: Some(RawPixelAspect {
                horizontal: 1,
                vertical: 1,
            }),
            ..RawGeometryEvidence::default()
        },
        calibration: RawCalibrationEvidence::default(),
    };
    let dng = RawMetadataEvidence {
        provenance: provenance(RawMetadataSourceKind::DngStandardized, "dng-ifd-tags-v1", 2),
        identity: RawIdentityEvidence {
            unique_model: Some("FUJIFILM X-Pro2".to_owned()),
            ..RawIdentityEvidence::default()
        },
        geometry: RawGeometryEvidence {
            pixel_aspect: Some(RawPixelAspect {
                horizontal: 3,
                vertical: 2,
            }),
            ..RawGeometryEvidence::default()
        },
        calibration: RawCalibrationEvidence {
            matrices: [RawIlluminant::A, RawIlluminant::D65]
                .into_iter()
                .map(|illuminant| RawCalibrationMatrix {
                    kind: RawCalibrationMatrixKind::ForwardMatrix,
                    illuminant,
                    rows: 3,
                    columns: 3,
                    coefficients: vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                    status: RawMetadataStatus::Complete,
                    provenance: provenance(
                        RawMetadataSourceKind::DngStandardized,
                        "dng-ifd-tags-v1",
                        2,
                    ),
                })
                .collect(),
            as_shot_neutral: Some(vec![0.5, 1.0, 0.625]),
            baseline_exposure: Some(0.25),
            ..RawCalibrationEvidence::default()
        },
    };
    let invalid_override = RawMetadataEvidence {
        provenance: provenance(
            RawMetadataSourceKind::ReviewedOverride,
            "override-table-v1",
            3,
        ),
        identity: RawIdentityEvidence::default(),
        geometry: RawGeometryEvidence {
            default_crop: Some(RawRect {
                x: 9_999,
                y: 0,
                width: 10,
                height: 10,
            }),
            ..RawGeometryEvidence::default()
        },
        calibration: RawCalibrationEvidence {
            matrices: vec![RawCalibrationMatrix {
                kind: RawCalibrationMatrixKind::ForwardMatrix,
                illuminant: RawIlluminant::D65,
                rows: 3,
                columns: 3,
                coefficients: vec![1.0, 2.0, 3.0, 2.0, 4.0, 6.0, 3.0, 6.0, 9.0],
                status: RawMetadataStatus::Complete,
                provenance: provenance(
                    RawMetadataSourceKind::ReviewedOverride,
                    "override-table-v1",
                    3,
                ),
            }],
            ..RawCalibrationEvidence::default()
        },
    };

    let forward = vec![maker_note.clone(), dng.clone(), invalid_override.clone()];
    let mut reverse = forward.clone();
    reverse.reverse();
    let first = normalize_raw_metadata(&decoded.frame, &context, &forward).expect("metadata");
    let second = normalize_raw_metadata(&decoded.frame, &context, &reverse).expect("metadata");

    assert_eq!(first.normalized_sha256, second.normalized_sha256);
    assert_eq!(
        first.normalized.camera.source.unique_model.as_deref(),
        Some("FUJIFILM X-Pro2")
    );
    assert_eq!(
        first.normalized.geometry.pixel_aspect,
        Some(RawPixelAspect {
            horizontal: 3,
            vertical: 2
        })
    );
    assert!(first.conflicts.iter().any(|value| {
        value.field == RawMetadataField::PixelAspect
            && value.selected.source == RawMetadataSourceKind::DngStandardized
            && value.rejected.source == RawMetadataSourceKind::MakerNote
    }));
    assert!(first.normalized.findings.iter().any(|value| {
        value.field == RawMetadataField::ColorMatrix
            && value.code == RawMetadataFindingCode::SingularMatrix
            && value.source.source == RawMetadataSourceKind::ReviewedOverride
    }));
    assert!(first.fallbacks.iter().any(|value| {
        value.field == RawMetadataField::ColorMatrix
            && value.selected.source == RawMetadataSourceKind::DngStandardized
            && value.invalid.source == RawMetadataSourceKind::ReviewedOverride
    }));
    assert!(first.fallbacks.iter().any(|value| {
        value.field == RawMetadataField::DefaultCrop
            && value.invalid.source == RawMetadataSourceKind::ReviewedOverride
    }));
}

#[test]
fn mono_linear_and_missing_calibration_remain_explicit() {
    let decoded = decode_fixture();
    let mut parts = decoded.frame.clone().into_parts();
    parts.planes[0].layout = RawPlaneLayout::Linear {
        channels: vec![RawChannel::Unknown],
    };
    parts.camera.maker = "FUJI PHOTO FILM CO., LTD.".to_owned();
    parts.camera.model = "X-Pro 2".to_owned();
    parts.color_matrices.clear();
    let frame = RawFrame::try_new(parts, RawDecodeLimits::standard()).expect("linear frame");
    let context = RawMetadataContext {
        primary_source: provenance(RawMetadataSourceKind::DngStandardized, "dng-linear-v1", 4),
        backend_profile_id: "rusttable.dng".to_owned(),
        content_sha256: [5; 32],
    };
    let evidence = RawMetadataEvidence {
        provenance: provenance(
            RawMetadataSourceKind::ReviewedOverride,
            "linear-sensor-table-v1",
            6,
        ),
        identity: RawIdentityEvidence::default(),
        geometry: RawGeometryEvidence {
            dual_gain_planes: Some(2),
            ..RawGeometryEvidence::default()
        },
        calibration: RawCalibrationEvidence::default(),
    };

    let receipt = normalize_raw_metadata(&frame, &context, &[evidence]).expect("metadata");
    assert!(matches!(
        receipt.normalized.geometry.layout,
        RawPlaneLayout::Linear { ref channels } if channels == &[RawChannel::Unknown]
    ));
    assert_eq!(receipt.normalized.geometry.dual_gain_planes, Some(2));
    assert_eq!(
        receipt
            .normalized
            .camera
            .key
            .as_ref()
            .map(|key| (key.maker.as_str(), key.model.as_str())),
        Some(("FUJIFILM", "X-Pro2"))
    );
    assert_eq!(
        receipt.normalized.camera.source.maker.as_deref(),
        Some("FUJI PHOTO FILM CO., LTD.")
    );
    assert!(receipt.normalized.calibration.matrices.is_empty());
    assert!(receipt.normalized.findings.iter().any(|finding| {
        finding.field == RawMetadataField::ColorMatrix
            && finding.code == RawMetadataFindingCode::MissingCalibration
    }));
}
