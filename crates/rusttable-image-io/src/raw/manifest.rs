use std::sync::OnceLock;

use rawler::decoders::Camera;
use sha2::{Digest, Sha256};

use super::{
    RAWLER_BACKEND_ID, RawCapabilityDescriptor, RawCapabilityKey, RawCapabilityManifest,
    RawCompression, RawContainerKind,
};

/// Pinned camera/container rows generated from `rawler` 0.7.2.
pub const RAWLER_CAPABILITY_MANIFEST_CAMERA_COUNT: usize = 2_108;
/// Canonical SHA-256 of the pinned generated rows.
pub const RAWLER_CAPABILITY_MANIFEST_SHA256: [u8; 32] = [
    18, 207, 220, 255, 176, 171, 206, 249, 231, 104, 188, 117, 134, 11, 186, 131, 43, 151, 150,
    168, 70, 36, 96, 93, 147, 96, 116, 252, 181, 115, 50, 150,
];

static MANIFEST: OnceLock<RawCapabilityManifest> = OnceLock::new();

/// Returns the deterministic capability manifest generated from the pinned backend database.
#[must_use]
pub fn rawler_capability_manifest() -> &'static RawCapabilityManifest {
    MANIFEST.get_or_init(generate_manifest)
}

fn generate_manifest() -> RawCapabilityManifest {
    let mut entries = Vec::new();
    for camera in rawler::global_loader().get_cameras().values() {
        for container in containers_for(camera) {
            entries.push(descriptor(camera, container));
        }
    }
    entries.sort_by(|left, right| left.key.cmp(&right.key));
    entries.dedup_by(|left, right| left.key == right.key);
    let digest = manifest_digest(&entries);
    RawCapabilityManifest::generated(entries, digest)
}

fn descriptor(camera: &Camera, container: RawContainerKind) -> RawCapabilityDescriptor {
    let corpus_fixtures = if container == RawContainerKind::Raf
        && camera.clean_make.eq_ignore_ascii_case("Fujifilm")
        && camera.clean_model.eq_ignore_ascii_case("X-Pro2")
    {
        vec!["rusttable-testkit.raw.synthetic-compressed-raf".to_owned()]
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
        normalized_maker: bounded(&camera.clean_make),
        normalized_model: bounded(&camera.clean_model),
        bit_depth: camera
            .bps
            .or((camera.real_bps != 0).then_some(camera.real_bps))
            .and_then(|value| u8::try_from(value).ok()),
        corpus_fixtures,
    }
}

fn containers_for(camera: &Camera) -> Vec<RawContainerKind> {
    let maker = camera.make.to_ascii_uppercase();
    if maker.contains("FUJI") {
        vec![RawContainerKind::Raf]
    } else if maker.contains("CANON") {
        vec![
            RawContainerKind::Cr2,
            RawContainerKind::Cr3,
            RawContainerKind::Crw,
        ]
    } else if maker.contains("NIKON") {
        vec![RawContainerKind::Nef]
    } else if maker.contains("SONY") {
        vec![RawContainerKind::Arw]
    } else if maker.contains("OLYMPUS") || maker.contains("OM DIGITAL") {
        vec![RawContainerKind::Orf]
    } else if maker.contains("PANASONIC") || maker.contains("LEICA") {
        vec![RawContainerKind::Rw2]
    } else if maker.contains("PENTAX") || maker.contains("RICOH") {
        vec![RawContainerKind::Pef]
    } else if maker.contains("SAMSUNG") {
        vec![RawContainerKind::Srw]
    } else if maker.contains("EPSON") {
        vec![RawContainerKind::Erf]
    } else if maker.contains("PHASE ONE") || maker.contains("LEAF") {
        vec![RawContainerKind::Iiq]
    } else if maker.contains("SIGMA") {
        vec![RawContainerKind::X3f]
    } else if maker.contains("MINOLTA") {
        vec![RawContainerKind::Mrw]
    } else {
        vec![RawContainerKind::TiffRaw]
    }
}

fn manifest_digest(entries: &[RawCapabilityDescriptor]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.raw-capability-manifest.v1\0");
    hasher.update(RAWLER_BACKEND_ID.as_bytes());
    for entry in entries {
        hasher.update(b"\0entry\0");
        for value in [
            entry.key.maker.as_str(),
            entry.key.model.as_str(),
            entry.key.mode.as_str(),
            entry.key.cfa.as_str(),
            entry.normalized_maker.as_str(),
            entry.normalized_model.as_str(),
        ] {
            hasher.update(value.as_bytes());
            hasher.update(b"\0");
        }
        hasher.update(format!(
            "{:?}\0{:?}\0",
            entry.key.container, entry.key.compression
        ));
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
    fn generated_manifest_matches_pinned_backend_data() {
        let manifest = rawler_capability_manifest();
        assert_eq!(manifest.backend, RAWLER_BACKEND_ID);
        assert_eq!(
            manifest.entries().len(),
            RAWLER_CAPABILITY_MANIFEST_CAMERA_COUNT
        );
        assert_eq!(manifest.sha256, RAWLER_CAPABILITY_MANIFEST_SHA256);
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
