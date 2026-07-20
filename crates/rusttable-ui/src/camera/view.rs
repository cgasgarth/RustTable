use std::cell::RefCell;
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::prelude::*;
use rusttable_camera::{
    CameraCapability, CameraFrame, CameraFrameOrientation, SettingKind, SettingValue,
};

use super::{CameraAction, CameraViewModel};

/// Stable keyboard/focus order for the tethered-capture surface.
pub const CAMERA_FOCUS_ORDER: [&str; 11] = [
    "camera-device",
    "camera-open",
    "camera-live-view",
    "camera-capture",
    "camera-policy",
    "camera-capabilities",
    "camera-live-frame",
    "camera-progress",
    "camera-reconcile",
    "camera-receipt",
    "camera-status",
];

type ActionHandler = Rc<dyn Fn(CameraAction)>;

/// GTK4 camera panel. It only renders service projections and emits typed actions.
#[derive(Clone)]
pub struct CameraPanel {
    root: gtk4::Box,
    devices: gtk4::DropDown,
    device_model: gtk4::StringList,
    device_ids: Rc<RefCell<Vec<String>>>,
    status: gtk4::Label,
    session: gtk4::Label,
    capabilities: gtk4::Box,
    picture: gtk4::Picture,
    frame_metadata: gtk4::Label,
    progress: gtk4::ProgressBar,
    receipt: gtk4::Label,
    policy: gtk4::DropDown,
    open: gtk4::Button,
    live_view: gtk4::Button,
    capture: gtk4::Button,
    reconcile: gtk4::Button,
    action: Rc<RefCell<Option<ActionHandler>>>,
    capture_id: Rc<RefCell<Option<String>>>,
}

impl Default for CameraPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl CameraPanel {
    #[must_use]
    pub fn new() -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        root.set_widget_name("camera-panel");
        root.set_accessible_role(gtk4::AccessibleRole::Region);
        root.update_property(&[Property::Label("Cameras and tethered capture")]);
        root.add_css_class("dt_camera_panel");

        let heading = gtk4::Label::new(Some("cameras"));
        heading.set_widget_name("camera-heading");
        heading.set_halign(gtk4::Align::Start);
        heading.add_css_class("dt_module_heading");
        root.append(&heading);

        let device_model = gtk4::StringList::new(&["No camera discovered"]);
        let devices = gtk4::DropDown::new(Some(device_model.clone()), None::<&gtk4::Expression>);
        devices.set_widget_name("camera-device");
        devices.set_hexpand(true);
        devices.set_accessible_role(gtk4::AccessibleRole::ComboBox);
        devices.set_tooltip_text(Some("Select a discovered camera"));
        root.append(&devices);

        let status = status_label("camera-status", "Camera discovery has not run");
        root.append(&status);
        let session = status_label("camera-session", "No active session");
        root.append(&session);

        let open = action_button("camera-open", "Open camera");
        root.append(&open);
        let live_view = action_button("camera-live-view", "Start live view");
        root.append(&live_view);

        let capabilities_frame = gtk4::Frame::new(Some("capabilities and confirmed settings"));
        capabilities_frame.set_widget_name("camera-capabilities");
        let capabilities = gtk4::Box::new(gtk4::Orientation::Vertical, 3);
        capabilities.set_margin_start(6);
        capabilities.set_margin_end(6);
        capabilities.set_margin_top(6);
        capabilities.set_margin_bottom(6);
        capabilities_frame.set_child(Some(&capabilities));
        root.append(&capabilities_frame);

        let picture = gtk4::Picture::new();
        picture.set_widget_name("camera-live-frame");
        picture.set_can_shrink(true);
        picture.set_content_fit(gtk4::ContentFit::Contain);
        picture.set_size_request(160, 100);
        picture.set_accessible_role(gtk4::AccessibleRole::Img);
        picture.update_property(&[Property::Label("Latest camera frame")]);
        root.append(&picture);
        let frame_metadata = status_label("camera-frame-metadata", "No live frame");
        root.append(&frame_metadata);

        let policy = gtk4::DropDown::from_strings(&[
            "Retain on camera (recommended)",
            "Delete after verified import",
        ]);
        policy.set_widget_name("camera-policy");
        policy.set_accessible_role(gtk4::AccessibleRole::ComboBox);
        root.append(&policy);
        let capture = action_button("camera-capture", "Capture and import");
        root.append(&capture);
        let progress = gtk4::ProgressBar::new();
        progress.set_widget_name("camera-progress");
        progress.set_show_text(true);
        progress.set_accessible_role(gtk4::AccessibleRole::ProgressBar);
        root.append(&progress);
        let reconcile = action_button("camera-reconcile", "Reconcile capture");
        reconcile.set_sensitive(false);
        root.append(&reconcile);
        let receipt = status_label("camera-receipt", "No receipt yet");
        root.append(&receipt);

