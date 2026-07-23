#![forbid(unsafe_code)]

use rusttable_core::PhotoId;
use rusttable_ui::gtk_shell::{
    DARKROOM_PANEL_WIDTHS, DARKROOM_VIEWPORT_WIDGET_IDS, DARKROOM_WIDGET_IDS,
    DARKTABLE_DESKTOP_SPEC, DESKTOP_REGIONS, DarkroomGeometryReceipt, LAYOUT_METRICS,
    LIGHTTABLE_COMPOSITION, LIGHTTABLE_PANEL_WIDTHS, LIGHTTABLE_RIGHT_MODULES, LIGHTTABLE_TOOLBAR,
    PANEL_SLOTS, PanelRole, ShellLayout, ShellRegion, THUMBNAIL_METRICS, ViewMode, WorkspaceRole,
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
            "darkroom-navigation-info",
            "darkroom-navigation-actions",
            "darkroom-snapshots",
            "darkroom-snapshots-info",
            "darkroom-snapshots-actions",
            "darkroom-history",
            "darkroom-history-info",
            "darkroom-history-actions",
            "darkroom-image-information",
            "darkroom-image-information-info",
            "darkroom-image-information-actions",
            "darkroom-right-panel",
            "darkroom-histogram",
            "darkroom-module-groups",
            "darkroom-module-search",
            "darkroom-left-module-scroll",
            "darkroom-right-module-scroll",
            "darkroom-right-modules",
            "exposure",
            "darkroom-left-panel-toggle",
            "darkroom-right-panel-toggle",
            "darkroom-filmstrip-toggle",
            "darkroom-status-bar",
            "darkroom-job-status",
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
fn darkroom_viewport_contract_exposes_live_histogram_and_truthful_overlay_slots() {
    let ids = DARKROOM_VIEWPORT_WIDGET_IDS
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), DARKROOM_VIEWPORT_WIDGET_IDS.len());
    for id in [
        "darkroom-viewport-overlay",
        "darkroom-overlay-before",
        "darkroom-overlay-soft-proof",
        "darkroom-overlay-gamut",
        "darkroom-overlay-histogram-sample",
    ] {
        assert!(ids.contains(id), "missing stable viewport contract ID {id}");
    }
}

#[test]
fn darkroom_geometry_keeps_both_rails_and_a_visible_center_at_supported_sizes() {
    for (window_width, window_height) in [(1_280, 768), (1_366, 768), (1_440, 900)] {
        let geometry =
            DarkroomGeometryReceipt::for_window(window_width, window_height, true, true, true);

        assert_eq!(geometry.left_panel_width_px, DARKROOM_PANEL_WIDTHS.left_px);
        assert_eq!(
            geometry.right_panel_width_px,
            DARKROOM_PANEL_WIDTHS.right_px
        );
        assert!(geometry.center_width_px() >= 650);
        assert!(geometry.center_width_px() > geometry.right_panel_width_px);
    }
}

#[test]
fn rail_resize_reallocates_the_center_from_the_same_display_free_contract() {
    let narrow_left =
        DarkroomGeometryReceipt::for_window_with_panel_widths(1_440, 900, 150, 150, true);
    let expanded_left =
        DarkroomGeometryReceipt::for_window_with_panel_widths(1_440, 900, 320, 150, true);
    let expanded_right =
        DarkroomGeometryReceipt::for_window_with_panel_widths(1_440, 900, 320, 300, true);

    assert_eq!(narrow_left.center_width_px(), 1_126);
    assert_eq!(expanded_left.center_width_px(), 956);
    assert_eq!(expanded_right.center_width_px(), 806);
    assert!(expanded_right.center_width_px() > 650);
    assert_eq!(expanded_right.filmstrip_height_px, 82);
}

#[test]
fn gtk_darkroom_layout_bounds_natural_module_width_before_paned_allocation() {
    let runtime_layout = include_str!("../src/gui/runtime/layout.rs");
    let darkroom_panels = include_str!("../src/views/darkroom/panel_widgets.rs");

    assert!(runtime_layout.contains(".propagate_natural_width(false)"));
    assert!(runtime_layout.contains("connect_right_rail_constraints"));
    assert!(runtime_layout.contains("connect_left_rail_constraints"));
    assert!(runtime_layout.contains("gtk4::glib::idle_add_local_once"));
    assert!(darkroom_panels.contains("darkroom-module-groups-scroll"));
    assert!(darkroom_panels.contains(".propagate_natural_width(false)"));
}

