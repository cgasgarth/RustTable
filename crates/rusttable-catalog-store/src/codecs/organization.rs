use postcard::{from_bytes, to_allocvec};
use rusttable_catalog::{ColorLabel, PhotoOrganizationState, Rating};
use rusttable_core::PhotoId;
use serde::{Deserialize, Serialize};

const ORGANIZATION_FORMAT_VERSION: u8 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct StoredOrganization {
    version: u8,
    rating: u8,
    rejected: bool,
    labels: u8,
}

pub(crate) fn encode(state: &PhotoOrganizationState) -> Result<Vec<u8>, ()> {
    let labels = state
        .color_labels
        .iter()
        .copied()
        .fold(0_u8, |bits, label| bits | 1 << label_index(label));
    to_allocvec(&StoredOrganization {
        version: ORGANIZATION_FORMAT_VERSION,
        rating: state.rating.as_u8(),
        rejected: state.rejected,
        labels,
    })
    .map_err(|_| ())
}

pub(crate) fn decode(photo_id: PhotoId, bytes: &[u8]) -> Result<PhotoOrganizationState, ()> {
    let stored: StoredOrganization = from_bytes(bytes).map_err(|_| ())?;
    if stored.version != ORGANIZATION_FORMAT_VERSION {
        return Err(());
    }
    let rating = Rating::from_u8(stored.rating).ok_or(())?;
    let color_labels = ColorLabel::ALL
        .into_iter()
        .enumerate()
        .filter_map(|(index, label)| (stored.labels & (1 << index) != 0).then_some(label))
        .collect();
    Ok(PhotoOrganizationState {
        photo_id,
        rating,
        rejected: stored.rejected,
        color_labels,
    })
}

const fn label_index(label: ColorLabel) -> u8 {
    match label {
        ColorLabel::Red => 0,
        ColorLabel::Yellow => 1,
        ColorLabel::Green => 2,
        ColorLabel::Blue => 3,
        ColorLabel::Purple => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};
    use rusttable_catalog::{ColorLabel, PhotoOrganizationState, Rating};
    use rusttable_core::PhotoId;

    #[test]
    fn organization_codec_round_trips_independent_labels() {
        let state = PhotoOrganizationState {
            photo_id: PhotoId::new(7).unwrap(),
            rating: Rating::Four,
            rejected: true,
            color_labels: [ColorLabel::Red, ColorLabel::Blue].into_iter().collect(),
        };
        assert_eq!(
            decode(state.photo_id, &encode(&state).unwrap()).unwrap(),
            state
        );
    }
}
