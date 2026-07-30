use rusttable_color::{
    IccColorSpace, IccParseErrorKind, IccProfileLimits, Pcs, Primaries, ProfileClass, ProfileId,
    ProfileModel, ProfileParserVersion, TransferFunction, WhitePoint, parse_icc_profile,
};
use rusttable_image::{SourceColor, SourceColorEvidence};

const ICC_HEADER_BYTES: usize = 128;
const ICC_TAG_TABLE_BYTES: usize = 132;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceColorParseError {
    TruncatedIcc,
    InvalidIcc,
    UnsupportedIcc,
    InvalidChromaticities,
}

pub(crate) fn embedded_icc(bytes: &[u8]) -> Result<SourceColor, SourceColorParseError> {
    let profile = IccMatrixProfile::parse(bytes)?;
    SourceColor::external(
        profile.id,
        profile.primaries,
        profile.transfer,
        SourceColorEvidence::EmbeddedIcc,
    )
    .map_err(|_| SourceColorParseError::InvalidIcc)
}

/// Retains a bounded, validated ICC profile even when matrix source-color
/// fields cannot express its grayscale or LUT transform.
pub(crate) fn embedded_icc_profile_authoritative(
    bytes: &[u8],
    expected_color_space: IccColorSpace,
) -> Result<SourceColor, SourceColorParseError> {
    let profile = parse_icc_profile(bytes, IccProfileLimits::strict_for_embedded_images())
        .map_err(|error| match error.kind() {
            IccParseErrorKind::Unsupported => SourceColorParseError::UnsupportedIcc,
            IccParseErrorKind::Malformed | IccParseErrorKind::ResourceLimit => {
                SourceColorParseError::InvalidIcc
            }
        })?;
    if profile.header().data_color_space() != expected_color_space {
        return Err(SourceColorParseError::InvalidIcc);
    }
    if let Ok(source) = embedded_icc(bytes) {
        return Ok(source);
    }
    Ok(SourceColor::profile_authoritative_icc(profile.profile_id()))
}

pub(crate) fn embedded_chromaticities(
    primaries: Primaries,
    transfer: TransferFunction,
    evidence: SourceColorEvidence,
) -> Result<SourceColor, SourceColorParseError> {
    let mut identity = Vec::with_capacity(72);
    identity.extend_from_slice(b"rusttable.source-chromaticities.v1");
    for pair in [primaries.red(), primaries.green(), primaries.blue()] {
        identity.extend_from_slice(&pair.0.get().to_bits().to_le_bytes());
        identity.extend_from_slice(&pair.1.get().to_bits().to_le_bytes());
    }
    let (white_x, white_y) = primaries.white().xy();
    identity.extend_from_slice(&white_x.to_bits().to_le_bytes());
    identity.extend_from_slice(&white_y.to_bits().to_le_bytes());
    identity.extend_from_slice(
        &postcard::to_allocvec(&transfer).map_err(|_| SourceColorParseError::InvalidIcc)?,
    );
    let id = ProfileId::from_content(
        &identity,
        ProfileClass::Input,
        ProfileModel::Matrix,
        Pcs::XyzD50,
        parser_version()?,
    )
    .map_err(|_| SourceColorParseError::InvalidIcc)?;
    SourceColor::external(id, primaries, transfer, evidence)
        .map_err(|_| SourceColorParseError::InvalidChromaticities)
}

struct IccMatrixProfile {
    id: ProfileId,
    primaries: Primaries,
    transfer: TransferFunction,
}

