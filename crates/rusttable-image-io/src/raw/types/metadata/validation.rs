use super::super::{
    RawCfa, RawColorMatrix, RawDimensions, RawIlluminant, RawLevelPattern, RawOrientation,
    RawPlaneLayout, RawRect,
};
use super::{
    RawCalibrationMatrix, RawCalibrationMatrixKind, RawCalibrationMetadata, RawMetadataConflict,
    RawMetadataContext, RawMetadataEvidence, RawMetadataField, RawMetadataFinding,
    RawMetadataFindingCode, RawMetadataProvenance, RawMetadataSelection, RawMetadataStatus,
    RawNoiseProfile, choose_optional, hash_value, section_status,
};

#[allow(clippy::too_many_lines)]
pub(super) fn normalize_calibration(
    frame_matrices: &[RawColorMatrix],
    frame_white_balance: &[Option<f32>],
    context: &RawMetadataContext,
    evidence: &[RawMetadataEvidence],
    findings: &mut Vec<RawMetadataFinding>,
    selections: &mut Vec<RawMetadataSelection>,
    conflicts: &mut Vec<RawMetadataConflict>,
) -> RawCalibrationMetadata {
    let mut matrices: Vec<_> = frame_matrices
        .iter()
        .map(|value| RawCalibrationMatrix {
            kind: RawCalibrationMatrixKind::CameraToXyz,
            illuminant: value.illuminant,
            rows: value.rows,
            columns: value.columns,
            coefficients: value.coefficients.clone(),
            status: RawMetadataStatus::Complete,
            provenance: context.primary_source.clone(),
        })
        .chain(
            evidence
                .iter()
                .flat_map(|item| item.calibration.matrices.iter().cloned()),
        )
        .collect();
    matrices.sort_by_key(|matrix| {
        (
            matrix.kind,
            matrix.illuminant,
            matrix.provenance.source.precedence(),
            matrix.provenance.source_id.clone(),
        )
    });
    for matrix in &mut matrices {
        let codes = matrix_findings(matrix);
        matrix.status = if codes.is_empty() {
            RawMetadataStatus::Complete
        } else {
            RawMetadataStatus::Invalid
        };
        for code in codes {
            findings.push(RawMetadataFinding {
                field: RawMetadataField::ColorMatrix,
                code,
                source: matrix.provenance.clone(),
            });
        }
    }
    let mut matrix_groups = std::collections::BTreeMap::new();
    for matrix in &matrices {
        if matrix.status != RawMetadataStatus::Complete {
            continue;
        }
        let key = (matrix.kind, matrix.illuminant);
        let replace = matrix_groups
            .get(&key)
            .is_none_or(|current: &&RawCalibrationMatrix| {
                matrix.provenance.source.precedence() > current.provenance.source.precedence()
            });
        if replace {
            matrix_groups.insert(key, matrix);
        }
    }
    for matrix in matrix_groups.values() {
        selections.push(RawMetadataSelection {
            field: RawMetadataField::ColorMatrix,
            source: matrix.provenance.clone(),
            value_sha256: hash_value(&(
                matrix.kind,
                matrix.illuminant,
                matrix.rows,
                matrix.columns,
                &matrix.coefficients,
            )),
        });
        for rejected in matrices.iter().filter(|candidate| {
            candidate.status == RawMetadataStatus::Complete
                && candidate.kind == matrix.kind
                && candidate.illuminant == matrix.illuminant
                && candidate.provenance != matrix.provenance
                && candidate.coefficients != matrix.coefficients
        }) {
            conflicts.push(RawMetadataConflict {
                field: RawMetadataField::ColorMatrix,
                selected: matrix.provenance.clone(),
                rejected: rejected.provenance.clone(),
                rejected_value_sha256: hash_value(&rejected.coefficients),
            });
            findings.push(RawMetadataFinding {
                field: RawMetadataField::ColorMatrix,
                code: RawMetadataFindingCode::Conflict,
                source: rejected.provenance.clone(),
            });
        }
    }
    if matrix_groups.is_empty() {
        findings.push(RawMetadataFinding {
            field: RawMetadataField::ColorMatrix,
            code: RawMetadataFindingCode::MissingCalibration,
            source: context.primary_source.clone(),
        });
    }

    let analog_balance = choose_optional(
        RawMetadataField::AnalogBalance,
        None,
        evidence.iter().filter_map(|item| {
            item.calibration
                .analog_balance
                .clone()
                .map(|value| (&item.provenance, value))
        }),
        |values| valid_positive_vector(values),
        findings,
        selections,
        conflicts,
        RawMetadataFindingCode::InvalidWhiteBalance,
    );
    let as_shot_neutral = choose_optional(
        RawMetadataField::AsShotNeutral,
        None,
        evidence.iter().filter_map(|item| {
            item.calibration
                .as_shot_neutral
                .clone()
                .map(|value| (&item.provenance, value))
        }),
        |values| valid_positive_vector(values),
        findings,
        selections,
        conflicts,
        RawMetadataFindingCode::InvalidWhiteBalance,
    );
    let baseline_exposure = choose_optional(
        RawMetadataField::BaselineExposure,
        None,
        evidence.iter().filter_map(|item| {
            item.calibration
                .baseline_exposure
                .map(|value| (&item.provenance, value))
        }),
        |value| value.is_finite(),
        findings,
        selections,
        conflicts,
        RawMetadataFindingCode::Conflict,
    );
    let noise_profile = choose_optional(
        RawMetadataField::NoiseProfile,
        None,
        evidence.iter().filter_map(|item| {
            item.calibration
                .noise_profile
                .clone()
                .map(|value| (&item.provenance, value))
        }),
        valid_noise_profile,
        findings,
        selections,
        conflicts,
        RawMetadataFindingCode::InvalidNoiseProfile,
    );
    let white_balance = choose_optional(
        RawMetadataField::WhiteBalance,
        Some((&context.primary_source, frame_white_balance.to_vec())),
        evidence.iter().filter_map(|item| {
            item.calibration
                .white_balance
                .clone()
                .map(|value| (&item.provenance, value))
        }),
        |values| {
            !values.is_empty()
                && values.len() <= 4
                && values
                    .iter()
                    .flatten()
                    .all(|value| value.is_finite() && *value > 0.0)
        },
        findings,
        selections,
        conflicts,
        RawMetadataFindingCode::InvalidWhiteBalance,
    )
    .unwrap_or_default();

    let mut provenance: Vec<_> = evidence
        .iter()
        .map(|item| item.provenance.clone())
        .chain([context.primary_source.clone()])
        .collect();
    provenance.sort();
    provenance.dedup();
    RawCalibrationMetadata {
        matrices,
        analog_balance,
        as_shot_neutral,
        white_balance,
        baseline_exposure,
        noise_profile,
        status: section_status(
            findings,
            &[
                RawMetadataField::ColorMatrix,
                RawMetadataField::AnalogBalance,
                RawMetadataField::AsShotNeutral,
                RawMetadataField::WhiteBalance,
                RawMetadataField::NoiseProfile,
            ],
        ),
        provenance,
    }
}

