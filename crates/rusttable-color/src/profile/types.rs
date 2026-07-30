use super::{IccByteIdentity, IccSemanticIdentity, ProfileId};
use crate::{FiniteF32, Matrix3};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ProfileClass {
    Input,
    Display,
    Output,
    DeviceLink,
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

/// Four-byte ICC signature. Valid parsed signatures contain printable ASCII or are zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IccSignature(pub(crate) [u8; 4]);

impl IccSignature {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 4]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 4] {
        self.0
    }

    #[must_use]
    pub fn is_zero(self) -> bool {
        self.0 == [0; 4]
    }
}

impl fmt::Display for IccSignature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_zero() {
            return formatter.write_str("00000000");
        }
        for byte in self.0 {
            formatter.write_str(char::from(byte).encode_utf8(&mut [0; 4]))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IccVersion {
    major: u8,
    minor: u8,
    bugfix: u8,
}

impl IccVersion {
    pub(crate) const fn new(major: u8, minor: u8, bugfix: u8) -> Self {
        Self {
            major,
            minor,
            bugfix,
        }
    }

    #[must_use]
    pub const fn major(self) -> u8 {
        self.major
    }

    #[must_use]
    pub const fn minor(self) -> u8 {
        self.minor
    }

    #[must_use]
    pub const fn bugfix(self) -> u8 {
        self.bugfix
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IccColorSpace {
    Rgb,
    Gray,
    Xyz,
    Lab,
}

impl IccColorSpace {
    #[must_use]
    pub const fn channels(self) -> u8 {
        match self {
            Self::Gray => 1,
            Self::Rgb | Self::Xyz | Self::Lab => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IccRenderingIntent {
    Perceptual,
    MediaRelativeColorimetric,
    Saturation,
    IccAbsoluteColorimetric,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IccDateTime {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IccXyz {
    values: [FiniteF32; 3],
}

impl IccXyz {
    pub(crate) const fn new(values: [FiniteF32; 3]) -> Self {
        Self { values }
    }

    #[must_use]
    pub fn values(self) -> [f32; 3] {
        self.values.map(FiniteF32::get)
    }

    #[must_use]
    pub const fn finite_values(self) -> [FiniteF32; 3] {
        self.values
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IccHeader {
    pub(crate) declared_size: u32,
    pub(crate) preferred_cmm: IccSignature,
    pub(crate) version: IccVersion,
    pub(crate) class: ProfileClass,
    pub(crate) data_color_space: IccColorSpace,
    pub(crate) connection_space: IccColorSpace,
    pub(crate) created_at: IccDateTime,
    pub(crate) platform: IccSignature,
    pub(crate) flags: u32,
    pub(crate) manufacturer: IccSignature,
    pub(crate) model: IccSignature,
    pub(crate) attributes: u64,
    pub(crate) intent: IccRenderingIntent,
    pub(crate) illuminant: IccXyz,
    pub(crate) creator: IccSignature,
    pub(crate) embedded_profile_id: Option<[u8; 16]>,
}

impl IccHeader {
    #[must_use]
    pub const fn declared_size(&self) -> u32 {
        self.declared_size
    }

    #[must_use]
    pub const fn version(&self) -> IccVersion {
        self.version
    }

    #[must_use]
    pub const fn class(&self) -> ProfileClass {
        self.class
    }

    #[must_use]
    pub const fn data_color_space(&self) -> IccColorSpace {
        self.data_color_space
    }

    #[must_use]
    pub const fn connection_space(&self) -> IccColorSpace {
        self.connection_space
    }

    #[must_use]
    pub const fn created_at(&self) -> IccDateTime {
        self.created_at
    }

    #[must_use]
    pub const fn preferred_cmm(&self) -> IccSignature {
        self.preferred_cmm
    }

    #[must_use]
    pub const fn platform(&self) -> IccSignature {
        self.platform
    }

    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    #[must_use]
    pub const fn manufacturer(&self) -> IccSignature {
        self.manufacturer
    }

    #[must_use]
    pub const fn model(&self) -> IccSignature {
        self.model
    }

    #[must_use]
    pub const fn attributes(&self) -> u64 {
        self.attributes
    }

    #[must_use]
    pub const fn intent(&self) -> IccRenderingIntent {
        self.intent
    }

    #[must_use]
    pub const fn illuminant(&self) -> IccXyz {
        self.illuminant
    }

    #[must_use]
    pub const fn creator(&self) -> IccSignature {
        self.creator
    }

    #[must_use]
    pub const fn embedded_profile_id(&self) -> Option<[u8; 16]> {
        self.embedded_profile_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IccCurve {
    Identity,
    Gamma(FiniteF32),
    Table(Vec<u16>),
    Parametric(IccParametricCurve),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IccParametricCurve {
    function: u16,
    parameters: Vec<FiniteF32>,
}

impl IccParametricCurve {
    pub(crate) const fn new(function: u16, parameters: Vec<FiniteF32>) -> Self {
        Self {
            function,
            parameters,
        }
    }

    #[must_use]
    pub const fn function(&self) -> u16 {
        self.function
    }

    #[must_use]
    pub fn parameters(&self) -> &[FiniteF32] {
        &self.parameters
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IccClut {
    U8(Vec<u8>),
    U16(Vec<u16>),
}

impl IccClut {
    #[must_use]
    pub fn sample_count(&self) -> usize {
        match self {
            Self::U8(values) => values.len(),
            Self::U16(values) => values.len(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IccLutDirection {
    AToB,
    BToA,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IccMultiStageLut {
    pub(crate) direction: IccLutDirection,
    pub(crate) input_channels: u8,
    pub(crate) output_channels: u8,
    pub(crate) a_curves: Vec<IccCurve>,
    pub(crate) clut_grid_points: Vec<u8>,
    pub(crate) clut: Option<IccClut>,
    pub(crate) m_curves: Vec<IccCurve>,
    pub(crate) matrix: Option<Matrix3>,
    pub(crate) matrix_offset: Option<[FiniteF32; 3]>,
    pub(crate) b_curves: Vec<IccCurve>,
}

impl IccMultiStageLut {
    #[must_use]
    pub const fn direction(&self) -> IccLutDirection {
        self.direction
    }

    #[must_use]
    pub const fn input_channels(&self) -> u8 {
        self.input_channels
    }

    #[must_use]
    pub const fn output_channels(&self) -> u8 {
        self.output_channels
    }

    #[must_use]
    pub fn a_curves(&self) -> &[IccCurve] {
        &self.a_curves
    }

    #[must_use]
    pub fn clut_grid_points(&self) -> &[u8] {
        &self.clut_grid_points
    }

    #[must_use]
    pub const fn clut(&self) -> Option<&IccClut> {
        self.clut.as_ref()
    }

    #[must_use]
    pub fn m_curves(&self) -> &[IccCurve] {
        &self.m_curves
    }

    #[must_use]
    pub const fn matrix(&self) -> Option<Matrix3> {
        self.matrix
    }

    #[must_use]
    pub const fn matrix_offset(&self) -> Option<[FiniteF32; 3]> {
        self.matrix_offset
    }

    #[must_use]
    pub fn b_curves(&self) -> &[IccCurve] {
        &self.b_curves
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IccLut {
    Lut8 {
        input_channels: u8,
        output_channels: u8,
        grid_points: u8,
        matrix: Matrix3,
        input_tables: Vec<Vec<u8>>,
        clut: Vec<u8>,
        output_tables: Vec<Vec<u8>>,
    },
    Lut16 {
        input_channels: u8,
        output_channels: u8,
        grid_points: u8,
        matrix: Matrix3,
        input_tables: Vec<Vec<u16>>,
        clut: Vec<u16>,
        output_tables: Vec<Vec<u16>>,
    },
    MultiStage(IccMultiStageLut),
}

impl IccLut {
    #[must_use]
    pub const fn channels(&self) -> (u8, u8) {
        match self {
            Self::Lut8 {
                input_channels,
                output_channels,
                ..
            }
            | Self::Lut16 {
                input_channels,
                output_channels,
                ..
            } => (*input_channels, *output_channels),
            Self::MultiStage(value) => (value.input_channels, value.output_channels),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IccDescription {
    pub(crate) ascii: String,
    pub(crate) unicode: Option<String>,
    pub(crate) script: Option<String>,
}

impl IccDescription {
    #[must_use]
    pub fn ascii(&self) -> &str {
        &self.ascii
    }

    #[must_use]
    pub fn unicode(&self) -> Option<&str> {
        self.unicode.as_deref()
    }

    #[must_use]
    pub fn script(&self) -> Option<&str> {
        self.script.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IccLocalizedString {
    pub(crate) language: [u8; 2],
    pub(crate) country: [u8; 2],
    pub(crate) text: String,
}

impl IccLocalizedString {
    #[must_use]
    pub const fn language(&self) -> [u8; 2] {
        self.language
    }

    #[must_use]
    pub const fn country(&self) -> [u8; 2] {
        self.country
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IccCicp {
    pub color_primaries: u8,
    pub transfer_characteristics: u8,
    pub matrix_coefficients: u8,
    pub full_range: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IccViewingCondition {
    pub illuminant: IccXyz,
    pub surround: IccXyz,
    pub illuminant_type: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IccOpaqueTag {
    pub(crate) type_signature: IccSignature,
    pub(crate) bytes: Box<[u8]>,
}

impl IccOpaqueTag {
    #[must_use]
    pub const fn type_signature(&self) -> IccSignature {
        self.type_signature
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IccTagValue {
    Xyz(Vec<IccXyz>),
    Curve(IccCurve),
    Lut(IccLut),
    ChromaticAdaptation(Matrix3),
    Description(IccDescription),
    Localized(Vec<IccLocalizedString>),
    Cicp(IccCicp),
    ViewingCondition(IccViewingCondition),
    Opaque(IccOpaqueTag),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IccTag {
    pub(crate) signature: IccSignature,
    pub(crate) offset: u32,
    pub(crate) size: u32,
    pub(crate) value: IccTagValue,
}

impl IccTag {
    #[must_use]
    pub const fn signature(&self) -> IccSignature {
        self.signature
    }

    #[must_use]
    pub const fn offset(&self) -> u32 {
        self.offset
    }

    #[must_use]
    pub const fn size(&self) -> u32 {
        self.size
    }

    #[must_use]
    pub const fn value(&self) -> &IccTagValue {
        &self.value
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IccProfileIdentity {
    byte: IccByteIdentity,
    semantic: IccSemanticIdentity,
}

impl IccProfileIdentity {
    pub(crate) const fn new(byte: IccByteIdentity, semantic: IccSemanticIdentity) -> Self {
        Self { byte, semantic }
    }

    #[must_use]
    pub const fn byte(self) -> IccByteIdentity {
        self.byte
    }

    /// Canonical identity over behavior-bearing header fields and signature-sorted tag payloads.
    ///
    /// It deliberately excludes tag-table order, tag offsets, alignment padding, profile ID,
    /// creation time, and device/vendor provenance fields. It includes every tag payload,
    /// including descriptions and unknown tags, so it is not an equivalence proof for transforms.
    #[must_use]
    pub const fn semantic(self) -> IccSemanticIdentity {
        self.semantic
    }
}

/// Validated, owned profile data with no native handles or executable transform plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IccProfile {
    pub(crate) header: IccHeader,
    pub(crate) tags: Vec<IccTag>,
    pub(crate) model: ProfileModel,
    pub(crate) identity: IccProfileIdentity,
    pub(crate) profile_id: ProfileId,
}

impl IccProfile {
    #[must_use]
    pub const fn header(&self) -> &IccHeader {
        &self.header
    }

    #[must_use]
    pub fn tags(&self) -> &[IccTag] {
        &self.tags
    }

    #[must_use]
    pub fn tag(&self, signature: IccSignature) -> Option<&IccTag> {
        self.tags.iter().find(|tag| tag.signature == signature)
    }

    #[must_use]
    pub const fn model(&self) -> ProfileModel {
        self.model
    }

    #[must_use]
    pub const fn identity(&self) -> IccProfileIdentity {
        self.identity
    }

    #[must_use]
    pub const fn profile_id(&self) -> ProfileId {
        self.profile_id
    }
}