impl IccMatrixProfile {
    fn parse(bytes: &[u8]) -> Result<Self, SourceColorParseError> {
        if bytes.len() < ICC_TAG_TABLE_BYTES {
            return Err(SourceColorParseError::TruncatedIcc);
        }
        let declared = usize::try_from(be_u32(bytes, 0)?).map_err(|_| invalid())?;
        if declared != bytes.len() || declared < ICC_TAG_TABLE_BYTES || &bytes[36..40] != b"acsp" {
            return Err(SourceColorParseError::InvalidIcc);
        }
        if &bytes[16..20] != b"RGB " {
            return Err(SourceColorParseError::UnsupportedIcc);
        }
        let pcs = match &bytes[20..24] {
            b"XYZ " => Pcs::XyzD50,
            b"Lab " => Pcs::LabD50,
            _ => Pcs::Unknown,
        };
        if pcs == Pcs::Unknown {
            return Err(SourceColorParseError::UnsupportedIcc);
        }
        let tags = tags(bytes)?;
        let red = xyz_xy(tag(bytes, &tags, *b"rXYZ")?)?;
        let green = xyz_xy(tag(bytes, &tags, *b"gXYZ")?)?;
        let blue = xyz_xy(tag(bytes, &tags, *b"bXYZ")?)?;
        let white = tag_optional(bytes, &tags, *b"wtpt")?
            .map(xyz_xy)
            .transpose()?
            .map_or(Ok(WhitePoint::D50), |(x, y)| {
                WhitePoint::custom(x, y).map_err(|_| SourceColorParseError::InvalidIcc)
            })?;
        let primaries = Primaries::new(red, green, blue, white)
            .map_err(|_| SourceColorParseError::InvalidIcc)?;
        let transfer = rgb_transfer(bytes, &tags)?;
        let model = if tags
            .iter()
            .any(|tag| matches!(&tag.signature, b"A2B0" | b"B2A0"))
        {
            ProfileModel::Lut
        } else {
            ProfileModel::Matrix
        };
        if model != ProfileModel::Matrix {
            return Err(SourceColorParseError::UnsupportedIcc);
        }
        let id = ProfileId::from_content(
            bytes,
            profile_class(bytes[12..16].try_into().map_err(|_| invalid())?),
            model,
            pcs,
            parser_version()?,
        )
        .map_err(|_| SourceColorParseError::InvalidIcc)?;
        Ok(Self {
            id,
            primaries,
            transfer,
        })
    }
}

#[derive(Clone, Copy)]
struct IccTag {
    signature: [u8; 4],
    offset: usize,
    length: usize,
}

fn tags(bytes: &[u8]) -> Result<Vec<IccTag>, SourceColorParseError> {
    let count = usize::try_from(be_u32(bytes, ICC_HEADER_BYTES)?).map_err(|_| invalid())?;
    let table_end = count
        .checked_mul(12)
        .and_then(|value| value.checked_add(ICC_TAG_TABLE_BYTES))
        .ok_or_else(invalid)?;
    if table_end > bytes.len() {
        return Err(SourceColorParseError::TruncatedIcc);
    }
    let mut tags = Vec::new();
    tags.try_reserve_exact(count).map_err(|_| invalid())?;
    for index in 0..count {
        let start = ICC_TAG_TABLE_BYTES + index * 12;
        let signature = bytes[start..start + 4].try_into().map_err(|_| invalid())?;
        let offset = usize::try_from(be_u32(bytes, start + 4)?).map_err(|_| invalid())?;
        let length = usize::try_from(be_u32(bytes, start + 8)?).map_err(|_| invalid())?;
        let end = offset.checked_add(length).ok_or_else(invalid)?;
        if offset < table_end || length < 12 || end > bytes.len() {
            return Err(SourceColorParseError::InvalidIcc);
        }
        if tags.iter().any(|tag: &IccTag| tag.signature == signature) {
            return Err(SourceColorParseError::InvalidIcc);
        }
        tags.push(IccTag {
            signature,
            offset,
            length,
        });
    }
    Ok(tags)
}

fn tag<'a>(
    bytes: &'a [u8],
    tags: &[IccTag],
    signature: [u8; 4],
) -> Result<&'a [u8], SourceColorParseError> {
    tag_optional(bytes, tags, signature)?.ok_or(SourceColorParseError::UnsupportedIcc)
}

