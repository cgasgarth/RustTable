use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ProfileClass {
    Input,
    Display,
    Output,
    Working,
    Proof,
    Analysis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ProfileModel {
    Matrix,
    Lut,
    Named,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Pcs {
    XyzD50,
    LabD50,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ProfileParserVersion(u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileIdError {
    EmptyProfile,
    InvalidSize,
    InvalidParserVersion,
}

impl ProfileParserVersion {
    pub fn new(value: u16) -> Result<Self, ProfileIdError> {
        (value > 0)
            .then_some(Self(value))
            .ok_or(ProfileIdError::InvalidParserVersion)
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ProfileId {
    sha256: [u8; 32],
    size: u64,
    class: ProfileClass,
    model: ProfileModel,
    pcs: Pcs,
    parser_version: ProfileParserVersion,
}

impl ProfileId {
    /// Creates a content-addressed profile identity without retaining profile bytes.
    pub fn from_content(
        bytes: &[u8],
        class: ProfileClass,
        model: ProfileModel,
        pcs: Pcs,
        parser_version: ProfileParserVersion,
    ) -> Result<Self, ProfileIdError> {
        if bytes.is_empty() {
            return Err(ProfileIdError::EmptyProfile);
        }
        Self::new(
            Sha256::digest(bytes).into(),
            u64::try_from(bytes.len()).map_err(|_| ProfileIdError::InvalidSize)?,
            class,
            model,
            pcs,
            parser_version,
        )
    }

    pub fn new(
        sha256: [u8; 32],
        size: u64,
        class: ProfileClass,
        model: ProfileModel,
        pcs: Pcs,
        parser_version: ProfileParserVersion,
    ) -> Result<Self, ProfileIdError> {
        if size == 0 || sha256 == [0; 32] {
            return Err(ProfileIdError::InvalidSize);
        }
        Ok(Self {
            sha256,
            size,
            class,
            model,
            pcs,
            parser_version,
        })
    }

    #[must_use]
    pub const fn sha256(self) -> [u8; 32] {
        self.sha256
    }

    #[must_use]
    pub const fn size(self) -> u64 {
        self.size
    }

    #[must_use]
    pub const fn class(self) -> ProfileClass {
        self.class
    }

    #[must_use]
    pub const fn model(self) -> ProfileModel {
        self.model
    }

    #[must_use]
    pub const fn pcs(self) -> Pcs {
        self.pcs
    }

    #[must_use]
    pub const fn parser_version(self) -> ProfileParserVersion {
        self.parser_version
    }

    #[must_use]
    pub fn digest_hex(self) -> String {
        let mut digest = String::with_capacity(64);
        for byte in self.sha256 {
            let _ = write!(digest, "{byte:02x}");
        }
        digest
    }
}

impl fmt::Display for ProfileId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{}:{}",
            self.digest_hex(),
            self.size,
            self.parser_version.0
        )
    }
}

impl fmt::Display for ProfileIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyProfile => "profile content must not be empty",
            Self::InvalidSize => "profile size and digest must be nonzero",
            Self::InvalidParserVersion => "profile parser version must be nonzero",
        })
    }
}

impl std::error::Error for ProfileIdError {}
