use super::{
    ArtifactKind, BlinkPlanes, DIFF_SCHEMA_VERSION, DiffArtifact, DiffError, DiffPolicy,
    ImageBuffer,
};

const BLINK_MAGIC: &[u8; 8] = b"RTBLKBLK";
const BLINK_SCHEMA_VERSION: u32 = 1;

pub(super) fn validate(artifact: &DiffArtifact) -> Result<(), DiffError> {
    if artifact.schema_version != DIFF_SCHEMA_VERSION {
        return Err(DiffError::Artifact(
            "unsupported artifact schema version".to_owned(),
        ));
    }
    let pixels = super::checked_pixel_count(artifact.width, artifact.height)?;
    match artifact.kind {
        ArtifactKind::HeatmapRgba8 => {
            let expected = pixels
                .checked_mul(4)
                .ok_or_else(|| DiffError::Artifact("heatmap length overflow".to_owned()))?;
            if artifact.bytes.len() != expected {
                return Err(DiffError::Artifact(
                    "heatmap is not a full-dimension RGBA8 plane".to_owned(),
                ));
            }
        }
        ArtifactKind::BlinkRgba32 => {
            let _ = blink_planes(artifact)?;
        }
    }
    Ok(())
}

pub(super) fn blink_planes(artifact: &DiffArtifact) -> Result<BlinkPlanes, DiffError> {
    if artifact.kind != ArtifactKind::BlinkRgba32 {
        return Err(DiffError::Artifact(
            "artifact is not a blink plane".to_owned(),
        ));
    }
    let header_len = BLINK_MAGIC.len() + 4 + 4 + 4 + 8 + 8;
    if artifact.bytes.len() < header_len || &artifact.bytes[..BLINK_MAGIC.len()] != BLINK_MAGIC {
        return Err(DiffError::Artifact("invalid blink manifest".to_owned()));
    }
    let mut offset = BLINK_MAGIC.len();
    let version = read_u32(&artifact.bytes, &mut offset)?;
    let width = read_u32(&artifact.bytes, &mut offset)?;
    let height = read_u32(&artifact.bytes, &mut offset)?;
    let source_len = usize::try_from(read_u64(&artifact.bytes, &mut offset)?)
        .map_err(|_| DiffError::Artifact("blink source length overflow".to_owned()))?;
    let reference_len = usize::try_from(read_u64(&artifact.bytes, &mut offset)?)
        .map_err(|_| DiffError::Artifact("blink reference length overflow".to_owned()))?;
    if version != BLINK_SCHEMA_VERSION || (width, height) != (artifact.width, artifact.height) {
        return Err(DiffError::Artifact(
            "invalid blink manifest metadata".to_owned(),
        ));
    }
    let plane_len = super::checked_pixel_count(width, height)?
        .checked_mul(4)
        .and_then(|length| length.checked_mul(4))
        .ok_or_else(|| DiffError::Artifact("blink plane length overflow".to_owned()))?;
    if source_len != plane_len || reference_len != plane_len {
        return Err(DiffError::Artifact("invalid blink plane length".to_owned()));
    }
    let end_source = offset
        .checked_add(source_len)
        .ok_or_else(|| DiffError::Artifact("blink length overflow".to_owned()))?;
    let end_reference = end_source
        .checked_add(reference_len)
        .ok_or_else(|| DiffError::Artifact("blink length overflow".to_owned()))?;
    if end_reference != artifact.bytes.len() {
        return Err(DiffError::Artifact(
            "blink has trailing or missing bytes".to_owned(),
        ));
    }
    Ok(BlinkPlanes {
        source: decode_f32_plane(&artifact.bytes[offset..end_source])?,
        reference: decode_f32_plane(&artifact.bytes[end_source..end_reference])?,
    })
}

