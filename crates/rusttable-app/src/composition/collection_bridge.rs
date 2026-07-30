use std::cell::RefCell;
use std::rc::Rc;

use rusttable_ui::{
    CollectionControlAction, CollectionControlState, CollectionFilterState, CollectionProperty,
    LighttableToolbarAction, SelectionModifiers,
};

use crate::gtk_controller::{
    CollectionController, CollectionSnapshot, GtkCatalogController, GtkCatalogMutationError,
};

#[cfg(test)]
pub(super) fn apply_collection_action(
    controller: &mut CollectionController,
    action: CollectionControlAction,
) {
    let generation = match &action {
        CollectionControlAction::SetProperty { generation, .. }
        | CollectionControlAction::SetSearchText { generation, .. }
        | CollectionControlAction::Clear { generation } => *generation,
    };
    if !controller.accept_generation(generation) {
        return;
    }
    match action {
        CollectionControlAction::SetProperty { property, .. } => controller.set_property(property),
        CollectionControlAction::SetSearchText { search_text, .. } => {
            controller.set_search_text(search_text);
        }
        CollectionControlAction::Clear { .. } => controller.clear(),
    }
}

pub(super) fn apply_lighttable_toolbar_action(
    catalog: &mut GtkCatalogController,
    controller: &mut CollectionController,
    action: LighttableToolbarAction,
) -> Result<(), GtkCatalogMutationError> {
    match action {
        LighttableToolbarAction::SetProperty(property) => {
            catalog.apply_collection_controller_change(controller, |controller| {
                controller.set_property(property);
            })?;
        }
        LighttableToolbarAction::SetSearchText(search_text) => {
            catalog.apply_collection_controller_change(controller, |controller| {
                controller.set_search_text(search_text);
            })?;
        }
        LighttableToolbarAction::SetSort(sort) => {
            catalog.apply_collection_controller_change(controller, |controller| {
                controller.set_sort(sort);
            })?;
        }
        LighttableToolbarAction::SetSortDirection(direction) => {
            catalog.apply_collection_controller_change(controller, |controller| {
                controller.set_sort_direction(direction);
            })?;
        }
        LighttableToolbarAction::SetRating(rating) => {
            let Some(command) = controller.organization_command_for_rating(rating) else {
                return Ok(());
            };
            let (event, states) = catalog.apply_organization_command(&command)?;
            tracing::debug!(
                target: "rusttable.catalog",
                revision = event.revision().get(),
                photos = event.photo_ids().len(),
                "lighttable organization changed"
            );
            controller.replace_organization(states.into_values());
        }
        LighttableToolbarAction::ToggleColorLabel(label) => {
            let Some(command) = controller.organization_command_for_color_label(label) else {
                return Ok(());
            };
            let (event, states) = catalog.apply_organization_command(&command)?;
            tracing::debug!(
                target: "rusttable.catalog",
                revision = event.revision().get(),
                photos = event.photo_ids().len(),
                "lighttable color label changed"
            );
            controller.replace_organization(states.into_values());
        }
        LighttableToolbarAction::ClearReset => {
            catalog.apply_collection_controller_change(
                controller,
                CollectionController::clear_reset,
            )?;
        }
    }
    Ok(())
}

pub(super) fn apply_collection_action_persisted(
    catalog: &GtkCatalogController,
    controller: &mut CollectionController,
    action: CollectionControlAction,
) -> Result<(), GtkCatalogMutationError> {
    let generation = match &action {
        CollectionControlAction::SetProperty { generation, .. }
        | CollectionControlAction::SetSearchText { generation, .. }
        | CollectionControlAction::Clear { generation } => *generation,
    };
    let mut next = controller.clone();
    if !next.accept_generation(generation) {
        return Ok(());
    }
    match action {
        CollectionControlAction::SetProperty { property, .. } => next.set_property(property),
        CollectionControlAction::SetSearchText { search_text, .. } => {
            next.set_search_text(search_text);
        }
        CollectionControlAction::Clear { .. } => next.clear(),
    }
    catalog.persist_active_collection_state(&next.active_state())?;
    *controller = next;
    Ok(())
}

pub(super) fn apply_photo_selection(
    controller: &mut CollectionController,
    photo_id: rusttable_core::PhotoId,
    modifiers: SelectionModifiers,
) -> bool {
    if modifiers.range() {
        controller.select_range(photo_id, modifiers.extend())
    } else if modifiers.extend() {
        controller.toggle_selection(photo_id)
    } else {
        controller.select_only(photo_id)
    }
}

