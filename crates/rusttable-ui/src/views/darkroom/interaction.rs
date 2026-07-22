//! Darkroom-local viewport adjuncts: histogram rendering and filmstrip routing.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::accessible::{Property, State};
use gtk4::gdk;
use gtk4::prelude::*;
use rusttable_core::PhotoId;

use crate::libs::histogram::{
    DARKROOM_HISTOGRAM_BINS, HistogramData, HistogramError, HistogramSample,
};
use crate::viewport_presentation::ViewportGeneration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum HistogramSurfaceState {
    Empty,
    Loading {
        generation: ViewportGeneration,
    },
    Ready {
        generation: ViewportGeneration,
        data: HistogramData,
    },
    Failed {
        generation: ViewportGeneration,
        error: HistogramError,
    },
    Stale {
        expected: ViewportGeneration,
        received: ViewportGeneration,
    },
}

#[derive(Clone)]
pub(super) struct HistogramView {
    stack: gtk4::Stack,
    chart: gtk4::DrawingArea,
    data: Rc<RefCell<Option<HistogramData>>>,
    state: Rc<RefCell<HistogramSurfaceState>>,
    failure: gtk4::Label,
    stale: gtk4::Label,
    selected_sample: Rc<RefCell<Option<HistogramSample>>>,
}

impl HistogramView {
    pub(super) fn new(stack: gtk4::Stack) -> Self {
        clear_children(&stack);
        let data = Rc::new(RefCell::new(None));
        let selected_sample = Rc::new(RefCell::new(None));
        let chart = gtk4::DrawingArea::new();
        chart.set_widget_name("darkroom-histogram-chart");
        // The right rail is a live Paned allocation. A fixed content width makes
        // GTK widen the rail back to the old natural size during reflow.
        chart.set_content_width(0);
        chart.set_content_height(128);
        chart.set_hexpand(true);
        chart.set_vexpand(true);
        chart.connect_resize(|chart, _, _| chart.queue_draw());
        chart.set_accessible_role(gtk4::AccessibleRole::Img);
        chart.update_property(&[Property::Label("Rendered image histogram")]);
        chart.set_tooltip_text(Some("Rendered image histogram"));
        install_histogram_draw(&chart, &data);

        let empty = histogram_status(
            "darkroom-histogram-empty",
            "select a photo to show the histogram",
        );
        let loading = histogram_status("darkroom-histogram-loading", "calculating histogram…");
        let failure = histogram_status(
            "darkroom-histogram-failure",
            "histogram unavailable for this preview",
        );
        let stale = histogram_status(
            "darkroom-histogram-stale",
            "preview changed; waiting for current histogram",
        );
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&loading, Some("loading"));
        stack.add_named(&failure, Some("failure"));
        stack.add_named(&stale, Some("stale"));
        stack.add_named(&chart, Some("ready"));
        stack.set_visible_child_name("empty");

        install_histogram_sample_selection(&chart, &data, &selected_sample);

