use super::{ICC_PROFILE_PARSER_VERSION, IccHeader, IccSignature, Pcs, ProfileClass, ProfileModel};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fmt::Write as _;

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
        hex(&self.sha256)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IccByteIdentity {
    sha256: [u8; 32],
    size: u32,
}

impl IccByteIdentity {
    pub(crate) fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            sha256: Sha256::digest(bytes).into(),
            size: u32::try_from(bytes.len()).expect("validated ICC size fits u32"),
        }
    }

    #[must_use]
    pub const fn sha256(self) -> [u8; 32] {
        self.sha256
    }

    #[must_use]
    pub const fn size(self) -> u32 {
        self.size
    }

    #[must_use]
    pub fn digest_hex(self) -> String {
        hex(&self.sha256)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IccSemanticIdentity([u8; 32]);

impl IccSemanticIdentity {
    pub(crate) const fn new(sha256: [u8; 32]) -> Self {
        Self(sha256)
    }

    #[must_use]
    pub const fn sha256(self) -> [u8; 32] {
        self.0
    }

    #[must_use]
    pub fn digest_hex(self) -> String {
        hex(&self.0)
    }
}

pub(crate) fn semantic_identity(
    header: &IccHeader,
    entries: &[(IccSignature, &[u8])],
) -> IccSemanticIdentity {
    let mut digest = Sha256::new();
    digest.update(b"rusttable.icc.semantic.v2\0");
    digest.update([
        header.version.major(),
        header.version.minor(),
        header.version.bugfix(),
    ]);
    digest.update(class_signature(header.class));
    digest.update(color_signature(header.data_color_space));
    digest.update(color_signature(header.connection_space));
    digest.update((header.intent as u32).to_be_bytes());
    for value in header.illuminant.finite_values() {
        digest.update(value.bits().to_be_bytes());
    }
    for (signature, bytes) in entries {
        digest.update(signature.bytes());
        digest.update(
            u64::try_from(bytes.len())
                .expect("bounded tag length")
                .to_be_bytes(),
        );
        digest.update(*bytes);
    }
    IccSemanticIdentity::new(digest.finalize().into())
}

pub(crate) fn product_profile_id(
    bytes: &[u8],
    header: &IccHeader,
    model: ProfileModel,
) -> Result<ProfileId, ProfileIdError> {
    let pcs = match header.connection_space {
        super::IccColorSpace::Xyz => Pcs::XyzD50,
        super::IccColorSpace::Lab => Pcs::LabD50,
        super::IccColorSpace::Rgb | super::IccColorSpace::Gray => Pcs::Unknown,
    };
    ProfileId::from_content(
        bytes,
        header.class,
        model,
        pcs,
        ProfileParserVersion::new(ICC_PROFILE_PARSER_VERSION)?,
    )
}

const fn class_signature(class: ProfileClass) -> [u8; 4] {
    match class {
        ProfileClass::Input => *b"scnr",
        ProfileClass::Display => *b"mntr",
        ProfileClass::Output => *b"prtr",
        ProfileClass::DeviceLink => *b"link",
        ProfileClass::Working => *b"spac",
        ProfileClass::Proof => *b"pruf",
        ProfileClass::Analysis => *b"abst",
    }
}

const fn color_signature(space: super::IccColorSpace) -> [u8; 4] {
    match space {
        super::IccColorSpace::Rgb => *b"RGB ",
        super::IccColorSpace::Gray => *b"GRAY",
        super::IccColorSpace::Xyz => *b"XYZ ",
        super::IccColorSpace::Lab => *b"Lab ",
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut digest = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(digest, "{byte:02x}").expect("writing to a string cannot fail");
    }
    digest
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
