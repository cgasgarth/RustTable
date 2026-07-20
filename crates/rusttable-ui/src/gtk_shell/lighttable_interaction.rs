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
            .copied()
            .or(self.anchor);
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
                self.apply(LighttableSelectionAction::Select {
                    photo_id: self.ordered_ids[next],
                    modifiers,
                })
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
}
