//! GTK realization of the lighttable's photo grid, filmstrip, and selection.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use gtk4::accessible::{Property, State};
use gtk4::gdk;
use gtk4::prelude::*;
use rusttable_core::PhotoId;

use super::{PhotoSelectedHandler, lighttable_window::ThumbnailWindowChanged};
use crate::external_editor::ExternalEditorPanel;
use crate::gui::{
    DARKTABLE_UI_TOKENS, DarkroomView, ExportPanel, LighttableColorLabel, LighttableContentState,
    LighttableInteractionState, LighttablePhotoState, LighttableRating, LighttableSelectionAction,
    PhotoPreview, SelectionModifiers, ThemeRole, WorkspaceRole, apply_theme_role,
};
use crate::presentation::{PhotoDetailViewModel, PhotoWorkspaceViewModel, SelectedPreviewState};
use crate::views::lighttable::{FilmstripSpec, LighttableCollectionState, LighttableGridSpec};
use crate::widgets::thumbnail::{ThumbnailPair, ThumbnailState, ThumbnailSurface};

#[derive(Clone)]
pub(crate) struct WorkspaceRenderHandle {
    pub(super) lighttable: gtk4::GridView,
    pub(super) lighttable_empty_state: gtk4::Stack,
    pub(super) filmstrip: gtk4::FlowBox,
    pub(super) filmstrip_root: gtk4::Box,
    pub(super) darkroom_preview: PhotoPreview,
    pub(super) darkroom: DarkroomView,
    pub(super) workspace: gtk4::Stack,
    pub(super) photo_selected: Rc<RefCell<Option<PhotoSelectedHandler>>>,
    pub(super) thumbnail_window_changed: ThumbnailWindowChanged,
    pub(super) export_panel: ExportPanel,
    pub(super) external_editor_panel: ExternalEditorPanel,
    pub(super) photo_tiles: Rc<RefCell<BTreeMap<PhotoId, PhotoTilePair>>>,
    pub(super) interaction: Rc<RefCell<LighttableInteractionState>>,
    pub(super) photo_details: Rc<RefCell<BTreeMap<PhotoId, PhotoDetailViewModel>>>,
    pub(super) lighttable_workspace: Rc<RefCell<Option<PhotoWorkspaceViewModel>>>,
    pub(super) lighttable_filter: Rc<RefCell<Option<Vec<PhotoId>>>>,
    pub(super) photo_states: Rc<RefCell<BTreeMap<PhotoId, LighttablePhotoState>>>,
}

#[derive(Clone)]
pub(super) struct PhotoTilePair {
    pub(super) thumbnails: ThumbnailPair,
    pub(super) lighttable_button: Option<gtk4::Button>,
    filmstrip_button: gtk4::Button,
    filmstrip_pointer: gtk4::DrawingArea,
}

