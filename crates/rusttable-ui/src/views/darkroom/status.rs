//! Darkroom status and background-job boundary.

use gtk4::accessible::Property;
use gtk4::prelude::*;

use crate::gui::{ThemeRole, apply_theme_role};

#[derive(Clone)]
pub(super) struct DarkroomStatusSurface {
    root: gtk4::CenterBox,
    status: gtk4::Label,
    job_status: gtk4::Label,
    end: gtk4::Box,
}

impl DarkroomStatusSurface {
    pub(super) fn new() -> Self {
        let root = gtk4::CenterBox::new();
        root.set_widget_name("darkroom-status-bar");
        root.set_height_request(28);
        root.set_hexpand(true);
        root.set_valign(gtk4::Align::Center);
        apply_theme_role(&root, ThemeRole::Toolbar);

        let status = status_label("darkroom-status", "no photo selected");
        status.set_hexpand(true);
        status.set_halign(gtk4::Align::Center);
        let job_status = status_label("darkroom-job-status", "");
        job_status.set_halign(gtk4::Align::End);
        job_status.set_visible(false);
        let end = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        end.set_halign(gtk4::Align::End);
        end.append(&job_status);
        root.set_center_widget(Some(&status));
        root.set_end_widget(Some(&end));

        Self {
            root,
            status,
            job_status,
            end,
        }
    }

    pub(super) fn widget(&self) -> &gtk4::CenterBox {
        &self.root
    }

    pub(super) fn set_controls<W: IsA<gtk4::Widget>>(&self, widget: &W) {
        self.root.set_start_widget(Some(widget));
    }

    pub(super) fn append<W: IsA<gtk4::Widget>>(&self, widget: &W) {
        widget.set_visible(false);
        self.end.append(widget);
    }

    pub(super) fn set_status(&self, text: &str) {
        self.status.set_tooltip_text(Some(text));
        self.status.update_property(&[Property::Label(text)]);
    }

    pub(super) fn set_image_information(&self, text: &str) {
        self.status.set_text(text);
    }

    pub(super) fn set_job_status(&self, text: &str) {
        self.job_status.set_text(text);
        self.job_status.set_tooltip_text(Some(text));
        self.job_status.set_visible(!is_idle_job_status(text));
    }
}

fn is_idle_job_status(text: &str) -> bool {
    text.is_empty()
        || text == "background jobs: idle"
        || text.starts_with("Ready to export")
        || text.starts_with("Select a photo to export")
}

fn status_label(id: &str, text: &str) -> gtk4::Label {
    let label = gtk4::Label::new(Some(text));
    label.set_widget_name(id);
    label.set_halign(gtk4::Align::Start);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    label.set_single_line_mode(true);
    label.add_css_class("dim-label");
    label.set_accessible_role(gtk4::AccessibleRole::Status);
    label.update_property(&[Property::Label(text)]);
    label
}
