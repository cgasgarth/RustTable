//! GTK realization of the lighttable's photo grid, filmstrip, and selection.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use gtk4::gdk;
use gtk4::prelude::*;
use rusttable_core::PhotoId;

use super::lighttable::{LighttableCollectionState, LighttableGridSpec};
use super::runtime::PhotoSelectedHandler;
use super::thumbnail::{ThumbnailPair, ThumbnailState, ThumbnailSurface};
use super::{
    ExportPanel, LighttableContentState, LighttableInteractionState, LighttableSelectionAction,
    PhotoPreview, SelectionModifiers, THUMBNAIL_METRICS, ThemeRole, WorkspaceRole,
    apply_theme_role,
};
use crate::external_editor::ExternalEditorPanel;
use crate::presentation::{PhotoDetailViewModel, PhotoWorkspaceViewModel};

#[derive(Clone)]
pub(super) struct WorkspaceRenderHandle {
    pub(super) lighttable: gtk4::FlowBox,
    pub(super) lighttable_empty_state: gtk4::Stack,
    pub(super) filmstrip: gtk4::FlowBox,
    pub(super) darkroom_preview: PhotoPreview,
    pub(super) workspace: gtk4::Stack,
    pub(super) photo_selected: Rc<RefCell<Option<PhotoSelectedHandler>>>,
    pub(super) export_panel: ExportPanel,
    pub(super) external_editor_panel: ExternalEditorPanel,
    pub(super) photo_tiles: Rc<RefCell<BTreeMap<PhotoId, PhotoTilePair>>>,
    pub(super) interaction: Rc<RefCell<LighttableInteractionState>>,
    pub(super) photo_details: Rc<RefCell<BTreeMap<PhotoId, PhotoDetailViewModel>>>,
    pub(super) lighttable_workspace: Rc<RefCell<Option<PhotoWorkspaceViewModel>>>,
    pub(super) lighttable_filter: Rc<RefCell<Option<BTreeSet<PhotoId>>>>,
}

#[derive(Clone)]
pub(super) struct PhotoTilePair {
    pub(super) thumbnails: ThumbnailPair,
    lighttable_button: gtk4::Button,
    filmstrip_button: gtk4::Button,
}

#[derive(Clone)]
struct PhotoSelectionContext {
    darkroom_preview: PhotoPreview,
    workspace: gtk4::Stack,
    photo_selected: Rc<RefCell<Option<PhotoSelectedHandler>>>,
    export_panel: ExportPanel,
    external_editor_panel: ExternalEditorPanel,
    photo_tiles: Rc<RefCell<BTreeMap<PhotoId, PhotoTilePair>>>,
    interaction: Rc<RefCell<LighttableInteractionState>>,
    photo_details: Rc<RefCell<BTreeMap<PhotoId, PhotoDetailViewModel>>>,
}

