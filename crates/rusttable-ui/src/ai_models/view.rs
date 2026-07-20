//! GTK4 view for AI model settings. It emits typed actions and never inspects package bytes.

#![allow(clippy::missing_panics_doc)]

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::prelude::*;

use super::model::{
    AiModelsAction, AiModelsSnapshot, AiModelsViewModel, AiProvider, AiProviderPolicy, AiTask,
    InstallSummary, InstalledModel, ModelHash,
};

type ActionHandler = Rc<dyn Fn(AiModelsAction)>;

#[derive(Clone)]
pub struct AiModelsPanel {
    root: gtk4::Box,
    window: gtk4::Window,
    picker: gtk4::Button,
    confirm_install: gtk4::Button,
    cancel_install: gtk4::Button,
    provider_policy: gtk4::DropDown,
    task: gtk4::DropDown,
    model: gtk4::DropDown,
    model_rows: gtk4::ListBox,
    qualify: gtk4::Button,
    enabled: gtk4::CheckButton,
    remove: gtk4::Button,
    cancel: gtk4::Button,
    progress: gtk4::ProgressBar,
    status: gtk4::Label,
    review: gtk4::Label,
    model_hashes: Rc<RefCell<Vec<ModelHash>>>,
    task_models: Rc<RefCell<Vec<ModelHash>>>,
    selected_model: Rc<RefCell<Option<ModelHash>>>,
    selected_task: Rc<RefCell<AiTask>>,
    qualification_job: Rc<RefCell<Option<u64>>>,
    staged: Rc<RefCell<Option<InstallSummary>>>,
}

impl AiModelsPanel {
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn new() -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        root.set_widget_name("ai-models");
        root.set_margin_top(16);
        root.set_margin_bottom(16);
        root.set_margin_start(16);
        root.set_margin_end(16);
        root.set_accessible_role(gtk4::AccessibleRole::Region);
        root.update_property(&[Property::Label("AI Models settings")]);

        let heading = gtk4::Label::new(Some("AI Models"));
        heading.set_halign(gtk4::Align::Start);
        heading.add_css_class("title-2");
        root.append(&heading);

        let explanation = gtk4::Label::new(Some(
            "Install local .rtmodel packages, inspect qualification, and choose exact model identities. No downloads are performed.",
        ));
        explanation.set_halign(gtk4::Align::Start);
        explanation.set_wrap(true);
        explanation.add_css_class("dim-label");
        root.append(&explanation);

        let picker = gtk4::Button::with_label("Choose local .rtmodel…");
        picker.set_widget_name("ai-models-package-picker");
        picker.update_property(&[Property::Label("Choose a local RT model package")]);
        root.append(&labeled_row("Package", &picker));

        let review = gtk4::Label::new(Some("No package staged"));
        review.set_widget_name("ai-models-install-review");
        review.set_halign(gtk4::Align::Start);
        review.set_wrap(true);
        review.set_accessible_role(gtk4::AccessibleRole::Status);
        root.append(&indent(&review));
        let install_actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let confirm_install = gtk4::Button::with_label("Install validated package");
        confirm_install.set_widget_name("ai-models-confirm-install");
        confirm_install.add_css_class("suggested-action");
        let cancel_install = gtk4::Button::with_label("Cancel");
        cancel_install.set_widget_name("ai-models-cancel-install");
        install_actions.append(&confirm_install);
        install_actions.append(&cancel_install);
        root.append(&indent(&install_actions));

        let provider_policy =
            gtk4::DropDown::from_strings(&AiProviderPolicy::all().map(AiProviderPolicy::label));
        provider_policy.set_widget_name("ai-models-provider-policy");
        provider_policy.set_hexpand(true);
        root.append(&labeled_row("Default provider", &provider_policy));

        let task = gtk4::DropDown::from_strings(&AiTask::all().map(AiTask::label));
        task.set_widget_name("ai-models-task");
        task.set_hexpand(true);
        root.append(&labeled_row("Task", &task));

        let model = gtk4::DropDown::from_strings(&["No installed models"]);
        model.set_widget_name("ai-models-model");
        model.set_hexpand(true);
        root.append(&labeled_row("Task default", &model));

        let model_rows = gtk4::ListBox::new();
        model_rows.set_widget_name("ai-models-installed-list");
        model_rows.set_selection_mode(gtk4::SelectionMode::None);
        model_rows.set_accessible_role(gtk4::AccessibleRole::List);
        let models_scroll = gtk4::ScrolledWindow::builder()
            .child(&model_rows)
            .min_content_height(180)
            .vexpand(true)
            .build();
        root.append(&labeled_row("Installed", &models_scroll));

        let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let qualify = gtk4::Button::with_label("Qualify provider");
        qualify.set_widget_name("ai-models-qualify");
        let enabled = gtk4::CheckButton::with_label("Enabled for task selection");
        enabled.set_widget_name("ai-models-enabled");
        let remove = gtk4::Button::with_label("Remove safe version");
        remove.set_widget_name("ai-models-remove");
        actions.append(&qualify);
        actions.append(&enabled);
        actions.append(&remove);
        root.append(&actions);

