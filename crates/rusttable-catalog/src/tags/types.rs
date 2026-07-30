use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use super::TagError;

pub const TAG_SCHEMA_VERSION: u16 = 1;
pub const TAG_HIERARCHY_SEPARATOR: char = '|';
pub const MAX_TAG_NAME_BYTES: usize = 256;
pub const MAX_TAG_ALIAS_BYTES: usize = 1_024;
pub const MAX_TAG_ALIASES: usize = 64;
pub const MAX_ASSIGNMENT_PHOTOS: usize = 4_096;
pub const MAX_ASSIGNMENT_TAGS: usize = 256;
pub const MAX_TAGS_PER_PHOTO: usize = 1_024;
pub(crate) const MAX_TAGS: usize = 65_536;
pub(crate) const MAX_TAGGED_PHOTOS: usize = 1_000_000;
pub(crate) const MAX_TAG_DEPTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TagId(u128);

impl TagId {
    #[must_use]
    pub const fn new(value: u128) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    #[must_use]
    pub const fn get(self) -> u128 {
        self.0
    }

    /// Derives a reproducible creation identity from a stable parent identity and canonical name.
    /// The resulting value remains the tag identity after later rename or reparent operations.
    #[must_use]
    pub fn deterministic(parent_id: Option<Self>, name: &TagName) -> Self {
        let mut digest = Sha256::new();
        digest.update(b"rusttable-tag-id-v1\0");
        digest.update(parent_id.map_or(0, Self::get).to_be_bytes());
        digest.update((name.canonical.len() as u64).to_be_bytes());
        digest.update(name.canonical.as_bytes());
        let bytes: [u8; 32] = digest.finalize().into();
        let mut id_bytes = [0_u8; 16];
        id_bytes.copy_from_slice(&bytes[..16]);
        let value = u128::from_be_bytes(id_bytes) | 1;
        Self(value)
    }
}

impl fmt::Display for TagId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:032x}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TagName {
    display: String,
    canonical: String,
}

impl TagName {
    /// Creates one bounded hierarchy segment with a Unicode-compatible lookup key.
    ///
    /// # Errors
    /// Returns a typed error for empty, oversized, control-containing, or hierarchical input.
    pub fn new(value: impl AsRef<str>) -> Result<Self, TagError> {
        let value = value.as_ref();
        if value.len() > MAX_TAG_NAME_BYTES {
            return Err(TagError::NameTooLong);
        }
        if value.chars().any(char::is_control) {
            return Err(TagError::InvalidControl);
        }
        if value.contains(TAG_HIERARCHY_SEPARATOR) {
            return Err(TagError::HierarchySeparator);
        }
        let display = compatible_text(value, false);
        if display.is_empty() {
            return Err(TagError::EmptyName);
        }
        let canonical = compatible_text(&display, true);
        Ok(Self { display, canonical })
    }

    #[must_use]
    pub fn display(&self) -> &str {
        &self.display
    }

    #[must_use]
    pub fn canonical(&self) -> &str {
        &self.canonical
    }

    pub(crate) fn validate(&self) -> Result<(), TagError> {
        if Self::new(&self.display)? == *self {
            Ok(())
        } else {
            Err(TagError::CorruptPersistedData)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TagAlias(String);

impl TagAlias {
    /// Creates a normalized alias, optionally containing a complete hierarchy path.
    ///
    /// # Errors
    /// Returns a typed error for empty segments, controls, or excessive input.
    pub fn new(value: impl AsRef<str>) -> Result<Self, TagError> {
        let value = value.as_ref();
        if value.len() > MAX_TAG_ALIAS_BYTES {
            return Err(TagError::AliasTooLong);
        }
        if value.chars().any(char::is_control) {
            return Err(TagError::InvalidControl);
        }
        let mut segments = Vec::new();
        for segment in value.split(TAG_HIERARCHY_SEPARATOR) {
            let normalized = compatible_text(segment, true);
            if normalized.is_empty() {
                return Err(TagError::EmptyPathSegment);
            }
            segments.push(normalized);
        }
        Ok(Self(segments.join("|")))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn validate(&self) -> Result<(), TagError> {
        if Self::new(&self.0)? == *self {
            Ok(())
        } else {
            Err(TagError::CorruptPersistedData)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagDefinition {
    id: TagId,
    parent_id: Option<TagId>,
    name: TagName,
    aliases: BTreeSet<TagAlias>,
}

impl TagDefinition {
    /// Creates a stable tag definition while canonicalizing alias order.
    ///
    /// # Errors
    /// Returns an error for an invalid identity or excessive/duplicate aliases.
    pub fn new(
        id: TagId,
        parent_id: Option<TagId>,
        name: TagName,
        aliases: impl IntoIterator<Item = TagAlias>,
    ) -> Result<Self, TagError> {
        if id.get() == 0 {
            return Err(TagError::InvalidTagId);
        }
        let aliases = aliases.into_iter().collect::<Vec<_>>();
        if aliases.len() > MAX_TAG_ALIASES {
            return Err(TagError::TooManyAliases);
        }
        let unique = aliases.iter().cloned().collect::<BTreeSet<_>>();
        if unique.len() != aliases.len() {
            return Err(TagError::DuplicateAlias);
        }
        Ok(Self {
            id,
            parent_id,
            name,
            aliases: unique,
        })
    }

    #[must_use]
    pub const fn id(&self) -> TagId {
        self.id
    }

    #[must_use]
    pub const fn parent_id(&self) -> Option<TagId> {
        self.parent_id
    }

    #[must_use]
    pub const fn name(&self) -> &TagName {
        &self.name
    }

    #[must_use]
    pub fn aliases(&self) -> impl ExactSizeIterator<Item = &TagAlias> {
        self.aliases.iter()
    }

    pub(crate) fn validate(&self) -> Result<(), TagError> {
        if self.id.get() == 0 {
            return Err(TagError::InvalidTagId);
        }
        self.name.validate()?;
        if self.aliases.len() > MAX_TAG_ALIASES {
            return Err(TagError::TooManyAliases);
        }
        for alias in &self.aliases {
            alias.validate()?;
        }
        Ok(())
    }
}

pub(crate) fn canonical_lookup(value: &str) -> Result<String, TagError> {
    TagAlias::new(value).map(|alias| alias.0)
}

fn compatible_text(value: &str, lowercase: bool) -> String {
    let compatible = value
        .nfkc()
        .map(|(character, _)| character)
        .collect::<String>();
    let collapsed = compatible.split_whitespace().collect::<Vec<_>>().join(" ");
    if lowercase {
        collapsed.to_lowercase()
    } else {
        collapsed
    }
}
