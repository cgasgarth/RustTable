use std::fmt;
use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use rusttable_core::ContentHash;

use crate::{ArtifactBuffer, DependencySnapshot, EncoderSettings};

pub(crate) fn artifact_hash(buffer: &ArtifactBuffer, metadata: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(buffer.dimensions().width().to_be_bytes());
    hasher.update(buffer.dimensions().height().to_be_bytes());
    hasher.update(buffer.stride().to_be_bytes());
    hasher.update(format!("{:?}", buffer.encoding()).as_bytes());
    hasher.update(buffer.bytes());
    hasher.update(metadata);
    hasher.finalize().into()
}

pub(crate) fn dependency_hash(snapshot: Option<&DependencySnapshot>) -> [u8; 32] {
    let mut hasher = Sha256::new();
    if let Some(snapshot) = snapshot {
        hasher.update(snapshot.catalog_revision().get().to_be_bytes());
        hasher.update(snapshot.edit_revision().get().to_be_bytes());
        if let Some(hash) = snapshot.style_hash() {
            hasher.update(hash.bytes());
        }
        if let Some(profile) = snapshot.profile() {
            hasher.update(profile.sha256());
            hasher.update(profile.size().to_be_bytes());
        }
        for asset in snapshot.assets() {
            hasher.update(asset.id().as_bytes());
            hasher.update(asset.content_hash().bytes());
        }
    }
    hasher.finalize().into()
}

pub(crate) fn hash_option(value: Option<ContentHash>) -> String {
    value.map_or_else(|| "none".to_owned(), |hash| hex(hash.bytes()))
}

pub(crate) fn opaque_string_hash(value: &str) -> String {
    hex(&Sha256::digest(value.as_bytes()).into())
}

pub(crate) fn encoder_settings_hash(settings: &EncoderSettings) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{:?}\n", settings.format()).as_bytes());
    for (name, value) in settings.parameters() {
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update(value.as_bytes());
        hasher.update([0]);
    }
    hex(&hasher.finalize().into())
}

pub(crate) fn hex(bytes: &[u8; 32]) -> String {
    let mut output = String::with_capacity(64);
    for byte in bytes {
        write!(output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

#[allow(dead_code)]
pub(crate) fn display_option<T: fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(|| "none".to_owned(), |value| value.to_string())
}