pub(super) fn apply_selection_projection(
    catalog: &Rc<RefCell<GtkCatalogController>>,
    collection: &Rc<RefCell<Option<CollectionController>>>,
    shell: &rusttable_ui::GtkShell,
    photo_id: rusttable_core::PhotoId,
    modifiers: rusttable_ui::SelectionModifiers,
) -> Result<(bool, bool), GtkCatalogMutationError> {
    let mut next_catalog = catalog.borrow().clone();
    let catalog_changed = next_catalog.select_photo(photo_id);
    let mut next_collection = collection.borrow().clone();
    let collection_changed = next_collection
        .as_mut()
        .is_some_and(|controller| apply_photo_selection(controller, photo_id, modifiers));
    if collection_changed && let Some(controller) = next_collection.as_ref() {
        next_catalog.persist_active_collection_state(&controller.active_state())?;
    }
    *catalog.borrow_mut() = next_catalog;
    *collection.borrow_mut() = next_collection;
    if let Some(controller) = collection.borrow().as_ref() {
        shell.set_collection_filter_state(&collection_filter_state(&controller.snapshot()));
    }
    Ok((catalog_changed, collection_changed))
}

pub(super) fn collection_filter_state(snapshot: &CollectionSnapshot) -> CollectionFilterState {
    let controls = CollectionControlState::new(snapshot.property(), snapshot.total_count())
        .with_results(snapshot.search_text(), snapshot.result_count())
        .with_generation(snapshot.generation());
    CollectionFilterState::new(controls, snapshot.matching_photo_ids().collect())
        .with_lighttable_state(snapshot.photo_states().cloned(), snapshot.toolbar().clone())
}

pub(super) fn empty_collection_filter_state() -> CollectionFilterState {
    CollectionFilterState::new(
        CollectionControlState::new(CollectionProperty::Filename, 0),
        Vec::new(),
    )
}

pub(super) fn failed_collection_filter_state(
    snapshot: &CollectionSnapshot,
) -> CollectionFilterState {
    let controls = CollectionControlState::new(snapshot.property(), snapshot.total_count())
        .with_results(snapshot.search_text(), snapshot.result_count())
        .with_generation(snapshot.generation())
        .failed();
    CollectionFilterState::new(controls, snapshot.matching_photo_ids().collect())
        .with_lighttable_state(snapshot.photo_states().cloned(), snapshot.toolbar().clone())
}

#[cfg(test)]
mod tests {
    use rusttable_catalog::{ImportCandidate, ImportRecord, ImportRepository, SourcePath};
    use rusttable_catalog_store::RedbCatalogRepository;
    use rusttable_core::{
        Asset, AssetId, AssetRole, ByteLength, ContentHash, ImageMetadata, Photo, PhotoId,
    };
    use rusttable_image::{ImageDimensions, ImageProbe, InputFormat};
    use rusttable_ui::{
        CollectionProperty, LighttableColorLabel, LighttableRating, LighttableSort,
        LighttableSortDirection, LighttableToolbarAction,
    };

    use super::apply_lighttable_toolbar_action;
    use crate::gtk_controller::GtkCatalogController;

