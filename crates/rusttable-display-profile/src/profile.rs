use rusttable_color::{
    Adaptation, AdaptationMethod, AlphaTransform, BlackPointCompensation, ColorEncoding, ColorRole,
    ColorTransformRequest, ExtendedRange, Pcs, Precision, Primaries, ProfileClass, ProfileId,
    ProfileModel, ProfileParserVersion, RenderingIntent, TransferFunction, TransformPlan,
    TransformPlanError, TransformStep, WhitePoint, rgb_to_xyz_matrix,
};
use std::{collections::BTreeMap, fmt, sync::Arc};

pub const MAX_PROFILE_BYTES: usize = 64 * 1024 * 1024;
pub const MIN_PROFILE_BYTES: usize = 128;

pub type DisplayProfileId = ProfileId;

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

    /// Builds the typed transform used by selected-preview presentation.
    ///
    /// Profile bytes remain owned by the profile store; callers receive only the validated
    /// matrix/transfer plan needed to adapt already-rendered sRGB pixels for the active monitor.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileTransformError`] when the profile lacks usable matrix evidence, contains
    /// malformed tags, or cannot produce a valid typed color transform plan.
    pub fn presentation_plan(
        &self,
        intent: RenderingIntent,
    ) -> Result<TransformPlan, ProfileTransformError> {
        let matrix = MatrixProfile::parse(&self.bytes)?;
        matrix.plan(self.id, intent)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileTransformError {
    InvalidTagTable,
    MissingMatrixTag,
    InvalidMatrix,
    InvalidWhitePoint,
    UnsupportedTransfer,
    Plan(TransformPlanError),
}

impl fmt::Display for ProfileTransformError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidTagTable => "ICC profile tag table is invalid",
            Self::MissingMatrixTag => "ICC display profile has no complete matrix",
            Self::InvalidMatrix => "ICC display profile matrix is invalid",
            Self::InvalidWhitePoint => "ICC display profile white point is invalid",
            Self::UnsupportedTransfer => "ICC display profile transfer curve is unsupported",
            Self::Plan(_) => "ICC display profile transform plan is invalid",
        })
    }
}

impl std::error::Error for ProfileTransformError {}

#[derive(Debug, Clone, Copy)]
struct MatrixProfile {
    primaries: Primaries,
    transfer: TransferFunction,
}

impl MatrixProfile {
    fn parse(bytes: &[u8]) -> Result<Self, ProfileTransformError> {
        let tags = tags(bytes)?;
        let red = xyz_tag(bytes, tags.get(b"rXYZ".as_slice()))?;
        let green = xyz_tag(bytes, tags.get(b"gXYZ".as_slice()))?;
        let blue = xyz_tag(bytes, tags.get(b"bXYZ".as_slice()))?;
        let white = tags
            .get(b"wtpt".as_slice())
            .map_or(Ok(WhitePoint::D50), |tag| {
                white_point(xyz_tag(bytes, Some(tag))?)
            });
        let primaries = Primaries::new(
            chromaticity(red)?,
            chromaticity(green)?,
            chromaticity(blue)?,
            white?,
        )
        .map_err(|_| ProfileTransformError::InvalidMatrix)?;
        let transfer = tags
            .get(b"rTRC".as_slice())
            .map_or(Ok(TransferFunction::Srgb), |tag| transfer(bytes, tag))?;
        Ok(Self {
            primaries,
            transfer,
        })
    }

    fn plan(
        self,
        profile_id: DisplayProfileId,
        intent: RenderingIntent,
    ) -> Result<TransformPlan, ProfileTransformError> {
        let request = ColorTransformRequest::new(
            ColorEncoding::SrgbD65,
            ColorEncoding::External(profile_id),
            ColorRole::Working,
            intent,
            BlackPointCompensation::Disabled,
            AdaptationMethod::Bradford,
            Precision::F32,
            AlphaTransform::Preserve,
            ExtendedRange::Extended,
            1,
        )
        .map_err(|_| ProfileTransformError::InvalidMatrix)?;
        let source = rgb_to_xyz_matrix(
            [
                pair(Primaries::srgb().red()),
                pair(Primaries::srgb().green()),
                pair(Primaries::srgb().blue()),
            ],
            WhitePoint::D65,
        )
        .map_err(|_| ProfileTransformError::InvalidMatrix)?;
        let target = rgb_to_xyz_matrix(
            [
                pair(self.primaries.red()),
                pair(self.primaries.green()),
                pair(self.primaries.blue()),
            ],
            self.primaries.white(),
        )
        .map_err(|_| ProfileTransformError::InvalidMatrix)?
        .inverse()
        .map_err(|_| ProfileTransformError::InvalidMatrix)?;
        let mut steps = vec![
            TransformStep::Transfer {
                function: TransferFunction::Srgb,
                decode: true,
            },
            TransformStep::Matrix(source),
        ];
        if self.primaries.white() != WhitePoint::D65 {
            steps.push(TransformStep::Adaptation(
                Adaptation::between(
                    WhitePoint::D65,
                    self.primaries.white(),
                    AdaptationMethod::Bradford,
                )
                .map_err(|_| ProfileTransformError::InvalidWhitePoint)?,
            ));
        }
        steps.push(TransformStep::Matrix(target));
        if !matches!(self.transfer, TransferFunction::Linear) {
            steps.push(TransformStep::Transfer {
                function: self.transfer,
                decode: false,
            });
        }
        TransformPlan::new(request, steps).map_err(ProfileTransformError::Plan)
    }
}

