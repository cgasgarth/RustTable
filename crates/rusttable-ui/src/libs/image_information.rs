//! Darkroom image-information model and GTK4 left-rail projection.

use gtk4::prelude::*;

use crate::libs::panel::{
    DarkroomPanelProjection, DarkroomPanelState, append_fact, append_status, panel_expander,
};
use crate::presentation::{PhotoFactViewModel, PresentationText, PresentationTextError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DarkroomImageInformationViewModel {
    title: PresentationText,
    facts: Vec<PhotoFactViewModel>,
    unsupported: Vec<PresentationText>,
}

impl DarkroomImageInformationViewModel {
    /// Keeps unavailable metadata explicit instead of fabricating values.
    ///
    /// # Errors
    ///
    /// Returns a presentation-text validation error for an invalid title or field label.
    pub fn new(
        title: impl Into<String>,
        facts: Vec<PhotoFactViewModel>,
        unsupported: Vec<String>,
    ) -> Result<Self, PresentationTextError> {
        Ok(Self {
            title: PresentationText::new(title)?,
            facts,
            unsupported: unsupported
                .into_iter()
                .map(PresentationText::new)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }

    #[must_use]
    pub fn title(&self) -> &PresentationText {
        &self.title
    }

    #[must_use]
    pub fn facts(&self) -> impl ExactSizeIterator<Item = &PhotoFactViewModel> {
        self.facts.iter()
    }

    #[must_use]
    pub fn unsupported(&self) -> impl ExactSizeIterator<Item = &PresentationText> {
        self.unsupported.iter()
    }
}

#[must_use]
pub fn build_image_information_panel(
    projection: &DarkroomPanelProjection<DarkroomImageInformationViewModel>,
) -> gtk4::Expander {
    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 3);
    match projection.state() {
        DarkroomPanelState::Empty => append_status(&body, "image information unavailable"),
        DarkroomPanelState::Loading => append_status(&body, "loading image information…"),
        DarkroomPanelState::Error(error) => {
            append_status(&body, &format!("Error · {}", error.as_str()));
        }
        DarkroomPanelState::Ready(info) => {
            let title = gtk4::Label::new(Some(info.title().as_str()));
            title.set_halign(gtk4::Align::Start);
            title.add_css_class("title-4");
            body.append(&title);
            for fact in info.facts() {
                append_fact(&body, fact.label().as_str(), fact.value().as_str());
            }
            for unsupported in info.unsupported() {
                append_fact(&body, unsupported.as_str(), "Unavailable");
            }
        }
    }
    panel_expander(
        "darkroom-image-information",
        "image information",
        projection.expanded(),
        &body,
    )
}