#[derive(Clone)]
struct PhotoSelectionContext {
    darkroom_preview: PhotoPreview,
    darkroom: DarkroomView,
    workspace: gtk4::Stack,
    photo_selected: Rc<RefCell<Option<PhotoSelectedHandler>>>,
    export_panel: ExportPanel,
    external_editor_panel: ExternalEditorPanel,
    photo_tiles: Rc<RefCell<BTreeMap<PhotoId, PhotoTilePair>>>,
    interaction: Rc<RefCell<LighttableInteractionState>>,
    photo_details: Rc<RefCell<BTreeMap<PhotoId, PhotoDetailViewModel>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhotoSurface {
    Grid,
    Filmstrip,
}

impl WorkspaceRenderHandle {
    #[allow(clippy::too_many_lines)]
    pub(super) fn render(
        &self,
        view_model: &PhotoWorkspaceViewModel,
        matching_photo_ids: Option<&[PhotoId]>,
    ) {
        let previous_thumbnail_states = self
            .photo_tiles
            .borrow()
            .iter()
            .map(|(photo_id, tile)| (*photo_id, tile.thumbnails.state()))
            .collect::<BTreeMap<_, _>>();
        let previous_details = self.photo_details.borrow().clone();
        self.lighttable.set_model(None::<&gtk4::NoSelection>);
        clear_flow_box(&self.filmstrip);
        self.photo_tiles.borrow_mut().clear();
        self.photo_details.borrow_mut().clear();
        let zoom = self.interaction.borrow().zoom();
        let layout = self.interaction.borrow().layout();
        let grid = lighttable_grid_for_allocation(
            &self.lighttable,
            &self.lighttable_empty_state,
            zoom,
            layout,
        );
        let columns = u32::try_from(grid.columns()).expect("lighttable columns fit u32");
        self.lighttable.set_min_columns(if layout.shows_culling() {
            1
        } else {
            columns.max(1)
        });
        self.lighttable.set_max_columns(if layout.shows_culling() {
            1
        } else {
            columns.max(1)
        });
        let darkroom_visible = self.workspace.visible_child_name().as_deref()
            == Some(WorkspaceRole::Darkroom.stack_name());
        self.filmstrip_root
            .set_visible(darkroom_visible || layout.shows_filmstrip());
        if layout.shows_culling() {
            self.lighttable.add_css_class("dt_culling_surface");
        } else {
            self.lighttable.remove_css_class("dt_culling_surface");
        }
        let browser = crate::gui::LibraryBrowserModel::from_workspace(view_model);
        let photos_by_id = browser
            .photos()
            .map(|photo| (photo.id(), photo.clone()))
            .collect::<BTreeMap<_, _>>();
        let visible_ids = matching_photo_ids.map_or_else(
            || {
                browser
                    .photos()
                    .map(crate::gui::model::LibraryPhoto::id)
                    .collect()
            },
            <[PhotoId]>::to_vec,
        );
        let mut seen_ids = BTreeSet::new();
        let visible_ids = visible_ids
            .into_iter()
            .filter(|photo_id| {
                seen_ids.insert(*photo_id)
                    && photos_by_id.contains_key(photo_id)
                    && view_model.detail(*photo_id).is_some()
            })
            .collect::<Vec<_>>();
        {
            let mut interaction = self.interaction.borrow_mut();
            interaction.set_columns(if layout.shows_culling() {
                1
            } else {
                columns as usize
            });
            interaction.set_order(visible_ids.clone());
        }
        let display_ids = {
            let interaction = self.interaction.borrow();
            if layout.shows_culling() {
                interaction.culling_ids().collect::<Vec<_>>()
            } else {
                interaction.ordered().collect::<Vec<_>>()
            }
        };
        // GridView keeps its children start-aligned. Center the realized row
        // in the available Darktable thumbtable surface; using the configured
        // density here would pin a short final row to the upper-left corner.
        let viewport_width = u16::try_from(self.lighttable_empty_state.allocated_width())
            .unwrap_or(0)
            .saturating_sub(12);
        let centered_grid = grid.centered_for_visible_count(viewport_width, display_ids.len());
        self.lighttable
            .set_margin_start(i32::from(centered_grid.horizontal_offset_px()));
        let selection = PhotoSelectionContext {
            darkroom_preview: self.darkroom_preview.clone(),
            darkroom: self.darkroom.clone(),
            workspace: self.workspace.clone(),
            photo_selected: Rc::clone(&self.photo_selected),
            export_panel: self.export_panel.clone(),
            external_editor_panel: self.external_editor_panel.clone(),
            photo_tiles: Rc::clone(&self.photo_tiles),
            interaction: Rc::clone(&self.interaction),
            photo_details: Rc::clone(&self.photo_details),
        };

        for photo_id in visible_ids.iter().copied() {
            let Some(photo) = photos_by_id.get(&photo_id) else {
                continue;
            };
            let Some(detail) = view_model.detail(photo_id) else {
                continue;
            };
            let detail = detail.clone();
            self.photo_details
                .borrow_mut()
                .insert(photo_id, detail.clone());
            let organization = self.photo_states.borrow().get(&photo_id).cloned();
            let (filmstrip_item, filmstrip_thumbnail, filmstrip_pointer) = filmstrip_item(
                photo_id,
                photo.title(),
                photo.secondary(),
                organization.as_ref(),
            );
            let grid_thumbnail = ThumbnailSurface::new(
                &format!("photo-thumbnail-{photo_id}"),
                &format!("Thumbnail for {}", photo.title()),
                i32::from(grid.thumbnail_width_px()),
                i32::from(grid.thumbnail_height_px()),
            );
            let thumbnail_state = retained_thumbnail_state(
                photo_id,
                &detail,
                &previous_details,
                &previous_thumbnail_states,
            );
            let thumbnails = ThumbnailPair::new(grid_thumbnail, filmstrip_thumbnail);
            if thumbnails.set_state(&thumbnail_state).is_err() {
                thumbnails.set_failed();
            }
            connect_photo_selection(
                &filmstrip_item,
                photo_id,
                detail,
                PhotoSurface::Filmstrip,
                &selection,
            );
            self.filmstrip.insert(&filmstrip_item, -1);
            self.photo_tiles.borrow_mut().insert(
                photo_id,
                PhotoTilePair {
                    thumbnails,
                    lighttable_button: None,
                    filmstrip_button: filmstrip_item,
                    filmstrip_pointer,
                },
            );
        }
        center_filmstrip(&self.filmstrip);
        let filmstrip = self.filmstrip.clone();
        gtk4::glib::idle_add_local_once(move || center_filmstrip(&filmstrip));
        // The shell owns and rebuilds these buttons; darkroom owns the
        // generation-tagged selection state. Rebind the latter after every
        // projection without duplicating the shell's click controllers.
        self.darkroom.install_filmstrip_interaction(&self.filmstrip);
        let previous_thumbnail_states = Rc::new(previous_thumbnail_states);
        let previous_details = Rc::new(previous_details);
        let photo_tiles = Rc::clone(&self.photo_tiles);
        let photo_details = Rc::clone(&self.photo_details);
        let selection_for_bind = selection.clone();
        let thumbnail_window_changed = Rc::clone(&self.thumbnail_window_changed);
        let photos_by_id_for_bind = photos_by_id;
        let view_model_for_bind = view_model.clone();
        let factory = gtk4::SignalListItemFactory::new();
        factory.connect_bind(move |_, object| {
            let Some(list_item) = object.downcast_ref::<gtk4::ListItem>() else {
                return;
            };
            let Some(photo_id) = list_item_photo_id(list_item) else {
                return;
            };
            let Some(photo) = photos_by_id_for_bind.get(&photo_id) else {
                return;
            };
            let Some(detail) = view_model_for_bind.detail(photo_id).cloned() else {
                return;
            };
            photo_details.borrow_mut().insert(photo_id, detail.clone());
            let (card, card_thumbnail) = lighttable_card(
                photo_id,
                photo.title(),
                photo.secondary(),
                photo.indicators(),
                centered_grid,
                layout,
            );
            let thumbnail_state = retained_thumbnail_state(
                photo_id,
                &detail,
                &previous_details,
                &previous_thumbnail_states,
            );
            let Some(filmstrip_thumbnail) = photo_tiles
                .borrow()
                .get(&photo_id)
                .map(|pair| pair.thumbnails.filmstrip())
            else {
                return;
            };
            let thumbnails = ThumbnailPair::new(card_thumbnail, filmstrip_thumbnail);
            if thumbnails.set_state(&thumbnail_state).is_err() {
                thumbnails.set_failed();
            }
            connect_photo_selection(
                &card,
                photo_id,
                detail,
                PhotoSurface::Grid,
                &selection_for_bind,
            );
            if let Some(pair) = photo_tiles.borrow_mut().get_mut(&photo_id) {
                pair.thumbnails = thumbnails;
                pair.lighttable_button = Some(card.clone());
            }
            list_item.set_child(Some(&card));
            if let Some(handler) = thumbnail_window_changed.borrow().as_ref() {
                handler();
            }
        });
        let photo_tiles = Rc::clone(&self.photo_tiles);
        factory.connect_unbind(move |_, object| {
            let Some(list_item) = object.downcast_ref::<gtk4::ListItem>() else {
                return;
            };
            if let Some(photo_id) = list_item_photo_id(list_item)
                && let Some(pair) = photo_tiles.borrow_mut().get_mut(&photo_id)
            {
                pair.lighttable_button = None;
            }
            list_item.set_child(None::<&gtk4::Widget>);
        });
        let display_ids = display_ids
            .into_iter()
            .filter(|photo_id| visible_ids.contains(photo_id))
            .collect::<Vec<_>>();
        let display_strings = display_ids
            .iter()
            .map(|photo_id| photo_id.get().to_string())
            .collect::<Vec<_>>();
        let display_string_refs = display_strings
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let model = gtk4::StringList::new(&display_string_refs);
        let selection_model = gtk4::NoSelection::new(Some(model));
        self.lighttable.set_factory(Some(&factory));
        self.lighttable.set_model(Some(&selection_model));
        let rendered_photos = display_ids.len();
        let collection_state = if rendered_photos == 0 {
            LighttableCollectionState::Empty
        } else {
            LighttableCollectionState::Ready(rendered_photos)
        };
        self.lighttable_empty_state.set_visible_child_name(
            LighttableContentState::from_rendered_count(collection_state.rendered_count())
                .stack_name(),
        );
        // Keep the native GridView visible whenever the projection contains cards.
        self.lighttable.set_visible(rendered_photos > 0);
        self.lighttable_empty_state.set_tooltip_text(
            (!collection_state.status_text().is_empty()).then_some(collection_state.status_text()),
        );
        self.sync_selection_styles();
        if let Some(handler) = self.thumbnail_window_changed.borrow().as_ref() {
            handler();
        }
    }