        Self {
            stack,
            chart,
            data,
            state: Rc::new(RefCell::new(HistogramSurfaceState::Empty)),
            failure,
            stale,
            selected_sample,
        }
    }

    pub(super) fn clear(&self) {
        self.data.replace(None);
        self.selected_sample.replace(None);
        self.state.replace(HistogramSurfaceState::Empty);
        self.stack.set_visible_child_name("empty");
        self.chart.queue_draw();
    }

    pub(super) fn widget(&self) -> &gtk4::Stack {
        &self.stack
    }

    pub(super) fn loading(&self, generation: ViewportGeneration) {
        self.data.replace(None);
        self.selected_sample.replace(None);
        self.state
            .replace(HistogramSurfaceState::Loading { generation });
        self.stack.set_visible_child_name("loading");
        self.chart.queue_draw();
    }

    pub(super) fn failure(&self, generation: ViewportGeneration, error: HistogramError) {
        self.data.replace(None);
        self.selected_sample.replace(None);
        self.failure
            .set_tooltip_text(Some(&format_histogram_error(error)));
        self.state
            .replace(HistogramSurfaceState::Failed { generation, error });
        self.stack.set_visible_child_name("failure");
        self.chart.queue_draw();
    }

    pub(super) fn stale(&self, expected: ViewportGeneration, received: ViewportGeneration) {
        self.data.replace(None);
        self.selected_sample.replace(None);
        self.stale.set_tooltip_text(Some(&format!(
            "preview changed; ignored histogram generation {}",
            received.get()
        )));
        self.state
            .replace(HistogramSurfaceState::Stale { expected, received });
        self.stack.set_visible_child_name("stale");
        self.chart.queue_draw();
    }

    pub(super) fn set_data(&self, generation: ViewportGeneration, data: HistogramData) {
        self.selected_sample.replace(None);
        self.data.replace(Some(data));
        let data = self
            .data
            .borrow()
            .clone()
            .expect("histogram data just installed");
        self.state
            .replace(HistogramSurfaceState::Ready { generation, data });
        self.stack.set_visible_child_name("ready");
        self.chart.queue_draw();
    }

    pub(super) fn state(&self) -> HistogramSurfaceState {
        self.state.borrow().clone()
    }

    pub(super) fn is_ready(&self) -> bool {
        matches!(self.state(), HistogramSurfaceState::Ready { .. })
    }

    pub(super) fn selected_sample(&self) -> Option<HistogramSample> {
        *self.selected_sample.borrow()
    }

    pub(super) fn connect_sample<F>(&self, handler: F)
    where
        F: Fn(HistogramSample) + 'static,
    {
        let selected_sample = Rc::clone(&self.selected_sample);
        let data = Rc::clone(&self.data);
        let chart = self.chart.clone();
        let callback = Rc::new(handler);
        let click = gtk4::GestureClick::new();
        click.set_button(1);
        click.connect_pressed(move |_, _, x, _| {
            let data_guard = data.borrow();
            let Some(data) = data_guard.as_ref() else {
                return;
            };
            let width = f64::from(chart.width().max(1));
            let bin = histogram_bin_for_x(x, width);
            let Some(sample) = data.sample(bin) else {
                return;
            };
            drop(data_guard);
            selected_sample.replace(Some(sample));
            callback(sample);
        });
        self.chart.add_controller(click);
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
        let maximum = data.maximum();
        if maximum == 0 {
            return;
        }
        let channels = [
            (
                crate::libs::histogram::HistogramChannel::Red,
                (0.9, 0.22, 0.22, 0.78),
            ),
            (
                crate::libs::histogram::HistogramChannel::Green,
                (0.22, 0.9, 0.28, 0.78),
            ),
            (
                crate::libs::histogram::HistogramChannel::Blue,
                (0.3, 0.5, 1.0, 0.78),
            ),
            (
                crate::libs::histogram::HistogramChannel::Luminance,
                (0.92, 0.92, 0.92, 0.65),
            ),
        ];
        let bin_count = u32::try_from(data.bins().len()).expect("histogram bins are bounded");
        let bin_width = width / f64::from(bin_count);
        for (channel, color) in channels {
            let mut first = true;
            for (index, bin) in data.bins().iter().enumerate() {
                let index = u32::try_from(index).expect("histogram bin index is bounded");
                let x = (f64::from(index) + 0.5) * bin_width;
                let y = height - (f64::from(bin.channel(channel)) / f64::from(maximum) * height);
                if first {
                    context.move_to(x, height);
                    context.line_to(x, y);
                    first = false;
                } else {
                    context.line_to(x, y);
                }
            }
            if !first {
                context.line_to(width, height);
                context.close_path();
                context.set_source_rgba(color.0, color.1, color.2, color.3 * 0.24);
                let _ = context.fill();
            }
            // The filled trace above uses the source path once; redraw the
            // trace from the bins so the curve remains crisp at rail widths.
            context.new_path();
            for (index, bin) in data.bins().iter().enumerate() {
                let index = u32::try_from(index).expect("histogram bin index is bounded");
                let x = (f64::from(index) + 0.5) * bin_width;
                let y = height - (f64::from(bin.channel(channel)) / f64::from(maximum) * height);
                if index == 0 {
                    context.move_to(x, y);
                } else {
                    context.line_to(x, y);
                }
            }
            context.set_source_rgba(color.0, color.1, color.2, color.3);
            context.set_line_width(1.0);
            let _ = context.stroke();
        }
    });
}

