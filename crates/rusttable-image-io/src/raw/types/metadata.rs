use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    RawCapabilityDescriptor, RawDimensions, RawFrame, RawIlluminant, RawLevelPattern,
    RawMetadataError, RawOrientation, RawPlaneLayout, RawRect,
};

mod validation;

use validation::{
    normalize_calibration, normalize_cfa_phase, rect_contains, valid_black_levels, valid_layout,
    validate_geometry,
};

pub const RAW_METADATA_SCHEMA_VERSION: u16 = 1;
const MAX_EVIDENCE_ITEMS: usize = 64;
const MAX_SOURCE_ID_BYTES: usize = 128;
const MAX_RECEIPT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RawMetadataSourceKind {
    Derived,
    BackendProfile,
    MakerNote,
    DngStandardized,
    ReviewedOverride,
}

impl RawMetadataSourceKind {
    const fn precedence(self) -> u8 {
        match self {
            Self::Derived => 0,
            Self::BackendProfile => 1,
            Self::MakerNote => 2,
            Self::DngStandardized => 3,
            Self::ReviewedOverride => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RawMetadataProvenance {
    pub source: RawMetadataSourceKind,
    /// A bounded schema/profile/tag-set identifier. Paths and serial numbers have no field here.
    pub source_id: String,
    pub source_sha256: [u8; 32],
}

impl RawMetadataProvenance {
    #[must_use]
    pub fn new(
        source: RawMetadataSourceKind,
        source_id: impl Into<String>,
        hash: [u8; 32],
    ) -> Self {
        Self {
            source,
            source_id: source_id.into(),
            source_sha256: hash,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RawMetadataStatus {
    Complete,
    Partial,
    Unknown,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RawCameraKey {
    pub maker: String,
    pub model: String,
    pub variant: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RawSensorKey {
    pub camera: RawCameraKey,
    pub layout: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawSourceCameraIdentity {
    pub maker: Option<String>,
    pub model: Option<String>,
    pub unique_model: Option<String>,
    pub backend_profile_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawCameraMetadata {
    pub source: RawSourceCameraIdentity,
    pub key: Option<RawCameraKey>,
    pub sensor_key: Option<RawSensorKey>,
    pub status: RawMetadataStatus,
    pub provenance: Vec<RawMetadataProvenance>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawPixelAspect {
    pub horizontal: u32,
    pub vertical: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawSensorGeometry {
    pub full_mosaic: RawDimensions,
    pub active_area: RawRect,
    pub default_crop: RawRect,
    pub masked_areas: Vec<RawRect>,
    pub orientation: RawOrientation,
    pub pixel_aspect: Option<RawPixelAspect>,
    /// CFA phase is normalized to the selected default crop and orientation.
    pub layout: RawPlaneLayout,
    pub channels: u8,
    pub sensor_precision_bits: u8,
    pub black_levels: RawLevelPattern,
    pub white_levels: Vec<f32>,
    pub plane_count: u8,
    pub dual_gain_planes: Option<u8>,
    pub status: RawMetadataStatus,
    pub provenance: Vec<RawMetadataProvenance>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RawCalibrationMatrixKind {
    CameraToXyz,
    CameraCalibration,
    ForwardMatrix,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawCalibrationMatrix {
    pub kind: RawCalibrationMatrixKind,
    pub illuminant: RawIlluminant,
    pub rows: u8,
    pub columns: u8,
    pub coefficients: Vec<f32>,
    pub status: RawMetadataStatus,
    pub provenance: RawMetadataProvenance,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawNoiseProfile {
    pub iso: Option<u32>,
    pub shot_noise: Vec<f32>,
    pub read_noise: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawCalibrationMetadata {
    pub matrices: Vec<RawCalibrationMatrix>,
    pub analog_balance: Option<Vec<f32>>,
    pub as_shot_neutral: Option<Vec<f32>>,
    pub white_balance: Vec<Option<f32>>,
    pub baseline_exposure: Option<f32>,
    pub noise_profile: Option<RawNoiseProfile>,
    pub status: RawMetadataStatus,
    pub provenance: Vec<RawMetadataProvenance>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RawMetadataField {
    CameraIdentity,
    UniqueCameraModel,
    FullDimensions,
    ActiveArea,
    DefaultCrop,
    MaskedAreas,
    Orientation,
    PixelAspect,
    CfaLayout,
    SensorPrecision,
    BlackLevels,
    WhiteLevels,
    DualGain,
    ColorMatrix,
    AnalogBalance,
    AsShotNeutral,
    WhiteBalance,
    BaselineExposure,
    NoiseProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RawMetadataFindingCode {
    Conflict,
    UnknownCamera,
    InvalidCrop,
    InvalidMaskedArea,
    InvalidPixelAspect,
    InvalidLevels,
    InvalidWhiteBalance,
    InvalidMatrixShape,
    NonFiniteMatrix,
    SingularMatrix,
    IllConditionedMatrix,
    MissingIlluminant,
    MissingCalibration,
    InvalidNoiseProfile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawMetadataFinding {
    pub field: RawMetadataField,
    pub code: RawMetadataFindingCode,
    pub source: RawMetadataProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawMetadataSelection {
    pub field: RawMetadataField,
    pub source: RawMetadataProvenance,
    pub value_sha256: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawMetadataConflict {
    pub field: RawMetadataField,
    pub selected: RawMetadataProvenance,
    pub rejected: RawMetadataProvenance,
    pub rejected_value_sha256: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawMetadataFallback {
    pub field: RawMetadataField,
    pub selected: RawMetadataProvenance,
    pub invalid: RawMetadataProvenance,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawMetadataRecord {
    pub schema_version: u16,
    pub camera: RawCameraMetadata,
    pub geometry: RawSensorGeometry,
    pub calibration: RawCalibrationMetadata,
    pub findings: Vec<RawMetadataFinding>,
}

impl RawMetadataRecord {
    /// Computes the deterministic hash of the canonical normalized record.
    ///
    /// # Panics
    ///
    /// Panics only if this fixed, derive-backed schema cannot be serialized.
    #[must_use]
    pub fn stable_hash(&self) -> [u8; 32] {
        let bytes = postcard::to_allocvec(self).expect("RAW metadata record is serializable");
        Sha256::digest(bytes).into()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawMetadataReceipt {
    pub normalized: RawMetadataRecord,
    pub sources: Vec<RawMetadataProvenance>,
    pub selections: Vec<RawMetadataSelection>,
    pub conflicts: Vec<RawMetadataConflict>,
    pub fallbacks: Vec<RawMetadataFallback>,
    pub normalized_sha256: [u8; 32],
}

impl RawMetadataReceipt {
    /// Serializes the canonical receipt without including a path or camera serial.
    ///
    /// # Errors
    ///
    /// Returns [`RawMetadataError::Serialization`] if the fixed schema cannot serialize.
    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>, RawMetadataError> {
        postcard::to_allocvec(self).map_err(|_| RawMetadataError::Serialization)
    }

    /// Restores a bounded receipt and verifies its schema and normalized-record hash.
    ///
    /// # Errors
    ///
    /// Returns a typed error for over-limit, malformed, unsupported, or hash-mismatched bytes.
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self, RawMetadataError> {
        if bytes.len() > MAX_RECEIPT_BYTES {
            return Err(RawMetadataError::CorruptReceipt);
        }
        let value: Self =
            postcard::from_bytes(bytes).map_err(|_| RawMetadataError::CorruptReceipt)?;
        if value.normalized.schema_version != RAW_METADATA_SCHEMA_VERSION {
            return Err(RawMetadataError::UnsupportedSchema {
                version: value.normalized.schema_version,
            });
        }
        if value.normalized.stable_hash() != value.normalized_sha256 {
            return Err(RawMetadataError::CorruptReceipt);
        }
        Ok(value)
    }

    #[must_use]
    pub fn diagnostic(&self) -> RawMetadataDiagnostic {
        RawMetadataDiagnostic {
            normalized_sha256: self.normalized_sha256,
            source_kinds: self.sources.iter().map(|value| value.source).collect(),
            finding_codes: self
                .normalized
                .findings
                .iter()
                .map(|value| value.code)
                .collect(),
            conflict_count: u16::try_from(self.conflicts.len()).unwrap_or(u16::MAX),
            fallback_count: u16::try_from(self.fallbacks.len()).unwrap_or(u16::MAX),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawMetadataDiagnostic {
    pub normalized_sha256: [u8; 32],
    pub source_kinds: Vec<RawMetadataSourceKind>,
    pub finding_codes: Vec<RawMetadataFindingCode>,
    pub conflict_count: u16,
    pub fallback_count: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawMetadataContext {
    pub primary_source: RawMetadataProvenance,
    pub backend_profile_id: String,
    pub content_sha256: [u8; 32],
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawIdentityEvidence {
    pub maker: Option<String>,
    pub model: Option<String>,
    pub unique_model: Option<String>,
    pub variant: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RawGeometryEvidence {
    pub full_mosaic: Option<RawDimensions>,
    pub active_area: Option<RawRect>,
    pub default_crop: Option<RawRect>,
    pub masked_areas: Option<Vec<RawRect>>,
    pub orientation: Option<RawOrientation>,
    pub pixel_aspect: Option<RawPixelAspect>,
    pub layout: Option<RawPlaneLayout>,
    pub sensor_precision_bits: Option<u8>,
    pub black_levels: Option<RawLevelPattern>,
    pub white_levels: Option<Vec<f32>>,
    pub dual_gain_planes: Option<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RawCalibrationEvidence {
    pub matrices: Vec<RawCalibrationMatrix>,
    pub analog_balance: Option<Vec<f32>>,
    pub as_shot_neutral: Option<Vec<f32>>,
    pub white_balance: Option<Vec<Option<f32>>>,
    pub baseline_exposure: Option<f32>,
    pub noise_profile: Option<RawNoiseProfile>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawMetadataEvidence {
    pub provenance: RawMetadataProvenance,
    pub identity: RawIdentityEvidence,
    pub geometry: RawGeometryEvidence,
    pub calibration: RawCalibrationEvidence,
}

/// Reconciles already-parsed metadata. It never parses input bytes or changes sensor samples.
///
/// # Errors
///
/// Returns a typed error when evidence exceeds the bound or has an unsafe source identifier.
#[allow(clippy::too_many_lines)]
pub fn normalize_raw_metadata(
    frame: &RawFrame,
    context: &RawMetadataContext,
    evidence: &[RawMetadataEvidence],
) -> Result<RawMetadataReceipt, RawMetadataError> {
    if evidence.len() > MAX_EVIDENCE_ITEMS {
        return Err(RawMetadataError::TooManyEvidenceItems {
            actual: evidence.len(),
            limit: MAX_EVIDENCE_ITEMS,
        });
    }
    validate_provenance(&context.primary_source)?;
    for item in evidence {
        validate_provenance(&item.provenance)?;
    }

    let parts = frame.parts();
    let derived_source = RawMetadataProvenance::new(
        RawMetadataSourceKind::Derived,
        "raw-content",
        context.content_sha256,
    );
    let mut sources = vec![context.primary_source.clone(), derived_source.clone()];
    sources.extend(evidence.iter().map(|item| item.provenance.clone()));
    sources.sort();
    sources.dedup();

    let mut findings = Vec::new();
    let mut selections = Vec::new();
    let mut conflicts = Vec::new();

    let identity = normalize_identity(parts, context, evidence, &mut findings, &mut selections);

    let dimensions = choose_optional(
        RawMetadataField::FullDimensions,
        Some((&context.primary_source, parts.planes[0].dimensions)),
        evidence.iter().filter_map(|item| {
            item.geometry
                .full_mosaic
                .map(|value| (&item.provenance, value))
        }),
        |value| value.width > 0 && value.height > 0,
        &mut findings,
        &mut selections,
        &mut conflicts,
        RawMetadataFindingCode::InvalidCrop,
    )
    .unwrap_or(parts.planes[0].dimensions);
    let sensor_rect = RawRect {
        x: 0,
        y: 0,
        width: dimensions.width,
        height: dimensions.height,
    };
    let active_area = choose_optional(
        RawMetadataField::ActiveArea,
        Some((&context.primary_source, parts.active_area)),
        evidence.iter().filter_map(|item| {
            item.geometry
                .active_area
                .map(|value| (&item.provenance, value))
        }),
        |value| rect_contains(sensor_rect, *value),
        &mut findings,
        &mut selections,
        &mut conflicts,
        RawMetadataFindingCode::InvalidCrop,
    )
    .unwrap_or_else(|| {
        selections.push(RawMetadataSelection {
            field: RawMetadataField::ActiveArea,
            source: derived_source.clone(),
            value_sha256: hash_value(&sensor_rect),
        });
        sensor_rect
    });
    let default_crop = choose_optional(
        RawMetadataField::DefaultCrop,
        Some((&context.primary_source, parts.crop_area)),
        evidence.iter().filter_map(|item| {
            item.geometry
                .default_crop
                .map(|value| (&item.provenance, value))
        }),
        |value| rect_contains(active_area, *value),
        &mut findings,
        &mut selections,
        &mut conflicts,
        RawMetadataFindingCode::InvalidCrop,
    )
    .unwrap_or_else(|| {
        selections.push(RawMetadataSelection {
            field: RawMetadataField::DefaultCrop,
            source: derived_source.clone(),
            value_sha256: hash_value(&active_area),
        });
        active_area
    });
    let masked_areas = choose_optional(
        RawMetadataField::MaskedAreas,
        Some((&context.primary_source, parts.masked_areas.clone())),
        evidence.iter().filter_map(|item| {
            item.geometry
                .masked_areas
                .clone()
                .map(|value| (&item.provenance, value))
        }),
        |values| {
            values
                .iter()
                .all(|value| rect_contains(sensor_rect, *value))
        },
        &mut findings,
        &mut selections,
        &mut conflicts,
        RawMetadataFindingCode::InvalidMaskedArea,
    )
    .unwrap_or_else(|| {
        selections.push(RawMetadataSelection {
            field: RawMetadataField::MaskedAreas,
            source: derived_source.clone(),
            value_sha256: hash_value(&Vec::<RawRect>::new()),
        });
        Vec::new()
    });
    let orientation = choose_optional(
        RawMetadataField::Orientation,
        Some((&context.primary_source, parts.orientation)),
        evidence.iter().filter_map(|item| {
            item.geometry
                .orientation
                .map(|value| (&item.provenance, value))
        }),
        |_| true,
        &mut findings,
        &mut selections,
        &mut conflicts,
        RawMetadataFindingCode::Conflict,
    )
    .unwrap_or(parts.orientation);
    let mut layout = choose_optional(
        RawMetadataField::CfaLayout,
        Some((&context.primary_source, parts.planes[0].layout.clone())),
        evidence.iter().filter_map(|item| {
            item.geometry
                .layout
                .clone()
                .map(|value| (&item.provenance, value))
        }),
        valid_layout,
        &mut findings,
        &mut selections,
        &mut conflicts,
        RawMetadataFindingCode::Conflict,
    )
    .unwrap_or_else(|| parts.planes[0].layout.clone());
    normalize_cfa_phase(&mut layout, active_area, default_crop, orientation);
    let channels = match &layout {
        RawPlaneLayout::Mosaic(_) => 1,
        RawPlaneLayout::Linear { channels } => u8::try_from(channels.len()).unwrap_or(u8::MAX),
    };
    let plane_count = u8::try_from(parts.planes.len()).unwrap_or(u8::MAX);
    let sensor_precision_bits = choose_optional(
        RawMetadataField::SensorPrecision,
        Some((&context.primary_source, parts.bit_depth)),
        evidence.iter().filter_map(|item| {
            item.geometry
                .sensor_precision_bits
                .map(|value| (&item.provenance, value))
        }),
        |value| (1..=16).contains(value),
        &mut findings,
        &mut selections,
        &mut conflicts,
        RawMetadataFindingCode::InvalidLevels,
    )
    .unwrap_or(parts.bit_depth);
    let black_levels = choose_optional(
        RawMetadataField::BlackLevels,
        Some((&context.primary_source, parts.black_levels.clone())),
        evidence.iter().filter_map(|item| {
            item.geometry
                .black_levels
                .clone()
                .map(|value| (&item.provenance, value))
        }),
        valid_black_levels,
        &mut findings,
        &mut selections,
        &mut conflicts,
        RawMetadataFindingCode::InvalidLevels,
    )
    .unwrap_or_else(|| parts.black_levels.clone());
    let white_levels = choose_optional(
        RawMetadataField::WhiteLevels,
        Some((&context.primary_source, parts.white_levels.clone())),
        evidence.iter().filter_map(|item| {
            item.geometry
                .white_levels
                .clone()
                .map(|value| (&item.provenance, value))
        }),
        |values| {
            let minimum = values.iter().copied().fold(f32::INFINITY, f32::min);
            !values.is_empty()
                && values.iter().all(|value| value.is_finite() && *value > 0.0)
                && black_levels.values.iter().all(|value| *value < minimum)
        },
        &mut findings,
        &mut selections,
        &mut conflicts,
        RawMetadataFindingCode::InvalidLevels,
    )
    .unwrap_or_else(|| parts.white_levels.clone());
    let pixel_aspect = choose_optional(
        RawMetadataField::PixelAspect,
        None,
        evidence.iter().filter_map(|item| {
            item.geometry
                .pixel_aspect
                .map(|value| (&item.provenance, value))
        }),
        |value| value.horizontal > 0 && value.vertical > 0,
        &mut findings,
        &mut selections,
        &mut conflicts,
        RawMetadataFindingCode::InvalidPixelAspect,
    );
    let dual_gain_planes = choose_optional(
        RawMetadataField::DualGain,
        None,
        evidence.iter().filter_map(|item| {
            item.geometry
                .dual_gain_planes
                .map(|value| (&item.provenance, value))
        }),
        |value| (2..=4).contains(value),
        &mut findings,
        &mut selections,
        &mut conflicts,
        RawMetadataFindingCode::Conflict,
    );

    validate_geometry(
        dimensions,
        active_area,
        default_crop,
        &masked_areas,
        &black_levels,
        &white_levels,
        &context.primary_source,
        &mut findings,
    );

    let geometry = RawSensorGeometry {
        full_mosaic: dimensions,
        active_area,
        default_crop,
        masked_areas,
        orientation,
        pixel_aspect,
        layout,
        channels,
        sensor_precision_bits,
        black_levels,
        white_levels,
        plane_count,
        dual_gain_planes,
        status: section_status(
            &findings,
            &[
                RawMetadataField::ActiveArea,
                RawMetadataField::DefaultCrop,
                RawMetadataField::MaskedAreas,
                RawMetadataField::PixelAspect,
                RawMetadataField::BlackLevels,
                RawMetadataField::WhiteLevels,
            ],
        ),
        provenance: sources.clone(),
    };

    let calibration = normalize_calibration(
        parts.color_matrices.as_slice(),
        parts.white_balance.as_slice(),
        context,
        evidence,
        &mut findings,
        &mut selections,
        &mut conflicts,
    );

    findings.sort_by_key(|finding| (finding.field, finding.code, finding.source.clone()));
    selections.sort_by_key(|selection| (selection.field, selection.source.clone()));
    conflicts.sort_by_key(|conflict| (conflict.field, conflict.rejected.clone()));
    let mut fallbacks: Vec<_> = findings
        .iter()
        .filter(|finding| finding.code != RawMetadataFindingCode::Conflict)
        .filter_map(|finding| {
            selections
                .iter()
                .filter(|selection| {
                    selection.field == finding.field
                        && selection.source.source.precedence() < finding.source.source.precedence()
                })
                .max_by_key(|selection| {
                    (
                        selection.source.source.precedence(),
                        selection.source.source_id.as_str(),
                    )
                })
                .map(|selection| RawMetadataFallback {
                    field: finding.field,
                    selected: selection.source.clone(),
                    invalid: finding.source.clone(),
                })
        })
        .collect();
    fallbacks.sort_by_key(|fallback| (fallback.field, fallback.invalid.clone()));
    fallbacks.dedup();

    let normalized = RawMetadataRecord {
        schema_version: RAW_METADATA_SCHEMA_VERSION,
        camera: identity,
        geometry,
        calibration,
        findings,
    };
    let normalized_sha256 = normalized.stable_hash();
    Ok(RawMetadataReceipt {
        normalized,
        sources,
        selections,
        conflicts,
        fallbacks,
        normalized_sha256,
    })
}

pub(crate) fn normalize_backend_metadata(
    frame: &RawFrame,
    profile: &RawCapabilityDescriptor,
    content_sha256: [u8; 32],
) -> Result<RawMetadataReceipt, RawMetadataError> {
    normalize_raw_metadata(
        frame,
        &RawMetadataContext {
            primary_source: RawMetadataProvenance::new(
                RawMetadataSourceKind::BackendProfile,
                profile.profile_id.clone(),
                super::super::manifest::rawler_capability_manifest().sha256,
            ),
            backend_profile_id: profile.profile_id.clone(),
            content_sha256,
        },
        &[],
    )
}

pub(crate) fn normalize_dng_metadata(
    frame: &RawFrame,
    content_sha256: [u8; 32],
) -> Result<RawMetadataReceipt, RawMetadataError> {
    normalize_raw_metadata(
        frame,
        &RawMetadataContext {
            primary_source: RawMetadataProvenance::new(
                RawMetadataSourceKind::DngStandardized,
                "dng-tags-v1",
                content_sha256,
            ),
            backend_profile_id: "rusttable.dng".to_owned(),
            content_sha256,
        },
        &[],
    )
}

fn validate_provenance(value: &RawMetadataProvenance) -> Result<(), RawMetadataError> {
    if value.source_id.is_empty()
        || value.source_id.len() > MAX_SOURCE_ID_BYTES
        || value.source_id.chars().any(char::is_control)
        || value.source_id.contains(['/', '\\'])
    {
        return Err(RawMetadataError::UnsafeSourceId);
    }
    Ok(())
}

fn normalize_identity(
    parts: &super::RawFrameParts,
    context: &RawMetadataContext,
    evidence: &[RawMetadataEvidence],
    findings: &mut Vec<RawMetadataFinding>,
    selections: &mut Vec<RawMetadataSelection>,
) -> RawCameraMetadata {
    let maker = nonempty(&parts.camera.maker);
    let model = nonempty(&parts.camera.model);
    let unique_model = evidence
        .iter()
        .filter_map(|item| {
            item.identity
                .unique_model
                .as_ref()
                .map(|value| (&item.provenance, value))
        })
        .max_by_key(|(source, _)| (source.source.precedence(), source.source_id.as_str()))
        .map(|(_, value)| value.clone());
    let variant = evidence
        .iter()
        .filter_map(|item| {
            item.identity
                .variant
                .as_ref()
                .map(|value| (&item.provenance, value))
        })
        .max_by_key(|(source, _)| (source.source.precedence(), source.source_id.as_str()))
        .map(|(_, value)| value.clone())
        .or_else(|| nonempty(&parts.camera.mode).filter(|mode| mode != "dng"));

    let key = reviewed_camera_key(
        maker.as_deref(),
        model.as_deref(),
        &parts.camera.normalized_maker,
        &parts.camera.normalized_model,
        variant,
        context.primary_source.source,
    );
    if key.is_none() {
        findings.push(RawMetadataFinding {
            field: RawMetadataField::CameraIdentity,
            code: RawMetadataFindingCode::UnknownCamera,
            source: context.primary_source.clone(),
        });
    }
    selections.push(RawMetadataSelection {
        field: RawMetadataField::CameraIdentity,
        source: context.primary_source.clone(),
        value_sha256: hash_value(&(maker.clone(), model.clone(), unique_model.clone())),
    });
    let sensor_key = key.clone().map(|camera| RawSensorKey {
        camera,
        layout: layout_name(&parts.planes[0].layout),
    });
    RawCameraMetadata {
        source: RawSourceCameraIdentity {
            maker,
            model,
            unique_model,
            backend_profile_id: nonempty(&context.backend_profile_id),
        },
        status: if key.is_some() {
            RawMetadataStatus::Complete
        } else {
            RawMetadataStatus::Unknown
        },
        key,
        sensor_key,
        provenance: vec![context.primary_source.clone()],
    }
}

fn reviewed_camera_key(
    maker: Option<&str>,
    model: Option<&str>,
    backend_maker: &str,
    backend_model: &str,
    variant: Option<String>,
    primary_kind: RawMetadataSourceKind,
) -> Option<RawCameraKey> {
    let maker = maker?.trim();
    let model = model?.trim();
    let fuji_maker = ["FUJIFILM", "FUJI PHOTO FILM CO., LTD."]
        .iter()
        .any(|alias| maker.eq_ignore_ascii_case(alias));
    let xpro2 = ["X-Pro2", "X-Pro 2", "XPRO2"]
        .iter()
        .any(|alias| model.eq_ignore_ascii_case(alias));
    if fuji_maker && xpro2 {
        return Some(RawCameraKey {
            maker: "FUJIFILM".to_owned(),
            model: "X-Pro2".to_owned(),
            variant,
        });
    }
    // Backend normalized names are accepted only when supplied by a reviewed capability profile.
    if primary_kind == RawMetadataSourceKind::BackendProfile
        && !backend_maker.is_empty()
        && !backend_model.is_empty()
    {
        return Some(RawCameraKey {
            maker: backend_maker.to_owned(),
            model: backend_model.to_owned(),
            variant,
        });
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub(super) fn choose_optional<'a, T, I, V>(
    field: RawMetadataField,
    base: Option<(&'a RawMetadataProvenance, T)>,
    values: I,
    valid: V,
    findings: &mut Vec<RawMetadataFinding>,
    selections: &mut Vec<RawMetadataSelection>,
    conflicts: &mut Vec<RawMetadataConflict>,
    invalid_code: RawMetadataFindingCode,
) -> Option<T>
where
    T: Clone + PartialEq + Serialize,
    I: IntoIterator<Item = (&'a RawMetadataProvenance, T)>,
    V: Fn(&T) -> bool,
{
    let mut candidates: Vec<_> = values.into_iter().collect();
    if let Some(value) = base {
        candidates.push(value);
    }
    candidates.sort_by_key(|(source, _)| (source.source.precedence(), source.source_id.as_str()));
    let mut selected: Option<(&RawMetadataProvenance, T)> = None;
    for (source, value) in candidates.into_iter().rev() {
        if !valid(&value) {
            findings.push(RawMetadataFinding {
                field,
                code: invalid_code,
                source: source.clone(),
            });
            continue;
        }
        if let Some((chosen_source, chosen)) = &selected {
            if chosen != &value {
                conflicts.push(RawMetadataConflict {
                    field,
                    selected: (*chosen_source).clone(),
                    rejected: source.clone(),
                    rejected_value_sha256: hash_value(&value),
                });
                findings.push(RawMetadataFinding {
                    field,
                    code: RawMetadataFindingCode::Conflict,
                    source: source.clone(),
                });
            }
        } else {
            selections.push(RawMetadataSelection {
                field,
                source: source.clone(),
                value_sha256: hash_value(&value),
            });
            selected = Some((source, value));
        }
    }
    selected.map(|(_, value)| value)
}

pub(super) fn section_status(
    findings: &[RawMetadataFinding],
    fields: &[RawMetadataField],
) -> RawMetadataStatus {
    if findings
        .iter()
        .any(|finding| fields.contains(&finding.field))
    {
        RawMetadataStatus::Partial
    } else {
        RawMetadataStatus::Complete
    }
}

fn layout_name(layout: &RawPlaneLayout) -> String {
    match layout {
        RawPlaneLayout::Mosaic(cfa) => format!("mosaic-{}x{}", cfa.width, cfa.height),
        RawPlaneLayout::Linear { channels } => format!("linear-{}", channels.len()),
    }
}

fn nonempty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.to_owned())
}

pub(super) fn hash_value<T: Serialize>(value: &T) -> [u8; 32] {
    let bytes =
        postcard::to_allocvec(value).expect("normalized RAW metadata value is serializable");
    Sha256::digest(bytes).into()
}