impl WorkspaceRenderHandle {
    pub(super) fn render(
        &self,
        view_model: &PhotoWorkspaceViewModel,
        matching_photo_ids: Option<&BTreeSet<PhotoId>>,
    ) {
        let mut previous_thumbnail_states = self
            .photo_tiles
            .borrow()
            .iter()
            .map(|(photo_id, tile)| (*photo_id, tile.thumbnails.state()))
            .collect::<BTreeMap<_, _>>();
        let previous_details = self.photo_details.borrow().clone();
        clear_children(&self.lighttable);
        clear_children(&self.filmstrip);
        self.photo_tiles.borrow_mut().clear();
        self.photo_details.borrow_mut().clear();
        let zoom = self.interaction.borrow().zoom();
        let grid = LighttableGridSpec::for_zoom(zoom);
        let columns = u32::try_from(grid.columns()).expect("lighttable columns fit u32");
        self.lighttable.set_max_children_per_line(columns);
        let browser = super::LibraryBrowserModel::from_workspace(view_model);
        let visible_ids = browser
            .photos()
            .filter(|photo| matching_photo_ids.is_none_or(|ids| ids.contains(&photo.id())))
            .filter(|photo| view_model.detail(photo.id()).is_some())
            .map(super::model::LibraryPhoto::id)
            .collect::<Vec<_>>();
        {
            let mut interaction = self.interaction.borrow_mut();
            interaction.set_columns(columns as usize);
            interaction.set_order(visible_ids);
        }
        let mut rendered_photos = 0;
        let selection = PhotoSelectionContext {
            darkroom_preview: self.darkroom_preview.clone(),
            workspace: self.workspace.clone(),
            photo_selected: Rc::clone(&self.photo_selected),
            export_panel: self.export_panel.clone(),
            external_editor_panel: self.external_editor_panel.clone(),
            photo_tiles: Rc::clone(&self.photo_tiles),
            interaction: Rc::clone(&self.interaction),
            photo_details: Rc::clone(&self.photo_details),
        };

        for photo in browser.photos() {
            if matching_photo_ids.is_some_and(|ids| !ids.contains(&photo.id())) {
                continue;
            }
            let Some(detail) = view_model.detail(photo.id()) else {
                continue;
            };
            let detail = detail.clone();
            self.photo_details
                .borrow_mut()
                .insert(photo.id(), detail.clone());
            let (card, card_thumbnail) = lighttable_card(
                photo.id(),
                photo.title(),
                photo.secondary(),
                photo.indicators(),
                grid,
            );
            let (filmstrip_item, filmstrip_thumbnail) = filmstrip_item(photo.id(), photo.title());
            let thumbnail_state = retained_thumbnail_state(
                photo.id(),
                &detail,
                &previous_details,
                &mut previous_thumbnail_states,
            );
            let thumbnails = ThumbnailPair::new(card_thumbnail, filmstrip_thumbnail);
            if thumbnails.set_state(&thumbnail_state).is_err() {
                thumbnails.set_failed();
            }
            connect_photo_selection(&card, photo.id(), detail.clone(), &selection);
            connect_photo_selection(&filmstrip_item, photo.id(), detail, &selection);
            self.lighttable.insert(&card, -1);
            self.filmstrip.insert(&filmstrip_item, -1);
            self.photo_tiles.borrow_mut().insert(
                photo.id(),
                PhotoTilePair {
                    thumbnails,
                    lighttable_button: card,
                    filmstrip_button: filmstrip_item,
                },
            );
            rendered_photos += 1;
        }
        let collection_state = if rendered_photos == 0 {
            LighttableCollectionState::Empty
        } else {
            LighttableCollectionState::Ready(rendered_photos)
        };
        self.lighttable_empty_state.set_visible_child_name(
            LighttableContentState::from_rendered_count(collection_state.rendered_count())
                .stack_name(),
        );
        self.lighttable_empty_state.set_tooltip_text(
            (!collection_state.status_text().is_empty()).then_some(collection_state.status_text()),
        );
        self.sync_selection_styles();
    }

    pub(super) fn rerender_current(&self) {
        let workspace = self.lighttable_workspace.borrow();
        let Some(view_model) = workspace.as_ref() else {
            return;
        };
        let filter = self.lighttable_filter.borrow();
        self.render(view_model, filter.as_ref());
    }

    pub(super) fn sync_selection_styles(&self) {
        let state = self.interaction.borrow();
        let selected = state.selected().collect::<BTreeSet<_>>();
        let focus = state.focus();
        drop(state);
        for (id, pair) in self.photo_tiles.borrow().iter() {
            let css_classes = if selected.contains(id) {
                vec![
                    ThemeRole::PhotoCard.class_name(),
                    ThemeRole::SelectedPhoto.class_name(),
                ]
            } else {
                vec![ThemeRole::PhotoCard.class_name()]
            };
            for button in [&pair.lighttable_button, &pair.filmstrip_button] {
                button.set_css_classes(&css_classes);
                button.set_focusable(focus == Some(*id));
            }
        }
    }

    pub(super) fn focus_selected(&self) {
        let Some(focus) = self.interaction.borrow().focus() else {
            return;
        };
        if let Some(pair) = self.photo_tiles.borrow().get(&focus) {
            pair.lighttable_button.grab_focus();
        }
    }

    pub(super) fn open_focused(&self) {
        let Some(photo_id) = self
            .interaction
            .borrow_mut()
            .apply(LighttableSelectionAction::OpenSelected)
        else {
            return;
        };
        self.open_photo(photo_id);
    }

    fn open_photo(&self, photo_id: PhotoId) {
        let Some(detail) = self.photo_details.borrow().get(&photo_id).cloned() else {
            return;
        };
        show_photo_detail(&self.darkroom_preview, &detail);
        self.export_panel.set_selected(true);
        self.external_editor_panel.set_selection(1);
        self.workspace
            .set_visible_child_name(WorkspaceRole::Darkroom.stack_name());
        if let Some(handler) = self.photo_selected.borrow().as_ref() {
            handler(photo_id);
        }
    }
}