#[test]
fn gtk_parity_centers_short_surfaces_and_reserves_rail_actions() {
    let header = include_str!("../src/gui/header.rs");
    let runtime_layout = include_str!("../src/gui/runtime/layout.rs");
    let runtime_lighttable = include_str!("../src/gui/runtime/lighttable.rs");
    let components = include_str!("../src/gui/darktable_components.rs");
    let css = include_str!("../src/gui/theme.css");

    assert!(runtime_layout.contains(".shrink_start_child(true)"));
    assert!(runtime_layout.contains("photos.set_halign(gtk4::Align::Start)"));
    assert!(runtime_layout.contains("let strip_surface = gtk4::Grid::new"));
    assert!(runtime_lighttable.contains("centered_for_visible_count"));
    assert!(runtime_lighttable.contains("connect_filmstrip_resize"));
    assert!(runtime_lighttable.contains(".parent()"));
    assert!(components.contains("module_action_button"));
    assert!(runtime_lighttable.contains("lighttable_empty_state.allocated_width()"));
    assert!(runtime_lighttable.contains("viewport.allocated_height()"));
    assert!(css.contains(".dt_module_action"));
    assert!(css.contains(".dt_darkroom_section_label"));
    assert!(css.contains("#darkroom-filmstrip-boundary"));
    assert!(css.contains(".dt_view_switcher button.active"));
    assert!(header.contains("header_height.saturating_sub(HEADER_VIEWPORT_CHROME_PX)"));
    assert!(header.contains(".min_content_height(content_height)"));
    assert!(header.contains(".max_content_height(content_height)"));
    assert!(header.contains(".propagate_natural_height(true)"));
    assert!(header.contains(".has_frame(false)"));
}

#[test]
fn frame_edge_controls_stay_inside_darktable_outer_border() {
    let runtime_layout = include_str!("../src/gui/runtime/layout.rs");
    let runtime_shell = include_str!("../src/gui/runtime/mod.rs");
    let css = include_str!("../src/gui/theme.css");

    assert_eq!(LAYOUT_METRICS.outer_border_px, 7);
    assert!(runtime_shell.contains("workspace_frame(&shell)"));
    for id in [
        "workspace-left-edge-toggle",
        "workspace-right-edge-toggle",
        "workspace-top-edge-toggle",
        "workspace-bottom-edge-toggle",
    ] {
        assert!(runtime_layout.contains(id), "missing frame control {id}");
    }
    assert!(runtime_layout.contains("i32::from(layout.outer_border_px)"));
    assert!(runtime_layout.contains("triangle.set_content_width(4)"));
    assert!(runtime_layout.contains("triangle.set_content_height(10)"));
    assert!(runtime_layout.contains("triangle.set_content_width(10)"));
    assert!(runtime_layout.contains("triangle.set_content_height(4)"));
    assert!(css.contains(".dt_edge_toggle_vertical"));
    assert!(css.contains(".dt_edge_toggle_horizontal"));
    assert!(css.contains("min-width: {{outer_border_width}}px"));
    assert!(css.contains("min-height: {{outer_border_width}}px"));
}

#[test]
fn implemented_darkroom_panels_use_collapsed_shared_module_chrome() {
    let panels = include_str!("../src/views/darkroom/panel_widgets.rs");

    assert!(panels.contains("ImplementedModulePanel"));
    assert!(panels.contains("module.is_visible(group, query)"));
    for title in [
        "RGB AI denoise",
        "RAW AI denoise",
        "mask manager",
        "multiscale retouch",
    ] {
        assert!(panels.contains(title), "missing implemented panel {title}");
    }
    assert!(panels.contains("shared_module_expander"));
}

