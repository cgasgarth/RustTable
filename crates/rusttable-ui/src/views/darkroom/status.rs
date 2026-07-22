//! Darkroom status and background-job boundary.

use gtk4::accessible::Property;
use gtk4::prelude::*;
use rusttable_core::Revision;

use crate::gui::{ThemeRole, apply_theme_role};
use crate::presentation::PhotoDetailViewModel;

#[derive(Clone)]
pub(super) struct DarkroomStatusSurface {
    root: gtk4::CenterBox,
    status: gtk4::Label,
    job_status: gtk4::Label,
    module_order: gtk4::Label,
    pipeline: gtk4::Label,
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
        let module_order = status_label("darkroom-module-order", "module order");
        module_order.set_tooltip_text(Some("Current processing module order"));
        module_order.set_visible(false);
        let pipeline = status_label("darkroom-pipeline-state", "");
        pipeline.set_tooltip_text(Some(
            "Current processing pipeline revision and input format",
        ));
        pipeline.set_visible(false);
        end.append(&module_order);
        end.append(&pipeline);
        end.append(&job_status);
        root.set_center_widget(Some(&status));
        root.set_end_widget(Some(&end));

        Self {
            root,
            status,
            job_status,
            module_order,
            pipeline,
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

    pub(super) fn set_detail(&self, detail: &PhotoDetailViewModel, revision: Revision) {
        let (metadata, format) = status_metadata(detail);
        self.set_image_information(&metadata);
        self.module_order.set_visible(true);
        let pipeline = format.map_or_else(
            || format!("revision {revision}"),
            |format| format!("revision {revision} · {format}"),
        );
        self.pipeline.set_text(&pipeline);
        self.pipeline.set_visible(true);
    }

    pub(super) fn set_revision(&self, revision: Revision) {
        if self.pipeline.is_visible() {
            let format = self
                .pipeline
                .text()
                .split_once('·')
                .map(|(_, format)| format.trim().to_owned());
            let pipeline = format.map_or_else(
                || format!("revision {revision}"),
                |format| format!("revision {revision} · {format}"),
            );
            self.pipeline.set_text(&pipeline);
        }
    }

    pub(super) fn clear_detail(&self) {
        self.set_image_information("no photo selected");
        self.module_order.set_visible(false);
        self.pipeline.set_visible(false);
    }

    pub(super) fn set_job_status(&self, text: &str) {
        self.job_status.set_text(text);
        self.job_status.set_tooltip_text(Some(text));
        self.job_status.set_visible(!is_idle_job_status(text));
    }
}

fn status_metadata(detail: &PhotoDetailViewModel) -> (String, Option<String>) {
    const PRIORITY: [&str; 9] = [
        "exposure",
        "shutter speed",
        "aperture",
        "focal length",
        "iso",
        "camera",
        "lens",
        "format",
        "dimensions",
    ];
    let format = detail
        .facts()
        .find(|fact| fact.label().as_str().eq_ignore_ascii_case("format"))
        .map(|fact| fact.value().as_str().to_owned());
    let mut values = Vec::new();
    for label in PRIORITY {
        let Some(fact) = detail
            .facts()
            .find(|fact| fact.label().as_str().eq_ignore_ascii_case(label))
        else {
            continue;
        };
        let value = fact.value().as_str();
        if label == "iso" && !value.to_ascii_lowercase().starts_with("iso") {
            values.push(format!("ISO {value}"));
        } else {
            values.push(value.to_owned());
        }
        if values.len() == 5 {
            break;
        }
    }
    if values.is_empty() {
        values.push(detail.title().as_str().to_owned());
    }
    (values.join(" · "), format)
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

#[cfg(test)]
mod tests {
    use rusttable_core::PhotoId;

    use super::status_metadata;
    use crate::presentation::{PhotoDetailViewModel, PhotoFactViewModel, PresentationText};

    fn text(value: &str) -> PresentationText {
        PresentationText::new(value).expect("valid test text")
    }

    #[test]
    fn bottom_status_prefers_photographic_fields_and_omits_file_size() {
        let detail = PhotoDetailViewModel::new(
            PhotoId::new(1).expect("photo"),
            text("photo.raw"),
            vec![
                PhotoFactViewModel::new(text("File size"), text("24 MB")),
                PhotoFactViewModel::new(text("ISO"), text("200")),
                PhotoFactViewModel::new(text("Aperture"), text("f/8.0")),
                PhotoFactViewModel::new(text("Exposure"), text("1/90")),
                PhotoFactViewModel::new(text("Focal length"), text("10.3 mm")),
                PhotoFactViewModel::new(text("Format"), text("RAW")),
            ],
        );

        assert_eq!(
            status_metadata(&detail),
            (
                "1/90 · f/8.0 · 10.3 mm · ISO 200 · RAW".to_owned(),
                Some("RAW".to_owned())
            )
        );
    }
}