        let action = Rc::new(RefCell::new(None));
        let capture_id = Rc::new(RefCell::new(None));
        let device_ids = Rc::new(RefCell::new(Vec::new()));
        let panel = Self {
            root,
            devices,
            device_model,
            device_ids,
            status,
            session,
            capabilities,
            picture,
            frame_metadata,
            progress,
            receipt,
            policy,
            open,
            live_view,
            capture,
            reconcile,
            action,
            capture_id,
        };
        panel.connect_static_actions();
        panel
    }

    #[must_use]
    pub const fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Connects all controls to one application-owned typed command handler.
    pub fn connect_action<F>(&self, handler: F)
    where
        F: Fn(CameraAction) + 'static,
    {
        self.action.replace(Some(Rc::new(handler)));
    }

    /// Projects a controller snapshot into native GTK widgets.
    pub fn set_state(&self, state: &CameraViewModel) {
        self.device_ids.replace(
            state
                .devices()
                .iter()
                .map(|device| device.id().to_owned())
                .collect(),
        );
        self.device_model
            .splice(0, self.device_model.n_items(), &[]);
        if state.devices().is_empty() {
            self.device_model.append("No camera discovered");
            self.devices.set_selected(0);
        } else {
            for device in state.devices() {
                self.device_model.append(&format!(
                    "{} · {}",
                    device.label(),
                    device.state().label()
                ));
            }
            if let Some(selected) = state.selected_device()
                && let Some(index) = state
                    .devices()
                    .iter()
                    .position(|device| device.id() == selected)
            {
                self.devices.set_selected(u32::try_from(index).unwrap_or(0));
            }
        }
        let availability = state
            .selected_device()
            .and_then(|selected| {
                state
                    .devices()
                    .iter()
                    .find(|device| device.id() == selected)
            })
            .map_or("Camera unavailable", |device| device.state().label());
        self.status
            .set_text(state.diagnostic().unwrap_or(availability));
        self.session.set_text(
            state
                .session()
                .map_or("No active session", |session| session.state().label()),
        );
        self.open.set_sensitive(state.selected_device().is_some());
        self.render_capabilities(state.capabilities());
        self.render_frame(state.latest_frame());
        if let Some(capture) = state.capture() {
            self.capture_id.replace(Some(capture.capture_id.clone()));
            self.progress.set_fraction(if capture.total == 0 {
                0.0
            } else {
                f64::from(capture.completed) / f64::from(capture.total)
            });
            self.progress.set_text(Some(capture.stage.label()));
            self.reconcile.set_sensitive(matches!(
                capture.stage,
                rusttable_camera::CaptureStage::Ambiguous
            ));
        } else {
            self.progress.set_fraction(0.0);
            self.progress.set_text(Some("No capture in progress"));
        }
        if let Some(receipt) = state.receipt() {
            self.receipt.set_text(&format!(
                "Receipt {} · {}",
                receipt.receipt_id, receipt.summary
            ));
        }
    }

    fn connect_static_actions(&self) {
        let action = Rc::clone(&self.action);
        let device_ids = Rc::clone(&self.device_ids);
        self.devices.connect_selected_notify(move |dropdown| {
            if let Some(device_id) = device_ids
                .borrow()
                .get(usize::try_from(dropdown.selected()).unwrap_or(usize::MAX))
            {
                emit(&action, CameraAction::SelectDevice(device_id.clone()));
            }
        });
        connect_button(&self.open, Rc::clone(&self.action), CameraAction::Open);
        let action = Rc::clone(&self.action);
        self.live_view.connect_clicked(move |button| {
            let start = button.label().as_deref() != Some("Stop live view");
            button.set_label(if start {
                "Stop live view"
            } else {
                "Start live view"
            });
            emit(
                &action,
                if start {
                    CameraAction::StartLiveView
                } else {
                    CameraAction::StopLiveView
                },
            );
        });
        let action = Rc::clone(&self.action);
        let policy = self.policy.clone();
        self.capture.connect_clicked(move |_| {
            let capture_policy = if policy.selected() == 1 {
                rusttable_camera::CapturePolicy::DeleteAfterVerifiedImport
            } else {
                rusttable_camera::CapturePolicy::RetainOnCamera
            };
            emit(&action, CameraAction::Capture(capture_policy));
        });
        let action = Rc::clone(&self.action);
        let capture_id = Rc::clone(&self.capture_id);
        self.reconcile.connect_clicked(move |_| {
            if let Some(capture_id) = capture_id.borrow().clone() {
                emit(&action, CameraAction::ReconcileCapture(capture_id));
            }
        });
    }

    fn render_capabilities(&self, values: &[CameraCapability]) {
        clear_children(&self.capabilities);
        if values.is_empty() {
            self.capabilities
                .append(&gtk4::Label::new(Some("Open a camera to inspect settings")));
            return;
        }
        for capability in values {
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
            let label = gtk4::Label::new(Some(capability.label()));
            label.set_halign(gtk4::Align::Start);
            label.set_hexpand(true);
            row.append(&label);
            let confirmed = gtk4::Label::new(Some(&setting_text(capability)));
            confirmed.set_widget_name(&format!("camera-setting-{}", capability.key()));
            confirmed.set_tooltip_text(Some("Confirmed by camera service"));
            row.append(&confirmed);
            let action = Rc::clone(&self.action);
            let key = capability.key().to_owned();
            match capability.kind() {
                SettingKind::Choice | SettingKind::Toggle => {
                    let apply = gtk4::Button::with_label("Apply");
                    apply.set_focus_on_click(false);
                    let value = capability.confirmed().clone();
                    apply.connect_clicked(move |_| {
                        emit(
                            &action,
                            CameraAction::SetSetting {
                                key: key.clone(),
                                value: value.clone(),
                            },
                        );
                    });
                    row.append(&apply);
                }
                SettingKind::Range | SettingKind::Action => {}
            }
            self.capabilities.append(&row);
        }
    }

    fn render_frame(&self, frame: Option<&CameraFrame>) {
        let Some(frame) = frame else {
            self.picture.set_paintable(None::<&gtk4::gdk::Paintable>);
            self.frame_metadata.set_text("No live frame");
            return;
        };
        let valid = frame.width() > 0
            && frame.height() > 0
            && frame.stride() >= frame.width().saturating_mul(4)
            && frame.rgba8().len() <= rusttable_camera::MAX_FRAME_BYTES
            && frame.rgba8().len()
                >= usize::try_from(frame.stride())
                    .unwrap_or(usize::MAX)
                    .saturating_mul(usize::try_from(frame.height()).unwrap_or(usize::MAX));
        if valid {
            let bytes = gtk4::glib::Bytes::from_owned(frame.rgba8().to_vec());
            let texture = gtk4::gdk::MemoryTexture::new(
                i32::try_from(frame.width()).unwrap_or(i32::MAX),
                i32::try_from(frame.height()).unwrap_or(i32::MAX),
                gtk4::gdk::MemoryFormat::R8g8b8a8,
                &bytes,
                usize::try_from(frame.stride()).unwrap_or(usize::MAX),
            );
            self.picture.set_paintable(Some(&texture));
        }
        self.frame_metadata.set_text(&format!(
            "Frame {} · {}×{} · {} · dropped {}",
            frame.sequence(),
            frame.width(),
            frame.height(),
            orientation_text(frame.orientation()),
            frame.dropped_frames()
        ));
    }
}

