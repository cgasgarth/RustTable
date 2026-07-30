use super::identity::{product_profile_id, semantic_identity};
use super::tags::parse_tag;
use super::{
    IccByteIdentity, IccColorSpace, IccDateTime, IccHeader, IccProfile, IccProfileIdentity,
    IccRenderingIntent, IccSignature, IccTag, IccTagValue, IccVersion, IccXyz, ProfileClass,
    ProfileModel,
};
use crate::{FiniteF32, Matrix3};
use std::fmt;

const HEADER_SIZE: usize = 128;
const TAG_TABLE_PREFIX_SIZE: usize = 132;
const D50: [f32; 3] = [0.964_2, 1.0, 0.824_9];
const D50_TOLERANCE: f32 = 2.0 / 65_536.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IccProfileLimits {
    pub max_profile_bytes: usize,
    pub max_tags: usize,
    pub max_tag_bytes: usize,
    pub max_curve_samples: usize,
    pub max_lut_samples: usize,
    pub max_localized_records: usize,
    pub max_text_bytes: usize,
}

impl Default for IccProfileLimits {
    fn default() -> Self {
        Self {
            max_profile_bytes: 64 * 1024 * 1024,
            max_tags: 4_096,
            max_tag_bytes: 32 * 1024 * 1024,
            max_curve_samples: 65_536,
            max_lut_samples: 65 * 65 * 65 * 3,
            max_localized_records: 256,
            max_text_bytes: 1024 * 1024,
        }
    }
}

impl IccProfileLimits {
    #[must_use]
    pub const fn strict_for_embedded_images() -> Self {
        Self {
            max_profile_bytes: 16 * 1024 * 1024,
            max_tags: 1_024,
            max_tag_bytes: 8 * 1024 * 1024,
            max_curve_samples: 65_536,
            max_lut_samples: 65 * 65 * 65 * 3,
            max_localized_records: 128,
            max_text_bytes: 256 * 1024,
        }
    }