fn tag_optional<'a>(
    bytes: &'a [u8],
    tags: &[IccTag],
    signature: [u8; 4],
) -> Result<Option<&'a [u8]>, SourceColorParseError> {
    tags.iter()
        .find(|tag| tag.signature == signature)
        .map(|tag| {
            bytes
                .get(tag.offset..tag.offset + tag.length)
                .ok_or(SourceColorParseError::TruncatedIcc)
        })
        .transpose()
}

fn xyz_xy(value: &[u8]) -> Result<(f32, f32), SourceColorParseError> {
    if value.get(..4) != Some(b"XYZ ") || value.len() < 20 {
        return Err(SourceColorParseError::InvalidIcc);
    }
    let xyz = [fixed(value, 8)?, fixed(value, 12)?, fixed(value, 16)?];
    let sum = xyz.iter().sum::<f32>();
    if !sum.is_finite() || sum <= 0.0 {
        return Err(SourceColorParseError::InvalidIcc);
    }
    Ok((xyz[0] / sum, xyz[1] / sum))
}

fn rgb_transfer(bytes: &[u8], tags: &[IccTag]) -> Result<TransferFunction, SourceColorParseError> {
    let transfers =
        [*b"rTRC", *b"gTRC", *b"bTRC"].map(|signature| curve(tag(bytes, tags, signature)?));
    let [red, green, blue] = transfers;
    let red = red?;
    if green? == red && blue? == red {
        Ok(red)
    } else {
        Err(SourceColorParseError::UnsupportedIcc)
    }
}

fn curve(value: &[u8]) -> Result<TransferFunction, SourceColorParseError> {
    match value.get(..4) {
        Some(b"curv") => match be_u32(value, 8)? {
            0 => Ok(TransferFunction::Linear),
            1 if value.len() >= 14 => {
                let gamma = f32::from(be_u16(value, 12)?) / 256.0;
                TransferFunction::gamma(gamma).map_err(|_| invalid())
            }
            _ => Err(SourceColorParseError::UnsupportedIcc),
        },
        Some(b"para") => parametric_curve(value),
        _ => Err(SourceColorParseError::UnsupportedIcc),
    }
}

fn parametric_curve(value: &[u8]) -> Result<TransferFunction, SourceColorParseError> {
    let kind = be_u16(value, 8)?;
    let gamma = fixed(value, 12)?;
    if kind == 0 {
        return TransferFunction::gamma(gamma).map_err(|_| invalid());
    }
    if kind == 3 && value.len() >= 32 {
        let parameters = [
            gamma,
            fixed(value, 16)?,
            fixed(value, 20)?,
            fixed(value, 24)?,
            fixed(value, 28)?,
        ];
        let srgb = [2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045];
        if parameters
            .iter()
            .zip(srgb)
            .all(|(actual, expected)| (actual - expected).abs() <= 0.000_2)
        {
            return Ok(TransferFunction::Srgb);
        }
    }
    Err(SourceColorParseError::UnsupportedIcc)
}

fn profile_class(value: [u8; 4]) -> ProfileClass {
    match &value {
        b"mntr" => ProfileClass::Display,
        b"prtr" => ProfileClass::Output,
        b"spac" => ProfileClass::Working,
        b"abst" => ProfileClass::Analysis,
        _ => ProfileClass::Input,
    }
}

fn parser_version() -> Result<ProfileParserVersion, SourceColorParseError> {
    ProfileParserVersion::new(1).map_err(|_| SourceColorParseError::InvalidIcc)
}

fn be_u16(bytes: &[u8], offset: usize) -> Result<u16, SourceColorParseError> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or(SourceColorParseError::TruncatedIcc)?;
    Ok(u16::from_be_bytes(value.try_into().map_err(|_| invalid())?))
}

fn be_u32(bytes: &[u8], offset: usize) -> Result<u32, SourceColorParseError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or(SourceColorParseError::TruncatedIcc)?;
    Ok(u32::from_be_bytes(value.try_into().map_err(|_| invalid())?))
}

