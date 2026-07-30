//! Lighttable organization and toolbar projection helpers.

use std::collections::{BTreeMap, BTreeSet};

use rusttable_catalog::{CatalogCommand, ColorLabel, PhotoOrganizationState, Rating};
use rusttable_core::PhotoId;
use rusttable_ui::{LighttableColorLabel, LighttablePhotoState, LighttableRating};

pub(super) fn rating_from_ui(rating: LighttableRating) -> Rating {
    Rating::from_u8(rating.stars().unwrap_or(0)).unwrap_or(Rating::Zero)
}

pub(super) const fn color_from_ui(label: LighttableColorLabel) -> ColorLabel {
    match label {
        LighttableColorLabel::Red => ColorLabel::Red,
        LighttableColorLabel::Yellow => ColorLabel::Yellow,
        LighttableColorLabel::Green => ColorLabel::Green,
        LighttableColorLabel::Blue => ColorLabel::Blue,
        LighttableColorLabel::Purple => ColorLabel::Purple,
    }
}

const fn color_to_ui(label: ColorLabel) -> LighttableColorLabel {
    match label {
        ColorLabel::Red => LighttableColorLabel::Red,
        ColorLabel::Yellow => LighttableColorLabel::Yellow,
        ColorLabel::Green => LighttableColorLabel::Green,
        ColorLabel::Blue => LighttableColorLabel::Blue,
        ColorLabel::Purple => LighttableColorLabel::Purple,
    }
}

pub(super) fn photo_state(
    photo_id: PhotoId,
    organization: &BTreeMap<PhotoId, PhotoOrganizationState>,
    selected: &BTreeSet<PhotoId>,
) -> LighttablePhotoState {
    let state = organization.get(&photo_id);
    let rating = state.map_or(LighttableRating::Zero, |state| {
        if state.rejected {
            LighttableRating::Rejected
        } else {
            ui_rating(state.rating)
        }
    });
    let labels = state
        .into_iter()
        .flat_map(|state| state.color_labels.iter().copied().map(color_to_ui));
    LighttablePhotoState::new(photo_id, selected.contains(&photo_id), rating, labels)
}

fn ui_rating(rating: Rating) -> LighttableRating {
    match rating {
        Rating::Zero => LighttableRating::Zero,
        Rating::One => LighttableRating::One,
        Rating::Two => LighttableRating::Two,
        Rating::Three => LighttableRating::Three,
        Rating::Four => LighttableRating::Four,
        Rating::Five => LighttableRating::Five,
    }
}

pub(super) fn uniform_rating(states: &[&PhotoOrganizationState]) -> Option<LighttableRating> {
    let first = states.first()?;
    let rating = if first.rejected {
        LighttableRating::Rejected
    } else {
        ui_rating(first.rating)
    };
    states
        .iter()
        .all(|state| {
            if state.rejected {
                rating == LighttableRating::Rejected
            } else {
                rating == ui_rating(state.rating)
            }
        })
        .then_some(rating)
}

pub(super) fn shared_labels(states: &[&PhotoOrganizationState]) -> BTreeSet<LighttableColorLabel> {
    let Some(first) = states.first() else {
        return BTreeSet::new();
    };
    first
        .color_labels
        .iter()
        .copied()
        .filter(|label| {
            states
                .iter()
                .all(|state| state.color_labels.contains(label))
        })
        .map(color_to_ui)
        .collect()
}

pub(super) fn apply_fallback_organization(
    organization: &mut BTreeMap<PhotoId, PhotoOrganizationState>,
    command: CatalogCommand,
) {
    match command {
        CatalogCommand::SetRating { photo_ids, rating } => {
            for photo_id in photo_ids {
                if let Some(state) = organization.get_mut(&photo_id) {
                    state.rating = rating;
                    state.rejected = false;
                }
            }
        }
        CatalogCommand::SetRejection {
            photo_ids,
            rejected,
        } => {
            for photo_id in photo_ids {
                if let Some(state) = organization.get_mut(&photo_id) {
                    state.rejected = rejected;
                }
            }
        }
        CatalogCommand::SetColorLabel {
            photo_ids,
            label,
            enabled,
        } => {
            for photo_id in photo_ids {
                if let Some(state) = organization.get_mut(&photo_id) {
                    if enabled {
                        state.color_labels.insert(label);
                    } else {
                        state.color_labels.remove(&label);
                    }
                }
            }
        }
        CatalogCommand::ToggleColorLabel { photo_ids, label } => {
            for photo_id in photo_ids {
                if let Some(state) = organization.get_mut(&photo_id)
                    && !state.color_labels.insert(label)
                {
                    state.color_labels.remove(&label);
                }
            }
        }
        CatalogCommand::RegisterPhoto(_)
        | CatalogCommand::CreateEdit(_)
        | CatalogCommand::ReplaceEdit { .. } => {}
    }
}