fn install_histogram_sample_selection(
    chart: &gtk4::DrawingArea,
    _data: &Rc<RefCell<Option<HistogramData>>>,
    _selected_sample: &Rc<RefCell<Option<HistogramSample>>>,
) {
    chart.set_tooltip_text(Some("Click the histogram to select a luminance/RGB range"));
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn histogram_bin_for_x(x: f64, width: f64) -> usize {
    ((x / width) * DARKROOM_HISTOGRAM_BINS as f64)
        .floor()
        .clamp(0.0, (DARKROOM_HISTOGRAM_BINS - 1) as f64) as usize
}

fn format_histogram_error(error: HistogramError) -> String {
    match error {
        HistogramError::NonFinite { .. } => "histogram failed: non-finite preview pixel".to_owned(),
        HistogramError::IncorrectByteLength { .. }
        | HistogramError::IncorrectSampleLength { .. }
        | HistogramError::SizeOverflow
        | HistogramError::Empty => "histogram failed: invalid preview data".to_owned(),
        HistogramError::PreviewUnavailable => "histogram failed: preview unavailable".to_owned(),
    }
}

fn histogram_status(id: &str, text: &str) -> gtk4::Label {
    let label = gtk4::Label::new(None);
    label.set_widget_name(id);
    label.set_halign(gtk4::Align::Center);
    label.set_valign(gtk4::Align::Center);
    label.set_hexpand(true);
    label.set_vexpand(true);
    label.add_css_class("dim-label");
    label.set_accessible_role(gtk4::AccessibleRole::Status);
    label.set_tooltip_text(Some(text));
    label.update_property(&[Property::Label(text)]);
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
                button.add_css_class("dt_selected");
            } else {
                button.remove_css_class("dt_selected");
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
    use super::{FilmstripState, HistogramSurfaceState, HistogramView};
    use crate::viewport_presentation::ViewportGeneration;
    use rusttable_core::PhotoId;

    fn id(value: u128) -> PhotoId {
        PhotoId::new(value).expect("test photo ID")
    }

    #[test]
    fn histogram_surface_state_names_loading_failure_and_stale_generations() {
        assert!(matches!(
            HistogramSurfaceState::Loading {
                generation: ViewportGeneration::new(2)
            },
            HistogramSurfaceState::Loading { .. }
        ));
        assert!(matches!(
            HistogramSurfaceState::Stale {
                expected: ViewportGeneration::new(2),
                received: ViewportGeneration::new(1)
            },
            HistogramSurfaceState::Stale { .. }
        ));
        let _ = std::mem::size_of::<HistogramView>();
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

    #[cfg(target_os = "linux")]
    #[test]
    fn histogram_view_owns_one_stack_child_per_state() {
        if gtk4::init().is_err() {
            return;
        }
        let stack = gtk4::Stack::new();
        let _view = HistogramView::new(stack.clone());
        let mut child = stack.first_child();
        let mut names = Vec::new();
        while let Some(widget) = child {
            names.push(
                stack
                    .page(&widget)
                    .name()
                    .expect("histogram child has a stable name")
                    .to_owned(),
            );
            child = widget.next_sibling();
        }
        assert_eq!(names, ["empty", "loading", "failure", "stale", "ready"]);
    }
}
