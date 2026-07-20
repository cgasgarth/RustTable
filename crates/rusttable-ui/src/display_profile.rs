//! GTK4 boundary for monitor inventory and visible display-profile state.

use gtk4::gdk::prelude::MonitorExt;
use gtk4::prelude::*;
use rusttable_display_profile::{
    DisplayProfileSnapshot, DisplayProvider, HdrDescriptor, ProfileProbe, ProfileSelection,
    ProviderMonitor, SystemProfileAdapter, descriptor_from_gdk_evidence,
};

/// Enumerates monitors through GDK without retaining native monitor handles.
#[derive(Debug, Default, Clone, Copy)]
pub struct GtkMonitorInventory;

impl GtkMonitorInventory {
    #[must_use]
    pub fn discover(self) -> Vec<ProviderMonitor> {
        let Some(display) = gtk4::gdk::Display::default() else {
            return Vec::new();
        };
        let monitors = display.monitors();
        let provider = SystemProfileAdapter::current().provider();
        let mut discovered = Vec::new();
        for index in 0..monitors.n_items() {
            let Some(monitor) = monitors.item(index).and_downcast::<gtk4::gdk::Monitor>() else {
                continue;
            };
            let geometry = monitor.geometry();
            let Ok(descriptor) = descriptor_from_gdk_evidence(
                provider_name(provider),
                monitor.connector().as_deref(),
                monitor.manufacturer().as_deref(),
                monitor.model().as_deref(),
                None,
                format!("Display {}", index + 1),
                (
                    geometry.x(),
                    geometry.y(),
                    geometry.width().unsigned_abs(),
                    geometry.height().unsigned_abs(),
                    monitor.scale_factor().unsigned_abs(),
                ),
                HdrDescriptor {
                    supported: false,
                    active: false,
                },
            ) else {
                continue;
            };
            discovered.push(ProviderMonitor::new(
                descriptor,
                provider,
                ProfileProbe::Unavailable,
            ));
        }
        discovered
    }
}

/// A non-modal Darktable-style status surface. It never opens a profile prompt during startup.
#[derive(Clone)]
pub struct DisplayProfileBanner {
    label: gtk4::Label,
}

impl DisplayProfileBanner {
    #[must_use]
    pub fn new() -> Self {
        let label = gtk4::Label::new(Some("Display profile: waiting for monitor evidence"));
        label.set_widget_name("display-profile-status");
        label.add_css_class("dim-label");
        Self { label }
    }

    #[must_use]
    pub fn widget(&self) -> &gtk4::Label {
        &self.label
    }

    /// Projects only privacy-safe snapshot state into the visible shell.
    pub fn set_snapshot(&self, snapshot: Option<&DisplayProfileSnapshot>) {
        let text = snapshot.map_or_else(
            || "Display profile: no active monitor".to_owned(),
            |snapshot| {
                let state = match snapshot.selection() {
                    ProfileSelection::Override => "explicit override",
                    ProfileSelection::OperatingSystem => "operating-system profile",
                    ProfileSelection::UserFallback => "explicit fallback",
                    ProfileSelection::Unprofiled => "Unprofiled",
                };
                format!(
                    "Display profile: {state} · generation {}",
                    snapshot.generation()
                )
            },
        );
        self.label.set_label(&text);
        self.label.remove_css_class("warning");
        if snapshot.is_none_or(|snapshot| {
            !matches!(
                snapshot.status(),
                rusttable_display_profile::SelectionStatus::Active
            )
        }) {
            self.label.add_css_class("warning");
        }
    }
}

impl Default for DisplayProfileBanner {
    fn default() -> Self {
        Self::new()
    }
}

fn provider_name(provider: DisplayProvider) -> &'static str {
    match provider {
        DisplayProvider::Colord => "colord",
        DisplayProvider::X11 => "x11",
        DisplayProvider::Wayland => "wayland",
        DisplayProvider::ColorSync => "colorsync",
        DisplayProvider::WindowsWcs => "windows-wcs",
        DisplayProvider::Synthetic => "synthetic",
    }
}