    pub(super) fn rerender_current(&self) {
        let workspace = self.lighttable_workspace.borrow();
        let Some(view_model) = workspace.as_ref() else {
            return;
        };
        let filter = self.lighttable_filter.borrow();
        self.render(view_model, filter.as_deref());
    }

    pub(super) fn sync_selection_styles(&self) {
        let state = self.interaction.borrow();
        sync_photo_buttons(&self.photo_tiles.borrow(), &state);
    }

    pub(super) fn focus_selected(&self) {
        let Some(focus) = self.interaction.borrow().focus() else {
            return;
        };
        if let Some(pair) = self.photo_tiles.borrow().get(&focus) {
            if pair.filmstrip_button.has_focus() {
                pair.filmstrip_button.grab_focus();
            } else if let Some(button) = pair.lighttable_button.as_ref() {
                button.grab_focus();
            }
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
        let Some(detail) = self.photo_details.borrow().get(&photo_id).cloned() else {
            return;
        };
        self.open_photo(photo_id, &detail);
    }

    pub(super) fn open_photo_by_id(&self, photo_id: PhotoId) -> bool {
        let Some(detail) = self.photo_details.borrow().get(&photo_id).cloned() else {
            return false;
        };
        let selected = self
            .interaction
            .borrow_mut()
            .apply(LighttableSelectionAction::Select {
                photo_id,
                modifiers: SelectionModifiers::default(),
            });
        if selected.is_none() {
            return false;
        }
        self.sync_selection_styles();
        self.open_photo(photo_id, &detail);
        true
    }

    pub(super) fn move_focus(
        &self,
        direction: crate::gui::NavigationDirection,
        modifiers: SelectionModifiers,
    ) -> Option<PhotoId> {
        let previous_focus = self.interaction.borrow().focus();
        let photo_id = self
            .interaction
            .borrow_mut()
            .apply(LighttableSelectionAction::Move {
                direction,
                modifiers,
            })?;
        self.sync_selection_styles();
        self.focus_selected();

        let darkroom_visible = self.workspace.visible_child_name().as_deref()
            == Some(WorkspaceRole::Darkroom.stack_name());
        if darkroom_visible && previous_focus != Some(photo_id) {
            let _ = self
                .interaction
                .borrow_mut()
                .apply(LighttableSelectionAction::Select {
                    photo_id,
                    modifiers: SelectionModifiers::default(),
                });
            self.sync_selection_styles();
            if let Some(detail) = self.photo_details.borrow().get(&photo_id).cloned() {
                self.open_photo(photo_id, &detail);
            }
        }
        Some(photo_id)
    }

    fn open_photo(&self, photo_id: PhotoId, detail: &PhotoDetailViewModel) {
        show_photo_detail(&self.darkroom_preview, detail);
        self.darkroom.set_detail(detail);
        self.darkroom
            .set_status(&format!("selected · {}", detail.title().as_str()));
        self.export_panel.set_selected(true);
        self.external_editor_panel.set_selection(1);
        if let Some(handler) = self.photo_selected.borrow().as_ref() {
            handler(photo_id, SelectionModifiers::default());
        }
        // The selection callback owns the application token, viewport generation, preview
        // loading state, histogram generation, and controller-owned rails. Present the new
        // child only after that binding is complete so GTK cannot paint the old lighttable
        // surface as the first darkroom frame.
        self.workspace
            .set_visible_child_name(WorkspaceRole::Darkroom.stack_name());
    }
}

pub(super) fn connect_filmstrip_resize(filmstrip: &gtk4::FlowBox) {
    let filmstrip_for_schedule = filmstrip.clone();
    let pending = Rc::new(std::cell::Cell::new(false));
    let schedule: Rc<dyn Fn()> = Rc::new(move || {
        if pending.replace(true) {
            return;
        }
        let pending = Rc::clone(&pending);
        let filmstrip = filmstrip_for_schedule.clone();
        gtk4::glib::idle_add_local_once(move || {
            pending.set(false);
            center_filmstrip(&filmstrip);
        });
    });
    filmstrip.connect_notify_local(Some("width"), {
        let schedule = Rc::clone(&schedule);
        move |_, _| schedule()
    });
    filmstrip.connect_notify_local(Some("height"), move |_, _| schedule());
}

fn center_filmstrip(filmstrip: &gtk4::FlowBox) {
    let mut item_count = 0_usize;
    let mut child = filmstrip.first_child();
    while let Some(widget) = child {
        item_count = item_count.saturating_add(1);
        child = widget.next_sibling();
    }
    let mut surface_width = filmstrip.allocated_width();
    let mut ancestor = filmstrip.parent();
    while let Some(widget) = ancestor {
        surface_width = surface_width.max(widget.allocated_width());
        ancestor = widget.parent();
    }
    let viewport_width = u16::try_from(surface_width).unwrap_or(0);
    let offset = FilmstripSpec::darktable().leading_offset_px(viewport_width, item_count);
    filmstrip.set_margin_start(i32::from(offset));
}

pub(super) fn connect_lighttable_resize(
    lighttable: &gtk4::GridView,
    render: WorkspaceRenderHandle,
) {
    let last_geometry = Rc::new(std::cell::Cell::new((0_u16, 0_u16)));
    let pending = Rc::new(std::cell::Cell::new(false));
    let lighttable = lighttable.clone();
    let measured_lighttable = lighttable.clone();
    let measured_viewport = render.lighttable_empty_state.clone();
    let observed_viewport = measured_viewport.clone();
    let schedule: Rc<dyn Fn()> = Rc::new(move || {
        let grid = lighttable_grid_for_allocation(
            &measured_lighttable,
            &measured_viewport,
            render.interaction.borrow().zoom(),
            render.interaction.borrow().layout(),
        );
        let geometry = (grid.card_width_px(), grid.thumbnail_height_px());
        if geometry == last_geometry.get() || pending.replace(true) {
            return;
        }
        last_geometry.set(geometry);
        let pending = Rc::clone(&pending);
        let render = render.clone();
        gtk4::glib::idle_add_local_once(move || {
            pending.set(false);
            render.rerender_current();
        });
    });
    lighttable.connect_notify_local(Some("width"), {
        let schedule = Rc::clone(&schedule);
        move |_, _| schedule()
    });
    lighttable.connect_notify_local(Some("height"), {
        let schedule = Rc::clone(&schedule);
        move |_, _| schedule()
    });
    observed_viewport.connect_notify_local(Some("width"), {
        let schedule = Rc::clone(&schedule);
        move |_, _| schedule()
    });
    observed_viewport.connect_notify_local(Some("height"), move |_, _| schedule());
}

fn sync_photo_buttons(
    photo_tiles: &BTreeMap<PhotoId, PhotoTilePair>,
    interaction: &LighttableInteractionState,
) {
    let selected = interaction.selected().collect::<BTreeSet<_>>();
    let focus = interaction.focus();
    for (id, pair) in photo_tiles {
        pair.filmstrip_pointer.set_visible(selected.contains(id));
        for button in pair
            .lighttable_button
            .iter()
            .chain(std::iter::once(&pair.filmstrip_button))
        {
            button.add_css_class(ThemeRole::PhotoCard.class_name());
            if selected.contains(id) {
                button.add_css_class(ThemeRole::SelectedPhoto.class_name());
            } else {
                button.remove_css_class(ThemeRole::SelectedPhoto.class_name());
            }
            button.set_focusable(focus == Some(*id));
            button.update_state(&[State::Selected(Some(selected.contains(id)))]);
        }
    }
}

fn connect_photo_selection(
    button: &gtk4::Button,
    photo_id: PhotoId,
    _detail: PhotoDetailViewModel,
    surface: PhotoSurface,
    context: &PhotoSelectionContext,
) {
    let photo_details = Rc::clone(&context.photo_details);
    let selection = context.clone();
    let button_for_gesture = button.clone();
    let gesture = gtk4::GestureClick::new();
    gesture.set_button(1);
    gesture.connect_pressed(move |gesture, n_press, _, _| {
        let state = gesture.current_event_state();
        let modifiers = SelectionModifiers::new(
            state.contains(gdk::ModifierType::CONTROL_MASK)
                || state.contains(gdk::ModifierType::SUPER_MASK),
            state.contains(gdk::ModifierType::SHIFT_MASK),
        );
        select_photo(
            &button_for_gesture,
            photo_id,
            modifiers,
            surface,
            &selection,
        );
        if surface == PhotoSurface::Grid
            && n_press >= 2
            && let Some(detail) = photo_details.borrow().get(&photo_id)
        {
            open_photo(&selection, photo_id, detail);
        }
    });
    button.add_controller(gesture);

    let selection = context.clone();
    let button_for_keyboard = button.clone();
    let key = gtk4::EventControllerKey::new();
    key.set_propagation_phase(gtk4::PropagationPhase::Capture);
    key.connect_key_pressed(move |_, key, _, modifiers| {
        if !matches!(key, gdk::Key::space | gdk::Key::Return | gdk::Key::KP_Enter) {
            return gtk4::glib::Propagation::Proceed;
        }
        select_photo(
            &button_for_keyboard,
            photo_id,
            SelectionModifiers::new(
                modifiers.contains(gdk::ModifierType::CONTROL_MASK)
                    || modifiers.contains(gdk::ModifierType::SUPER_MASK),
                modifiers.contains(gdk::ModifierType::SHIFT_MASK),
            ),
            surface,
            &selection,
        );
        gtk4::glib::Propagation::Stop
    });
    button.add_controller(key);
}

fn open_photo(context: &PhotoSelectionContext, photo_id: PhotoId, detail: &PhotoDetailViewModel) {
    show_photo_detail(&context.darkroom_preview, detail);
    context.darkroom.set_detail(detail);
    context
        .darkroom
        .set_status(&format!("selected · {}", detail.title().as_str()));
    context.export_panel.set_selected(true);
    context.external_editor_panel.set_selection(1);
    if let Some(handler) = context.photo_selected.borrow().as_ref() {
        handler(photo_id, SelectionModifiers::default());
    }
    // Keep the first darkroom frame coherent with the application selection token and
    // generation. The callback must finish binding those surfaces before GTK presents the
    // darkroom child, just as the native-open path does above.
    context
        .workspace
        .set_visible_child_name(WorkspaceRole::Darkroom.stack_name());
}

fn select_photo(
    button: &gtk4::Button,
    photo_id: PhotoId,
    modifiers: SelectionModifiers,
    surface: PhotoSurface,
    context: &PhotoSelectionContext,
) {
    let _ = context
        .interaction
        .borrow_mut()
        .apply(LighttableSelectionAction::Select {
            photo_id,
            modifiers,
        });
    let state = context.interaction.borrow();
    sync_photo_buttons(&context.photo_tiles.borrow(), &state);
    let detail = context.photo_details.borrow().get(&photo_id).cloned();
    if let Some(detail) = detail.as_ref() {
        show_photo_detail(&context.darkroom_preview, detail);
        context.darkroom.set_detail(detail);
        context
            .darkroom
            .set_status(&format!("selected · {}", detail.title().as_str()));
    }
    context
        .export_panel
        .set_selected(state.selected_count() > 0);
    context
        .external_editor_panel
        .set_selection(state.selected_count());
    drop(state);
    button.grab_focus();
    let darkroom_visible = context.workspace.visible_child_name().as_deref()
        == Some(WorkspaceRole::Darkroom.stack_name());
    if surface == PhotoSurface::Filmstrip && darkroom_visible {
        let _ = context.darkroom.select_filmstrip_photo(photo_id);
        if let Some(detail) = detail.as_ref() {
            open_photo(context, photo_id, detail);
        }
    } else if let Some(handler) = context.photo_selected.borrow().as_ref() {
        handler(photo_id, modifiers);
    }
}

fn lighttable_card(
    photo_id: PhotoId,
    title: &str,
    secondary: Option<&str>,
    indicators: crate::presentation::ThumbnailIndicators,
    grid: LighttableGridSpec,
    layout: crate::gui::LighttableLayout,
) -> (gtk4::Button, ThumbnailSurface) {
    let preview = layout == crate::gui::LighttableLayout::Preview;
    let card = gtk4::Box::new(gtk4::Orientation::Vertical, if preview { 0 } else { 4 });
    card.add_css_class("dt_photo_card");
    if preview {
        card.add_css_class("dt_preview_card");
    }
    let card_margin = if preview { 0 } else { 4 };
    card.set_margin_top(card_margin);
    card.set_margin_bottom(card_margin);
    card.set_margin_start(card_margin);
    card.set_margin_end(card_margin);
    let thumbnail = ThumbnailSurface::new(
        &format!("photo-thumbnail-{photo_id}"),
        &format!("Thumbnail for {title}"),
        i32::from(grid.thumbnail_width_px()),
        i32::from(grid.thumbnail_height_px()),
    );
    apply_theme_role(thumbnail.widget(), ThemeRole::ThumbnailImage);
    let thumbnail_overlay = gtk4::Overlay::new();
    thumbnail_overlay.set_child(Some(thumbnail.widget()));
    if !preview {
        let badges = thumbnail_badges(indicators);
        badges.set_halign(gtk4::Align::End);
        badges.set_valign(gtk4::Align::Start);
        thumbnail_overlay.add_overlay(&badges);
    }
    card.append(&thumbnail_overlay);
    if !preview {
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
    }
    let button = gtk4::Button::new();
    button.set_widget_name(&format!("photo-{photo_id}"));
    apply_theme_role(&button, ThemeRole::PhotoCard);
    if preview {
        button.add_css_class("dt_preview_card");
    }
    button.set_child(Some(&card));
    let metadata_height =
        i32::from(DARKTABLE_UI_TOKENS.cards.metadata_height_px) * i32::from(u8::from(!preview));
    button.set_size_request(
        i32::from(grid.card_width_px()),
        i32::from(grid.thumbnail_height_px()).saturating_add(metadata_height),
    );
    button.set_tooltip_text(Some(title));
    button.set_accessible_role(gtk4::AccessibleRole::Button);
    button.update_property(&[Property::Label(&format!("Select {title}"))]);
    button.set_focus_on_click(true);
    (button, thumbnail)
}

fn lighttable_grid_for_allocation(
    lighttable: &gtk4::GridView,
    viewport: &gtk4::Stack,
    zoom: crate::gui::LighttableZoom,
    layout: crate::gui::LighttableLayout,
) -> LighttableGridSpec {
    LighttableGridSpec::for_layout_viewport(
        layout,
        zoom,
        u16::try_from(viewport.allocated_width().max(lighttable.allocated_width())).unwrap_or(0),
        u16::try_from(viewport.allocated_height()).unwrap_or(0),
    )
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

fn filmstrip_item(
    photo_id: PhotoId,
    title: &str,
    secondary: Option<&str>,
    organization: Option<&LighttablePhotoState>,
) -> (gtk4::Button, ThumbnailSurface, gtk4::DrawingArea) {
    let geometry = FilmstripSpec::darktable();
    let thumbnail = ThumbnailSurface::new(
        &format!("filmstrip-thumbnail-{photo_id}"),
        &format!("Filmstrip thumbnail for {title}"),
        i32::from(geometry.width_px()),
        i32::from(geometry.height_px()),
    );
    let button = gtk4::Button::new();
    button.set_widget_name(&format!("filmstrip-photo-{photo_id}"));
    apply_theme_role(&button, ThemeRole::PhotoCard);
    button.add_css_class("dt_filmstrip_item");
    button.set_size_request(
        i32::from(geometry.width_px()),
        i32::from(geometry.height_px()),
    );
    button.set_tooltip_text(Some(title));
    button.update_property(&[Property::Label(&format!("Select {title} in filmstrip"))]);
    button.set_focus_on_click(true);
    let surface = gtk4::Overlay::new();
    surface.set_child(Some(thumbnail.widget()));
    if let Some(format) = secondary
        .and_then(|text| text.split('\u{b7}').next())
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        let format_label = gtk4::Label::new(Some(format));
        format_label.set_widget_name(&format!("filmstrip-format-{photo_id}"));
        format_label.add_css_class("dt_filmstrip_format");
        format_label.set_halign(gtk4::Align::Start);
        format_label.set_valign(gtk4::Align::Start);
        surface.add_overlay(&format_label);
    }
    if let Some(organization) = organization {
        let metadata = filmstrip_organization(photo_id, organization);
        metadata.set_halign(gtk4::Align::Fill);
        metadata.set_valign(gtk4::Align::End);
        surface.add_overlay(&metadata);
    }
    let pointer = filmstrip_selection_pointer(photo_id);
    surface.add_overlay(&pointer);
    button.set_child(Some(&surface));
    (button, thumbnail, pointer)
}

fn filmstrip_selection_pointer(photo_id: PhotoId) -> gtk4::DrawingArea {
    let pointer = gtk4::DrawingArea::new();
    pointer.set_widget_name(&format!("filmstrip-selection-pointer-{photo_id}"));
    pointer.add_css_class("dt_filmstrip_selection_pointer");
    pointer.set_content_width(18);
    pointer.set_content_height(8);
    pointer.set_halign(gtk4::Align::Center);
    pointer.set_valign(gtk4::Align::Start);
    pointer.set_can_target(false);
    pointer.set_visible(false);
    pointer.set_draw_func(|_, context, width, height| {
        context.move_to(0.0, 0.0);
        context.line_to(f64::from(width), 0.0);
        context.line_to(f64::from(width) / 2.0, f64::from(height));
        context.close_path();
        context.set_source_rgb(0.93, 0.93, 0.93);
        let _ = context.fill();
    });
    pointer
}

fn filmstrip_organization(photo_id: PhotoId, state: &LighttablePhotoState) -> gtk4::Box {
    let metadata = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
    metadata.set_widget_name(&format!("filmstrip-metadata-{photo_id}"));
    metadata.add_css_class("dt_filmstrip_metadata");
    let rating = gtk4::Label::new(Some(match state.rating() {
        LighttableRating::Rejected => "\u{2715}",
        rating => match rating.stars().unwrap_or(0) {
            0 => "\u{2606}\u{2606}\u{2606}\u{2606}\u{2606}",
            1 => "\u{2605}\u{2606}\u{2606}\u{2606}\u{2606}",
            2 => "\u{2605}\u{2605}\u{2606}\u{2606}\u{2606}",
            3 => "\u{2605}\u{2605}\u{2605}\u{2606}\u{2606}",
            4 => "\u{2605}\u{2605}\u{2605}\u{2605}\u{2606}",
            _ => "\u{2605}\u{2605}\u{2605}\u{2605}\u{2605}",
        },
    }));
    rating.set_widget_name(&format!("filmstrip-rating-{photo_id}"));
    rating.set_tooltip_text(Some(state.rating().label()));
    metadata.append(&rating);
    let tags = gtk4::Box::new(gtk4::Orientation::Horizontal, 1);
    tags.set_hexpand(true);
    tags.set_halign(gtk4::Align::End);
    for label in state.color_labels() {
        tags.append(&filmstrip_color_tag(photo_id, label));
    }
    metadata.append(&tags);
    metadata
}

fn filmstrip_color_tag(photo_id: PhotoId, label: LighttableColorLabel) -> gtk4::Label {
    let tag = gtk4::Label::new(Some("\u{25cf}"));
    tag.set_widget_name(&format!("filmstrip-{}-tag-{photo_id}", label.label()));
    tag.add_css_class("dt_filmstrip_color_tag");
    tag.add_css_class(&format!("dt_color_{}", label.label()));
    tag.set_tooltip_text(Some(&format!("{} color label", label.label())));
    tag
}

fn show_photo_detail(preview: &PhotoPreview, detail: &PhotoDetailViewModel) {
    preview.set_detail(detail);
}

fn retained_thumbnail_state(
    photo_id: PhotoId,
    detail: &PhotoDetailViewModel,
    previous_details: &BTreeMap<PhotoId, PhotoDetailViewModel>,
    previous_states: &BTreeMap<PhotoId, ThumbnailState>,
) -> ThumbnailState {
    if let SelectedPreviewState::Ready(metadata) = detail.selected_preview() {
        return ThumbnailState::Ready(metadata.clone());
    }
    if previous_details.get(&photo_id) == Some(detail) {
        previous_states
            .get(&photo_id)
            .cloned()
            .unwrap_or(ThumbnailState::Loading)
    } else {
        ThumbnailState::Loading
    }
}

fn clear_flow_box(flow_box: &gtk4::FlowBox) {
    // Remove FlowBox wrappers through FlowBox so GTK keeps its sibling list synchronized.
    while let Some(child) = flow_box.first_child() {
        flow_box.remove(&child);
    }
}

fn list_item_photo_id(list_item: &gtk4::ListItem) -> Option<PhotoId> {
    let object = list_item.item()?.downcast::<gtk4::StringObject>().ok()?;
    object.string().parse::<u128>().ok().and_then(PhotoId::new)
}

#[cfg(test)]
#[path = "lighttable/tests.rs"]
mod tests;
