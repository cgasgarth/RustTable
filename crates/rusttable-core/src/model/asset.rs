use crate::AssetId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssetRole {
    Primary,
    Sidecar,
    Auxiliary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteLength(u64);

impl ByteLength {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn from_bytes(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgorithm {
    Sha256,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentHash {
    Sha256([u8; 32]),
}

impl ContentHash {
    #[must_use]
    pub const fn algorithm(self) -> HashAlgorithm {
        match self {
            Self::Sha256(_) => HashAlgorithm::Sha256,
        }
    }

    #[must_use]
    pub const fn bytes(&self) -> &[u8; 32] {
        match self {
            Self::Sha256(bytes) => bytes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asset {
    id: AssetId,
    role: AssetRole,
    content_hash: ContentHash,
    byte_length: ByteLength,
}

impl Asset {
    #[must_use]
    pub const fn new(
        id: AssetId,
        role: AssetRole,
        content_hash: ContentHash,
        byte_length: ByteLength,
    ) -> Self {
        Self {
            id,
            role,
            content_hash,
            byte_length,
        }
    }

    #[must_use]
    pub const fn id(&self) -> AssetId {
        self.id
    }

    #[must_use]
    pub const fn role(&self) -> AssetRole {
        self.role
    }

    #[must_use]
    pub const fn content_hash(&self) -> ContentHash {
        self.content_hash
    }

    #[must_use]
    pub const fn byte_length(&self) -> ByteLength {
        self.byte_length
    }
}