    #[test]
    fn toolbar_route_persists_single_and_multi_selection_before_refresh() {
        let path = std::env::temp_dir().join(format!(
            "rusttable-issue-882-routing-{}.redb",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        {
            let mut repository = RedbCatalogRepository::open(&path).unwrap();
            repository.commit(&record(1)).unwrap();
            repository.commit(&record(2)).unwrap();
        }

        let mut catalog = GtkCatalogController::load_catalog_at(path.clone());
        let mut collection = catalog.collection_controller().unwrap();
        assert!(collection.select_only(photo(1)));
        apply_lighttable_toolbar_action(
            &mut catalog,
            &mut collection,
            LighttableToolbarAction::SetRating(LighttableRating::Four),
        )
        .unwrap();
        assert_eq!(
            collection.snapshot().toolbar().selected_rating(),
            Some(LighttableRating::Four)
        );
        apply_lighttable_toolbar_action(
            &mut catalog,
            &mut collection,
            LighttableToolbarAction::SetRating(LighttableRating::Rejected),
        )
        .unwrap();
        assert_eq!(
            collection.snapshot().toolbar().selected_rating(),
            Some(LighttableRating::Rejected)
        );
        apply_lighttable_toolbar_action(
            &mut catalog,
            &mut collection,
            LighttableToolbarAction::SetRating(LighttableRating::Rejected),
        )
        .unwrap();
        assert_eq!(
            collection.snapshot().toolbar().selected_rating(),
            Some(LighttableRating::Four)
        );

        assert!(collection.toggle_selection(photo(2)));
        apply_lighttable_toolbar_action(
            &mut catalog,
            &mut collection,
            LighttableToolbarAction::ToggleColorLabel(LighttableColorLabel::Red),
        )
        .unwrap();
        for state in collection.snapshot().photo_states() {
            assert_eq!(
                state.color_labels().collect::<Vec<_>>(),
                [LighttableColorLabel::Red]
            );
        }

        drop(collection);
        drop(catalog);
        let restarted = GtkCatalogController::load_catalog_at(path.clone());
        let snapshot = restarted.collection_controller().unwrap().snapshot();
        assert_eq!(snapshot.photo_states().count(), 2);
        assert_eq!(
            snapshot.photo_states().next().unwrap().rating(),
            LighttableRating::Four
        );
        assert!(snapshot.photo_states().all(|state| {
            state
                .color_labels()
                .any(|label| label == LighttableColorLabel::Red)
        }));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn active_lighttable_rule_sort_direction_and_selection_survive_restart() {
        let path = std::env::temp_dir().join(format!(
            "rusttable-issue-884-restart-{}.redb",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        {
            let mut repository = RedbCatalogRepository::open(&path).unwrap();
            repository.commit(&record(1)).unwrap();
            repository.commit(&record(2)).unwrap();
        }

        let mut catalog = GtkCatalogController::load_catalog_at(path.clone());
        let mut collection = catalog.collection_controller().unwrap();
        assert!(collection.select_only(photo(2)));
        catalog
            .persist_active_collection_state(&collection.active_state())
            .unwrap();
        apply_lighttable_toolbar_action(
            &mut catalog,
            &mut collection,
            LighttableToolbarAction::SetProperty(CollectionProperty::Filename),
        )
        .unwrap();
        apply_lighttable_toolbar_action(
            &mut catalog,
            &mut collection,
            LighttableToolbarAction::SetSearchText("photo-2".to_owned()),
        )
        .unwrap();
        apply_lighttable_toolbar_action(
            &mut catalog,
            &mut collection,
            LighttableToolbarAction::SetSort(LighttableSort::Rating),
        )
        .unwrap();
        apply_lighttable_toolbar_action(
            &mut catalog,
            &mut collection,
            LighttableToolbarAction::SetSortDirection(LighttableSortDirection::Descending),
        )
        .unwrap();
        drop(collection);
        drop(catalog);

        let restarted = GtkCatalogController::load_catalog_at(path.clone());
        let restored = restarted.collection_controller().unwrap();
        assert_eq!(restored.rule().property(), CollectionProperty::Filename);
        assert_eq!(restored.rule().search_text(), "photo-2");
        assert_eq!(restored.snapshot().toolbar().sort(), LighttableSort::Rating);
        assert_eq!(
            restored.snapshot().toolbar().sort_direction(),
            LighttableSortDirection::Descending
        );
        assert_eq!(
            restored.selected_photo_ids().collect::<Vec<_>>(),
            vec![photo(2)]
        );
        let _ = std::fs::remove_file(path);
    }

    fn record(value: u128) -> ImportRecord {
        let photo_id = photo(value);
        let asset_id = AssetId::new(value + 10).unwrap();
        let source_text = format!("photo-{value}.png");
        let source = SourcePath::new(&source_text).unwrap();
        let candidate = ImportCandidate::new(
            photo_id,
            asset_id,
            source,
            ContentHash::Sha256([u8::try_from(value).unwrap(); 32]),
            ByteLength::from_bytes(4),
            ImageProbe::new(InputFormat::Png, ImageDimensions::new(2, 2).unwrap()),
            ImageMetadata::empty(),
        )
        .unwrap();
        ImportRecord::new(
            &candidate,
            Photo::new(
                photo_id,
                [Asset::new(
                    asset_id,
                    AssetRole::Primary,
                    candidate.content_hash(),
                    candidate.byte_length(),
                )],
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn photo(value: u128) -> PhotoId {
        PhotoId::new(value).unwrap()
    }
}
