use rusttable_ui::{
    CollectionControlAction, CollectionControlState, CollectionFilterState, CollectionProperty,
    LighttableToolbarAction, SelectionModifiers,
};

use crate::gtk_controller::{
    CollectionController, CollectionSnapshot, GtkCatalogController, GtkCatalogMutationError,
};

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
        LighttableToolbarAction::SetProperty(property) => controller.set_property(property),
        LighttableToolbarAction::SetSearchText(search_text) => {
            controller.set_search_text(search_text);
        }
        LighttableToolbarAction::SetSort(sort) => controller.set_sort(sort),
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
        LighttableToolbarAction::ClearReset => controller.clear_reset(),
    }
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
    use rusttable_ui::{LighttableColorLabel, LighttableRating, LighttableToolbarAction};

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