fn fixed(bytes: &[u8], offset: usize) -> Result<f32, SourceColorParseError> {
    let raw = i32::from_be_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or(SourceColorParseError::TruncatedIcc)?
            .try_into()
            .map_err(|_| invalid())?,
    );
    let integral = i16::try_from(raw >> 16).map_err(|_| invalid())?;
    let fraction = u16::try_from(raw & 0xffff).map_err(|_| invalid())?;
    Ok(f32::from(integral) + f32::from(fraction) / 65_536.0)
}

const fn invalid() -> SourceColorParseError {
    SourceColorParseError::InvalidIcc
}

impl std::fmt::Display for SourceColorParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::TruncatedIcc => "embedded ICC profile is truncated",
            Self::InvalidIcc => "embedded ICC profile is malformed",
            Self::UnsupportedIcc => "embedded ICC profile uses an unsupported structure",
            Self::InvalidChromaticities => "embedded chromaticities are invalid",
        })
    }
}

impl std::error::Error for SourceColorParseError {}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest as _;

    #[test]
    fn matrix_icc_preserves_profile_primaries_transfer_and_identity() {
        let profile = matrix_profile(2.2);
        let source = embedded_icc(&profile).expect("supported matrix ICC");

        assert_eq!(source.evidence(), SourceColorEvidence::EmbeddedIcc);
        assert!(source.profile().is_some());
        assert_eq!(
            source.transfer(),
            Some(TransferFunction::gamma(563.0_f32 / 256.0_f32).expect("ICC u8.8 gamma"))
        );
        let primaries = source.primaries().expect("matrix primaries");
        assert!((primaries.red().0.get() - 0.68).abs() < 0.000_1);
        assert!((primaries.green().1.get() - 0.69).abs() < 0.000_1);
        let (white_x, white_y) = source.white_point().expect("matrix white point").xy();
        assert!((white_x - 0.3127).abs() < 0.000_1);
        assert!((white_y - 0.3290).abs() < 0.000_1);
    }

    #[test]
    fn chromaticity_profiles_are_content_addressed_and_keep_linear_fallback_evidence() {
        let source = embedded_chromaticities(
            Primaries::srgb(),
            TransferFunction::Linear,
            SourceColorEvidence::EmbeddedChromaticities,
        )
        .expect("valid chromaticities");

        assert_eq!(source.transfer(), Some(TransferFunction::Linear));
        assert_eq!(source.primaries(), Some(Primaries::srgb()));
        assert_eq!(
            source.evidence(),
            SourceColorEvidence::EmbeddedChromaticities
        );
        assert!(source.profile().is_some());
    }

    #[test]
    fn profile_authoritative_gray_icc_is_retained_without_an_implicit_transfer() {
        let profile = test_profiles::gray();
        let source = embedded_icc_profile_authoritative(&profile, IccColorSpace::Gray)
            .expect("valid grayscale ICC");
        let identity = source.profile().expect("external profile identity");

        assert_eq!(
            identity.sha256(),
            <[u8; 32]>::from(sha2::Sha256::digest(&profile))
        );
        assert_eq!(identity.model(), ProfileModel::Matrix);
        assert_eq!(source.transfer(), None);
        assert_eq!(source.primaries(), None);
        assert_eq!(source.white_point(), None);
        assert_eq!(source.matrix(), None);
        assert_eq!(source.evidence(), SourceColorEvidence::EmbeddedIcc);
        assert_eq!(source.fallback_used(), None);
    }

    #[test]
    fn profile_authoritative_lut_icc_is_retained_without_matrix_projection() {
        let profile = test_profiles::lut();
        let source = embedded_icc_profile_authoritative(&profile, IccColorSpace::Rgb)
            .expect("valid LUT ICC");
        let identity = source.profile().expect("external profile identity");

        assert_eq!(
            identity.sha256(),
            <[u8; 32]>::from(sha2::Sha256::digest(&profile))
        );
        assert_eq!(identity.model(), ProfileModel::Lut);
        assert_eq!(source.transfer(), None);
        assert_eq!(source.primaries(), None);
        assert_eq!(source.white_point(), None);
        assert_eq!(source.matrix(), None);
        assert_eq!(source.evidence(), SourceColorEvidence::EmbeddedIcc);
        assert_eq!(source.fallback_used(), None);
    }

    #[test]
    fn profile_authoritative_icc_rejects_header_color_mismatch() {
        assert_eq!(
            embedded_icc_profile_authoritative(&test_profiles::gray(), IccColorSpace::Rgb),
            Err(SourceColorParseError::InvalidIcc)
        );
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the synthetic ICC fixture writes bounded, validated fixed-point values"
    )]
    fn matrix_profile(gamma: f32) -> Vec<u8> {
        const TABLE_END: usize = 132 + 7 * 12;
        let xyz = [
            (*b"rXYZ", xy_xyz(0.68, 0.32)),
            (*b"gXYZ", xy_xyz(0.265, 0.69)),
            (*b"bXYZ", xy_xyz(0.15, 0.06)),
            (*b"wtpt", xy_xyz(0.3127, 0.3290)),
        ];
        let curve_offset = TABLE_END + xyz.len() * 20;
        let size = curve_offset + 14;
        let mut profile = vec![0_u8; size];
        put_u32(&mut profile, 0, u32::try_from(size).expect("small profile"));
        profile[12..16].copy_from_slice(b"mntr");
        profile[16..20].copy_from_slice(b"RGB ");
        profile[20..24].copy_from_slice(b"XYZ ");
        profile[36..40].copy_from_slice(b"acsp");
        put_u32(&mut profile, 128, 7);
        for (index, (signature, values)) in xyz.into_iter().enumerate() {
            let offset = TABLE_END + index * 20;
            put_tag(&mut profile, index, signature, offset, 20);
            profile[offset..offset + 4].copy_from_slice(b"XYZ ");
            for (component, value) in values.into_iter().enumerate() {
                let fixed = (value * 65_536.0).round() as i32;
                profile[offset + 8 + component * 4..offset + 12 + component * 4]
                    .copy_from_slice(&fixed.to_be_bytes());
            }
        }
        for (index, signature) in [*b"rTRC", *b"gTRC", *b"bTRC"].into_iter().enumerate() {
            put_tag(&mut profile, index + 4, signature, curve_offset, 14);
        }
        profile[curve_offset..curve_offset + 4].copy_from_slice(b"curv");
        put_u32(&mut profile, curve_offset + 8, 1);
        profile[curve_offset + 12..curve_offset + 14]
            .copy_from_slice(&((gamma * 256.0).round() as u16).to_be_bytes());
        profile
    }

    fn xy_xyz(x: f32, y: f32) -> [f32; 3] {
        [x / y, 1.0, (1.0 - x - y) / y]
    }

    fn put_tag(bytes: &mut [u8], index: usize, signature: [u8; 4], offset: usize, size: usize) {
        let start = 132 + index * 12;
        bytes[start..start + 4].copy_from_slice(&signature);
        put_u32(
            bytes,
            start + 4,
            u32::try_from(offset).expect("small offset"),
        );
        put_u32(bytes, start + 8, u32::try_from(size).expect("small tag"));
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }
}

