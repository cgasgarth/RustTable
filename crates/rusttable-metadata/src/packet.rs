use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use crate::policy::{
    CanonicalMetadataPolicy, MetadataAction, MetadataBuildError, MetadataCategory,
    MetadataProperty, MetadataSource, MetadataValue,
};
use crate::views::{FormatView, FormatViewKind, build_views};

const MAX_PROPERTIES: usize = 4096;
const MAX_VALUE_BYTES: usize = 1 << 20;
const MAX_PACKET_BYTES: usize = 16 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataPacket {
    pub(crate) properties: Vec<MetadataProperty>,
    pub(crate) views: BTreeMap<FormatViewKind, FormatView>,
    pub(crate) policy: CanonicalMetadataPolicy,
    pub(crate) canonical_hash: [u8; 32],
}

impl MetadataPacket {
    #[must_use]
    pub fn properties(&self) -> &[MetadataProperty] {
        &self.properties
    }
    #[must_use]
    pub const fn policy(&self) -> CanonicalMetadataPolicy {
        self.policy
    }
    #[must_use]
    pub const fn canonical_hash(&self) -> [u8; 32] {
        self.canonical_hash
    }
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        encode_properties(&self.properties)
    }
    #[must_use]
    pub fn view_bytes(&self, kind: FormatViewKind) -> Option<&[u8]> {
        self.views.get(&kind).map(|view| view.bytes.as_slice())
    }
    #[must_use]
    pub fn property_names(&self) -> BTreeSet<String> {
        self.properties
            .iter()
            .map(|value| format!("{}:{}", value.namespace, value.name))
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct MetadataPacketBuilder {
    policy: CanonicalMetadataPolicy,
    candidates: Vec<MetadataProperty>,
    clears: BTreeMap<(String, String), MetadataSource>,
}

impl MetadataPacketBuilder {
    #[must_use]
    pub fn new(policy: CanonicalMetadataPolicy) -> Self {
        Self {
            policy,
            candidates: Vec::new(),
            clears: BTreeMap::new(),
        }
    }
    #[must_use]
    pub fn policy(mut self, policy: CanonicalMetadataPolicy) -> Self {
        self.policy = policy;
        self
    }
    #[must_use]
    pub fn add_property(mut self, property: MetadataProperty) -> Self {
        self.candidates.push(property);
        self
    }
    #[must_use]
    pub fn clear(
        mut self,
        namespace: impl Into<String>,
        name: impl Into<String>,
        source: MetadataSource,
    ) -> Self {
        self.clears.insert((namespace.into(), name.into()), source);
        self
    }

    /// # Errors
    ///
    /// Returns a typed error when a property is malformed or the packet exceeds a bound.
    pub fn build(self) -> Result<MetadataPacket, MetadataBuildError> {
        if self.candidates.len() > MAX_PROPERTIES {
            return Err(MetadataBuildError::TooManyProperties);
        }
        let mut selected: BTreeMap<(String, String), MetadataProperty> = BTreeMap::new();
        for mut property in self.candidates {
            normalize(&mut property)?;
            let key = (property.namespace.clone(), property.name.clone());
            let cleared = self
                .clears
                .get(&key)
                .is_some_and(|source| source.precedence() >= property.source.precedence());
            if cleared {
                continue;
            }
            let replace = selected
                .get(&key)
                .is_none_or(|current| property.source.precedence() >= current.source.precedence());
            if replace {
                selected.insert(key, property);
            }
        }
        let mut properties = Vec::new();
        for mut property in selected.into_values() {
            match self.policy.action(property.category) {
                MetadataAction::Include => properties.push(property),
                MetadataAction::Exclude => {}
                MetadataAction::Redact => {
                    if let Some(property) = redact(&mut property) {
                        properties.push(property?);
                    }
                }
            }
        }
        let orientation = MetadataProperty::new(
            "exif",
            "Orientation",
            MetadataCategory::Technical,
            MetadataSource::GeneratedTechnical,
            MetadataValue::Integer(1),
        );
        properties.retain(|property| property.name != "Orientation");
        properties.push(orientation);
        properties.sort_by(|left, right| {
            (left.namespace.as_str(), left.name.as_str())
                .cmp(&(right.namespace.as_str(), right.name.as_str()))
        });
        let bytes = encode_properties(&properties);
        if bytes.len() > MAX_PACKET_BYTES {
            return Err(MetadataBuildError::TooManyBytes);
        }
        let canonical_hash = Sha256::digest(&bytes).into();
        let views = build_views(&properties);
        Ok(MetadataPacket {
            properties,
            views,
            policy: self.policy,
            canonical_hash,
        })
    }
}

fn normalize(property: &mut MetadataProperty) -> Result<(), MetadataBuildError> {
    if property.namespace.trim().is_empty() || property.name.trim().is_empty() {
        return Err(MetadataBuildError::EmptyName);
    }
    property.namespace = property
        .namespace
        .nfc()
        .map(|(character, _)| character)
        .collect();
    property.name = property
        .name
        .nfc()
        .map(|(character, _)| character)
        .collect();
    normalize_value(&mut property.value, &property.name)
}

fn normalize_value(value: &mut MetadataValue, name: &str) -> Result<(), MetadataBuildError> {
    match value {
        MetadataValue::Text(text) | MetadataValue::DateTime(text) => {
            *text = text.nfc().map(|(character, _)| character).collect();
            if text.len() > MAX_VALUE_BYTES || text.contains('\0') {
                return Err(MetadataBuildError::ValueLimit {
                    property: name.to_owned(),
                });
            }
        }
        MetadataValue::Binary(bytes) if bytes.len() > MAX_VALUE_BYTES => {
            return Err(MetadataBuildError::ValueLimit {
                property: name.to_owned(),
            });
        }
        MetadataValue::Array(values) => {
            for value in values {
                normalize_value(value, name)?;
            }
        }
        MetadataValue::Rational { denominator, .. } if *denominator == 0 => {
            return Err(MetadataBuildError::InvalidRational {
                property: name.to_owned(),
            });
        }
        MetadataValue::SignedRational { denominator, .. } if *denominator == 0 => {
            return Err(MetadataBuildError::InvalidRational {
                property: name.to_owned(),
            });
        }
        MetadataValue::Float(bits) if !f64::from_bits(*bits).is_finite() => {
            return Err(MetadataBuildError::NonFinite {
                property: name.to_owned(),
            });
        }
        _ => {}
    }
    Ok(())
}

fn redact(property: &mut MetadataProperty) -> Option<Result<MetadataProperty, MetadataBuildError>> {
    match property.category {
        MetadataCategory::GpsLocation
        | MetadataCategory::PeopleRegions
        | MetadataCategory::SourceIdentity
        | MetadataCategory::EditHistory
        | MetadataCategory::Thumbnail
        | MetadataCategory::UnknownImported => None,
        _ => {
            property.value = MetadataValue::Text("[redacted]".to_owned());
            Some(Ok(property.clone()))
        }
    }
}

fn encode_properties(properties: &[MetadataProperty]) -> Vec<u8> {
    let mut bytes = b"RustTable-MetadataPacket/1\0".to_vec();
    for property in properties {
        bytes.extend_from_slice(property.namespace.as_bytes());
        bytes.push(b'\0');
        bytes.extend_from_slice(property.name.as_bytes());
        bytes.push(b'\0');
        bytes.push(property.category as u8);
        bytes.push(property.source as u8);
        encode_value(&mut bytes, &property.value);
    }
    bytes
}

fn encode_value(bytes: &mut Vec<u8>, value: &MetadataValue) {
    match value {
        MetadataValue::Text(value) | MetadataValue::DateTime(value) => {
            bytes.push(b't');
            bytes.extend_from_slice(value.as_bytes());
        }
        MetadataValue::Integer(value) => {
            bytes.push(b'i');
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        MetadataValue::Rational {
            numerator,
            denominator,
        } => {
            bytes.push(b'r');
            bytes.extend_from_slice(&numerator.to_be_bytes());
            bytes.extend_from_slice(&denominator.to_be_bytes());
        }
        MetadataValue::SignedRational {
            numerator,
            denominator,
        } => {
            bytes.push(b's');
            bytes.extend_from_slice(&numerator.to_be_bytes());
            bytes.extend_from_slice(&denominator.to_be_bytes());
        }
        MetadataValue::Float(value) => {
            bytes.push(b'f');
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        MetadataValue::Binary(value) => {
            bytes.push(b'b');
            bytes.extend_from_slice(value);
        }
        MetadataValue::Array(values) => {
            bytes.push(b'a');
            for value in values {
                encode_value(bytes, value);
            }
        }
        MetadataValue::Region {
            x,
            y,
            width,
            height,
        } => {
            bytes.push(b'g');
            bytes.extend_from_slice(&x.to_be_bytes());
            bytes.extend_from_slice(&y.to_be_bytes());
            bytes.extend_from_slice(&width.to_be_bytes());
            bytes.extend_from_slice(&height.to_be_bytes());
        }
    }
    bytes.push(0);
}
