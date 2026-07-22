#![forbid(unsafe_code)]

use rusttable_ui::gtk_shell::{
    DARKROOM_GEOMETRY, DARKTABLE_DESKTOP_SPEC, DARKTABLE_UI_TOKENS, ModuleControlAllocationReceipt,
    ResponsiveGeometryReceipt,
};

#[test]
fn installed_darktable_scale_is_explicit_and_readable() {
    let tokens = DARKTABLE_UI_TOKENS;
    let panels = DARKTABLE_DESKTOP_SPEC.layout.side_panel_widths;

    assert_eq!(tokens.typography.base_pt, 12);
    assert_eq!(tokens.typography.compact_pt, 11);
    assert_eq!(tokens.controls.control_height, 28);
    assert_eq!(tokens.controls.module_row_height, 30);
    assert_eq!(tokens.controls.module_title_height, 30);
    assert_eq!(tokens.controls.toolbar_height, 34);
    assert_eq!(panels.minimum_px, 180);
    assert_eq!(panels.preferred_px, 220);
    assert!(panels.accepts(panels.minimum_px));
    assert_eq!(DARKROOM_GEOMETRY.histogram_height_px, 180);
}

#[test]
fn module_controls_fit_inside_scrollbar_allocation_at_supported_sizes() {
    for (width, height) in [(1_280, 768), (1_366, 768), (1_440, 900)] {
        let geometry = ResponsiveGeometryReceipt::for_window(width, height);
        let allocation = ModuleControlAllocationReceipt::for_rail(geometry.right_rail_width_px);

        assert!(allocation.fits(), "control allocation at {width}x{height}");
        assert_eq!(allocation.control_width_px, 96);
        assert!(allocation.label_width_px >= 80);
        assert_eq!(allocation.scrollbar_width_px, 14);
        assert!(
            allocation.content_width_px + allocation.scrollbar_width_px <= allocation.rail_width_px
        );
    }
}

#[test]
fn supported_windows_keep_both_rails_viewport_and_histogram_synchronized() {
    for (width, height) in [(1_280, 768), (1_366, 768), (1_440, 900)] {
        let geometry = ResponsiveGeometryReceipt::for_window(width, height);

        assert_eq!(geometry.left_rail_width_px, geometry.right_rail_width_px);
        assert_eq!(geometry.histogram_width_px, geometry.right_rail_width_px);
        assert_eq!(geometry.histogram_height_px, 180);
        assert!(geometry.left_rail_width_px >= 180);
        assert!(geometry.center_width_px >= 650);
        assert!(geometry.viewport_height_px >= 496);
        assert_eq!(
            geometry.left_rail_width_px + geometry.center_width_px + geometry.right_rail_width_px,
            DARKTABLE_DESKTOP_SPEC.layout.content_width_px(width)
        );
    }
}

#[test]
fn lighttable_cards_grow_between_target_viewports_without_dominating_center() {
    let cards = DARKTABLE_UI_TOKENS.cards;
    let compact = ResponsiveGeometryReceipt::for_window(1_280, 768);
    let full = ResponsiveGeometryReceipt::for_window(1_440, 900);
    let compact_card = cards.width_for_viewport(compact.center_width_px, 5);
    let full_card = cards.width_for_viewport(full.center_width_px, 5);

    assert_eq!(compact_card, 159);
    assert_eq!(full_card, 191);
    assert!(full_card > compact_card);
    assert!(full_card <= cards.maximum_width_px);
    assert_eq!(cards.image_width_px(compact_card), 147);
    assert_eq!(cards.image_height_px(147), 110);
}

#[test]
fn shared_css_and_runtime_own_all_scale_and_resize_behavior() {
    let css = rusttable_ui::gtk_shell::darktable_theme_css();
    let components = include_str!("../src/gui/darktable_components.rs");
    let layout = include_str!("../src/gui/runtime/layout.rs");
    let lighttable = include_str!("../src/gui/runtime/lighttable.rs");

    assert!(!css.contains("{{"));
    for declaration in [
        "font-size: 12pt",
        "min-width: 180px",
        "min-height: 28px",
        "min-height: 30px",
        "min-height: 180px",
    ] {
        assert!(
            css.contains(declaration),
            "missing generated CSS {declaration}"
        );
    }
    assert!(components.contains("DARKTABLE_UI_TOKENS"));
    assert!(layout.contains("connect_right_rail_constraints"));
    assert!(layout.contains(".clamp(minimum, maximum)"));
    assert!(layout.contains("connect_allocation_refresh"));
    assert!(lighttable.contains("connect_lighttable_resize"));
    assert!(lighttable.contains("lighttable_grid_for_allocation"));
}