#[test]
fn gtk_workspace_ownership_keeps_lighttable_rail_and_darkroom_refresh_separate() {
    let runtime_layout = include_str!("../src/gui/runtime/layout.rs");
    let runtime_shell = include_str!("../src/gui/runtime/mod.rs");
    let components = include_str!("../src/gui/darktable_components.rs");
    let css = include_str!("../src/gui/theme.css");

    assert!(runtime_layout.contains(".child(toolbar.widget())"));
    assert!(runtime_layout.contains("center.append(&toolbar_clip)"));
    assert!(!runtime_layout.contains("lighttable_page.append(lighttable_toolbar.widget())"));
    assert!(!runtime_layout.contains("center.append(external_editor_panel.widget())"));
    assert!(!runtime_layout.contains("center.append(ai_batch_panel.widget())"));
    assert!(!runtime_layout.contains("center.append(camera_panel.widget())"));
    assert!(runtime_layout.contains("connect_position_notify"));
    assert!(runtime_shell.contains("darkroom.refresh_geometry()"));
    assert!(!runtime_shell.contains("self.window.maximize()"));
    for class in [
        "module_expander",
        "module_row",
        "scale_row",
        "dropdown",
        "rail_scroll",
    ] {
        assert!(
            components.contains(class),
            "shared component helper missing {class}"
        );
    }
    for selector in [
        ".dt_rail",
        ".dt_module_expander",
        ".dt_module_row",
        ".dt_field",
    ] {
        assert!(
            css.contains(selector),
            "shared Darktable CSS selector missing {selector}"
        );
    }
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
            "export",
        ]
    );
    assert_eq!(THUMBNAIL_METRICS.filmstrip_width_px, 104);
    assert_eq!(THUMBNAIL_METRICS.filmstrip_height_px, 78);
    assert_eq!(LAYOUT_METRICS.filmstrip_heights.preferred_px, 82);
    assert_eq!(LIGHTTABLE_PANEL_WIDTHS.left_px, 140);
    assert_eq!(LIGHTTABLE_PANEL_WIDTHS.right_px, 164);
    assert_eq!(DARKROOM_PANEL_WIDTHS.left_px, 180);
    assert_eq!(DARKROOM_PANEL_WIDTHS.right_px, 180);
}

#[test]
fn lighttable_contract_cannot_reintroduce_obsolete_neural_restore_text() {
    assert!(
        LIGHTTABLE_RIGHT_MODULES
            .iter()
            .all(|module| module.widget_name != "neural-restore")
    );
    assert!(
        LIGHTTABLE_RIGHT_MODULES
            .iter()
            .all(|module| !module.title.eq_ignore_ascii_case("neural restore"))
    );
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
fn gtk_selection_route_projects_preview_and_darkroom_filmstrip_activation() {
    let runtime = include_str!("../src/gui/runtime/lighttable.rs");
    let shell_runtime = include_str!("../src/gui/runtime/mod.rs");

    assert!(runtime.contains("show_photo_detail(&context.darkroom_preview, detail);"));
    assert!(runtime.contains("surface == PhotoSurface::Filmstrip && darkroom_visible"));
    assert!(shell_runtime.contains("render.move_focus(direction, modifiers)"));
    assert!(runtime.contains("OpenSelected"));
}

#[test]
fn darkroom_activation_binds_selection_before_presenting_the_stack_child() {
    let runtime = include_str!("../src/gui/runtime/lighttable.rs");
    let shell_runtime = include_str!("../src/gui/runtime/mod.rs");

    let callback_positions =
        runtime.match_indices("handler(photo_id, SelectionModifiers::default())");
    let stack_positions =
        runtime.match_indices("set_visible_child_name(WorkspaceRole::Darkroom.stack_name())");
    let callbacks = callback_positions.collect::<Vec<_>>();
    let stacks = stack_positions.collect::<Vec<_>>();
    assert_eq!(
        callbacks.len(),
        2,
        "native-open and GTK activation paths must notify once"
    );
    assert_eq!(
        stacks.len(),
        2,
        "native-open and GTK activation paths must present once"
    );
    for ((callback, _), (stack, _)) in callbacks.iter().zip(stacks.iter()) {
        assert!(
            callback < stack,
            "selection callback must precede GTK stack presentation"
        );
    }
    assert!(
        shell_runtime.contains("set_filmstrip_items(filmstrip_ids, Some(photo_id), generation)")
    );
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