fn matrix_findings(matrix: &RawCalibrationMatrix) -> Vec<RawMetadataFindingCode> {
    let mut output = Vec::new();
    let expected = usize::from(matrix.rows) * usize::from(matrix.columns);
    if matrix.rows < 3 || matrix.columns != 3 || expected != matrix.coefficients.len() {
        output.push(RawMetadataFindingCode::InvalidMatrixShape);
        return output;
    }
    if matrix.coefficients.iter().any(|value| !value.is_finite()) {
        output.push(RawMetadataFindingCode::NonFiniteMatrix);
        return output;
    }
    if matrix.illuminant == RawIlluminant::Unknown {
        output.push(RawMetadataFindingCode::MissingIlluminant);
    }
    let (rank, condition) = matrix_rank_condition(matrix);
    if rank < 3 {
        output.push(RawMetadataFindingCode::SingularMatrix);
    } else if condition > 1_000_000.0 {
        output.push(RawMetadataFindingCode::IllConditionedMatrix);
    }
    output
}

fn matrix_rank_condition(matrix: &RawCalibrationMatrix) -> (u8, f64) {
    let mut rows: Vec<[f64; 3]> = matrix
        .coefficients
        .as_chunks::<3>()
        .0
        .iter()
        .map(|row| [f64::from(row[0]), f64::from(row[1]), f64::from(row[2])])
        .collect();
    let scale = rows
        .iter()
        .flatten()
        .copied()
        .map(f64::abs)
        .fold(0.0, f64::max);
    if scale == 0.0 {
        return (0, f64::INFINITY);
    }
    let tolerance = scale * 1.0e-7;
    let mut rank = 0_usize;
    let mut pivots = Vec::new();
    for column in 0..3 {
        let Some((relative, pivot_value)) = rows[rank..]
            .iter()
            .enumerate()
            .map(|(index, row)| (index, row[column].abs()))
            .max_by(|left, right| left.1.total_cmp(&right.1))
        else {
            break;
        };
        if pivot_value <= tolerance {
            continue;
        }
        rows.swap(rank, rank + relative);
        let pivot = rows[rank][column];
        pivots.push(pivot.abs());
        let pivot_row = rows[rank];
        for row in rows.iter_mut().skip(rank + 1) {
            let factor = row[column] / pivot;
            for index in column..3 {
                row[index] -= factor * pivot_row[index];
            }
        }
        rank += 1;
        if rank == 3 {
            break;
        }
    }
    let minimum = pivots.iter().copied().fold(f64::INFINITY, f64::min);
    let maximum = pivots.iter().copied().fold(0.0, f64::max);
    (u8::try_from(rank).unwrap_or(3), maximum / minimum)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_geometry(
    dimensions: RawDimensions,
    active: RawRect,
    crop: RawRect,
    masked: &[RawRect],
    black: &RawLevelPattern,
    white: &[f32],
    source: &RawMetadataProvenance,
    findings: &mut Vec<RawMetadataFinding>,
) {
    if !rect_contains(
        RawRect {
            x: 0,
            y: 0,
            width: dimensions.width,
            height: dimensions.height,
        },
        active,
    ) || !rect_contains(active, crop)
    {
        findings.push(RawMetadataFinding {
            field: RawMetadataField::DefaultCrop,
            code: RawMetadataFindingCode::InvalidCrop,
            source: source.clone(),
        });
    }
    for area in masked {
        if !rect_contains(
            RawRect {
                x: 0,
                y: 0,
                width: dimensions.width,
                height: dimensions.height,
            },
            *area,
        ) {
            findings.push(RawMetadataFinding {
                field: RawMetadataField::MaskedAreas,
                code: RawMetadataFindingCode::InvalidMaskedArea,
                source: source.clone(),
            });
        }
    }
    let minimum_white = white.iter().copied().fold(f32::INFINITY, f32::min);
    if white.is_empty()
        || white
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        || black
            .values
            .iter()
            .any(|value| !value.is_finite() || *value >= minimum_white)
    {
        findings.push(RawMetadataFinding {
            field: RawMetadataField::WhiteLevels,
            code: RawMetadataFindingCode::InvalidLevels,
            source: source.clone(),
        });
    }
}

pub(super) fn normalize_cfa_phase(
    layout: &mut RawPlaneLayout,
    active: RawRect,
    crop: RawRect,
    orientation: RawOrientation,
) {
    let RawPlaneLayout::Mosaic(RawCfa {
        width,
        height,
        phase_x,
        phase_y,
        pattern,
    }) = layout
    else {
        return;
    };
    let dx = u8::try_from((crop.x - active.x) % u32::from(*width)).unwrap_or_default();
    let dy = u8::try_from((crop.y - active.y) % u32::from(*height)).unwrap_or_default();
    let source_width = *width;
    let source_height = *height;
    let crop_phase_x = (*phase_x + dx) % source_width;
    let crop_phase_y = (*phase_y + dy) % source_height;
    let shifted: Vec<_> = (0..source_height)
        .flat_map(|y| {
            let pattern = &*pattern;
            (0..source_width).map(move |x| {
                let source_x = (x + crop_phase_x) % source_width;
                let source_y = (y + crop_phase_y) % source_height;
                pattern[usize::from(source_y) * usize::from(source_width) + usize::from(source_x)]
            })
        })
        .collect();
    let swapped = matches!(
        orientation,
        RawOrientation::Transpose
            | RawOrientation::Rotate90
            | RawOrientation::Transverse
            | RawOrientation::Rotate270
    );
    let output_width = if swapped { source_height } else { source_width };
    let output_height = if swapped { source_width } else { source_height };
    *pattern = (0..output_height)
        .flat_map(|output_y| {
            let shifted = &shifted;
            (0..output_width).map(move |output_x| {
                let (source_x, source_y) = match orientation {
                    RawOrientation::HorizontalFlip => (source_width - 1 - output_x, output_y),
                    RawOrientation::Rotate180 => {
                        (source_width - 1 - output_x, source_height - 1 - output_y)
                    }
                    RawOrientation::VerticalFlip => (output_x, source_height - 1 - output_y),
                    RawOrientation::Transpose => (output_y, output_x),
                    RawOrientation::Rotate90 => (output_y, source_height - 1 - output_x),
                    RawOrientation::Transverse => {
                        (source_width - 1 - output_y, source_height - 1 - output_x)
                    }
                    RawOrientation::Rotate270 => (source_width - 1 - output_y, output_x),
                    RawOrientation::Normal | RawOrientation::Unknown => (output_x, output_y),
                };
                shifted[usize::from(source_y) * usize::from(source_width) + usize::from(source_x)]
            })
        })
        .collect();
    *width = output_width;
    *height = output_height;
    *phase_x = 0;
    *phase_y = 0;
}

pub(super) fn rect_contains(outer: RawRect, inner: RawRect) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.x.checked_add(inner.width) <= outer.x.checked_add(outer.width)
        && inner.y.checked_add(inner.height) <= outer.y.checked_add(outer.height)
}

