use std::sync::OnceLock;

use rawler::decoders::{Camera, FormatHint};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{
    RAWLER_BACKEND_ID, RawCapabilityDescriptor, RawCapabilityKey, RawCapabilityLayout,
    RawCapabilityManifest, RawCompression, RawContainerKind, RawVendorFamily,
};

const FAMILY_SCHEMA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../architecture/raw-camera-capabilities.toml"
));

/// Number of camera/container rows expanded from the pinned backend and reviewed family policy.
pub const RAWLER_CAPABILITY_MANIFEST_CAMERA_COUNT: usize = 1_458;
/// Canonical SHA-256 of the pinned generated rows and family schema.
pub const RAWLER_CAPABILITY_MANIFEST_SHA256: [u8; 32] = [
    69, 161, 54, 13, 154, 216, 125, 2, 81, 66, 177, 67, 64, 90, 215, 87, 194, 249, 87, 203, 127,
    116, 123, 64, 218, 119, 81, 130, 102, 18, 111, 51,
];

static MANIFEST: OnceLock<RawCapabilityManifest> = OnceLock::new();

#[derive(Debug, Deserialize)]
struct CapabilityFile {
    schema: String,
    backend: String,
    reference: String,
    families: Vec<FamilyRule>,
}

#[derive(Debug, Deserialize)]
struct FamilyRule {
    id: String,
    maker_aliases: Vec<String>,
    containers: Vec<String>,
    backend_formats: Vec<String>,
    decoder_path: String,
    compression_modes: Vec<String>,
    layout: String,
    #[serde(default)]
    bit_depth: Option<u8>,
    fixture_ids: Vec<String>,
    reference_evidence: Vec<String>,
    #[serde(default)]
    quirk_ids: Vec<String>,
}

/// Returns the deterministic capability manifest generated from rawler plus reviewed family data.
#[must_use]
pub fn rawler_capability_manifest() -> &'static RawCapabilityManifest {
    MANIFEST.get_or_init(generate_manifest)
}

fn generate_manifest() -> RawCapabilityManifest {
    let file: CapabilityFile =
        toml::from_str(FAMILY_SCHEMA).expect("raw camera capability schema must be valid TOML");
    assert_eq!(file.schema, "rusttable.raw-camera-capabilities.v1");
    assert_eq!(file.backend, RAWLER_BACKEND_ID);
    let mut entries = Vec::new();
    for camera in rawler::global_loader().get_cameras().values() {
        if let Some(rule) = family_for_camera(&file.families, camera) {
            for container_name in &rule.containers {
                entries.push(descriptor(camera, rule, container_name));
            }
        } else {
            entries.push(unreviewed_descriptor(camera));
        }
    }
    entries.sort_by(|left, right| left.key.cmp(&right.key));
    entries.dedup_by(|left, right| left.key == right.key);
    let digest = manifest_digest(&file, &entries);
    RawCapabilityManifest::generated(entries, digest)
}

fn family_for_camera<'a>(families: &'a [FamilyRule], camera: &Camera) -> Option<&'a FamilyRule> {
    families.iter().find(|family| {
        family.maker_aliases.iter().any(|alias| {
            alias.eq_ignore_ascii_case(&camera.make)
                || alias.eq_ignore_ascii_case(&camera.clean_make)
                || camera
                    .make
                    .to_ascii_uppercase()
                    .contains(&alias.to_ascii_uppercase())
        })
    })
}

fn descriptor(camera: &Camera, rule: &FamilyRule, container_name: &str) -> RawCapabilityDescriptor {
    let container = parse_container(container_name).expect("capability schema container");
    let backend_format = backend_format(rule, container);
    let family = parse_family(&rule.id).expect("capability schema family");
    let profile_id = profile_id(family, &camera.make, &camera.model, &camera.mode, container);
    let corpus_fixtures = if family == RawVendorFamily::Fujifilm
        && camera.clean_model.eq_ignore_ascii_case("X-Pro2")
        && container == RawContainerKind::Raf
    {
        rule.fixture_ids.clone()
    } else {
        Vec::new()
    };
    RawCapabilityDescriptor {
        key: RawCapabilityKey {
            maker: bounded(&camera.make),
            model: bounded(&camera.model),
            mode: bounded(&camera.mode),
            container,
            compression: RawCompression::BackendDefined,
            cfa: bounded(&camera.cfa.name),
        },
        family,
        profile_id,
        backend_format,
        decoder_path: rule.decoder_path.clone(),
        layout: if rule.layout == "bayer-or-xtrans" {
            RawCapabilityLayout::BayerOrXTrans
        } else {
            RawCapabilityLayout::BayerOrLinear
        },
        quirk_ids: rule.quirk_ids.clone(),
        reference_evidence: rule.reference_evidence.clone(),
        reviewed: true,
        normalized_maker: bounded(&camera.clean_make),
        normalized_model: bounded(&camera.clean_model),
        bit_depth: rule.bit_depth,
        corpus_fixtures,
    }
}