pub(super) fn make_artifacts(
    source: &ImageBuffer,
    reference: &ImageBuffer,
    policy: &DiffPolicy,
    severities: &[f32],
) -> Result<Vec<DiffArtifact>, DiffError> {
    let pixel_count = source.pixel_count()?;
    if policy.include_heatmap && severities.len() != pixel_count {
        return Err(DiffError::Artifact(
            "severity plane length mismatch".to_owned(),
        ));
    }
    let mut artifacts = Vec::new();
    if policy.include_heatmap {
        let capacity = pixel_count
            .checked_mul(4)
            .ok_or_else(|| DiffError::Artifact("heatmap allocation overflow".to_owned()))?;
        let mut bytes = Vec::with_capacity(capacity);
        for severity in severities {
            let value = if severity.is_finite() {
                severity_to_byte(*severity)
            } else {
                255
            };
            bytes.extend_from_slice(&[value, 0, 0, 255]);
        }
        artifacts.push(DiffArtifact {
            schema_version: DIFF_SCHEMA_VERSION,
            kind: ArtifactKind::HeatmapRgba8,
            width: source.width,
            height: source.height,
            bytes,
        });
    }
    if policy.include_blink {
        let plane_len = source
            .pixels
            .len()
            .checked_mul(4)
            .ok_or_else(|| DiffError::Artifact("blink allocation overflow".to_owned()))?;
        let capacity = BLINK_MAGIC
            .len()
            .checked_add(4 + 4 + 4 + 8 + 8)
            .and_then(|header| header.checked_add(plane_len.checked_mul(2)?))
            .ok_or_else(|| DiffError::Artifact("blink allocation overflow".to_owned()))?;
        let mut bytes = Vec::with_capacity(capacity);
        bytes.extend_from_slice(BLINK_MAGIC);
        bytes.extend_from_slice(&BLINK_SCHEMA_VERSION.to_le_bytes());
        bytes.extend_from_slice(&source.width.to_le_bytes());
        bytes.extend_from_slice(&source.height.to_le_bytes());
        bytes.extend_from_slice(&(plane_len as u64).to_le_bytes());
        bytes.extend_from_slice(&(plane_len as u64).to_le_bytes());
        encode_f32_plane(&mut bytes, &source.pixels);
        encode_f32_plane(&mut bytes, &reference.pixels);
        artifacts.push(DiffArtifact {
            schema_version: DIFF_SCHEMA_VERSION,
            kind: ArtifactKind::BlinkRgba32,
            width: source.width,
            height: source.height,
            bytes,
        });
    }
    for artifact in &artifacts {
        validate(artifact)?;
    }
    Ok(artifacts)
}

fn encode_f32_plane(bytes: &mut Vec<u8>, plane: &[f32]) {
    for value in plane {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
}

fn decode_f32_plane(bytes: &[u8]) -> Result<Vec<f32>, DiffError> {
    if !bytes.len().is_multiple_of(4) {
        return Err(DiffError::Artifact(
            "float plane is not 32-bit aligned".to_owned(),
        ));
    }
    let (chunks, remainder) = bytes.as_chunks::<4>();
    if !remainder.is_empty() {
        return Err(DiffError::Artifact(
            "float plane is not 32-bit aligned".to_owned(),
        ));
    }
    Ok(chunks
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect())
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn severity_to_byte(severity: f32) -> u8 {
    (severity.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn read_u32(bytes: &[u8], offset: &mut usize) -> Result<u32, DiffError> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| DiffError::Artifact("manifest length overflow".to_owned()))?;
    let value = bytes
        .get(*offset..end)
        .ok_or_else(|| DiffError::Artifact("truncated blink manifest".to_owned()))?;
    *offset = end;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn read_u64(bytes: &[u8], offset: &mut usize) -> Result<u64, DiffError> {
    let end = offset
        .checked_add(8)
        .ok_or_else(|| DiffError::Artifact("manifest length overflow".to_owned()))?;
    let value = bytes
        .get(*offset..end)
        .ok_or_else(|| DiffError::Artifact("truncated blink manifest".to_owned()))?;
    *offset = end;
    Ok(u64::from_le_bytes([
        value[0], value[1], value[2], value[3], value[4], value[5], value[6], value[7],
    ]))
}
