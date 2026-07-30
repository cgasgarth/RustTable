use std::panic::{AssertUnwindSafe, catch_unwind};

use rawler::decoders::FormatHint;
use rawler::rawimage::{RawImage, RawPhotometricInterpretation};
use rawler::rawsource::RawSource;

use super::{declared_sensor_bit_depth, map_backend_error, safe_text};
use crate::raw::{
    RawCapabilityError, RawCapabilityEvidence, RawCapabilityKind, RawCapabilityLayout,
    RawCapabilityResolveError, RawContainerProbe, RawDecodeError,
};

pub(super) fn backend_format_hint(
    source: &RawSource,
    probe: &RawContainerProbe,
) -> Result<FormatHint, RawDecodeError> {
    catch_unwind(AssertUnwindSafe(|| rawler::get_decoder(source)))
        .map_err(|_| RawDecodeError::Malformed {
            container: Some(probe.container),
            message: "backend decoder selection panicked".to_owned(),
        })?
        .map(|decoder| decoder.format_hint())
        .map_err(|error| map_backend_error(error, probe))
}

pub(super) fn validate_manifest_profile(
    image: &RawImage,
    probe: &RawContainerProbe,
    hint: FormatHint,
) -> Result<&'static crate::raw::RawCapabilityDescriptor, RawDecodeError> {
    let manifest = crate::raw::manifest::rawler_capability_manifest();
    let (profile, _used_alias) = manifest
        .resolve_backend(
            &image.camera.make,
            &image.camera.model,
            &image.camera.mode,
            probe.container,
        )
        .map_err(|error| manifest_error(image, probe, hint, error))?;
    if !profile.reviewed {
        return Err(manifest_error(
            image,
            probe,
            hint,
            RawCapabilityResolveError::Unsupported,
        ));
    }
    let actual_bits = declared_sensor_bit_depth(image, probe);
    if profile
        .bit_depth
        .is_some_and(|expected| actual_bits != Some(expected))
    {
        return Err(manifest_drift(
            image,
            probe,
            hint,
            &format!(
                "backend bit depth {actual_bits:?} disagrees with camera profile {:?}",
                profile.bit_depth
            ),
        ));
    }
    let is_xtrans = matches!(
        &image.photometric,
        RawPhotometricInterpretation::Cfa(config)
            if config.cfa.width == 6 && config.cfa.height == 6
    );
    if matches!(profile.layout, RawCapabilityLayout::BayerOrLinear) && is_xtrans {
        return Err(manifest_drift(
            image,
            probe,
            hint,
            "backend returned X-Trans data for a non-X-Trans camera profile",
        ));
    }
    Ok(profile)
}

fn manifest_error(
    image: &RawImage,
    probe: &RawContainerProbe,
    hint: FormatHint,
    error: RawCapabilityResolveError,
) -> RawDecodeError {
    let detail = match error {
        RawCapabilityResolveError::Unsupported => {
            "camera/container is not present in the reviewed rawler capability manifest"
        }
        RawCapabilityResolveError::Ambiguous { .. } => {
            "camera aliases resolve to multiple rawler capability profiles"
        }
    };
    let missing = match error {
        RawCapabilityResolveError::Unsupported => RawCapabilityKind::Camera,
        RawCapabilityResolveError::Ambiguous { .. } => RawCapabilityKind::ManifestDrift,
    };
    RawDecodeError::Capability(RawCapabilityError {
        missing,
        container: Some(probe.container),
        maker: safe_text(&image.make),
        model: safe_text(&image.model),
        mode: safe_text(&image.camera.mode),
        detail: format!("{detail}; backend format {hint:?}"),
        evidence: Box::new(capability_evidence(probe, hint, image)),
    })
}

fn manifest_drift(
    image: &RawImage,
    probe: &RawContainerProbe,
    hint: FormatHint,
    detail: &str,
) -> RawDecodeError {
    RawDecodeError::Capability(RawCapabilityError {
        missing: RawCapabilityKind::ManifestDrift,
        container: Some(probe.container),
        maker: safe_text(&image.make),
        model: safe_text(&image.model),
        mode: safe_text(&image.camera.mode),
        detail: safe_text(detail),
        evidence: Box::new(capability_evidence(probe, hint, image)),
    })
}

fn capability_evidence(
    probe: &RawContainerProbe,
    hint: FormatHint,
    image: &RawImage,
) -> RawCapabilityEvidence {
    RawCapabilityEvidence {
        signature: probe.evidence.signature.clone(),
        raw_tags: probe.evidence.raw_tags.clone(),
        backend_format: crate::raw::manifest::backend_format_name(hint),
        compression: probe.evidence.compression.clone(),
        bit_depth: probe
            .evidence
            .bit_depth
            .or_else(|| u8::try_from(image.bps).ok()),
    }
}
