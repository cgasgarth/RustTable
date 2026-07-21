//! Darkroom-local viewport adjuncts: histogram rendering and filmstrip routing.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::accessible::{Property, State};
use gtk4::gdk;
use gtk4::prelude::*;
use rusttable_core::PhotoId;

use crate::viewport_presentation::ViewportGeneration;

const MAX_HISTOGRAM_BINS: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HistogramState {
    Empty,
    Unavailable,
    Ready,
}

#[derive(Debug, Clone, PartialEq)]
struct HistogramData {
    bins: Vec<[f32; 3]>,
}

impl HistogramData {
    fn new(red: &[f32], green: &[f32], blue: &[f32]) -> Option<Self> {
        if red.is_empty()
            || red.len() != green.len()
            || red.len() != blue.len()
            || red.len() > MAX_HISTOGRAM_BINS
            || red
                .iter()
                .chain(green)
                .chain(blue)
                .any(|value| !value.is_finite() || *value < 0.0)
        {
            return None;
        }
        Some(Self {
            bins: red
                .iter()
                .zip(green)
                .zip(blue)
                .map(|((red, green), blue)| [*red, *green, *blue])
                .collect(),
        })
    }
}

#[derive(Clone)]
pub(super) struct HistogramView {
    stack: gtk4::Stack,
    chart: gtk4::DrawingArea,
    data: Rc<RefCell<Option<HistogramData>>>,
    state: Rc<RefCell<HistogramState>>,
}

impl HistogramView {
    pub(super) fn new(stack: gtk4::Stack) -> Self {
        clear_children(&stack);
        let data = Rc::new(RefCell::new(None));
        let chart = gtk4::DrawingArea::new();
        chart.set_widget_name("darkroom-histogram-chart");
        chart.set_content_width(220);
        chart.set_content_height(92);
        chart.set_hexpand(true);
        chart.set_vexpand(true);
        chart.set_accessible_role(gtk4::AccessibleRole::Img);
        chart.update_property(&[Property::Label("Rendered image histogram")]);
        chart.set_tooltip_text(Some("Rendered image histogram"));
        install_histogram_draw(&chart, &data);

        let empty = histogram_status(
            "darkroom-histogram-empty",
            "select a photo to show the histogram",
        );
        let unavailable = histogram_status(
            "darkroom-histogram-unavailable",
            "histogram unavailable for this preview",
        );
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&unavailable, Some("unavailable"));
        stack.add_named(&chart, Some("ready"));
        stack.set_visible_child_name("empty");

        Self {
            stack,
            chart,
            data,
            state: Rc::new(RefCell::new(HistogramState::Empty)),
        }
    }

    pub(super) fn clear(&self) {
        self.data.replace(None);
        self.state.replace(HistogramState::Empty);
        self.stack.set_visible_child_name("empty");
        self.chart.queue_draw();
    }

    pub(super) fn unavailable(&self) {
        self.data.replace(None);
        self.state.replace(HistogramState::Unavailable);
        self.stack.set_visible_child_name("unavailable");
        self.chart.queue_draw();
    }

    pub(super) fn set_bins(&self, red: &[f32], green: &[f32], blue: &[f32]) -> bool {
        let Some(data) = HistogramData::new(red, green, blue) else {
            self.unavailable();
            return false;
        };
        self.data.replace(Some(data));
        self.state.replace(HistogramState::Ready);
        self.stack.set_visible_child_name("ready");
        self.chart.queue_draw();
        true
    }

    pub(super) fn state(&self) -> HistogramState {
        *self.state.borrow()
    }

    pub(super) fn is_ready(&self) -> bool {
        self.state() == HistogramState::Ready
    }
}

fn install_histogram_draw(chart: &gtk4::DrawingArea, data: &Rc<RefCell<Option<HistogramData>>>) {
    let data = Rc::clone(data);
    chart.set_draw_func(move |_, context, width, height| {
        let width = f64::from(width.max(1));
        let height = f64::from(height.max(1));
        context.set_source_rgb(0.08, 0.08, 0.08);
        let _ = context.paint();
        let Some(data) = data.borrow().as_ref().cloned() else {
            return;
        };
        let maximum = data
            .bins
            .iter()
            .flat_map(|bin| bin.iter().copied())
            .fold(0.0_f32, f32::max);
        if maximum <= f32::EPSILON {
            return;
        }
        let colors = [(0.9, 0.22, 0.22), (0.22, 0.9, 0.28), (0.3, 0.5, 1.0)];
        let bin_count = u32::try_from(data.bins.len()).expect("histogram bins are bounded");
        let bin_width = width / f64::from(bin_count);
        for channel in 0..3 {
            context.set_source_rgba(colors[channel].0, colors[channel].1, colors[channel].2, 0.8);
            context.set_line_width(1.0);
            for (index, bin) in data.bins.iter().enumerate() {
                let index = u32::try_from(index).expect("histogram bin index is bounded");
                let x = (f64::from(index) + 0.5) * bin_width;
                let y = height - (f64::from(bin[channel] / maximum) * height);
                if index == 0 {
                    context.move_to(x, y);
                } else {
                    context.line_to(x, y);
                }
            }
            let _ = context.stroke();
        }
    });
}

