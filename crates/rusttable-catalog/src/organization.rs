use std::collections::BTreeSet;

use rusttable_core::PhotoId;

use crate::PhotoGroupId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rating {
    Zero,
    One,
    Two,
    Three,
    Four,
    Five,
}

impl Rating {
    #[must_use]
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Zero),
            1 => Some(Self::One),
            2 => Some(Self::Two),
            3 => Some(Self::Three),
            4 => Some(Self::Four),
            5 => Some(Self::Five),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Zero => 0,
            Self::One => 1,
            Self::Two => 2,
            Self::Three => 3,
            Self::Four => 4,
            Self::Five => 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ColorLabel {
    Red,
    Yellow,
    Green,
    Blue,
    Purple,
}

impl ColorLabel {
    pub const ALL: [Self; 5] = [
        Self::Red,
        Self::Yellow,
        Self::Green,
        Self::Blue,
        Self::Purple,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhotoOrganizationState {
    pub photo_id: PhotoId,
    pub rating: Rating,
    pub rejected: bool,
    pub color_labels: BTreeSet<ColorLabel>,
}

impl PhotoOrganizationState {
    #[must_use]
    pub fn new(photo_id: PhotoId) -> Self {
        Self {
            photo_id,
            rating: Rating::Zero,
            rejected: false,
            color_labels: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CatalogQuery {
    pub rating: Option<Rating>,
    pub rejected: Option<bool>,
    pub color_label: Option<ColorLabel>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrganizationProjection {
    pub photo_id: PhotoId,
    pub rating: Rating,
    pub rejected: bool,
    pub color_labels: Vec<ColorLabel>,
    pub group_id: Option<PhotoGroupId>,
    pub is_representative: bool,
}