fn valid_positive_vector(values: &[f32]) -> bool {
    !values.is_empty()
        && values.len() <= 4
        && values.iter().all(|value| value.is_finite() && *value > 0.0)
}

pub(super) fn valid_layout(value: &RawPlaneLayout) -> bool {
    match value {
        RawPlaneLayout::Mosaic(cfa) => cfa.validate(144).is_ok(),
        RawPlaneLayout::Linear { channels } => !channels.is_empty() && channels.len() <= 4,
    }
}

pub(super) fn valid_black_levels(value: &RawLevelPattern) -> bool {
    let expected = usize::from(value.repeat_width)
        .checked_mul(usize::from(value.repeat_height))
        .and_then(|cells| cells.checked_mul(usize::from(value.channels)));
    expected == Some(value.values.len())
        && !value.values.is_empty()
        && value
            .values
            .iter()
            .all(|level| level.is_finite() && *level >= 0.0)
}

fn valid_noise_profile(value: &RawNoiseProfile) -> bool {
    !value.shot_noise.is_empty()
        && value.shot_noise.len() == value.read_noise.len()
        && value.shot_noise.len() <= 4
        && value
            .shot_noise
            .iter()
            .chain(&value.read_noise)
            .all(|coefficient| coefficient.is_finite() && *coefficient >= 0.0)
}