        let progress = gtk4::ProgressBar::new();
        progress.set_widget_name("ai-models-progress");
        progress.set_show_text(true);
        progress.set_accessible_role(gtk4::AccessibleRole::ProgressBar);
        root.append(&progress);
        let cancel = gtk4::Button::with_label("Cancel qualification");
        cancel.set_widget_name("ai-models-cancel");
        root.append(&cancel);

        let status = gtk4::Label::new(Some("AI model registry unavailable"));
        status.set_widget_name("ai-models-status");
        status.set_halign(gtk4::Align::Start);
        status.set_wrap(true);
        status.set_accessible_role(gtk4::AccessibleRole::Status);
        status.update_property(&[Property::Label("AI model settings status")]);
        root.append(&status);

        let window = gtk4::Window::builder()
            .title("AI Models")
            .default_width(760)
            .default_height(640)
            .hide_on_close(true)
            .child(&root)
            .build();
        Self {
            root,
            window,
            picker,
            confirm_install,
            cancel_install,
            provider_policy,
            task,
            model,
            model_rows,
            qualify,
            enabled,
            remove,
            cancel,
            progress,
            status,
            review,
            model_hashes: Rc::new(RefCell::new(Vec::new())),
            task_models: Rc::new(RefCell::new(Vec::new())),
            selected_model: Rc::new(RefCell::new(None)),
            selected_task: Rc::new(RefCell::new(AiTask::RawBayerDenoise)),
            qualification_job: Rc::new(RefCell::new(None)),
            staged: Rc::new(RefCell::new(None)),
        }
    }

    #[must_use]
    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    pub fn present(&self, parent: &impl IsA<gtk4::Window>) {
        self.window.set_transient_for(Some(parent));
        self.window.present();
        self.picker.grab_focus();
    }

    pub fn set_state(&self, state: &AiModelsViewModel) {
        let snapshot = state.snapshot();
        self.status.set_text(state.status());
        self.provider_policy
            .set_selected(policy_index(snapshot.provider_policy()));
        self.render_models(snapshot.models());
        if let Some(staged) = state.staging() {
            self.staged.replace(Some(staged.clone()));
            self.review.set_text(&format!(
                "Validated locally by the service: {} · {} {} · {} · {} bytes · {}",
                staged.file_name(),
                staged.model_id(),
                staged.version(),
                staged.hash(),
                staged.package_bytes(),
                staged.validation()
            ));
            self.review.set_visible(true);
            self.confirm_install.set_sensitive(true);
            self.cancel_install.set_sensitive(true);
        } else {
            self.staged.replace(None);
            self.review.set_text("No package staged");
            self.confirm_install.set_sensitive(false);
            self.cancel_install.set_sensitive(false);
        }
        if let Some(job) = state.qualification() {
            self.qualification_job.replace(Some(job.id()));
            self.progress.set_fraction(job.fraction());
            self.progress.set_text(Some(&format!(
                "{}: {}",
                job.provider().label(),
                job.detail()
            )));
            self.cancel.set_sensitive(true);
        } else {
            self.qualification_job.replace(None);
            self.progress.set_fraction(0.0);
            self.progress.set_text(Some("No qualification job"));
            self.cancel.set_sensitive(false);
        }
        self.update_selected_model(snapshot);
    }

    fn render_models(&self, models: &[InstalledModel]) {
        clear_children(&self.model_rows);
        self.model_hashes
            .replace(models.iter().map(|model| model.hash().clone()).collect());
        for model in models {
            let row = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
            row.set_margin_top(4);
            row.set_margin_bottom(4);
            let title = gtk4::Label::new(Some(&format!(
                "{} {} · {}",
                model.model_id(),
                model.version(),
                model.task().label()
            )));
            title.set_halign(gtk4::Align::Start);
            let details = gtk4::Label::new(Some(&format!(
                "hash {} · {} bytes · {} · {} · {}",
                model.hash(),
                model.package_bytes(),
                model.tensor_summary(),
                model.tile_summary(),
                model.color_summary()
            )));
            details.set_halign(gtk4::Align::Start);
            details.set_wrap(true);
            details.add_css_class("dim-label");
            let provider = model
                .providers()
                .iter()
                .map(|item| format!("{}: {}", item.provider().label(), item.state().label()))
                .collect::<Vec<_>>()
                .join(" · ");
            let qualification = gtk4::Label::new(Some(&format!(
                "Providers: {provider} · runtime: {}",
                model.runtime_compatibility()
            )));
            qualification.set_halign(gtk4::Align::Start);
            qualification.set_wrap(true);
            qualification.add_css_class("dim-label");
            row.append(&title);
            row.append(&details);
            row.append(&qualification);
            self.model_rows.append(&row);
        }
    }

    fn update_selected_model(&self, snapshot: &AiModelsSnapshot) {
        let task = AiTask::all()
            .get(self.task.selected() as usize)
            .copied()
            .unwrap_or(AiTask::RawBayerDenoise);
        self.selected_task.replace(task);
        let models = snapshot
            .models()
            .iter()
            .filter(|model| model.task() == task)
            .collect::<Vec<_>>();
        let labels = if models.is_empty() {
            vec!["No enabled model for this task".to_owned()]
        } else {
            models
                .iter()
                .map(|model| format!("{} {}", model.model_id(), model.version()))
                .collect()
        };
        self.task_models
            .replace(models.iter().map(|model| model.hash().clone()).collect());
        self.model.set_model(Some(&gtk4::StringList::new(
            &labels.iter().map(String::as_str).collect::<Vec<_>>(),
        )));
        self.model.set_selected(0);
        self.selected_model
            .replace(models.first().map(|model| model.hash().clone()));
        self.enabled
            .set_active(models.first().is_some_and(|model| model.enabled()));
    }

    pub fn connect_action<F>(&self, handler: F)
    where
        F: Fn(AiModelsAction) + 'static,
    {
        let handler: ActionHandler = Rc::new(handler);
        let picker = self.picker.clone();
        let callback = Rc::clone(&handler);
        picker.connect_clicked(move |_| {
            let dialog = gtk4::FileDialog::builder()
                .title("Choose .rtmodel package")
                .accept_label("Stage")
                .modal(true)
                .build();
            let callback = Rc::clone(&callback);
            dialog.open(
                None::<&gtk4::Window>,
                None::<&gtk4::gio::Cancellable>,
                move |result| {
                    let Ok(file) = result else { return };
                    let Some(path) = file.path() else { return };
                    callback(AiModelsAction::SelectLocalPackage(path));
                },
            );
        });
        connect_button(
            &self.confirm_install,
            Rc::clone(&handler),
            AiModelsAction::ConfirmInstall,
        );
        connect_button(
            &self.cancel_install,
            Rc::clone(&handler),
            AiModelsAction::CancelInstall,
        );
        let callback = Rc::clone(&handler);
        self.provider_policy
            .connect_selected_notify(move |dropdown| {
                if let Some(policy) = AiProviderPolicy::all().get(dropdown.selected() as usize) {
                    callback(AiModelsAction::SetProviderPolicy(*policy));
                }
            });
        let callback = Rc::clone(&handler);
        let selected_task = Rc::clone(&self.selected_task);
        self.task.connect_selected_notify(move |dropdown| {
            if let Some(task) = AiTask::all().get(dropdown.selected() as usize) {
                selected_task.replace(*task);
                callback(AiModelsAction::SetTaskDefault {
                    task: *task,
                    model: None,
                });
            }
        });
        let callback = Rc::clone(&handler);
        let task_models = Rc::clone(&self.task_models);
        let selected_task = Rc::clone(&self.selected_task);
        self.model.connect_selected_notify(move |dropdown| {
            let selected = task_models
                .borrow()
                .get(dropdown.selected() as usize)
                .cloned();
            callback(AiModelsAction::SetTaskDefault {
                task: *selected_task.borrow(),
                model: selected,
            });
        });
        let callback = Rc::clone(&handler);
        let selected_model = Rc::clone(&self.selected_model);
        self.qualify.connect_clicked(move |_| {
            if let Some(model) = selected_model.borrow().clone() {
                callback(AiModelsAction::Qualify {
                    model,
                    provider: AiProvider::Cpu,
                });
            }
        });
        let callback = Rc::clone(&handler);
        let selected_model = Rc::clone(&self.selected_model);
        self.enabled.connect_toggled(move |check| {
            if let Some(model) = selected_model.borrow().clone() {
                callback(AiModelsAction::SetEnabled {
                    model,
                    enabled: check.is_active(),
                });
            }
        });
        let callback = Rc::clone(&handler);
        let selected_model = Rc::clone(&self.selected_model);
        self.remove.connect_clicked(move |_| {
            if let Some(model) = selected_model.borrow().clone() {
                callback(AiModelsAction::Remove(model));
            }
        });
        let callback = Rc::clone(&handler);
        let qualification_job = Rc::clone(&self.qualification_job);
        self.cancel.connect_clicked(move |_| {
            if let Some(job) = *qualification_job.borrow() {
                callback(AiModelsAction::CancelQualification(job));
            }
        });
    }
}

impl Default for AiModelsPanel {
    fn default() -> Self {
        Self::new()
    }
}

fn policy_index(policy: AiProviderPolicy) -> u32 {
    AiProviderPolicy::all()
        .iter()
        .position(|item| *item == policy)
        .and_then(|index| u32::try_from(index).ok())
        .unwrap_or_default()
}

fn labeled_row(label: &str, widget: &impl IsA<gtk4::Widget>) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let label = gtk4::Label::new(Some(label));
    label.set_width_chars(18);
    label.set_halign(gtk4::Align::Start);
    row.append(&label);
    row.append(widget);
    row
}

fn indent(widget: &impl IsA<gtk4::Widget>) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    row.set_margin_start(18);
    row.append(widget);
    row
}

fn connect_button(button: &gtk4::Button, handler: ActionHandler, action: AiModelsAction) {
    button.connect_clicked(move |_| handler(action.clone()));
}

fn clear_children(container: &impl IsA<gtk4::Widget>) {
    while let Some(child) = container.first_child() {
        child.unparent();
    }
}
