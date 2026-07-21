//! GTK-independent lighttable selection, navigation, zoom, and filmstrip routing.

use std::collections::BTreeSet;

use rusttable_core::PhotoId;

/// Modifier state relevant to Darktable lighttable selection semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SelectionModifiers {
    extend: bool,
    range: bool,
}

impl SelectionModifiers {
    #[must_use]
    pub const fn new(extend: bool, range: bool) -> Self {
        Self { extend, range }
    }

    #[must_use]
    pub const fn extend(self) -> bool {
        self.extend
    }

    #[must_use]
    pub const fn range(self) -> bool {
        self.range
    }
}

/// Direction of keyboard movement in the lighttable grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationDirection {
    Previous,
    Next,
    RowPrevious,
    RowNext,
}

/// Surface selected by the lighttable layout control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LighttableLayout {
    #[default]
    FileManager,
    Zoomable,
    Culling,
    CullingDynamic,
    Preview,
}

impl LighttableLayout {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::FileManager => "file manager",
            Self::Zoomable => "zoomable",
            Self::Culling => "culling",
            Self::CullingDynamic => "dynamic culling",
            Self::Preview => "preview",
        }
    }

    #[must_use]
    pub const fn shows_grid(self) -> bool {
        matches!(self, Self::FileManager | Self::Zoomable)
    }

    #[must_use]
    pub const fn shows_culling(self) -> bool {
        matches!(self, Self::Culling | Self::CullingDynamic | Self::Preview)
    }

    #[must_use]
    pub const fn shows_filmstrip(self) -> bool {
        matches!(self, Self::Culling | Self::CullingDynamic | Self::Preview)
    }
}

/// Image set used by a culling surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CullingRestriction {
    #[default]
    Automatic,
    Collection,
    Selection,
}

/// Bounded integer pan offset and inclusive viewport bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PanOffset {
    x: i32,
    y: i32,
}

impl PanOffset {
    #[must_use]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    #[must_use]
    pub const fn x(self) -> i32 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> i32 {
        self.y
    }

    #[must_use]
    pub const fn saturating_add(self, delta: Self) -> Self {
        Self {
            x: self.x.saturating_add(delta.x),
            y: self.y.saturating_add(delta.y),
        }
    }

