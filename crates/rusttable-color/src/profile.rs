//! Bounded, owned ICC v2/v4 profile parsing and identity.
//!
//! Parsing is intentionally separate from transform planning. A parsed profile
//! contains validated data only; it cannot be executed without the future
//! profile-to-plan policy owned by issue #261.

mod identity;
mod lut;
mod parser;
mod tags;
mod types;

pub use identity::{
    IccByteIdentity, IccSemanticIdentity, ProfileId, ProfileIdError, ProfileParserVersion,
};
pub use parser::{IccParseError, IccParseErrorKind, IccProfileLimits, parse_icc_profile};
pub use types::{
    IccCicp, IccClut, IccColorSpace, IccCurve, IccDateTime, IccDescription, IccHeader,
    IccLocalizedString, IccLut, IccLutDirection, IccMultiStageLut, IccOpaqueTag,
    IccParametricCurve, IccProfile, IccProfileIdentity, IccRenderingIntent, IccSignature, IccTag,
    IccTagValue, IccVersion, IccViewingCondition, IccXyz, Pcs, ProfileClass, ProfileModel,
};

/// Version of the parser and canonical semantic-identity contract in this module.
pub const ICC_PROFILE_PARSER_VERSION: u16 = 2;
