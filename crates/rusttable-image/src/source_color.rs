use rusttable_color::{
    BuiltinSpace, ColorEncoding, Primaries, ProfileId, TransferFunction, WhitePoint,
};

/// Why `RustTable` selected the source transfer and primary interpretation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceColorEvidence {
    DeclaredEncoding,
    EmbeddedIcc,
    EmbeddedChromaticities,
    EmbeddedContainerMetadata,
    Fallback(SourceColorFallback),
}

/// A named interpretation used only when the source carries no usable evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceColorFallback {
    EncodedSrgb,
    LinearRec709,
}

/// Complete color evidence attached to one decoded RGB frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceColor {
    encoding: ColorEncoding,
    transfer: TransferFunction,
    primaries: Primaries,
    white_point: WhitePoint,
    profile: Option<ProfileId>,
    evidence: SourceColorEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceColorError {
    UnspecifiedEncoding,
    UnsupportedEncoding,
    MissingProfileIdentity,
    UnexpectedProfileIdentity,
    WhitePointMismatch,
}

impl SourceColor {
    /// Resolves a built-in RGB encoding without inventing profile evidence.
    ///
    /// # Errors
    ///
    /// Returns an error for unspecified, external, or non-RGB encodings.
    pub fn declared(encoding: ColorEncoding) -> Result<Self, SourceColorError> {
        let space = encoding.builtin().ok_or(match encoding {
            ColorEncoding::Unspecified => SourceColorError::UnspecifiedEncoding,
            ColorEncoding::External(_) => SourceColorError::MissingProfileIdentity,
            _ => SourceColorError::UnsupportedEncoding,
        })?;
        let primaries = space
            .primaries()
            .ok_or(SourceColorError::UnsupportedEncoding)?;
        let transfer = encoding
            .transfer()
            .ok_or(SourceColorError::UnsupportedEncoding)?;
        Self::new(
            encoding,
            transfer,
            primaries,
            None,
            SourceColorEvidence::DeclaredEncoding,
        )
    }

    /// Creates matrix/profile evidence for an embedded ICC or chromaticity source.
    ///
    /// # Errors
    ///
    /// Returns an error unless the external encoding identifies the supplied
    /// profile and the primary white point is internally consistent.
    pub fn external(
        profile: ProfileId,
        primaries: Primaries,
        transfer: TransferFunction,
        evidence: SourceColorEvidence,
    ) -> Result<Self, SourceColorError> {
        Self::new(
            ColorEncoding::External(profile),
            transfer,
            primaries,
            Some(profile),
            evidence,
        )
    }

    /// Resolves a required format fallback explicitly.
    #[must_use]
    pub fn fallback(fallback: SourceColorFallback) -> Self {
        let (encoding, transfer) = match fallback {
            SourceColorFallback::EncodedSrgb => (ColorEncoding::SrgbD65, TransferFunction::Srgb),
            SourceColorFallback::LinearRec709 => {
                (ColorEncoding::LinearSrgbD65, TransferFunction::Linear)
            }
        };
        Self::new(
            encoding,
            transfer,
            Primaries::srgb(),
            None,
            SourceColorEvidence::Fallback(fallback),
        )
        .unwrap_or_else(|_| unreachable!("published fallback color is internally consistent"))
    }

    /// Resolves a descriptor encoding for compatibility callers.
    ///
    /// Unspecified inputs become a named encoded-sRGB fallback. External
    /// encodings require the decoder to provide matrix/profile evidence.
    ///
    /// # Errors
    ///
    /// Returns an error when an external or unsupported non-RGB encoding has
    /// no complete source-color contract.
    pub fn from_encoding(encoding: ColorEncoding) -> Result<Self, SourceColorError> {
        if encoding == ColorEncoding::Unspecified {
            Ok(Self::fallback(SourceColorFallback::EncodedSrgb))
        } else {
            Self::declared(encoding)
        }
    }

    fn new(
        encoding: ColorEncoding,
        transfer: TransferFunction,
        primaries: Primaries,
        profile: Option<ProfileId>,
        evidence: SourceColorEvidence,
    ) -> Result<Self, SourceColorError> {
        match (encoding, profile) {
            (ColorEncoding::External(expected), Some(actual)) if expected == actual => {}
            (ColorEncoding::External(_), _) => {
                return Err(SourceColorError::MissingProfileIdentity);
            }
            (_, Some(_)) => return Err(SourceColorError::UnexpectedProfileIdentity),
            _ => {}
        }
        let white_point = primaries.white();
        if encoding
            .white_point()
            .is_some_and(|encoding_white| encoding_white != white_point)
        {
            return Err(SourceColorError::WhitePointMismatch);
        }
        Ok(Self {
            encoding,
            transfer,
            primaries,
            white_point,
            profile,
            evidence,
        })
    }

    #[must_use]
    pub const fn encoding(self) -> ColorEncoding {
        self.encoding
    }

    #[must_use]
    pub const fn transfer(self) -> TransferFunction {
        self.transfer
    }

    #[must_use]
    pub const fn primaries(self) -> Primaries {
        self.primaries
    }

    #[must_use]
    pub const fn white_point(self) -> WhitePoint {
        self.white_point
    }

    #[must_use]
    pub const fn profile(self) -> Option<ProfileId> {
        self.profile
    }

    #[must_use]
    pub const fn evidence(self) -> SourceColorEvidence {
        self.evidence
    }

    #[must_use]
    pub const fn fallback_used(self) -> Option<SourceColorFallback> {
        match self.evidence {
            SourceColorEvidence::Fallback(value) => Some(value),
            SourceColorEvidence::DeclaredEncoding
            | SourceColorEvidence::EmbeddedIcc
            | SourceColorEvidence::EmbeddedChromaticities
            | SourceColorEvidence::EmbeddedContainerMetadata => None,
        }
    }

    #[must_use]
    pub const fn builtin_space(self) -> Option<BuiltinSpace> {
        self.encoding.builtin()
    }
}

impl std::fmt::Display for SourceColorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::UnspecifiedEncoding => "source color encoding is unspecified",
            Self::UnsupportedEncoding => "source color encoding is not a supported RGB space",
            Self::MissingProfileIdentity => "external source color is missing its profile identity",
            Self::UnexpectedProfileIdentity => {
                "built-in source color unexpectedly carries an external profile identity"
            }
            Self::WhitePointMismatch => {
                "source color primaries and encoding have different white points"
            }
        })
    }
}

impl std::error::Error for SourceColorError {}
