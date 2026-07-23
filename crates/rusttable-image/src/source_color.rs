use rusttable_color::{
    BuiltinSpace, ColorEncoding, Primaries, ProfileId, ProfileModel, TransferFunction, WhitePoint,
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
///
/// Matrix-capable sources carry explicit primaries and transfer parameters.
/// Profile-authoritative ICC sources intentionally do not: their exact ICC
/// payload is the only valid color interpretation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceColor {
    encoding: ColorEncoding,
    profile: Option<ProfileId>,
    evidence: SourceColorEvidence,
    interpretation: SourceColorInterpretation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SourceColorInterpretation {
    Matrix {
        primaries: Primaries,
        transfer: TransferFunction,
    },
    ProfileAuthoritative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceColorError {
    UnspecifiedEncoding,
    UnsupportedEncoding,
    MissingProfileIdentity,
    UnexpectedProfileIdentity,
    NonMatrixProfile,
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
        if profile.model() != ProfileModel::Matrix {
            return Err(SourceColorError::NonMatrixProfile);
        }
        Self::new(
            ColorEncoding::External(profile),
            transfer,
            primaries,
            Some(profile),
            evidence,
        )
    }

    /// Retains an embedded ICC as the authoritative source interpretation.
    ///
    /// This state exposes no matrix primaries or transfer function. Callers
    /// that only support matrix transforms must reject it rather than project
    /// or substitute profile data.
    #[must_use]
    pub const fn profile_authoritative_icc(profile: ProfileId) -> Self {
        Self {
            encoding: ColorEncoding::External(profile),
            profile: Some(profile),
            evidence: SourceColorEvidence::EmbeddedIcc,
            interpretation: SourceColorInterpretation::ProfileAuthoritative,
        }
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
            profile,
            evidence,
            interpretation: SourceColorInterpretation::Matrix {
                primaries,
                transfer,
            },
        })
    }

    #[must_use]
    pub const fn encoding(self) -> ColorEncoding {
        self.encoding
    }

    #[must_use]
    pub const fn transfer(self) -> Option<TransferFunction> {
        match self.interpretation {
            SourceColorInterpretation::Matrix { transfer, .. } => Some(transfer),
            SourceColorInterpretation::ProfileAuthoritative => None,
        }
    }

    #[must_use]
    pub const fn primaries(self) -> Option<Primaries> {
        match self.interpretation {
            SourceColorInterpretation::Matrix { primaries, .. } => Some(primaries),
            SourceColorInterpretation::ProfileAuthoritative => None,
        }
    }

    #[must_use]
    pub const fn white_point(self) -> Option<WhitePoint> {
        match self.primaries() {
            Some(primaries) => Some(primaries.white()),
            None => None,
        }
    }

    /// Returns the complete matrix projection when one exists.
    #[must_use]
    pub const fn matrix(self) -> Option<(Primaries, TransferFunction)> {
        match self.interpretation {
            SourceColorInterpretation::Matrix {
                primaries,
                transfer,
            } => Some((primaries, transfer)),
            SourceColorInterpretation::ProfileAuthoritative => None,
        }
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
            Self::NonMatrixProfile => "matrix source color requires a matrix profile identity",
            Self::WhitePointMismatch => {
                "source color primaries and encoding have different white points"
            }
        })
    }
}

impl std::error::Error for SourceColorError {}

#[cfg(test)]
mod tests {
    use rusttable_color::{Pcs, ProfileClass, ProfileModel, ProfileParserVersion};

    use super::*;

    #[test]
    fn profile_authoritative_icc_has_no_matrix_projection() {
        let profile = ProfileId::from_content(
            b"opaque ICC",
            ProfileClass::Input,
            ProfileModel::Lut,
            Pcs::XyzD50,
            ProfileParserVersion::new(1).expect("parser version"),
        )
        .expect("profile identity");
        let source = SourceColor::profile_authoritative_icc(profile);

        assert_eq!(source.encoding(), ColorEncoding::External(profile));
        assert_eq!(source.profile(), Some(profile));
        assert_eq!(source.evidence(), SourceColorEvidence::EmbeddedIcc);
        assert_eq!(source.matrix(), None);
        assert_eq!(source.primaries(), None);
        assert_eq!(source.transfer(), None);
        assert_eq!(source.white_point(), None);
    }

    #[test]
    fn matrix_external_keeps_complete_projection() {
        let profile = ProfileId::from_content(
            b"matrix ICC",
            ProfileClass::Input,
            ProfileModel::Matrix,
            Pcs::XyzD50,
            ProfileParserVersion::new(1).expect("parser version"),
        )
        .expect("profile identity");
        let source = SourceColor::external(
            profile,
            Primaries::srgb(),
            TransferFunction::Srgb,
            SourceColorEvidence::EmbeddedIcc,
        )
        .expect("matrix source");

        assert_eq!(
            source.matrix(),
            Some((Primaries::srgb(), TransferFunction::Srgb))
        );
        assert_eq!(source.white_point(), Some(WhitePoint::D65));
    }

    #[test]
    fn matrix_external_rejects_lut_profile_identity() {
        let profile = ProfileId::from_content(
            b"LUT ICC",
            ProfileClass::Input,
            ProfileModel::Lut,
            Pcs::XyzD50,
            ProfileParserVersion::new(1).expect("parser version"),
        )
        .expect("profile identity");

        assert_eq!(
            SourceColor::external(
                profile,
                Primaries::srgb(),
                TransferFunction::Srgb,
                SourceColorEvidence::EmbeddedIcc,
            ),
            Err(SourceColorError::NonMatrixProfile)
        );
    }
}