    fn validate(self) -> Result<Self, IccParseError> {
        if self.max_profile_bytes < TAG_TABLE_PREFIX_SIZE
            || self.max_profile_bytes > u32::MAX as usize
            || self.max_tags == 0
            || self.max_tag_bytes < 8
            || self.max_curve_samples < 2
            || self.max_lut_samples == 0
            || self.max_localized_records == 0
            || self.max_text_bytes == 0
        {
            return Err(IccParseError::InvalidLimits);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IccParseErrorKind {
    Malformed,
    Unsupported,
    ResourceLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IccParseError {
    InvalidLimits,
    Empty,
    ProfileTooLarge {
        actual: usize,
        limit: usize,
    },
    Truncated,
    DeclaredSizeMismatch {
        declared: u32,
        actual: usize,
    },
    UnsupportedVersion {
        major: u8,
        minor: u8,
    },
    InvalidHeaderSignature,
    InvalidHeaderField,
    UnsupportedProfileClass(IccSignature),
    UnsupportedColorSpace(IccSignature),
    InvalidConnectionSpace,
    InvalidDate,
    InvalidRenderingIntent(u32),
    InvalidIlluminant,
    InvalidProfileId,
    TooManyTags {
        count: usize,
        limit: usize,
    },
    InvalidTagSignature(IccSignature),
    DuplicateTag(IccSignature),
    MisalignedTag(IccSignature),
    InvalidTagRange(IccSignature),
    OverlappingTags(IccSignature, IccSignature),
    TagTooLarge {
        signature: IccSignature,
        limit: usize,
    },
    InvalidTagType {
        tag: IccSignature,
        actual: IccSignature,
    },
    UnsupportedTagType {
        tag: IccSignature,
        actual: IccSignature,
    },
    InvalidTagData(IccSignature),
    CurveTooLarge {
        count: usize,
        limit: usize,
    },
    InvalidCurve(IccSignature),
    LutTooLarge {
        count: usize,
        limit: usize,
    },
    InvalidLut(IccSignature),
    InvalidText(IccSignature),
    MissingRequiredTag(IccSignature),
    UnsupportedProfileStructure,
    AllocationFailed,
    Identity,
}

impl IccParseError {
    #[must_use]
    pub const fn kind(self) -> IccParseErrorKind {
        match self {
            Self::UnsupportedVersion { .. }
            | Self::UnsupportedProfileClass(_)
            | Self::UnsupportedColorSpace(_)
            | Self::UnsupportedTagType { .. }
            | Self::UnsupportedProfileStructure => IccParseErrorKind::Unsupported,
            Self::ProfileTooLarge { .. }
            | Self::TooManyTags { .. }
            | Self::TagTooLarge { .. }
            | Self::CurveTooLarge { .. }
            | Self::LutTooLarge { .. }
            | Self::AllocationFailed
            | Self::InvalidLimits => IccParseErrorKind::ResourceLimit,
            _ => IccParseErrorKind::Malformed,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TagRecord {
    pub signature: IccSignature,
    pub offset: usize,
    pub size: usize,
}

/// Parses a complete ICC profile with explicit resource bounds.
///
/// The returned value owns all parsed and opaque tag data, performs no I/O, and
/// exposes no operation that can create or execute a transform plan.
pub fn parse_icc_profile(
    bytes: &[u8],
    limits: IccProfileLimits,
) -> Result<IccProfile, IccParseError> {
    let limits = limits.validate()?;
    if bytes.is_empty() {
        return Err(IccParseError::Empty);
    }
    if bytes.len() > limits.max_profile_bytes {
        return Err(IccParseError::ProfileTooLarge {
            actual: bytes.len(),
            limit: limits.max_profile_bytes,
        });
    }
    if bytes.len() < TAG_TABLE_PREFIX_SIZE {
        return Err(IccParseError::Truncated);
    }
    let declared = be_u32(bytes, 0)?;
    if usize::try_from(declared).ok() != Some(bytes.len()) {
        return Err(IccParseError::DeclaredSizeMismatch {
            declared,
            actual: bytes.len(),
        });
    }

    let header = parse_header(bytes, declared)?;
    validate_embedded_profile_id(bytes, header.embedded_profile_id)?;
    let records = parse_tag_table(bytes, limits)?;
    let mut tags = Vec::new();
    tags.try_reserve_exact(records.len())
        .map_err(|_| IccParseError::AllocationFailed)?;
    let mut identity_entries = Vec::new();
    identity_entries
        .try_reserve_exact(records.len())
        .map_err(|_| IccParseError::AllocationFailed)?;

    for record in records {
        let end = record
            .offset
            .checked_add(record.size)
            .ok_or(IccParseError::InvalidTagRange(record.signature))?;
        let data = bytes
            .get(record.offset..end)
            .ok_or(IccParseError::InvalidTagRange(record.signature))?;
        let value = parse_tag(record.signature, data, limits)?;
        identity_entries.push((record.signature, data));
        tags.push(IccTag {
            signature: record.signature,
            offset: u32::try_from(record.offset)
                .map_err(|_| IccParseError::InvalidTagRange(record.signature))?,
            size: u32::try_from(record.size)
                .map_err(|_| IccParseError::InvalidTagRange(record.signature))?,
            value,
        });
    }
    identity_entries.sort_unstable_by_key(|(signature, _)| *signature);
    let model = validate_profile_structure(&header, &tags)?;
    let byte = IccByteIdentity::from_bytes(bytes);
    let semantic = semantic_identity(&header, &identity_entries);
    let profile_id =
        product_profile_id(bytes, &header, model).map_err(|_| IccParseError::Identity)?;
    Ok(IccProfile {
        header,
        tags,
        model,
        identity: IccProfileIdentity::new(byte, semantic),
        profile_id,
    })
}

fn parse_header(bytes: &[u8], declared_size: u32) -> Result<IccHeader, IccParseError> {
    if bytes.get(36..40) != Some(b"acsp") {
        return Err(IccParseError::InvalidHeaderSignature);
    }
    let raw_version = be_u32(bytes, 8)?;
    if raw_version & 0xffff != 0 {
        return Err(IccParseError::InvalidHeaderField);
    }
    let major = u8::try_from(raw_version >> 24).map_err(|_| IccParseError::InvalidHeaderField)?;
    let minor =
        u8::try_from((raw_version >> 20) & 0x0f).map_err(|_| IccParseError::InvalidHeaderField)?;
    let bugfix =
        u8::try_from((raw_version >> 16) & 0x0f).map_err(|_| IccParseError::InvalidHeaderField)?;
    if !matches!(major, 2 | 4) {
        return Err(IccParseError::UnsupportedVersion { major, minor });
    }
    let class_signature = signature(bytes, 12)?;
    let class = match &class_signature.0 {
        b"scnr" => ProfileClass::Input,
        b"mntr" => ProfileClass::Display,
        b"prtr" => ProfileClass::Output,
        b"link" => ProfileClass::DeviceLink,
        b"spac" => ProfileClass::Working,
        _ => return Err(IccParseError::UnsupportedProfileClass(class_signature)),
    };
    let data_color_space = parse_color_space(signature(bytes, 16)?)?;
    if !matches!(data_color_space, IccColorSpace::Rgb | IccColorSpace::Gray) {
        return Err(IccParseError::UnsupportedColorSpace(signature(bytes, 16)?));
    }
    let connection_space = parse_color_space(signature(bytes, 20)?)?;
    let valid_connection = if class == ProfileClass::DeviceLink {
        matches!(
            connection_space,
            IccColorSpace::Rgb | IccColorSpace::Gray | IccColorSpace::Xyz | IccColorSpace::Lab
        )
    } else {
        matches!(connection_space, IccColorSpace::Xyz | IccColorSpace::Lab)
    };
    if !valid_connection {
        return Err(IccParseError::InvalidConnectionSpace);
    }
    let created_at = parse_date(bytes)?;
    let preferred_cmm = optional_signature(bytes, 4)?;
    let platform = optional_signature(bytes, 40)?;
    let flags = be_u32(bytes, 44)?;
    let attributes = be_u64(bytes, 56)?;
    if flags & !0b11 != 0
        || attributes & !0x0f != 0
        || bytes[100..HEADER_SIZE].iter().any(|byte| *byte != 0)
    {
        return Err(IccParseError::InvalidHeaderField);
    }
    let manufacturer = optional_signature(bytes, 48)?;
    let model = optional_signature(bytes, 52)?;
    let creator = optional_signature(bytes, 80)?;
    let intent_raw = be_u32(bytes, 64)?;
    let intent = match intent_raw {
        0 => IccRenderingIntent::Perceptual,
        1 => IccRenderingIntent::MediaRelativeColorimetric,
        2 => IccRenderingIntent::Saturation,
        3 => IccRenderingIntent::IccAbsoluteColorimetric,
        _ => return Err(IccParseError::InvalidRenderingIntent(intent_raw)),
    };
    let illuminant = xyz(bytes, 68)?;
    if illuminant
        .values()
        .into_iter()
        .zip(D50)
        .any(|(actual, expected)| (actual - expected).abs() > D50_TOLERANCE)
    {
        return Err(IccParseError::InvalidIlluminant);
    }
    let embedded: [u8; 16] = bytes[84..100]
        .try_into()
        .map_err(|_| IccParseError::Truncated)?;
    Ok(IccHeader {
        declared_size,
        preferred_cmm,
        version: IccVersion::new(major, minor, bugfix),
        class,
        data_color_space,
        connection_space,
        created_at,
        platform,
        flags,
        manufacturer,
        model,
        attributes,
        intent,
        illuminant,
        creator,
        embedded_profile_id: (embedded != [0; 16]).then_some(embedded),
    })
}

fn parse_tag_table(
    bytes: &[u8],
    limits: IccProfileLimits,
) -> Result<Vec<TagRecord>, IccParseError> {
    let count =
        usize::try_from(be_u32(bytes, HEADER_SIZE)?).map_err(|_| IccParseError::Truncated)?;
    if count > limits.max_tags {
        return Err(IccParseError::TooManyTags {
            count,
            limit: limits.max_tags,
        });
    }
    let table_end = count
        .checked_mul(12)
        .and_then(|size| TAG_TABLE_PREFIX_SIZE.checked_add(size))
        .ok_or(IccParseError::Truncated)?;
    if table_end > bytes.len() {
        return Err(IccParseError::Truncated);
    }
    let mut records = Vec::new();
    records
        .try_reserve_exact(count)
        .map_err(|_| IccParseError::AllocationFailed)?;
    for index in 0..count {
        let start = TAG_TABLE_PREFIX_SIZE + index * 12;
        let tag_signature = signature(bytes, start)?;
        if records
            .iter()
            .any(|record: &TagRecord| record.signature == tag_signature)
        {
            return Err(IccParseError::DuplicateTag(tag_signature));
        }
        let offset = usize::try_from(be_u32(bytes, start + 4)?)
            .map_err(|_| IccParseError::InvalidTagRange(tag_signature))?;
        let size = usize::try_from(be_u32(bytes, start + 8)?)
            .map_err(|_| IccParseError::InvalidTagRange(tag_signature))?;
        if offset % 4 != 0 {
            return Err(IccParseError::MisalignedTag(tag_signature));
        }
        if size < 8
            || offset < table_end
            || offset.checked_add(size).is_none_or(|end| end > bytes.len())
        {
            return Err(IccParseError::InvalidTagRange(tag_signature));
        }
        if size > limits.max_tag_bytes {
            return Err(IccParseError::TagTooLarge {
                signature: tag_signature,
                limit: limits.max_tag_bytes,
            });
        }
        records.push(TagRecord {
            signature: tag_signature,
            offset,
            size,
        });
    }
    let mut by_range = records.clone();
    by_range.sort_unstable_by_key(|record| (record.offset, record.size));
    for pair in by_range.windows(2) {
        let [left, right] = pair else {
            unreachable!("windows(2) always has two entries")
        };
        let left_end = left.offset + left.size;
        let shared = left.offset == right.offset && left.size == right.size;
        if !shared && right.offset < left_end {
            return Err(IccParseError::OverlappingTags(
                left.signature,
                right.signature,
            ));
        }
    }
    Ok(records)
}

fn validate_profile_structure(
    header: &IccHeader,
    tags: &[IccTag],
) -> Result<ProfileModel, IccParseError> {
    let has_lut = tags
        .iter()
        .any(|tag| is_transform_lut(tag.signature) && matches!(tag.value, IccTagValue::Lut(_)));
    if header.class == ProfileClass::DeviceLink {
        let a2b0 = IccSignature::from_bytes(*b"A2B0");
        if !tags
            .iter()
            .any(|tag| tag.signature == a2b0 && matches!(tag.value, IccTagValue::Lut(_)))
        {
            return Err(IccParseError::MissingRequiredTag(a2b0));
        }
        validate_lut_channels(header, tags)?;
        return Ok(ProfileModel::Lut);
    }
    if has_lut {
        validate_lut_channels(header, tags)?;
        return Ok(ProfileModel::Lut);
    }
    match header.data_color_space {
        IccColorSpace::Rgb => validate_rgb_matrix(tags)?,
        IccColorSpace::Gray => validate_gray_matrix(tags)?,
        IccColorSpace::Xyz | IccColorSpace::Lab => {
            return Err(IccParseError::UnsupportedProfileStructure);
        }
    }
    Ok(ProfileModel::Matrix)
}

fn validate_rgb_matrix(tags: &[IccTag]) -> Result<(), IccParseError> {
    let red = required_xyz(tags, *b"rXYZ")?;
    let green = required_xyz(tags, *b"gXYZ")?;
    let blue = required_xyz(tags, *b"bXYZ")?;
    let _white = required_xyz(tags, *b"wtpt")?;
    for signature in [*b"rTRC", *b"gTRC", *b"bTRC"] {
        required_curve(tags, signature)?;
    }
    let [r, g, b] = [red.values(), green.values(), blue.values()];
    Matrix3::new([r[0], g[0], b[0], r[1], g[1], b[1], r[2], g[2], b[2]])
        .map_err(|_| IccParseError::InvalidTagData(IccSignature::from_bytes(*b"rXYZ")))?;
    Ok(())
}

fn validate_gray_matrix(tags: &[IccTag]) -> Result<(), IccParseError> {
    let _white = required_xyz(tags, *b"wtpt")?;
    required_curve(tags, *b"kTRC")
}

fn validate_lut_channels(header: &IccHeader, tags: &[IccTag]) -> Result<(), IccParseError> {
    for tag in tags {
        let IccTagValue::Lut(lut) = &tag.value else {
            continue;
        };
        if !is_transform_lut(tag.signature) {
            continue;
        }
        let (input, output) = lut.channels();
        let (expected_input, expected_output) =
            if matches!(&tag.signature.0, b"B2A0" | b"B2A1" | b"B2A2" | b"B2A3") {
                (
                    header.connection_space.channels(),
                    header.data_color_space.channels(),
                )
            } else {
                (
                    header.data_color_space.channels(),
                    header.connection_space.channels(),
                )
            };
        if input != expected_input || output != expected_output {
            return Err(IccParseError::InvalidLut(tag.signature));
        }
    }
    Ok(())
}

fn is_transform_lut(signature: IccSignature) -> bool {
    matches!(
        &signature.0,
        b"A2B0" | b"A2B1" | b"A2B2" | b"A2B3" | b"B2A0" | b"B2A1" | b"B2A2" | b"B2A3"
    )
}

fn required_xyz(tags: &[IccTag], signature: [u8; 4]) -> Result<IccXyz, IccParseError> {
    let signature = IccSignature::from_bytes(signature);
    let tag = tags
        .iter()
        .find(|tag| tag.signature == signature)
        .ok_or(IccParseError::MissingRequiredTag(signature))?;
    let IccTagValue::Xyz(values) = &tag.value else {
        return Err(IccParseError::InvalidTagData(signature));
    };
    if values.len() != 1 {
        return Err(IccParseError::InvalidTagData(signature));
    }
    Ok(values[0])
}

fn required_curve(tags: &[IccTag], signature: [u8; 4]) -> Result<(), IccParseError> {
    let signature = IccSignature::from_bytes(signature);
    let tag = tags
        .iter()
        .find(|tag| tag.signature == signature)
        .ok_or(IccParseError::MissingRequiredTag(signature))?;
    matches!(tag.value, IccTagValue::Curve(_))
        .then_some(())
        .ok_or(IccParseError::InvalidTagData(signature))
}

fn validate_embedded_profile_id(
    bytes: &[u8],
    embedded: Option<[u8; 16]>,
) -> Result<(), IccParseError> {
    let Some(expected) = embedded else {
        return Ok(());
    };
    let mut context = md5::Context::new();
    context.consume(&bytes[..44]);
    context.consume([0_u8; 4]);
    context.consume(&bytes[48..64]);
    context.consume([0_u8; 4]);
    context.consume(&bytes[68..84]);
    context.consume([0_u8; 16]);
    context.consume(&bytes[100..]);
    if context.finalize().0 != expected {
        return Err(IccParseError::InvalidProfileId);
    }
    Ok(())
}

fn parse_color_space(signature: IccSignature) -> Result<IccColorSpace, IccParseError> {
    match &signature.0 {
        b"RGB " => Ok(IccColorSpace::Rgb),
        b"GRAY" => Ok(IccColorSpace::Gray),
        b"XYZ " => Ok(IccColorSpace::Xyz),
        b"Lab " => Ok(IccColorSpace::Lab),
        _ => Err(IccParseError::UnsupportedColorSpace(signature)),
    }
}

fn parse_date(bytes: &[u8]) -> Result<IccDateTime, IccParseError> {
    let date = IccDateTime {
        year: be_u16(bytes, 24)?,
        month: be_u16(bytes, 26)?,
        day: be_u16(bytes, 28)?,
        hour: be_u16(bytes, 30)?,
        minute: be_u16(bytes, 32)?,
        second: be_u16(bytes, 34)?,
    };
    let days = match date.month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(date.year) => 29,
        2 => 28,
        _ => return Err(IccParseError::InvalidDate),
    };
    if date.year == 0
        || date.day == 0
        || date.day > days
        || date.hour > 23
        || date.minute > 59
        || date.second > 59
    {
        return Err(IccParseError::InvalidDate);
    }
    Ok(date)
}

const fn is_leap_year(year: u16) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

pub(crate) fn signature(bytes: &[u8], offset: usize) -> Result<IccSignature, IccParseError> {
    let raw: [u8; 4] = bytes
        .get(offset..offset + 4)
        .ok_or(IccParseError::Truncated)?
        .try_into()
        .map_err(|_| IccParseError::Truncated)?;
    let signature = IccSignature::from_bytes(raw);
    if raw.iter().all(|byte| (0x20..=0x7e).contains(byte)) {
        Ok(signature)
    } else {
        Err(IccParseError::InvalidTagSignature(signature))
    }
}

fn optional_signature(bytes: &[u8], offset: usize) -> Result<IccSignature, IccParseError> {
    let raw: [u8; 4] = bytes
        .get(offset..offset + 4)
        .ok_or(IccParseError::Truncated)?
        .try_into()
        .map_err(|_| IccParseError::Truncated)?;
    if raw == [0; 4] || raw.iter().all(|byte| (0x20..=0x7e).contains(byte)) {
        Ok(IccSignature::from_bytes(raw))
    } else {
        Err(IccParseError::InvalidHeaderField)
    }
}

pub(crate) fn xyz(bytes: &[u8], offset: usize) -> Result<IccXyz, IccParseError> {
    Ok(IccXyz::new([
        fixed(bytes, offset)?,
        fixed(bytes, offset + 4)?,
        fixed(bytes, offset + 8)?,
    ]))
}

pub(crate) fn fixed(bytes: &[u8], offset: usize) -> Result<FiniteF32, IccParseError> {
    let raw = be_i32(bytes, offset)?;
    let value = f64::from(raw) / 65_536.0;
    #[allow(clippy::cast_possible_truncation)]
    FiniteF32::new(value as f32).map_err(|_| IccParseError::InvalidHeaderField)
}

pub(crate) fn be_u16(bytes: &[u8], offset: usize) -> Result<u16, IccParseError> {
    let raw: [u8; 2] = bytes
        .get(offset..offset + 2)
        .ok_or(IccParseError::Truncated)?
        .try_into()
        .map_err(|_| IccParseError::Truncated)?;
    Ok(u16::from_be_bytes(raw))
}

pub(crate) fn be_u32(bytes: &[u8], offset: usize) -> Result<u32, IccParseError> {
    let raw: [u8; 4] = bytes
        .get(offset..offset + 4)
        .ok_or(IccParseError::Truncated)?
        .try_into()
        .map_err(|_| IccParseError::Truncated)?;
    Ok(u32::from_be_bytes(raw))
}

pub(crate) fn be_i32(bytes: &[u8], offset: usize) -> Result<i32, IccParseError> {
    let raw: [u8; 4] = bytes
        .get(offset..offset + 4)
        .ok_or(IccParseError::Truncated)?
        .try_into()
        .map_err(|_| IccParseError::Truncated)?;
    Ok(i32::from_be_bytes(raw))
}

fn be_u64(bytes: &[u8], offset: usize) -> Result<u64, IccParseError> {
    let raw: [u8; 8] = bytes
        .get(offset..offset + 8)
        .ok_or(IccParseError::Truncated)?
        .try_into()
        .map_err(|_| IccParseError::Truncated)?;
    Ok(u64::from_be_bytes(raw))
}

impl fmt::Display for IccParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimits => formatter.write_str("ICC parser limits are invalid"),
            Self::Empty => formatter.write_str("ICC profile is empty"),
            Self::ProfileTooLarge { actual, limit } => write!(
                formatter,
                "ICC profile has {actual} bytes, limit is {limit}"
            ),
            Self::Truncated => formatter.write_str("ICC profile is truncated"),
            Self::DeclaredSizeMismatch { declared, actual } => write!(
                formatter,
                "ICC declared size {declared} differs from byte length {actual}"
            ),
            Self::UnsupportedVersion { major, minor } => {
                write!(formatter, "ICC version {major}.{minor} is unsupported")
            }
            Self::InvalidHeaderSignature => formatter.write_str("ICC header signature is not acsp"),
            Self::InvalidHeaderField => {
                formatter.write_str("ICC header contains an invalid or reserved field")
            }
            Self::UnsupportedProfileClass(value) => {
                write!(formatter, "ICC profile class {value} is unsupported")
            }
            Self::UnsupportedColorSpace(value) => {
                write!(formatter, "ICC color space {value} is unsupported")
            }
            Self::InvalidConnectionSpace => {
                formatter.write_str("ICC profile class and connection space are incompatible")
            }
            Self::InvalidDate => formatter.write_str("ICC creation date is invalid"),
            Self::InvalidRenderingIntent(value) => {
                write!(formatter, "ICC rendering intent {value} is invalid")
            }
            Self::InvalidIlluminant => formatter.write_str("ICC PCS illuminant is not D50"),
            Self::InvalidProfileId => {
                formatter.write_str("ICC embedded profile ID does not match profile bytes")
            }
            Self::TooManyTags { count, limit } => {
                write!(formatter, "ICC tag count {count} exceeds limit {limit}")
            }
            Self::InvalidTagSignature(value) => {
                write!(formatter, "ICC signature {value} is invalid")
            }
            Self::DuplicateTag(value) => write!(formatter, "ICC tag {value} occurs more than once"),
            Self::MisalignedTag(value) => {
                write!(formatter, "ICC tag {value} is not four-byte aligned")
            }
            Self::InvalidTagRange(value) => {
                write!(formatter, "ICC tag {value} has an invalid byte range")
            }
            Self::OverlappingTags(left, right) => {
                write!(formatter, "ICC tags {left} and {right} overlap")
            }
            Self::TagTooLarge { signature, limit } => {
                write!(formatter, "ICC tag {signature} exceeds byte limit {limit}")
            }
            Self::InvalidTagType { tag, actual } => {
                write!(formatter, "ICC tag {tag} has invalid type {actual}")
            }
            Self::UnsupportedTagType { tag, actual } => {
                write!(formatter, "ICC tag {tag} uses unsupported type {actual}")
            }
            Self::InvalidTagData(tag) => write!(formatter, "ICC tag {tag} is malformed"),
            Self::CurveTooLarge { count, limit } => {
                write!(formatter, "ICC curve has {count} samples, limit is {limit}")
            }
            Self::InvalidCurve(tag) => write!(formatter, "ICC curve tag {tag} is invalid"),
            Self::LutTooLarge { count, limit } => {
                write!(formatter, "ICC LUT has {count} samples, limit is {limit}")
            }
            Self::InvalidLut(tag) => write!(formatter, "ICC LUT tag {tag} is invalid"),
            Self::InvalidText(tag) => write!(formatter, "ICC text tag {tag} is invalid"),
            Self::MissingRequiredTag(tag) => {
                write!(formatter, "ICC profile is missing required tag {tag}")
            }
            Self::UnsupportedProfileStructure => {
                formatter.write_str("ICC profile has no supported matrix/TRC or LUT structure")
            }
            Self::AllocationFailed => formatter.write_str("ICC bounded allocation failed"),
            Self::Identity => formatter.write_str("ICC profile identity could not be constructed"),
        }
    }
}

impl std::error::Error for IccParseError {}
