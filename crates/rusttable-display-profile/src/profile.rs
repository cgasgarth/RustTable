use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt, sync::Arc};

pub const MAX_PROFILE_BYTES: usize = 64 * 1024 * 1024;
pub const MIN_PROFILE_BYTES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DisplayProfileId([u8; 32]);

impl DisplayProfileId {
    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Display for DisplayProfileId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileMetadata {
    byte_length: u64,
    color_space: [u8; 4],
}

impl ProfileMetadata {
    #[must_use]
    pub const fn byte_length(self) -> u64 {
        self.byte_length
    }

    #[must_use]
    pub const fn color_space(self) -> [u8; 4] {
        self.color_space
    }
}

#[derive(Debug, Clone)]
pub struct StoredProfile {
    id: DisplayProfileId,
    metadata: ProfileMetadata,
    bytes: Arc<[u8]>,
}

impl StoredProfile {
    #[must_use]
    pub const fn id(&self) -> DisplayProfileId {
        self.id
    }

    #[must_use]
    pub const fn metadata(&self) -> ProfileMetadata {
        self.metadata
    }

    #[must_use]
    pub fn bytes(&self) -> Arc<[u8]> {
        Arc::clone(&self.bytes)
    }
}

#[derive(Debug, Default, Clone)]
pub struct ManagedProfileStore {
    profiles: BTreeMap<DisplayProfileId, StoredProfile>,
}

impl ManagedProfileStore {
    /// Validates and stores an owned immutable copy of profile bytes.
    ///
    /// # Errors
    ///
    /// Returns a typed error for oversized, truncated, malformed, or unsupported ICC data.
    pub fn insert(&mut self, bytes: &[u8]) -> Result<StoredProfile, IccProfileError> {
        let metadata = validate_icc(bytes)?;
        let id = DisplayProfileId(Sha256::digest(bytes).into());
        let profile = self.profiles.entry(id).or_insert_with(|| StoredProfile {
            id,
            metadata,
            bytes: Arc::from(bytes.to_owned().into_boxed_slice()),
        });
        Ok(profile.clone())
    }

    #[must_use]
    pub fn get(&self, id: DisplayProfileId) -> Option<StoredProfile> {
        self.profiles.get(&id).cloned()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IccProfileError {
    Empty,
    Oversized,
    Truncated,
    InvalidHeader,
    UnsupportedDeviceLink,
    UnsupportedColorSpace,
}

impl fmt::Display for IccProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "ICC profile is empty",
            Self::Oversized => "ICC profile exceeds the 64 MiB acquisition limit",
            Self::Truncated => "ICC profile is shorter than its declared size",
            Self::InvalidHeader => "ICC profile header is malformed",
            Self::UnsupportedDeviceLink => "ICC device-link profiles are not display profiles",
            Self::UnsupportedColorSpace => "ICC profile does not describe an RGB display space",
        })
    }
}

impl std::error::Error for IccProfileError {}

fn validate_icc(bytes: &[u8]) -> Result<ProfileMetadata, IccProfileError> {
    if bytes.is_empty() {
        return Err(IccProfileError::Empty);
    }
    if bytes.len() > MAX_PROFILE_BYTES {
        return Err(IccProfileError::Oversized);
    }
    if bytes.len() < MIN_PROFILE_BYTES {
        return Err(IccProfileError::InvalidHeader);
    }
    let declared_size = u32::from_be_bytes(bytes[0..4].try_into().expect("bounded ICC header"));
    let declared_size =
        usize::try_from(declared_size).expect("u32 fits usize on supported targets");
    if declared_size > bytes.len() || declared_size < MIN_PROFILE_BYTES {
        return Err(IccProfileError::Truncated);
    }
    if &bytes[36..40] != b"acsp" {
        return Err(IccProfileError::InvalidHeader);
    }
    if &bytes[12..16] == b"link" {
        return Err(IccProfileError::UnsupportedDeviceLink);
    }
    let color_space = bytes[16..20].try_into().expect("bounded ICC header");
    if color_space != *b"RGB " {
        return Err(IccProfileError::UnsupportedColorSpace);
    }
    Ok(ProfileMetadata {
        byte_length: bytes.len() as u64,
        color_space,
    })
}