    #[must_use]
    pub const fn clamp(self, bounds: PanBounds) -> Self {
        Self {
            x: clamp_i32(self.x, bounds.min_x, bounds.max_x),
            y: clamp_i32(self.y, bounds.min_y, bounds.max_y),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanBounds {
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
}

impl PanBounds {
    #[must_use]
    pub fn new(min_x: i32, max_x: i32, min_y: i32, max_y: i32) -> Self {
        Self {
            min_x: min_x.min(max_x),
            max_x: min_x.max(max_x),
            min_y: min_y.min(max_y),
            max_y: min_y.max(max_y),
        }
    }
}

const fn clamp_i32(value: i32, minimum: i32, maximum: i32) -> i32 {
    if value < minimum {
        minimum
    } else if value > maximum {
        maximum
    } else {
        value
    }
}

/// Fixed-point culling zoom expressed as a percentage of fit size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CullingZoom(u16);

impl CullingZoom {
    pub const FIT_PERCENT: u16 = 100;
    pub const MAX_PERCENT: u16 = 800;

    #[must_use]
    pub const fn new(percent: u16) -> Self {
        Self(if percent < Self::FIT_PERCENT {
            Self::FIT_PERCENT
        } else if percent > Self::MAX_PERCENT {
            Self::MAX_PERCENT
        } else {
            percent
        })
    }

    #[must_use]
    pub const fn fit() -> Self {
        Self(Self::FIT_PERCENT)
    }

    #[must_use]
    pub const fn percent(self) -> u16 {
        self.0
    }

    #[must_use]
    pub fn add_percent(self, delta: i16) -> Self {
        let percent = i32::from(self.0)
            .saturating_add(i32::from(delta))
            .clamp(i32::from(Self::FIT_PERCENT), i32::from(Self::MAX_PERCENT));
        Self::new(match u16::try_from(percent) {
            Ok(value) => value,
            Err(_) => Self::MAX_PERCENT,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CullingViewport {
    zoom: CullingZoom,
    pan: PanOffset,
}

impl Default for CullingViewport {
    fn default() -> Self {
        Self::new()
    }
}

impl CullingViewport {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            zoom: CullingZoom::fit(),
            pan: PanOffset::new(0, 0),
        }
    }

    #[must_use]
    pub const fn zoom(self) -> CullingZoom {
        self.zoom
    }

    #[must_use]
    pub const fn pan(self) -> PanOffset {
        self.pan
    }

    pub fn zoom_by(&mut self, delta_percent: i16, bounds: PanBounds) {
        self.zoom = self.zoom.add_percent(delta_percent);
        self.pan = self.pan.clamp(bounds);
    }

    pub fn pan_by(&mut self, delta: PanOffset, bounds: PanBounds) {
        self.pan = self.pan.saturating_add(delta).clamp(bounds);
    }

    pub fn fit(&mut self) {
        self.zoom = CullingZoom::fit();
        self.pan = PanOffset::new(0, 0);
    }
}

/// User-visible lighttable thumbnail density.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum LighttableZoom {
    Small,
    #[default]
    Normal,
    Large,
}

impl LighttableZoom {
    /// Number of cards per row in the file-manager lighttable layout.
    ///
    /// These are deliberately discrete bounds: changing thumbnail density must not
    /// allow the grid to grow an unbounded number of layout variants.
    #[must_use]
    pub const fn columns(self) -> usize {
        match self {
            Self::Small => 8,
            Self::Normal => 6,
            Self::Large => 4,
        }
    }

    #[must_use]
    pub const fn scale(self) -> f64 {
        match self {
            Self::Small => 0.75,
            Self::Normal => 1.0,
            Self::Large => 1.25,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Normal => "normal",
            Self::Large => "large",
        }
    }

    #[must_use]
    pub const fn smaller(self) -> Self {
        match self {
            Self::Small | Self::Normal => Self::Small,
            Self::Large => Self::Normal,
        }
    }

    #[must_use]
    pub const fn larger(self) -> Self {
        match self {
            Self::Small => Self::Normal,
            Self::Normal | Self::Large => Self::Large,
        }
    }
}

/// Typed UI intent emitted by the lighttable and filmstrip widgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LighttableSelectionAction {
    Select {
        photo_id: PhotoId,
        modifiers: SelectionModifiers,
    },
    Move {
        direction: NavigationDirection,
        modifiers: SelectionModifiers,
    },
    OpenSelected,
    Clear,
    SetZoom(LighttableZoom),
    SetLayout(LighttableLayout),
    SetLeftPanelVisible(bool),
    SetRightPanelVisible(bool),
    SetCullingRestriction(CullingRestriction),
    SetCullingActive(PhotoId),
    PanCulling {
        delta: PanOffset,
        bounds: PanBounds,
    },
    ZoomCulling {
        delta_percent: i16,
        bounds: PanBounds,
    },
    FitCulling,
}

/// Deterministic selection state shared by the grid and filmstrip projections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LighttableInteractionState {
    ordered_ids: Vec<PhotoId>,
    selected: BTreeSet<PhotoId>,
    anchor: Option<PhotoId>,
    focus: Option<PhotoId>,
    columns: usize,
    zoom: LighttableZoom,
    layout: LighttableLayout,
    left_panel_visible: bool,
    right_panel_visible: bool,
    culling_restriction: CullingRestriction,
    culling_viewport: CullingViewport,
}

impl LighttableInteractionState {
    #[must_use]
    pub fn new(columns: usize) -> Self {
        Self {
            ordered_ids: Vec::new(),
            selected: BTreeSet::new(),
            anchor: None,
            focus: None,
            columns: columns.max(1),
            zoom: LighttableZoom::default(),
            layout: LighttableLayout::default(),
            left_panel_visible: true,
            right_panel_visible: true,
            culling_restriction: CullingRestriction::default(),
            culling_viewport: CullingViewport::new(),
        }
    }

    /// Reconciles the visible catalog order while retaining only live selection IDs.
    pub fn set_order(&mut self, ids: impl IntoIterator<Item = PhotoId>) {
        let mut seen = BTreeSet::new();
        self.ordered_ids = ids.into_iter().filter(|id| seen.insert(*id)).collect();
        self.selected.retain(|id| seen.contains(id));
        self.anchor = self.anchor.filter(|id| self.ordered_ids.contains(id));
        self.focus = self.focus.filter(|id| self.ordered_ids.contains(id));
        if self.focus.is_none() {
            self.focus = self.ordered_ids.first().copied();
        }
    }

    /// Reconciles controller-owned selection state after a collection refresh.
    pub fn reconcile_selection(&mut self, ids: impl IntoIterator<Item = PhotoId>) {
        self.selected = ids
            .into_iter()
            .filter(|id| self.ordered_ids.contains(id))
            .collect();
        self.anchor = self
            .ordered_ids
            .iter()
            .find(|id| self.selected.contains(id))
            .copied();
        if self.focus.is_none() {
            self.focus = self.anchor.or_else(|| self.ordered_ids.first().copied());
        }
    }

    /// Updates the number of columns used by row-wise keyboard movement.
    pub fn set_columns(&mut self, columns: usize) {
        self.columns = columns.max(1);
    }

    #[must_use]
    pub fn ordered(&self) -> impl ExactSizeIterator<Item = PhotoId> + '_ {
        self.ordered_ids.iter().copied()
    }

    #[must_use]
    pub fn selected(&self) -> impl ExactSizeIterator<Item = PhotoId> + '_ {
        self.selected.iter().copied()
    }

