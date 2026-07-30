//! Canonical lighttable ordering and collection-rule matching.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use rusttable_catalog::PhotoOrganizationState;
use rusttable_core::PhotoId;
use rusttable_i18n::LocaleCollator;
use rusttable_ui::{
    CollectionItem, CollectionProperty, CollectionRule, LighttableSort, LighttableSortDirection,
};

pub(super) fn sort_items(
    items: &mut [&CollectionItem],
    sort: LighttableSort,
    direction: LighttableSortDirection,
    organization: &BTreeMap<PhotoId, PhotoOrganizationState>,
    collator: Option<&LocaleCollator>,
) {
    items.sort_by(|left, right| {
        let primary = match sort {
            LighttableSort::Filename => compare_text(
                collator,
                left.filename_sort_key(),
                right.filename_sort_key(),
            ),
            LighttableSort::CaptureTime => left
                .capture_time_sort_key()
                .cmp(&right.capture_time_sort_key()),
            LighttableSort::Rating => organization_rating(organization, right.photo_id())
                .cmp(&organization_rating(organization, left.photo_id())),
        };
        let primary = match direction {
            LighttableSortDirection::Ascending => primary,
            LighttableSortDirection::Descending => primary.reverse(),
        };
        let secondary = match sort {
            LighttableSort::Filename => {
                compare_text(collator, left.path_sort_key(), right.path_sort_key())
            }
            LighttableSort::CaptureTime | LighttableSort::Rating => compare_text(
                collator,
                left.filename_sort_key(),
                right.filename_sort_key(),
            )
            .then_with(|| compare_text(collator, left.path_sort_key(), right.path_sort_key())),
        };
        primary
            .then(secondary)
            .then_with(|| left.photo_id().cmp(&right.photo_id()))
    });
}

fn compare_text(collator: Option<&LocaleCollator>, left: &str, right: &str) -> Ordering {
    collator.map_or_else(|| left.cmp(right), |collator| collator.compare(left, right))
}

fn organization_rating(
    organization: &BTreeMap<PhotoId, PhotoOrganizationState>,
    photo_id: PhotoId,
) -> u8 {
    organization
        .get(&photo_id)
        .map_or(0, |state| state.rating.as_u8())
}

pub(super) fn organization_matches(
    rule: &CollectionRule,
    photo_id: PhotoId,
    organization: &BTreeMap<PhotoId, PhotoOrganizationState>,
) -> bool {
    if rule.search_text().trim().is_empty() {
        return true;
    }
    let Some(state) = organization.get(&photo_id) else {
        return true;
    };
    match rule.property() {
        CollectionProperty::Rating => {
            let rating = if state.rejected {
                "rejected".to_owned()
            } else {
                state.rating.as_u8().to_string()
            };
            rule.matches_value(&rating)
        }
        CollectionProperty::ColorLabel => state.color_labels.iter().any(|label| {
            rule.matches_value(match label {
                rusttable_catalog::ColorLabel::Red => "red",
                rusttable_catalog::ColorLabel::Yellow => "yellow",
                rusttable_catalog::ColorLabel::Green => "green",
                rusttable_catalog::ColorLabel::Blue => "blue",
                rusttable_catalog::ColorLabel::Purple => "purple",
            })
        }),
        CollectionProperty::Filmroll
        | CollectionProperty::Folders
        | CollectionProperty::Filename => true,
    }
}