#[cfg(test)]
pub(crate) mod test_profiles {
    const D50: [f32; 3] = [0.964_2, 1.0, 0.824_9];

    pub(crate) fn gray() -> Vec<u8> {
        ProfileBuilder::new(*b"GRAY")
            .tag(*b"wtpt", xyz_tag(D50))
            .tag(*b"kTRC", gamma_curve(2.2))
            .build()
    }

    pub(crate) fn lut() -> Vec<u8> {
        ProfileBuilder::new(*b"RGB ")
            .tag(*b"A2B0", multi_stage_identity())
            .build()
    }

    struct ProfileBuilder {
        data_space: [u8; 4],
        tags: Vec<([u8; 4], Vec<u8>)>,
    }

    impl ProfileBuilder {
        fn new(data_space: [u8; 4]) -> Self {
            Self {
                data_space,
                tags: Vec::new(),
            }
        }

        fn tag(mut self, signature: [u8; 4], data: Vec<u8>) -> Self {
            self.tags.push((signature, data));
            self
        }

        fn build(self) -> Vec<u8> {
            let table_end = 132 + self.tags.len() * 12;
            let mut cursor = align4(table_end);
            let entries = self
                .tags
                .iter()
                .map(|(signature, data)| {
                    let entry = (*signature, cursor, data.len());
                    cursor = align4(cursor + data.len());
                    entry
                })
                .collect::<Vec<_>>();
            let mut profile = vec![0_u8; cursor];
            put_u32(
                &mut profile,
                0,
                u32::try_from(cursor).expect("bounded fixture"),
            );
            profile[8] = 4;
            profile[9] = 0x40;
            profile[12..16].copy_from_slice(b"mntr");
            profile[16..20].copy_from_slice(&self.data_space);
            profile[20..24].copy_from_slice(b"XYZ ");
            for (offset, value) in [(24, 2026), (26, 7), (28, 23), (30, 12), (32, 0), (34, 0)] {
                put_u16(&mut profile, offset, value);
            }
            profile[36..40].copy_from_slice(b"acsp");
            for (index, value) in D50.into_iter().enumerate() {
                put_fixed(&mut profile, 68 + index * 4, value);
            }
            put_u32(
                &mut profile,
                128,
                u32::try_from(entries.len()).expect("tag count"),
            );
            for (index, ((signature, data), (_, offset, size))) in
                self.tags.into_iter().zip(entries).enumerate()
            {
                let table = 132 + index * 12;
                profile[table..table + 4].copy_from_slice(&signature);
                put_u32(
                    &mut profile,
                    table + 4,
                    u32::try_from(offset).expect("tag offset"),
                );
                put_u32(
                    &mut profile,
                    table + 8,
                    u32::try_from(size).expect("tag size"),
                );
                profile[offset..offset + size].copy_from_slice(&data);
            }
            profile
        }
    }