    /// Returns the current selection in visible catalog order.
    ///
    /// `selected` remains ID-ordered for compatibility with controller snapshots;
    /// the grid and filmstrip use this projection when an ordered photo is needed.
    pub fn selected_in_order(&self) -> impl Iterator<Item = PhotoId> + '_ {
        self.ordered_ids
            .iter()
            .copied()
            .filter(|id| self.selected.contains(id))
    }

    #[must_use]
    pub fn is_selected(&self, photo_id: PhotoId) -> bool {
        self.selected.contains(&photo_id)
    }

    #[must_use]
    pub fn selected_count(&self) -> usize {
        self.selected.len()
    }

    #[must_use]
    pub const fn focus(&self) -> Option<PhotoId> {
        self.focus
    }

    #[must_use]
    pub const fn anchor(&self) -> Option<PhotoId> {
        self.anchor
    }

    #[must_use]
    pub const fn columns(&self) -> usize {
        self.columns
    }

    #[must_use]
    pub const fn zoom(&self) -> LighttableZoom {
        self.zoom
    }

    #[must_use]
    pub const fn layout(&self) -> LighttableLayout {
        self.layout
    }

    #[must_use]
    pub const fn left_panel_visible(&self) -> bool {
        self.left_panel_visible
    }

    #[must_use]
    pub const fn right_panel_visible(&self) -> bool {
        self.right_panel_visible
    }

    #[must_use]
    pub const fn culling_viewport(&self) -> CullingViewport {
        self.culling_viewport
    }

    #[must_use]
    pub const fn culling_restriction(&self) -> CullingRestriction {
        self.culling_restriction
    }

    pub fn culling_ids(&self) -> impl Iterator<Item = PhotoId> + '_ {
        let selection_only = self.culling_selection_only();
        self.ordered_ids
            .iter()
            .copied()
            .filter(move |id| !selection_only || self.selected.contains(id))
    }

    /// Returns the full ordered collection used by the persistent filmstrip.
    #[must_use]
    pub fn filmstrip_ids(&self) -> impl ExactSizeIterator<Item = PhotoId> + '_ {
        self.ordered_ids.iter().copied()
    }

    #[allow(clippy::too_many_lines)]
    pub fn apply(&mut self, action: LighttableSelectionAction) -> Option<PhotoId> {
        match action {
            LighttableSelectionAction::Select {
                photo_id,
                modifiers,
            } => {
                if !self.ordered_ids.contains(&photo_id) {
                    return None;
                }
                self.focus = Some(photo_id);
                if modifiers.range() {
                    self.select_range(photo_id, modifiers.extend());
                } else if modifiers.extend() {
                    if !self.selected.insert(photo_id) {
                        self.selected.remove(&photo_id);
                    }
                    self.anchor = Some(photo_id);
                } else {
                    self.selected.clear();
                    self.selected.insert(photo_id);
                    self.anchor = Some(photo_id);
                }
                Some(photo_id)
            }
            LighttableSelectionAction::Move {
                direction,
                modifiers,
            } => {
                let current = self.focus.or_else(|| self.ordered_ids.first().copied())?;
                let index = self.ordered_ids.iter().position(|id| *id == current)?;
                let next = match direction {
                    NavigationDirection::Previous => index.saturating_sub(1),
                    NavigationDirection::Next => index
                        .saturating_add(1)
                        .min(self.ordered_ids.len().saturating_sub(1)),
                    NavigationDirection::RowPrevious => index.saturating_sub(self.columns),
                    NavigationDirection::RowNext => index
                        .saturating_add(self.columns)
                        .min(self.ordered_ids.len().saturating_sub(1)),
                };
                let photo_id = self.ordered_ids[next];
                self.focus = Some(photo_id);
                if modifiers.extend() || modifiers.range() {
                    self.apply(LighttableSelectionAction::Select {
                        photo_id,
                        modifiers,
                    })
                } else {
                    // Darktable's ordinary arrow navigation moves the active
                    // thumbnail without replacing the persistent selection.
                    Some(photo_id)
                }
            }
            LighttableSelectionAction::OpenSelected => self
                .focus
                .filter(|focus| self.selected.contains(focus))
                .or_else(|| self.selected_in_order().next()),
            LighttableSelectionAction::Clear => {
                self.selected.clear();
                self.anchor = None;
                None
            }
            LighttableSelectionAction::SetZoom(zoom) => {
                self.zoom = zoom;
                self.columns = zoom.columns();
                None
            }
            LighttableSelectionAction::SetLayout(layout) => {
                if self.layout != layout && layout.shows_culling() {
                    self.culling_viewport.fit();
                }
                self.layout = layout;
                None
            }
            LighttableSelectionAction::SetLeftPanelVisible(visible) => {
                self.left_panel_visible = visible;
                None
            }
            LighttableSelectionAction::SetRightPanelVisible(visible) => {
                self.right_panel_visible = visible;
                None
            }
            LighttableSelectionAction::SetCullingRestriction(restriction) => {
                self.culling_restriction = restriction;
                None
            }
            LighttableSelectionAction::SetCullingActive(photo_id) => {
                if self.culling_ids().any(|id| id == photo_id) {
                    self.focus = Some(photo_id);
                    Some(photo_id)
                } else {
                    None
                }
            }
            LighttableSelectionAction::PanCulling { delta, bounds } => {
                self.culling_viewport.pan_by(delta, bounds);
                None
            }
            LighttableSelectionAction::ZoomCulling {
                delta_percent,
                bounds,
            } => {
                self.culling_viewport.zoom_by(delta_percent, bounds);
                None
            }
            LighttableSelectionAction::FitCulling => {
                self.culling_viewport.fit();
                None
            }
        }
    }

    fn select_range(&mut self, target: PhotoId, extend: bool) {
        let anchor = self.anchor.unwrap_or(target);
        let Some(left) = self.ordered_ids.iter().position(|id| *id == anchor) else {
            return;
        };
        let Some(right) = self.ordered_ids.iter().position(|id| *id == target) else {
            return;
        };
        if !extend {
            self.selected.clear();
        }
        let (start, end) = if left <= right {
            (left, right)
        } else {
            (right, left)
        };
        self.selected
            .extend(self.ordered_ids[start..=end].iter().copied());
    }

    fn culling_selection_only(&self) -> bool {
        match self.culling_restriction {
            CullingRestriction::Collection => false,
            CullingRestriction::Selection => true,
            CullingRestriction::Automatic => match self.layout {
                // Dynamic culling is explicitly driven by the selected set.
                LighttableLayout::CullingDynamic => true,
                // Preview follows the collection for one selected image, but
                // follows a multi-image selection when its active image is in it.
                LighttableLayout::Preview => {
                    self.selected.len() > 1
                        && self.focus.is_some_and(|id| self.selected.contains(&id))
                }
                // Fixed culling uses the collection when the selected images
                // already form the contiguous synchronized window. Otherwise
                // Darktable restricts navigation to the selection.
                LighttableLayout::Culling => {
                    self.selected.len() > 1
                        && self.focus.is_some_and(|id| self.selected.contains(&id))
                        && !self.selected_ids_are_contiguous()
                }
                LighttableLayout::FileManager | LighttableLayout::Zoomable => false,
            },
        }
    }

    fn selected_ids_are_contiguous(&self) -> bool {
        let Some(first) = self
            .ordered_ids
            .iter()
            .position(|id| self.selected.contains(id))
        else {
            return false;
        };
        let Some(last) = self
            .ordered_ids
            .iter()
            .rposition(|id| self.selected.contains(id))
        else {
            return false;
        };
        last.saturating_sub(first).saturating_add(1) == self.selected.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: u128) -> PhotoId {
        PhotoId::new(value).expect("non-zero test photo ID")
    }

    fn state() -> LighttableInteractionState {
        let mut state = LighttableInteractionState::new(2);
        state.set_order([id(1), id(2), id(3), id(4), id(5)]);
        state
    }

    #[test]
    fn plain_click_replaces_and_shift_click_selects_a_range() {
        let mut state = state();
        let _ = state.apply(LighttableSelectionAction::Select {
            photo_id: id(2),
            modifiers: SelectionModifiers::default(),
        });
        let _ = state.apply(LighttableSelectionAction::Select {
            photo_id: id(5),
            modifiers: SelectionModifiers::new(false, true),
        });
        assert_eq!(
            state.selected().collect::<Vec<_>>(),
            vec![id(2), id(3), id(4), id(5)]
        );
    }

    #[test]
    fn control_click_toggles_and_row_navigation_is_column_aware() {
        let mut state = state();
        let _ = state.apply(LighttableSelectionAction::Select {
            photo_id: id(2),
            modifiers: SelectionModifiers::default(),
        });
        let _ = state.apply(LighttableSelectionAction::Select {
            photo_id: id(4),
            modifiers: SelectionModifiers::new(true, false),
        });
        assert_eq!(state.selected().collect::<Vec<_>>(), vec![id(2), id(4)]);
        let _ = state.apply(LighttableSelectionAction::Move {
            direction: NavigationDirection::RowNext,
            modifiers: SelectionModifiers::default(),
        });
        assert_eq!(state.focus(), Some(id(5)));
        assert_eq!(state.selected().collect::<Vec<_>>(), vec![id(2), id(4)]);
    }

    #[test]
    fn navigation_clamps_at_grid_edges_and_reconciles_stale_catalog_updates() {
        let mut state = state();
        let _ = state.apply(LighttableSelectionAction::Select {
            photo_id: id(1),
            modifiers: SelectionModifiers::default(),
        });
        let _ = state.apply(LighttableSelectionAction::Move {
            direction: NavigationDirection::Previous,
            modifiers: SelectionModifiers::default(),
        });
        assert_eq!(state.focus(), Some(id(1)));
        let _ = state.apply(LighttableSelectionAction::Select {
            photo_id: id(5),
            modifiers: SelectionModifiers::default(),
        });
        state.set_order([id(1), id(2)]);
        assert!(state.selected().next().is_none());
        assert_eq!(state.apply(LighttableSelectionAction::OpenSelected), None);
    }

    #[test]
    fn zoom_is_bounded_and_controller_selection_is_ordered() {
        let mut state = state();
        state.reconcile_selection([id(4), id(2)]);
        assert_eq!(state.selected().collect::<Vec<_>>(), vec![id(2), id(4)]);
        assert_eq!(
            state.selected_in_order().collect::<Vec<_>>(),
            vec![id(2), id(4)]
        );
        let _ = state.apply(LighttableSelectionAction::SetZoom(LighttableZoom::Large));
        assert_eq!(state.zoom(), LighttableZoom::Large);
        assert_eq!(state.zoom().columns(), 4);
        assert_eq!(state.zoom().larger(), LighttableZoom::Large);
        assert_eq!(state.zoom().smaller(), LighttableZoom::Normal);
    }

    #[test]
    fn catalog_refresh_deduplicates_ids_without_reordering_live_selection() {
        let mut state = LighttableInteractionState::new(3);
        state.set_order([id(9), id(2), id(9), id(5)]);
        assert_eq!(
            state.ordered().collect::<Vec<_>>(),
            vec![id(9), id(2), id(5)]
        );
        let _ = state.apply(LighttableSelectionAction::Select {
            photo_id: id(5),
            modifiers: SelectionModifiers::default(),
        });
        state.set_order([id(5), id(2), id(5)]);
        assert_eq!(state.selected_count(), 1);
        assert!(state.is_selected(id(5)));
    }

    #[test]
    fn layout_and_culling_projection_keep_surfaces_typed() {
        let mut state = state();
        assert_eq!(LighttableLayout::FileManager.label(), "file manager");
        assert_eq!(LighttableLayout::CullingDynamic.label(), "dynamic culling");
        assert!(LighttableLayout::FileManager.shows_grid());
        assert!(LighttableLayout::Culling.shows_culling());
        assert!(!LighttableLayout::FileManager.shows_filmstrip());
        assert!(!LighttableLayout::Zoomable.shows_filmstrip());
        assert!(LighttableLayout::Preview.shows_filmstrip());
        state.apply(LighttableSelectionAction::Select {
            photo_id: id(4),
            modifiers: SelectionModifiers::default(),
        });
        state.apply(LighttableSelectionAction::SetCullingRestriction(
            CullingRestriction::Selection,
        ));
        assert_eq!(state.culling_ids().collect::<Vec<_>>(), vec![id(4)]);
        assert_eq!(
            state.apply(LighttableSelectionAction::SetCullingActive(id(3))),
            None
        );
        assert_eq!(
            state.apply(LighttableSelectionAction::SetCullingActive(id(4))),
            Some(id(4))
        );
    }

    #[test]
    fn culling_surface_keeps_collection_filmstrip_and_restricts_grid_ids() {
        let mut state = state();
        state.apply(LighttableSelectionAction::Select {
            photo_id: id(2),
            modifiers: SelectionModifiers::default(),
        });
        state.apply(LighttableSelectionAction::Select {
            photo_id: id(4),
            modifiers: SelectionModifiers::new(true, false),
        });
        state.apply(LighttableSelectionAction::SetLayout(
            LighttableLayout::CullingDynamic,
        ));
        assert_eq!(state.culling_ids().collect::<Vec<_>>(), vec![id(2), id(4)]);
        assert_eq!(
            state.filmstrip_ids().collect::<Vec<_>>(),
            vec![id(1), id(2), id(3), id(4), id(5)]
        );
    }

    #[test]
    fn automatic_culling_matches_darktable_selection_sync_rules() {
        let mut state = state();
        state.apply(LighttableSelectionAction::SetLayout(
            LighttableLayout::Preview,
        ));
        assert_eq!(
            state.culling_ids().collect::<Vec<_>>(),
            state.ordered().collect::<Vec<_>>()
        );

        state.apply(LighttableSelectionAction::Select {
            photo_id: id(2),
            modifiers: SelectionModifiers::default(),
        });
        assert_eq!(
            state.culling_ids().collect::<Vec<_>>(),
            state.ordered().collect::<Vec<_>>()
        );

        state.apply(LighttableSelectionAction::Select {
            photo_id: id(4),
            modifiers: SelectionModifiers::new(true, false),
        });
        assert_eq!(state.culling_ids().collect::<Vec<_>>(), vec![id(2), id(4)]);

        state.apply(LighttableSelectionAction::SetLayout(
            LighttableLayout::CullingDynamic,
        ));
        assert_eq!(state.culling_ids().collect::<Vec<_>>(), vec![id(2), id(4)]);
    }

    #[test]
    fn grid_zoom_and_culling_viewport_are_bounded() {
        let mut state = state();
        state.apply(LighttableSelectionAction::SetZoom(LighttableZoom::Large));
        state.apply(LighttableSelectionAction::SetLayout(
            LighttableLayout::Zoomable,
        ));
        assert_eq!(state.columns(), 4);
        assert_eq!(state.layout(), LighttableLayout::Zoomable);

        let bounds = PanBounds::new(-10, 20, -5, 15);
        state.apply(LighttableSelectionAction::PanCulling {
            delta: PanOffset::new(100, -100),
            bounds,
        });
        assert_eq!(state.culling_viewport().pan(), PanOffset::new(20, -5));
        state.apply(LighttableSelectionAction::ZoomCulling {
            delta_percent: 10_000,
            bounds,
        });
        assert_eq!(
            state.culling_viewport().zoom(),
            CullingZoom::new(CullingZoom::MAX_PERCENT)
        );
        state.apply(LighttableSelectionAction::FitCulling);
        assert_eq!(state.culling_viewport(), CullingViewport::new());
    }

    #[test]
    fn authoritative_empty_selection_clears_the_range_anchor() {
        let mut state = state();
        state.apply(LighttableSelectionAction::Select {
            photo_id: id(3),
            modifiers: SelectionModifiers::default(),
        });
        state.reconcile_selection([]);
        assert_eq!(state.anchor(), None);
    }

    #[test]
    fn mode_and_panel_projection_preserve_selection_and_filmstrip_state() {
        let mut state = state();
        state.apply(LighttableSelectionAction::Select {
            photo_id: id(2),
            modifiers: SelectionModifiers::default(),
        });
        let ordered = state.ordered().collect::<Vec<_>>();
        let selected = state.selected().collect::<Vec<_>>();
        state.apply(LighttableSelectionAction::SetLayout(
            LighttableLayout::Preview,
        ));
        state.apply(LighttableSelectionAction::SetLeftPanelVisible(false));
        state.apply(LighttableSelectionAction::SetRightPanelVisible(false));
        assert_eq!(state.ordered().collect::<Vec<_>>(), ordered);
        assert_eq!(state.selected().collect::<Vec<_>>(), selected);
        assert_eq!(state.filmstrip_ids().collect::<Vec<_>>(), ordered);
        assert_eq!(state.layout(), LighttableLayout::Preview);
        assert!(!state.left_panel_visible());
        assert!(!state.right_panel_visible());
    }
}
