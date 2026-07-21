#![forbid(unsafe_code)]

use rusttable_core::PhotoId;
use rusttable_ui::gtk_shell::{
    DARKROOM_WIDGET_IDS, DARKTABLE_DESKTOP_SPEC, DESKTOP_REGIONS, LAYOUT_METRICS,
    LIGHTTABLE_COMPOSITION, LIGHTTABLE_RIGHT_MODULES, LIGHTTABLE_TOOLBAR, PANEL_SLOTS, PanelRole,
    ShellLayout, ShellRegion, THUMBNAIL_METRICS, ViewMode, WorkspaceRole,
};
use rusttable_ui::{
    DarkroomWorkspaceViewModel, LighttableContentState, LighttableInteractionState,
    LighttableSelectionAction, LighttableToolbarState, ModuleControlKind, ModuleControlViewModel,
    ModulePanelViewModel, NavigationDirection, PhotoCardViewModel, PhotoDetailViewModel,
    PhotoFactViewModel, PhotoWorkspaceViewModel, PresentationText, SelectionModifiers,
    ThumbnailIndicators,
};

fn id(value: u128) -> PhotoId {
    PhotoId::new(value).expect("test photo IDs are non-zero")
}

fn text(value: &str) -> PresentationText {
    PresentationText::new(value).expect("test presentation text is valid")
}

fn detail(value: u128, title: &str) -> PhotoDetailViewModel {
    PhotoDetailViewModel::new(
        id(value),
        text(title),
        vec![PhotoFactViewModel::new(text("camera"), text("test camera"))],
    )
}

#[test]
fn darkroom_stack_keeps_darktable_regions_and_stable_widget_order() {
    assert_eq!(ViewMode::Lighttable.stack_name(), "lighttable");
    assert_eq!(ViewMode::Darkroom.stack_name(), "darkroom");
    assert_eq!(WorkspaceRole::Darkroom.stack_name(), "darkroom");

    assert_eq!(DARKTABLE_DESKTOP_SPEC.regions, &DESKTOP_REGIONS);
    assert_eq!(DARKTABLE_DESKTOP_SPEC.panel_slots, &PANEL_SLOTS);
    assert_eq!(ShellLayout::default().regions().len(), 5);
    assert_eq!(ShellLayout::default().panel_slots().len(), 12);
    assert_eq!(ShellRegion::Workspace.identifier(), "workspace");
    assert!(PanelRole::Left.is_side_panel());
    assert!(!PanelRole::BottomFilmstrip.is_side_panel());

    assert_eq!(
        DARKROOM_WIDGET_IDS,
        [
            "darkroom-page",
            "darkroom-toolbar-top",
            "darkroom-photo-preview",
            "darkroom-toolbar-bottom",
            "darkroom-left-panel",
            "darkroom-navigation",
            "darkroom-snapshots",
            "darkroom-history",
            "darkroom-image-information",
            "darkroom-right-panel",
            "darkroom-histogram",
            "darkroom-module-groups",
            "darkroom-module-search",
            "darkroom-left-module-scroll",
            "darkroom-right-module-scroll",
            "darkroom-right-modules",
            "exposure",
        ]
    );
}

#[test]
fn darkroom_model_keeps_left_and_right_module_stacks_typed() {
    let navigation = ModulePanelViewModel::new(
        text("navigation"),
        vec![ModuleControlViewModel::new(
            text("zoom"),
            ModuleControlKind::Choice,
        )],
    );
    let exposure = ModulePanelViewModel::new(
        text("exposure"),
        vec![
            ModuleControlViewModel::new(text("exposure"), ModuleControlKind::Slider),
            ModuleControlViewModel::new(text("enabled"), ModuleControlKind::Toggle),
        ],
    );
    let model = DarkroomWorkspaceViewModel::new(
        detail(7, "selected photo"),
        vec![navigation],
        vec![exposure],
    );

    assert_eq!(model.detail().id(), id(7));
    assert_eq!(
        model
            .left_modules()
            .map(|module| module.title().as_str())
            .collect::<Vec<_>>(),
        vec!["navigation"]
    );
    let right = model.right_modules().next().expect("exposure module");
    assert_eq!(right.title().as_str(), "exposure");
    assert_eq!(
        right
            .controls()
            .map(ModuleControlViewModel::kind)
            .collect::<Vec<_>>(),
        vec![ModuleControlKind::Slider, ModuleControlKind::Toggle]
    );
}

