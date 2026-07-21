//! Darkroom status and background-job boundary.

use gtk4::accessible::Property;
use gtk4::prelude::*;

use crate::gui::{ThemeRole, apply_theme_role};

#[derive(Clone)]
pub(super) struct DarkroomStatusSurface {
    root: gtk4::Box,
    status: gtk4::Label,
    job_status: gtk4::Label,
}

impl DarkroomStatusSurface {
    pub(super) fn new() -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        root.set_widget_name("darkroom-status-bar");
        root.set_height_request(20);
        root.set_hexpand(true);
        root.set_valign(gtk4::Align::Center);
        apply_theme_role(&root, ThemeRole::Toolbar);

        let status = status_label("darkroom-status", "ready");
        status.set_hexpand(true);
        let job_status = status_label("darkroom-job-status", "background jobs: idle");
        job_status.set_halign(gtk4::Align::End);
        root.append(&status);
        root.append(&job_status);

        Self {
            root,
            status,
            job_status,
        }
    }

    pub(super) fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    pub(super) fn append<W: IsA<gtk4::Widget>>(&self, widget: &W) {
        self.root.append(widget);
    }

    pub(super) fn set_status(&self, text: &str) {
        self.status.set_text(text);
    }

    pub(super) fn set_job_status(&self, text: &str) {
        self.job_status.set_text(text);
    }
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