fn unreviewed_descriptor(camera: &Camera) -> RawCapabilityDescriptor {
    let container = fallback_container(&camera.make);
    RawCapabilityDescriptor {
        key: RawCapabilityKey {
            maker: bounded(&camera.make),
            model: bounded(&camera.model),
            mode: bounded(&camera.mode),
            container,
            compression: RawCompression::BackendDefined,
            cfa: bounded(&camera.cfa.name),
        },
        family: RawVendorFamily::UnreviewedBackend,
        profile_id: profile_id(
            RawVendorFamily::UnreviewedBackend,
            &camera.make,
            &camera.model,
            &camera.mode,
            container,
        ),
        backend_format: "Unreviewed".to_owned(),
        decoder_path: "rawler::backend".to_owned(),
        layout: RawCapabilityLayout::BayerOrLinear,
        quirk_ids: Vec::new(),
        reference_evidence: Vec::new(),
        reviewed: false,
        normalized_maker: bounded(&camera.clean_make),
        normalized_model: bounded(&camera.clean_model),
        bit_depth: camera
            .bps
            .or((camera.real_bps != 0).then_some(camera.real_bps))
            .and_then(|value| u8::try_from(value).ok()),
        corpus_fixtures: Vec::new(),
    }
}

fn fallback_container(make: &str) -> RawContainerKind {
    let make = make.to_ascii_uppercase();
    if make.contains("SIGMA") {
        RawContainerKind::X3f
    } else if make.contains("MINOLTA") {
        RawContainerKind::Mrw
    } else {
        RawContainerKind::TiffRaw
    }
}

fn backend_format(rule: &FamilyRule, container: RawContainerKind) -> String {
    match container {
        RawContainerKind::Nef => "NEF",
        RawContainerKind::Nrw => "NRW",
        RawContainerKind::Cr2 => "CR2",
        RawContainerKind::Cr3 => "CR3",
        RawContainerKind::ThreeFr | RawContainerKind::Fff => "TFR",
        _ => rule
            .backend_formats
            .first()
            .map_or("UNKNOWN", String::as_str),
    }
    .to_owned()
}

fn parse_family(value: &str) -> Option<RawVendorFamily> {
    Some(match value {
        "canon" => RawVendorFamily::Canon,
        "nikon" => RawVendorFamily::Nikon,
        "sony" => RawVendorFamily::Sony,
        "fujifilm" => RawVendorFamily::Fujifilm,
        "olympus" => RawVendorFamily::Olympus,
        "panasonic-leica" => RawVendorFamily::PanasonicLeica,
        "pentax" => RawVendorFamily::Pentax,
        "samsung" => RawVendorFamily::Samsung,
        "hasselblad" => RawVendorFamily::Hasselblad,
        "phase-one" => RawVendorFamily::PhaseOne,
        "epson" => RawVendorFamily::Epson,
        _ => return None,
    })
}

fn parse_container(value: &str) -> Option<RawContainerKind> {
    Some(match value {
        "cr2" => RawContainerKind::Cr2,
        "cr3" => RawContainerKind::Cr3,
        "nef" => RawContainerKind::Nef,
        "nrw" => RawContainerKind::Nrw,
        "arw" => RawContainerKind::Arw,
        "sr2" => RawContainerKind::Sr2,
        "srf" => RawContainerKind::Srf,
        "raf" => RawContainerKind::Raf,
        "orf" => RawContainerKind::Orf,
        "rw2" => RawContainerKind::Rw2,
        "rwl" => RawContainerKind::Rwl,
        "pef" => RawContainerKind::Pef,
        "srw" => RawContainerKind::Srw,
        "3fr" => RawContainerKind::ThreeFr,
        "fff" => RawContainerKind::Fff,
        "iiq" => RawContainerKind::Iiq,
        "erf" => RawContainerKind::Erf,
        _ => return None,
    })
}

fn container_for_hint(hint: FormatHint) -> Option<RawContainerKind> {
    Some(match hint {
        FormatHint::CR2 => RawContainerKind::Cr2,
        FormatHint::CR3 => RawContainerKind::Cr3,
        FormatHint::NEF => RawContainerKind::Nef,
        FormatHint::NRW => RawContainerKind::Nrw,
        FormatHint::ARW => RawContainerKind::Arw,
        FormatHint::RAF => RawContainerKind::Raf,
        FormatHint::ORF => RawContainerKind::Orf,
        FormatHint::RW2 => RawContainerKind::Rw2,
        FormatHint::PEF => RawContainerKind::Pef,
        FormatHint::SRW => RawContainerKind::Srw,
        FormatHint::ERF => RawContainerKind::Erf,
        FormatHint::IIQ => RawContainerKind::Iiq,
        FormatHint::TFR => RawContainerKind::ThreeFr,
        _ => return None,
    })
}