    fn xyz_tag(values: [f32; 3]) -> Vec<u8> {
        let mut data = vec![0; 20];
        data[..4].copy_from_slice(b"XYZ ");
        for (index, value) in values.into_iter().enumerate() {
            put_fixed(&mut data, 8 + index * 4, value);
        }
        data
    }

    fn gamma_curve(gamma: f32) -> Vec<u8> {
        let mut data = vec![0; 14];
        data[..4].copy_from_slice(b"curv");
        put_u32(&mut data, 8, 1);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        put_u16(&mut data, 12, (gamma * 256.0).round() as u16);
        data
    }

    fn multi_stage_identity() -> Vec<u8> {
        let curve = {
            let mut value = vec![0; 12];
            value[..4].copy_from_slice(b"curv");
            value
        };
        let mut data = vec![0; 32 + curve.len() * 3];
        data[..4].copy_from_slice(b"mAB ");
        data[8..10].copy_from_slice(&[3, 3]);
        put_u32(&mut data, 12, 32);
        for index in 0..3 {
            let start = 32 + index * curve.len();
            data[start..start + curve.len()].copy_from_slice(&curve);
        }
        data
    }

    const fn align4(value: usize) -> usize {
        (value + 3) & !3
    }

    fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }

    #[allow(clippy::cast_possible_truncation)]
    fn put_fixed(bytes: &mut [u8], offset: usize, value: f32) {
        let fixed = (f64::from(value) * 65_536.0).round() as i32;
        bytes[offset..offset + 4].copy_from_slice(&fixed.to_be_bytes());
    }
}