fn connect_photo_selection(
    button: &gtk4::Button,
    photo_id: PhotoId,
    _detail: PhotoDetailViewModel,
    context: &PhotoSelectionContext,
) {
    let interaction = Rc::clone(&context.interaction);
    let photo_tiles = Rc::clone(&context.photo_tiles);
    let photo_details = Rc::clone(&context.photo_details);
    let darkroom_preview = context.darkroom_preview.clone();
    let workspace = context.workspace.clone();
    let handler = Rc::clone(&context.photo_selected);
    let export_panel = context.export_panel.clone();
    let external_editor_panel = context.external_editor_panel.clone();
    let gesture = gtk4::GestureClick::new();
    gesture.set_button(1);
    gesture.connect_pressed(move |gesture, n_press, _, _| {
        let state = gesture.current_event_state();
        let modifiers = SelectionModifiers::new(
            state.contains(gdk::ModifierType::CONTROL_MASK)
                || state.contains(gdk::ModifierType::SUPER_MASK),
            state.contains(gdk::ModifierType::SHIFT_MASK),
        );
        let _ = interaction
            .borrow_mut()
            .apply(LighttableSelectionAction::Select {
                photo_id,
                modifiers,
            });
        let selected = interaction.borrow().selected().collect::<BTreeSet<_>>();
        let focus = interaction.borrow().focus();
        for (id, pair) in photo_tiles.borrow().iter() {
            let css_classes = if selected.contains(id) {
                vec![
                    ThemeRole::PhotoCard.class_name(),
                    ThemeRole::SelectedPhoto.class_name(),
                ]
            } else {
                vec![ThemeRole::PhotoCard.class_name()]
            };
            for button in [&pair.lighttable_button, &pair.filmstrip_button] {
                button.set_css_classes(&css_classes);
                button.set_focusable(focus == Some(*id));
            }
        }
        export_panel.set_selected(!selected.is_empty());
        external_editor_panel.set_selection(selected.len());
        if let Some(handler) = handler.borrow().as_ref() {
            handler(photo_id);
        }
        if n_press >= 2
            && let Some(detail) = photo_details.borrow().get(&photo_id)
        {
            show_photo_detail(&darkroom_preview, detail);
            workspace.set_visible_child_name(WorkspaceRole::Darkroom.stack_name());
        }
    });
    button.add_controller(gesture);
}

fn lighttable_card(
    photo_id: PhotoId,
    title: &str,
    secondary: Option<&str>,
    indicators: crate::presentation::ThumbnailIndicators,
    grid: LighttableGridSpec,
) -> (gtk4::Button, ThumbnailSurface) {
    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    card.set_margin_top(4);
    card.set_margin_bottom(4);
    card.set_margin_start(4);
    card.set_margin_end(4);
    let thumbnail = ThumbnailSurface::new(
        &format!("photo-thumbnail-{photo_id}"),
        &format!("Thumbnail for {title}"),
        i32::from(grid.thumbnail_width_px()),
        i32::from(grid.thumbnail_height_px()),
    );
    apply_theme_role(thumbnail.widget(), ThemeRole::ThumbnailImage);
    let thumbnail_overlay = gtk4::Overlay::new();
    thumbnail_overlay.set_child(Some(thumbnail.widget()));
    let badges = thumbnail_badges(indicators);
    badges.set_halign(gtk4::Align::End);
    badges.set_valign(gtk4::Align::Start);
    thumbnail_overlay.add_overlay(&badges);
    card.append(&thumbnail_overlay);
    let title_label = gtk4::Label::new(Some(title));
    title_label.set_halign(gtk4::Align::Start);
    title_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    title_label.set_max_width_chars(22);
    title_label.set_single_line_mode(true);
    card.append(&title_label);
    if let Some(secondary) = secondary {
        let secondary_label = gtk4::Label::new(Some(secondary));
        secondary_label.set_halign(gtk4::Align::Start);
        secondary_label.add_css_class("dim-label");
        secondary_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        secondary_label.set_max_width_chars(22);
        secondary_label.set_single_line_mode(true);
        card.append(&secondary_label);
    }
    let button = gtk4::Button::new();
    button.set_widget_name(&format!("photo-{photo_id}"));
    apply_theme_role(&button, ThemeRole::PhotoCard);
    button.set_child(Some(&card));
    button.set_tooltip_text(Some(title));
    button.set_accessible_role(gtk4::AccessibleRole::Button);
    (button, thumbnail)
}

fn thumbnail_badges(indicators: crate::presentation::ThumbnailIndicators) -> gtk4::Box {
    let badges = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
    badges.set_widget_name("thumbnail-indicators");
    badges.add_css_class("dt_thumbnail_indicators");
    for (visible, text, name) in [
        (indicators.grouped, "G", "grouped photo"),
        (indicators.local_copy, "C", "local copy"),
        (indicators.altered, "●", "altered edit"),
    ] {
        if visible {
            let badge = gtk4::Label::new(Some(text));
            badge.set_tooltip_text(Some(name));
            badges.append(&badge);
        }
    }
    badges
}

