use rusttable_ui::{
    CollectionControlAction, CollectionControlState, CollectionFilterState, CollectionProperty,
    LighttableToolbarAction,
};

use crate::gtk_controller::{CollectionController, CollectionSnapshot};

pub(super) fn apply_collection_action(
    controller: &mut CollectionController,
    action: CollectionControlAction,
) {
    match action {
        CollectionControlAction::SetProperty(property) => controller.set_property(property),
        CollectionControlAction::SetSearchText(search_text) => {
            controller.set_search_text(search_text);
        }
        CollectionControlAction::Clear => controller.clear(),
    }
}

pub(super) fn apply_lighttable_toolbar_action(
    controller: &mut CollectionController,
    action: LighttableToolbarAction,
) {
    match action {
        LighttableToolbarAction::SetProperty(property) => controller.set_property(property),
        LighttableToolbarAction::SetSearchText(search_text) => {
            controller.set_search_text(search_text);
        }
        LighttableToolbarAction::SetSort(sort) => controller.set_sort(sort),
        LighttableToolbarAction::SetRating(rating) => controller.set_selected_rating(rating),
        LighttableToolbarAction::ToggleColorLabel(label) => {
            controller.toggle_selected_color_label(label);
        }
        LighttableToolbarAction::ClearReset => controller.clear_reset(),
    }
}

pub(super) fn collection_filter_state(snapshot: &CollectionSnapshot) -> CollectionFilterState {
    let controls = CollectionControlState::new(snapshot.property(), snapshot.total_count())
        .with_results(snapshot.search_text(), snapshot.result_count());
    CollectionFilterState::new(controls, snapshot.matching_photo_ids().collect())
        .with_lighttable_state(snapshot.photo_states().cloned(), snapshot.toolbar().clone())
}

pub(super) fn empty_collection_filter_state() -> CollectionFilterState {
    CollectionFilterState::new(
        CollectionControlState::new(CollectionProperty::Filename, 0),
        Vec::new(),
    )
}