fn tags(bytes: &[u8]) -> Result<BTreeMap<[u8; 4], (usize, usize)>, ProfileTransformError> {
    if bytes.len() < 132 {
        return Err(ProfileTransformError::InvalidTagTable);
    }
    let count = usize::try_from(u32::from_be_bytes(
        bytes[128..132].try_into().expect("bounded ICC tag count"),
    ))
    .map_err(|_| ProfileTransformError::InvalidTagTable)?;
    let table_end = 132_usize
        .checked_add(
            count
                .checked_mul(12)
                .ok_or(ProfileTransformError::InvalidTagTable)?,
        )
        .ok_or(ProfileTransformError::InvalidTagTable)?;
    if table_end > bytes.len() {
        return Err(ProfileTransformError::InvalidTagTable);
    }
    let mut tags = BTreeMap::new();
    for index in 0..count {
        let start = 132 + index * 12;
        let name = bytes[start..start + 4]
            .try_into()
            .expect("ICC tag signature is four bytes");
        let offset = usize::try_from(u32::from_be_bytes(
            bytes[start + 4..start + 8]
                .try_into()
                .expect("ICC tag offset is four bytes"),
        ))
        .map_err(|_| ProfileTransformError::InvalidTagTable)?;
        let length = usize::try_from(u32::from_be_bytes(
            bytes[start + 8..start + 12]
                .try_into()
                .expect("ICC tag length is four bytes"),
        ))
        .map_err(|_| ProfileTransformError::InvalidTagTable)?;
        let end = offset
            .checked_add(length)
            .ok_or(ProfileTransformError::InvalidTagTable)?;
        if offset < table_end || end > bytes.len() || length < 4 {
            return Err(ProfileTransformError::InvalidTagTable);
        }
        tags.insert(name, (offset, length));
    }
    Ok(tags)
}

fn xyz_tag(bytes: &[u8], tag: Option<&(usize, usize)>) -> Result<[f32; 3], ProfileTransformError> {
    let Some(&(offset, length)) = tag else {
        return Err(ProfileTransformError::MissingMatrixTag);
    };
    if length < 20 || bytes.get(offset..offset + 4) != Some(b"XYZ ") {
        return Err(ProfileTransformError::InvalidMatrix);
    }
    Ok([
        s15_fixed16(&bytes[offset + 8..offset + 12]),
        s15_fixed16(&bytes[offset + 12..offset + 16]),
        s15_fixed16(&bytes[offset + 16..offset + 20]),
    ])
}

fn chromaticity(xyz: [f32; 3]) -> Result<(f32, f32), ProfileTransformError> {
    let sum = xyz.iter().sum::<f32>();
    (sum.is_finite() && sum > 0.0)
        .then_some((xyz[0] / sum, xyz[1] / sum))
        .ok_or(ProfileTransformError::InvalidMatrix)
}

fn white_point(xyz: [f32; 3]) -> Result<WhitePoint, ProfileTransformError> {
    let (x, y) = chromaticity(xyz)?;
    if (x - 0.345_7).abs() < 0.02 && (y - 0.358_5).abs() < 0.02 {
        Ok(WhitePoint::D50)
    } else if (x - 0.312_7).abs() < 0.02 && (y - 0.329_0).abs() < 0.02 {
        Ok(WhitePoint::D65)
    } else {
        WhitePoint::custom(x, y).map_err(|_| ProfileTransformError::InvalidWhitePoint)
    }
}

fn transfer(
    bytes: &[u8],
    &(offset, length): &(usize, usize),
) -> Result<TransferFunction, ProfileTransformError> {
    if length < 12 || bytes.get(offset..offset + 4) != Some(b"curv") {
        return Err(ProfileTransformError::UnsupportedTransfer);
    }
    let count = u32::from_be_bytes(
        bytes[offset + 8..offset + 12]
            .try_into()
            .expect("validated curve count"),
    );
    match count {
        0 => Ok(TransferFunction::Linear),
        1 if length >= 14 => TransferFunction::gamma(
            f32::from(u16::from_be_bytes(
                bytes[offset + 12..offset + 14]
                    .try_into()
                    .expect("validated gamma"),
            )) / 256.0,
        )
        .map_err(|_| ProfileTransformError::UnsupportedTransfer),
        _ => Ok(TransferFunction::Srgb),
    }
}

#[allow(clippy::cast_precision_loss)]
fn s15_fixed16(bytes: &[u8]) -> f32 {
    i32::from_be_bytes(bytes.try_into().expect("ICC XYZ value")) as f32 / 65_536.0
}

fn pair(value: (rusttable_color::FiniteF32, rusttable_color::FiniteF32)) -> (f32, f32) {
    (value.0.get(), value.1.get())
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
        let id = ProfileId::from_content(
            bytes,
            ProfileClass::Display,
            ProfileModel::Unknown,
            Pcs::XyzD50,
            ProfileParserVersion::new(1).map_err(|_| IccProfileError::InvalidHeader)?,
        )
        .map_err(|_| IccProfileError::InvalidHeader)?;
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