fn filmstrip_item(photo_id: PhotoId, title: &str) -> (gtk4::Button, ThumbnailSurface) {
    let thumbnail = ThumbnailSurface::new(
        &format!("filmstrip-thumbnail-{photo_id}"),
        &format!("Filmstrip thumbnail for {title}"),
        i32::from(THUMBNAIL_METRICS.filmstrip_width_px),
        i32::from(THUMBNAIL_METRICS.filmstrip_height_px),
    );
    let button = gtk4::Button::new();
    button.set_widget_name(&format!("filmstrip-photo-{photo_id}"));
    apply_theme_role(&button, ThemeRole::PhotoCard);
    button.add_css_class("dt_filmstrip_item");
    button.set_tooltip_text(Some(title));
    button.set_child(Some(thumbnail.widget()));
    (button, thumbnail)
}

fn show_photo_detail(preview: &PhotoPreview, detail: &PhotoDetailViewModel) {
    preview.set_detail(detail);
}

fn retained_thumbnail_state(
    photo_id: PhotoId,
    detail: &PhotoDetailViewModel,
    previous_details: &BTreeMap<PhotoId, PhotoDetailViewModel>,
    previous_states: &mut BTreeMap<PhotoId, ThumbnailState>,
) -> ThumbnailState {
    if previous_details.get(&photo_id) == Some(detail) {
        previous_states
            .remove(&photo_id)
            .unwrap_or(ThumbnailState::Loading)
    } else {
        ThumbnailState::Loading
    }
}

fn clear_children(container: &impl IsA<gtk4::Widget>) {
    while let Some(child) = container.first_child() {
        child.unparent();
    }
}

#[cfg(test)]
mod tests {
    use super::{ThumbnailState, retained_thumbnail_state};
    use crate::presentation::{
        PhotoDetailViewModel, PresentationText, PreviewDimensions, Rgba8PreviewMetadata,
    };
    use rusttable_core::PhotoId;
    use std::collections::BTreeMap;

    fn id(value: u128) -> PhotoId {
        PhotoId::new(value).expect("non-zero test photo ID")
    }

    fn text(value: &str) -> PresentationText {
        PresentationText::new(value).expect("valid test text")
    }

    fn detail(photo_id: PhotoId, title: &str) -> PhotoDetailViewModel {
        PhotoDetailViewModel::new(photo_id, text(title), Vec::new())
    }

    fn ready_thumbnail() -> ThumbnailState {
        ThumbnailState::Ready(
            Rgba8PreviewMetadata::new(
                PreviewDimensions::new(2, 1).expect("non-zero dimensions"),
                text("thumbnail ready"),
                vec![0; 8],
            )
            .expect("valid RGBA8 thumbnail"),
        )
    }

    #[test]
    fn rerender_retains_completed_thumbnail_for_an_unchanged_detail() {
        let photo_id = id(1);
        let current = detail(photo_id, "photo.png");
        let mut previous_states = BTreeMap::from([(photo_id, ready_thumbnail())]);
        let previous_details = BTreeMap::from([(photo_id, current.clone())]);

        assert_eq!(
            retained_thumbnail_state(photo_id, &current, &previous_details, &mut previous_states,),
            ready_thumbnail()
        );
        assert!(previous_states.is_empty());
    }

    #[test]
    fn rerender_resets_thumbnail_when_catalog_detail_changes() {
        let photo_id = id(1);
        let current = detail(photo_id, "new-photo.png");
        let previous_details = BTreeMap::from([(photo_id, detail(photo_id, "old-photo.png"))]);
        let mut previous_states = BTreeMap::from([(photo_id, ready_thumbnail())]);

        assert_eq!(
            retained_thumbnail_state(photo_id, &current, &previous_details, &mut previous_states,),
            ThumbnailState::Loading
        );
        assert_eq!(previous_states.get(&photo_id), Some(&ready_thumbnail()));
    }

    #[test]
    fn rerender_retains_unavailable_and_failed_states() {
        let photo_id = id(1);
        let current = detail(photo_id, "photo.png");
        let previous_details = BTreeMap::from([(photo_id, current.clone())]);

        for state in [ThumbnailState::Unavailable, ThumbnailState::Failed] {
            let mut previous_states = BTreeMap::from([(photo_id, state.clone())]);
            assert_eq!(
                retained_thumbnail_state(
                    photo_id,
                    &current,
                    &previous_details,
                    &mut previous_states,
                ),
                state
            );
        }
    }
}