pub(super) fn container_for_backend_hint(
    probe: RawContainerKind,
    hint: FormatHint,
) -> RawContainerKind {
    container_for_hint(hint).unwrap_or(probe)
}

pub(super) fn backend_format_name(hint: FormatHint) -> String {
    format!("{hint:?}")
}

fn profile_id(
    family: RawVendorFamily,
    maker: &str,
    model: &str,
    mode: &str,
    container: RawContainerKind,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.raw-camera-profile.v1\0");
    hasher.update(format!("{family:?}\0{maker}\0{model}\0{mode}\0{container:?}").as_bytes());
    let digest = hasher.finalize();
    format!(
        "{family:?}-{:02x}{:02x}{:02x}{:02x}",
        digest[0], digest[1], digest[2], digest[3]
    )
}

fn manifest_digest(file: &CapabilityFile, entries: &[RawCapabilityDescriptor]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.raw-capability-manifest.v2\0");
    hasher.update(file.reference.as_bytes());
    for family in &file.families {
        for value in [
            family.id.as_str(),
            family.decoder_path.as_str(),
            family.layout.as_str(),
        ] {
            hasher.update(value.as_bytes());
            hasher.update(b"\0");
        }
        for mode in &family.compression_modes {
            hasher.update(mode.as_bytes());
            hasher.update(b"\0");
        }
        hasher.update([family.bit_depth.unwrap_or_default()]);
    }
    for entry in entries {
        hasher.update(b"\0entry\0");
        for value in [
            entry.key.maker.as_str(),
            entry.key.model.as_str(),
            entry.key.mode.as_str(),
            entry.key.cfa.as_str(),
            entry.normalized_maker.as_str(),
            entry.normalized_model.as_str(),
            entry.profile_id.as_str(),
            entry.backend_format.as_str(),
            entry.decoder_path.as_str(),
        ] {
            hasher.update(value.as_bytes());
            hasher.update(b"\0");
        }
        hasher.update(format!(
            "{:?}\0{:?}\0{:?}",
            entry.family, entry.key.container, entry.layout
        ));
        hasher.update([u8::from(entry.reviewed)]);
        hasher.update([entry.bit_depth.unwrap_or_default()]);
        for fixture in &entry.corpus_fixtures {
            hasher.update(fixture.as_bytes());
            hasher.update(b"\0");
        }
    }
    hasher.finalize().into()
}

fn bounded(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_manifest_is_scoped_to_reviewed_vendor_families() {
        let manifest = rawler_capability_manifest();
        assert_eq!(manifest.backend, RAWLER_BACKEND_ID);
        let camera_rows = manifest
            .entries()
            .iter()
            .map(|entry| (&entry.key.maker, &entry.key.model, &entry.key.mode))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(camera_rows.len(), RAWLER_CAPABILITY_MANIFEST_CAMERA_COUNT);
        assert_eq!(
            camera_rows.len(),
            rawler::global_loader().get_cameras().len()
        );
        for camera in rawler::global_loader().get_cameras().values() {
            assert!(
                manifest.entries().iter().any(|entry| {
                    entry.key.maker == camera.make
                        && entry.key.model == camera.model
                        && entry.key.mode == camera.mode
                }),
                "backend camera row was dropped: {:?}",
                (&camera.make, &camera.model, &camera.mode)
            );
        }
        assert_eq!(manifest.sha256, RAWLER_CAPABILITY_MANIFEST_SHA256);
        for family in [
            RawVendorFamily::Canon,
            RawVendorFamily::Nikon,
            RawVendorFamily::Sony,
            RawVendorFamily::Fujifilm,
            RawVendorFamily::Olympus,
            RawVendorFamily::PanasonicLeica,
            RawVendorFamily::Pentax,
            RawVendorFamily::Samsung,
            RawVendorFamily::Hasselblad,
            RawVendorFamily::PhaseOne,
            RawVendorFamily::Epson,
        ] {
            assert!(
                manifest
                    .entries()
                    .iter()
                    .any(|entry| entry.family == family)
            );
        }
    }

    #[test]
    fn manifest_links_the_synthetic_raf_corpus_fixture() {
        assert!(rawler_capability_manifest().entries().iter().any(|entry| {
            entry
                .corpus_fixtures
                .iter()
                .any(|fixture| fixture == "rusttable-testkit.raw.synthetic-compressed-raf")
        }));
    }
}