#[test]
fn lighttable_contract_has_one_filter_row_and_plain_bottom_filmstrip() {
    assert_eq!(
        LIGHTTABLE_TOOLBAR.widget_name,
        "lighttable-collection-toolbar"
    );
    assert_eq!(
        LIGHTTABLE_TOOLBAR.filter_entry_name,
        "lighttable-filter-entry"
    );
    assert_eq!(LIGHTTABLE_TOOLBAR.row_count, 1);
    assert_eq!(LIGHTTABLE_COMPOSITION.top_toolbar_rows, 1);
    assert_eq!(LIGHTTABLE_COMPOSITION.footer_toolbar_rows, 1);
    assert_eq!(LIGHTTABLE_COMPOSITION.filmstrip_toolbar_rows, 0);

    assert_eq!(
        LIGHTTABLE_RIGHT_MODULES
            .iter()
            .map(|module| module.widget_name)
            .collect::<Vec<_>>(),
        vec![
            "selection",
            "actions-on-selection",
            "history-stack",
            "styles",
            "metadata-editor",
            "tagging",
            "geotagging",
            "neural-restore",
            "export",
        ]
    );
    assert_eq!(THUMBNAIL_METRICS.filmstrip_width_px, 92);
    assert_eq!(THUMBNAIL_METRICS.filmstrip_height_px, 72);
    assert_eq!(LAYOUT_METRICS.filmstrip_heights.preferred_px, 120);
}

#[test]
fn lighttable_projection_preserves_grid_order_badges_and_empty_state() {
    let mut cards = vec![
        PhotoCardViewModel::new(id(3), text("third"), Some(text("raw"))),
        PhotoCardViewModel::new(id(1), text("first"), None),
    ];
    cards[1] = cards[1].clone().with_indicators(ThumbnailIndicators {
        grouped: true,
        local_copy: false,
        altered: true,
    });
    let source = PhotoWorkspaceViewModel::new(cards, vec![detail(1, "first"), detail(3, "third")])
        .expect("test workspace topology is valid");
    let browser = rusttable_ui::gtk_shell::LibraryBrowserModel::from_workspace(&source);
    let photos = browser.photos().collect::<Vec<_>>();

    assert_eq!(
        photos.iter().map(|photo| photo.id()).collect::<Vec<_>>(),
        vec![id(3), id(1)]
    );
    assert_eq!(photos[0].secondary(), Some("raw"));
    assert!(photos[1].indicators().grouped);
    assert!(photos[1].indicators().altered);
    assert_eq!(
        LighttableContentState::from_rendered_count(0).stack_name(),
        "empty"
    );
    assert_eq!(
        LighttableContentState::from_rendered_count(2).stack_name(),
        "grid"
    );
}

#[test]
fn lighttable_grid_and_filmstrip_share_selection_and_darkroom_open_route() {
    let mut state = LighttableInteractionState::new(2);
    state.set_order([id(10), id(20), id(30), id(40), id(50)]);
    assert_eq!(
        state.ordered().collect::<Vec<_>>(),
        vec![id(10), id(20), id(30), id(40), id(50)]
    );

    state.apply(LighttableSelectionAction::Select {
        photo_id: id(20),
        modifiers: SelectionModifiers::default(),
    });
    state.apply(LighttableSelectionAction::Select {
        photo_id: id(40),
        modifiers: SelectionModifiers::new(false, true),
    });
    assert_eq!(
        state.selected().collect::<Vec<_>>(),
        vec![id(20), id(30), id(40)]
    );
    assert_eq!(
        state.apply(LighttableSelectionAction::OpenSelected),
        Some(id(40))
    );
    assert_eq!(WorkspaceRole::Darkroom.stack_name(), "darkroom");

    state.apply(LighttableSelectionAction::Move {
        direction: NavigationDirection::RowNext,
        modifiers: SelectionModifiers::default(),
    });
    assert_eq!(state.focus(), Some(id(50)));
    state.apply(LighttableSelectionAction::Clear);
    assert!(state.selected().next().is_none());
}

#[test]
fn lighttable_filter_projection_keeps_matching_ids_and_selection_count() {
    let filter = rusttable_ui::gtk_shell::CollectionFilterState::new(
        rusttable_ui::CollectionControlState::new(rusttable_ui::CollectionProperty::Filename, 5)
            .with_results("trip", 2),
        vec![id(2), id(4)],
    )
    .with_lighttable_state(
        [rusttable_ui::LighttablePhotoState::new(
            id(2),
            true,
            rusttable_ui::LighttableRating::Four,
            [],
        )],
        LighttableToolbarState::new(5)
            .with_filter(rusttable_ui::CollectionProperty::Filename, "trip", 2)
            .with_selection(1, Some(rusttable_ui::LighttableRating::Four), []),
    );

    assert_eq!(filter.matching_photo_ids(), &[id(2), id(4)]);
    assert_eq!(filter.controls().search_text(), "trip");
    assert_eq!(filter.controls().result_count(), 2);
    assert_eq!(
        filter
            .photo_state(id(2))
            .map(rusttable_ui::LighttablePhotoState::selected),
        Some(true)
    );
    assert_eq!(filter.toolbar().selected_count(), 1);
    assert_eq!(filter.toolbar().visible_count(), 2);
}