fn histogram_status(id: &str, text: &str) -> gtk4::Label {
    let label = gtk4::Label::new(Some(text));
    label.set_widget_name(id);
    label.set_halign(gtk4::Align::Center);
    label.set_valign(gtk4::Align::Center);
    label.set_hexpand(true);
    label.set_vexpand(true);
    label.add_css_class("dim-label");
    label.set_accessible_role(gtk4::AccessibleRole::Status);
    label
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FilmstripSelection {
    pub(super) photo_id: PhotoId,
    pub(super) generation: ViewportGeneration,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct FilmstripState {
    ordered_ids: Vec<PhotoId>,
    selected: Option<PhotoId>,
    generation: ViewportGeneration,
}

impl FilmstripState {
    pub(super) fn set_items(
        &mut self,
        photo_ids: impl IntoIterator<Item = PhotoId>,
        selected: Option<PhotoId>,
        generation: ViewportGeneration,
    ) {
        self.ordered_ids = unique_ids(photo_ids);
        self.selected = selected.filter(|id| self.ordered_ids.contains(id));
        self.generation = generation;
    }

    pub(super) fn selected(&self) -> Option<PhotoId> {
        self.selected
    }

    pub(super) fn generation(&self) -> ViewportGeneration {
        self.generation
    }

    pub(super) fn set_generation(&mut self, generation: ViewportGeneration) {
        self.generation = generation;
    }

    pub(super) fn clear_selection(&mut self) {
        self.selected = None;
    }

    pub(super) fn select(&mut self, photo_id: PhotoId) -> Option<FilmstripSelection> {
        if !self.ordered_ids.contains(&photo_id) {
            return None;
        }
        self.selected = Some(photo_id);
        Some(FilmstripSelection {
            photo_id,
            generation: self.generation,
        })
    }

    pub(super) fn move_by(&mut self, offset: isize) -> Option<FilmstripSelection> {
        if self.ordered_ids.is_empty() {
            return None;
        }
        let current = self
            .selected
            .and_then(|selected| self.ordered_ids.iter().position(|id| *id == selected))
            .unwrap_or(0);
        let last = self.ordered_ids.len().saturating_sub(1);
        let index = current.saturating_add_signed(offset).min(last);
        self.select(self.ordered_ids[index])
    }
}

pub(super) type FilmstripHandler = Rc<RefCell<Option<Box<dyn Fn(FilmstripSelection)>>>>;

fn unique_ids(photo_ids: impl IntoIterator<Item = PhotoId>) -> Vec<PhotoId> {
    let mut ordered = Vec::new();
    for photo_id in photo_ids {
        if !ordered.contains(&photo_id) {
            ordered.push(photo_id);
        }
    }
    ordered
}

pub(super) fn install_filmstrip_keyboard(
    filmstrip: &gtk4::FlowBox,
    state: &Rc<RefCell<FilmstripState>>,
    handler: &FilmstripHandler,
) {
    let key = gtk4::EventControllerKey::new();
    key.set_propagation_phase(gtk4::PropagationPhase::Capture);
    let filmstrip_for_key = filmstrip.clone();
    let state_for_key = Rc::clone(state);
    let handler_for_key = Rc::clone(handler);
    key.connect_key_pressed(move |_, key, _, _| {
        let offset = match key {
            gdk::Key::Page_Up | gdk::Key::Home => Some(-1),
            gdk::Key::Page_Down | gdk::Key::End => Some(1),
            _ => None,
        };
        let Some(offset) = offset else {
            return gtk4::glib::Propagation::Proceed;
        };
        let selection = if matches!(key, gdk::Key::Home | gdk::Key::End) {
            let mut state = state_for_key.borrow_mut();
            let target = if key == gdk::Key::Home {
                state.ordered_ids.first().copied()
            } else {
                state.ordered_ids.last().copied()
            };
            target.and_then(|id| state.select(id))
        } else {
            state_for_key.borrow_mut().move_by(offset)
        };
        sync_filmstrip_buttons(&filmstrip_for_key, state_for_key.borrow().selected());
        emit_selection(&handler_for_key, selection);
        gtk4::glib::Propagation::Stop
    });
    filmstrip.add_controller(key);
}

pub(super) fn connect_filmstrip_button(
    button: &gtk4::Button,
    photo_id: PhotoId,
    filmstrip: &gtk4::FlowBox,
    state: &Rc<RefCell<FilmstripState>>,
    handler: &FilmstripHandler,
) {
    let filmstrip = filmstrip.clone();
    let state = Rc::clone(state);
    let handler = Rc::clone(handler);
    button.connect_clicked(move |_| {
        let selection = state.borrow_mut().select(photo_id);
        sync_filmstrip_buttons(&filmstrip, state.borrow().selected());
        emit_selection(&handler, selection);
    });
}

fn emit_selection(handler: &FilmstripHandler, selection: Option<FilmstripSelection>) {
    if let Some(selection) = selection
        && let Some(handler) = handler.borrow().as_ref()
    {
        handler(selection);
    }
}

pub(super) fn sync_filmstrip_buttons(filmstrip: &gtk4::FlowBox, selected: Option<PhotoId>) {
    let mut child = filmstrip.first_child();
    while let Some(widget) = child {
        if let Ok(flow_child) = widget.clone().downcast::<gtk4::FlowBoxChild>()
            && let Some(button) = flow_child
                .child()
                .and_then(|child| child.downcast::<gtk4::Button>().ok())
        {
            let is_selected = parse_filmstrip_id(&button) == selected;
            if is_selected {
                button.add_css_class("selected");
            } else {
                button.remove_css_class("selected");
            }
            button.update_state(&[State::Selected(Some(is_selected))]);
        }
        child = widget.next_sibling();
    }
}

pub(super) fn filmstrip_ids(filmstrip: &gtk4::FlowBox) -> Vec<PhotoId> {
    filmstrip_buttons(filmstrip)
        .into_iter()
        .map(|(photo_id, _)| photo_id)
        .collect()
}

pub(super) fn filmstrip_buttons(filmstrip: &gtk4::FlowBox) -> Vec<(PhotoId, gtk4::Button)> {
    let mut buttons = Vec::new();
    let mut child = filmstrip.first_child();
    while let Some(widget) = child {
        if let Ok(flow_child) = widget.clone().downcast::<gtk4::FlowBoxChild>()
            && let Some(button) = flow_child
                .child()
                .and_then(|child| child.downcast::<gtk4::Button>().ok())
            && let Some(photo_id) = parse_filmstrip_id(&button)
        {
            buttons.push((photo_id, button));
        }
        child = widget.next_sibling();
    }
    buttons
}

pub(super) fn parse_filmstrip_id(button: &gtk4::Button) -> Option<PhotoId> {
    button
        .widget_name()
        .strip_prefix("filmstrip-photo-")
        .and_then(|value| value.parse().ok())
}

fn clear_children(container: &impl IsA<gtk4::Widget>) {
    while let Some(child) = container.first_child() {
        child.unparent();
    }
}

#[cfg(test)]
mod tests {
    use super::{FilmstripState, HistogramData, HistogramState};
    use crate::viewport_presentation::ViewportGeneration;
    use rusttable_core::PhotoId;

    fn id(value: u128) -> PhotoId {
        PhotoId::new(value).expect("test photo ID")
    }

    #[test]
    fn histogram_rejects_mismatched_non_finite_or_negative_bins() {
        assert!(HistogramData::new(&[1.0], &[1.0, 2.0], &[1.0]).is_none());
        assert!(HistogramData::new(&[f32::NAN], &[1.0], &[1.0]).is_none());
        assert!(HistogramData::new(&[-1.0], &[1.0], &[1.0]).is_none());
    }

    #[test]
    fn histogram_accepts_bounded_rgb_bins() {
        let histogram =
            HistogramData::new(&[1.0, 2.0], &[2.0, 1.0], &[0.0, 1.0]).expect("valid histogram");
        assert_eq!(histogram.bins.len(), 2);
        assert_eq!(HistogramState::Ready, HistogramState::Ready);
    }

    #[test]
    fn filmstrip_selection_is_ordered_bounded_and_generation_tagged() {
        let generation = ViewportGeneration::new(9);
        let mut state = FilmstripState::default();
        state.set_items([id(2), id(1), id(2)], Some(id(2)), generation);
        assert_eq!(state.selected(), Some(id(2)));
        assert_eq!(state.move_by(1).expect("next").photo_id, id(1));
        assert_eq!(state.move_by(1).expect("clamped").photo_id, id(1));
        assert_eq!(state.generation(), generation);
        assert!(state.select(id(99)).is_none());
    }
}