fn setting_text(capability: &CameraCapability) -> String {
    match capability.confirmed() {
        SettingValue::Toggle(value) => value.to_string(),
        SettingValue::Choice(value) => value.clone(),
        SettingValue::Number(value) => capability
            .unit()
            .map_or_else(|| value.to_string(), |unit| format!("{value} {unit}")),
        SettingValue::Action => "action".to_owned(),
    }
}

fn orientation_text(orientation: CameraFrameOrientation) -> &'static str {
    match orientation {
        CameraFrameOrientation::Normal => "normal",
        CameraFrameOrientation::Rotate90 => "rotate 90°",
        CameraFrameOrientation::Rotate180 => "rotate 180°",
        CameraFrameOrientation::Rotate270 => "rotate 270°",
    }
}

fn status_label(id: &str, text: &str) -> gtk4::Label {
    let label = gtk4::Label::new(Some(text));
    label.set_widget_name(id);
    label.set_halign(gtk4::Align::Start);
    label.set_wrap(true);
    label.set_accessible_role(gtk4::AccessibleRole::Status);
    label
}

fn action_button(id: &str, label: &str) -> gtk4::Button {
    let button = gtk4::Button::with_label(label);
    button.set_widget_name(id);
    button.set_hexpand(true);
    button.set_focus_on_click(false);
    button.update_property(&[Property::Label(label)]);
    button
}

fn connect_button(
    button: &gtk4::Button,
    action: Rc<RefCell<Option<ActionHandler>>>,
    value: CameraAction,
) {
    button.connect_clicked(move |_| emit(&action, value.clone()));
}

fn emit(action: &RefCell<Option<ActionHandler>>, value: CameraAction) {
    if let Some(handler) = action.borrow().as_ref() {
        handler(value);
    }
}

fn clear_children(container: &impl IsA<gtk4::Widget>) {
    let mut child = container.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        current.unparent();
    }
}

#[cfg(test)]
mod tests {
    use super::CAMERA_FOCUS_ORDER;

    #[test]
    fn camera_focus_order_is_unique_and_has_status_and_recovery() {
        let unique = CAMERA_FOCUS_ORDER
            .iter()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), CAMERA_FOCUS_ORDER.len());
        assert!(CAMERA_FOCUS_ORDER.contains(&"camera-status"));
        assert!(CAMERA_FOCUS_ORDER.contains(&"camera-reconcile"));
    }
}
