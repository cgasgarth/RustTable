use rusttable_catalog::{
    DUPLICATE_EVIDENCE_VERSION, DuplicateEvidence, EmbeddedPhotoIdentity, ExactContentIdentity,
    ReferencePathIdentity, VisualFingerprint,
};
use rusttable_core::PhotoId;

const ENCODED_LENGTH: usize = 147;

pub(crate) fn encode(evidence: DuplicateEvidence) -> [u8; ENCODED_LENGTH] {
    let mut bytes = [0_u8; ENCODED_LENGTH];
    bytes[0] = evidence.version();
    bytes[1..17].copy_from_slice(&evidence.photo_id().get().to_be_bytes());
    bytes[17..49].copy_from_slice(&evidence.source().as_bytes());
    bytes[49..81].copy_from_slice(&evidence.exact().sha256());
    bytes[81..89].copy_from_slice(&evidence.exact().byte_length().to_be_bytes());
    if let Some(embedded) = evidence.embedded() {
        bytes[89] = 1;
        bytes[90..122].copy_from_slice(&embedded.digest());
    }
    if let Some(visual) = evidence.visual() {
        bytes[122] = 1;
        bytes[123..131].copy_from_slice(&visual.gradient().to_be_bytes());
        bytes[131..139].copy_from_slice(&visual.luminance().to_be_bytes());
        bytes[139..143].copy_from_slice(&visual.width().to_be_bytes());
        bytes[143..147].copy_from_slice(&visual.height().to_be_bytes());
    }
    bytes
}

pub(crate) fn decode(bytes: &[u8]) -> Result<DuplicateEvidence, ()> {
    if bytes.len() != ENCODED_LENGTH || bytes[0] != DUPLICATE_EVIDENCE_VERSION {
        return Err(());
    }
    let photo_id = PhotoId::new(u128::from_be_bytes(array(bytes, 1)?)).ok_or(())?;
    let source = ReferencePathIdentity::new(array(bytes, 17)?);
    let exact = ExactContentIdentity::new(array(bytes, 49)?, u64::from_be_bytes(array(bytes, 81)?));
    let embedded = match bytes[89] {
        0 if bytes[90..122].iter().all(|byte| *byte == 0) => None,
        1 => Some(EmbeddedPhotoIdentity::new(array(bytes, 90)?)),
        _ => return Err(()),
    };
    let visual = match bytes[122] {
        0 if bytes[123..].iter().all(|byte| *byte == 0) => None,
        1 => VisualFingerprint::new(
            u64::from_be_bytes(array(bytes, 123)?),
            u64::from_be_bytes(array(bytes, 131)?),
            u32::from_be_bytes(array(bytes, 139)?),
            u32::from_be_bytes(array(bytes, 143)?),
        )
        .ok_or(())?
        .into(),
        _ => return Err(()),
    };
    Ok(DuplicateEvidence::new(
        photo_id, source, exact, embedded, visual,
    ))
}

pub(crate) fn source_index_key(evidence: DuplicateEvidence) -> [u8; 48] {
    prefixed_photo_key(evidence.source().as_bytes(), evidence.photo_id())
}

pub(crate) fn exact_index_key(evidence: DuplicateEvidence) -> [u8; 56] {
    let mut prefix = [0_u8; 40];
    prefix[..32].copy_from_slice(&evidence.exact().sha256());
    prefix[32..].copy_from_slice(&evidence.exact().byte_length().to_be_bytes());
    prefixed_photo_key(prefix, evidence.photo_id())
}

pub(crate) fn embedded_index_key(evidence: DuplicateEvidence) -> Option<[u8; 48]> {
    evidence
        .embedded()
        .map(|embedded| prefixed_photo_key(embedded.digest(), evidence.photo_id()))
}

pub(crate) fn visual_index_keys(evidence: DuplicateEvidence) -> Option<[[u8; 19]; 8]> {
    let visual = evidence.visual()?;
    Some(std::array::from_fn(|index| {
        let mut key = [0_u8; 19];
        key[0] = u8::try_from(index).unwrap_or_default();
        key[1..3].copy_from_slice(&visual.index_chunks()[index].to_be_bytes());
        key[3..].copy_from_slice(&evidence.photo_id().get().to_be_bytes());
        key
    }))
}

fn prefixed_photo_key<const PREFIX: usize, const OUTPUT: usize>(
    prefix: [u8; PREFIX],
    photo_id: PhotoId,
) -> [u8; OUTPUT] {
    debug_assert_eq!(OUTPUT, PREFIX + 16);
    let mut key = [0_u8; OUTPUT];
    key[..PREFIX].copy_from_slice(&prefix);
    key[PREFIX..].copy_from_slice(&photo_id.get().to_be_bytes());
    key
}

fn array<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N], ()> {
    bytes
        .get(offset..offset + N)
        .ok_or(())?
        .try_into()
        .map_err(|_| ())
}
