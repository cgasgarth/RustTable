use rusttable_core::PhotoId;

use crate::input::{
    BasicEditIntent, ExportIntent, ExportSize, FocusTarget, InputEffect, InputIntent, InputState,
    UiMessage,
};
use crate::{LibraryState, NavigationState, WorkspaceRoute};

#[derive(Debug, PartialEq, Eq)]
pub struct UiState {
    sidebar_visible: bool,
    navigation: NavigationState,
    library_state: LibraryState,
    input: InputState,
    import_panel: crate::ImportPanelViewModel,
    basic_edit: Option<crate::presentation::BasicEditInspectorViewModel>,
    export_status: Option<(PhotoId, crate::PresentationText)>,
    export_size: ExportSize,
    export_cancellation_requested: Option<PhotoId>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            sidebar_visible: true,
            navigation: NavigationState::default(),
            library_state: LibraryState::default(),
            input: InputState::default(),
            import_panel: crate::ImportPanelViewModel::default(),
            basic_edit: None,
            export_status: None,
            export_size: ExportSize::Original,
            export_cancellation_requested: None,
        }
    }
}

impl UiState {
    #[must_use]
    pub fn with_library_state(library_state: LibraryState) -> Self {
        Self {
            library_state,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn with_photo_workspace(workspace: crate::PhotoWorkspaceViewModel) -> Self {
        Self::with_library_state(LibraryState::Ready(workspace))
    }

    #[must_use]
    pub fn sidebar_visible(&self) -> bool {
        self.sidebar_visible
    }

    #[must_use]
    pub fn route(&self) -> WorkspaceRoute {
        self.navigation.route()
    }

    #[must_use]
    pub fn library_state(&self) -> &LibraryState {
        &self.library_state
    }

    #[must_use]
    pub const fn basic_edit(&self) -> Option<&crate::presentation::BasicEditInspectorViewModel> {
        self.basic_edit.as_ref()
    }

    pub fn set_basic_edit_values(
        &mut self,
        photo_id: PhotoId,
        values: crate::presentation::BasicEditValues,
    ) {
        if matches!(self.route(), WorkspaceRoute::PhotoDetail(current) if current == photo_id) {
            if let Some(inspector) = self
                .basic_edit
                .as_mut()
                .filter(|inspector| inspector.photo_id() == photo_id)
            {
                inspector.apply_saved_values(values);
            } else {
                self.basic_edit = Some(
                    crate::presentation::BasicEditInspectorViewModel::with_values(photo_id, values),
                );
            }
            self.reconcile_input();
        }
    }

    pub fn set_export_status(&mut self, photo_id: PhotoId, status: String) {
        if self.selected_photo_detail(photo_id) {
            self.export_status = crate::PresentationText::new(status)
                .ok()
                .map(|status| (photo_id, status));
            if !self.export_is_active(photo_id) {
                self.export_cancellation_requested = None;
            }
            self.reconcile_input();
        }
    }

    /// Returns the bounded output-size setting selected for the next PNG save.
    #[must_use]
    pub const fn export_size(&self) -> ExportSize {
        self.export_size
    }

    /// Reports whether the save surface is waiting for its active task to finish.
    #[must_use]
    pub fn export_in_progress(&self, photo_id: PhotoId) -> bool {
        self.selected_photo_detail(photo_id)
            && self.export_status(photo_id).is_some()
            && self.export_is_active(photo_id)
    }

    /// Reports whether the UI has requested cooperative cancellation.
    #[must_use]
    pub fn export_cancellation_requested(&self, photo_id: PhotoId) -> bool {
        self.export_cancellation_requested == Some(photo_id)
    }

    #[must_use]
    pub fn export_status(&self, photo_id: PhotoId) -> Option<&crate::PresentationText> {
        self.export_status
            .as_ref()
            .and_then(|(selected, status)| (*selected == photo_id).then_some(status))
    }

    pub fn set_basic_edit_draft_values(
        &mut self,
        photo_id: PhotoId,
        values: crate::presentation::BasicEditValues,
    ) {
        if let Some(inspector) = self
            .basic_edit
            .as_mut()
            .filter(|inspector| inspector.photo_id() == photo_id)
        {
            inspector.set_draft_values(values);
            self.reconcile_input();
        }
    }

    pub fn begin_basic_edit_save(&mut self, photo_id: PhotoId) {
        if let Some(inspector) = self
            .basic_edit
            .as_mut()
            .filter(|inspector| inspector.photo_id() == photo_id)
        {
            inspector.begin_save();
        }
    }

    pub fn fail_basic_edit_save(&mut self, photo_id: PhotoId) {
        if let Some(inspector) = self
            .basic_edit
            .as_mut()
            .filter(|inspector| inspector.photo_id() == photo_id)
        {
            inspector.mark_save_failed();
        }
    }

    pub fn conflict_basic_edit_save(&mut self, photo_id: PhotoId) {
        if let Some(inspector) = self
            .basic_edit
            .as_mut()
            .filter(|inspector| inspector.photo_id() == photo_id)
        {
            inspector.mark_save_conflicted();
        }
    }

    #[must_use]
    pub fn is_focused(&self, target: FocusTarget) -> bool {
        self.input.is_focused(target)
    }

    pub fn set_library_state(&mut self, library_state: LibraryState) {
        self.library_state = library_state;
        self.reconcile_input();
    }

    pub fn begin_library_load(&mut self) {
        self.set_library_state(LibraryState::Loading);
    }

    #[must_use]
    pub const fn import_panel(&self) -> &crate::ImportPanelViewModel {
        &self.import_panel
    }

    pub fn set_import_panel(&mut self, panel: crate::ImportPanelViewModel) {
        self.import_panel = panel;
        self.reconcile_input();
    }

    pub fn update_import_row(&mut self, item_id: u64, state: crate::ImportRowState) {
        self.import_panel.update_state(item_id, state);
        self.reconcile_input();
    }

    #[must_use]
    pub fn handle(&mut self, message: UiMessage) -> UiEffect {
        match message {
            UiMessage::ToggleSidebar => {
                self.sidebar_visible = !self.sidebar_visible;
            }
            UiMessage::ImportFiles => return UiEffect::ImportFiles,
            UiMessage::CancelImport => return UiEffect::CancelImport,
            UiMessage::RetryImport(item_id) => return UiEffect::RetryImport(item_id),
            UiMessage::RemoveImportResult(item_id) => {
                self.import_panel.remove(item_id);
            }
            UiMessage::CloseImportPanel => {
                if !self.import_panel.active() {
                    self.import_panel = crate::ImportPanelViewModel::default();
                }
            }
            UiMessage::SaveRenderedCopy(photo_id) => {
                if self.selected_photo_detail(photo_id) {
                    return UiEffect::SaveRenderedCopy(photo_id);
                }
            }
            UiMessage::Navigate(intent) => {
                let _ = self.navigation.apply(intent);
                self.input.note_navigation(intent, &self.library_state);
            }
            UiMessage::RetryLibrary => return UiEffect::RetryLibrary,
            UiMessage::Input(intent) => {
                if let InputIntent::BasicEdit(edit_intent) = intent {
                    self.apply_basic_edit(edit_intent);
                    self.reconcile_input();
                    return UiEffect::None;
                }
                if let InputIntent::Export(export_intent) = intent {
                    return self.apply_export_intent(export_intent);
                }
                let export_in_progress = self
                    .selected_detail_id()
                    .is_some_and(|photo_id| self.export_in_progress(photo_id));
                let effect = self.input.apply_with_import_panel_and_export(
                    intent,
                    self.sidebar_visible,
                    self.route(),
                    &self.library_state,
                    &self.import_panel,
                    export_in_progress,
                );
                match effect {
                    InputEffect::None => {}
                    InputEffect::ToggleSidebar => self.sidebar_visible = !self.sidebar_visible,
                    InputEffect::Navigate(navigation) => {
                        let _ = self.navigation.apply(navigation);
                        self.input.note_navigation(navigation, &self.library_state);
                    }
                    InputEffect::RetryLibrary => return UiEffect::RetryLibrary,
                    InputEffect::ImportFiles => return UiEffect::ImportFiles,
                    InputEffect::CancelImport => return UiEffect::CancelImport,
                    InputEffect::RetryImport(item_id) => return UiEffect::RetryImport(item_id),
                    InputEffect::RemoveImportResult(item_id) => {
                        self.import_panel.remove(item_id);
                    }
                    InputEffect::CloseImportPanel => {
                        if !self.import_panel.active() {
                            self.import_panel = crate::ImportPanelViewModel::default();
                        }
                    }
                    InputEffect::SaveRenderedCopy(photo_id) => {
                        return UiEffect::SaveRenderedCopy(photo_id);
                    }
                    InputEffect::ExportSize(photo_id, size) => {
                        let _ = self.select_export_size(photo_id, size);
                    }
                    InputEffect::StartRenderedCopy(photo_id) => {
                        return self.start_export(photo_id);
                    }
                    InputEffect::CancelRenderedCopy(photo_id) => {
                        self.request_export_cancellation(photo_id);
                    }
                }
            }
        }
        self.reconcile_input();
        UiEffect::None
    }

    fn reconcile_input(&mut self) {
        let export_in_progress = self
            .selected_detail_id()
            .is_some_and(|photo_id| self.export_in_progress(photo_id));
        self.input.reconcile_with_import_panel_and_export(
            self.sidebar_visible,
            self.route(),
            &self.library_state,
            &self.import_panel,
            export_in_progress,
        );
        self.reconcile_basic_edit();
    }

    fn apply_basic_edit(&mut self, intent: BasicEditIntent) {
        let Some(inspector) = self.basic_edit.as_mut() else {
            return;
        };
        match intent {
            BasicEditIntent::Increment(field) => inspector.increment(field),
            BasicEditIntent::Decrement(field) => inspector.decrement(field),
            BasicEditIntent::Undo
            | BasicEditIntent::Redo
            | BasicEditIntent::Reload
            | BasicEditIntent::Reapply => {}
            BasicEditIntent::Reset => inspector.reset(),
            BasicEditIntent::Commit => inspector.request_save(),
        }
    }

    fn apply_export_intent(&mut self, intent: ExportIntent) -> UiEffect {
        let Some(photo_id) = self.selected_detail_id() else {
            return UiEffect::None;
        };
        match intent {
            ExportIntent::SelectSize(size) => {
                let _ = self.select_export_size(photo_id, size);
                UiEffect::None
            }
            ExportIntent::Start => self.start_export(photo_id),
            ExportIntent::Cancel => {
                self.request_export_cancellation(photo_id);
                UiEffect::None
            }
        }
    }

    fn select_export_size(&mut self, photo_id: PhotoId, size: ExportSize) -> bool {
        if !self.selected_photo_detail(photo_id) || self.export_in_progress(photo_id) {
            return false;
        }
        self.export_size = size;
        true
    }

    fn start_export(&mut self, photo_id: PhotoId) -> UiEffect {
        if !self.selected_photo_detail(photo_id) || self.export_in_progress(photo_id) {
            return UiEffect::None;
        }
        self.export_status = crate::PresentationText::new("Choosing PNG destination…".to_owned())
            .ok()
            .map(|status| (photo_id, status));
        self.reconcile_input();
        UiEffect::SaveRenderedCopy(photo_id)
    }

    fn request_export_cancellation(&mut self, photo_id: PhotoId) {
        if self.export_in_progress(photo_id) {
            self.export_cancellation_requested = Some(photo_id);
            self.export_status = crate::PresentationText::new("Cancelling PNG export…".to_owned())
                .ok()
                .map(|status| (photo_id, status));
            self.reconcile_input();
        }
    }

    fn reconcile_basic_edit(&mut self) {
        let selected_photo = match self.route() {
            WorkspaceRoute::PhotoDetail(photo_id) => self
                .library_state
                .ready_workspace()
                .and_then(|workspace| workspace.detail(photo_id).map(|_| photo_id)),
            WorkspaceRoute::Library => None,
        };
        match selected_photo {
            Some(photo_id)
                if self
                    .basic_edit
                    .is_none_or(|inspector| inspector.photo_id() != photo_id) =>
            {
                self.basic_edit = Some(crate::presentation::BasicEditInspectorViewModel::new(
                    photo_id,
                ));
            }
            Some(_) => {}
            None => self.basic_edit = None,
        }
    }

    fn selected_photo_detail(&self, photo_id: PhotoId) -> bool {
        matches!(self.route(), WorkspaceRoute::PhotoDetail(selected) if selected == photo_id)
            && self
                .library_state
                .ready_workspace()
                .and_then(|workspace| workspace.detail(photo_id))
                .is_some()
    }

    fn selected_detail_id(&self) -> Option<PhotoId> {
        match self.route() {
            WorkspaceRoute::PhotoDetail(photo_id) if self.selected_photo_detail(photo_id) => {
                Some(photo_id)
            }
            WorkspaceRoute::Library | WorkspaceRoute::PhotoDetail(_) => None,
        }
    }

    fn export_is_active(&self, photo_id: PhotoId) -> bool {
        let Some(status) = self.export_status(photo_id) else {
            return false;
        };
        status.as_str().starts_with("Choosing PNG destination")
            || status.as_str().starts_with("Rendering selected edit")
            || status
                .as_str()
                .starts_with("Encoding, verifying, and publishing PNG")
            || status.as_str().starts_with("Cancelling PNG export")
    }

    #[must_use]
    pub fn focused_photo(&self) -> Option<PhotoId> {
        match self.input.focused() {
            FocusTarget::PhotoCard(photo_id) => Some(photo_id),
            FocusTarget::SidebarToggle
            | FocusTarget::Library
            | FocusTarget::ImportFiles
            | FocusTarget::CancelImport
            | FocusTarget::RetryImport(_)
            | FocusTarget::RemoveImportResult(_)
            | FocusTarget::CloseImportPanel
            | FocusTarget::SaveRenderedCopy(_)
            | FocusTarget::ExportSize(_, _)
            | FocusTarget::StartRenderedCopy(_)
            | FocusTarget::CancelRenderedCopy(_)
            | FocusTarget::RetryLibrary
            | FocusTarget::Preview(_)
            | FocusTarget::BasicEdit(_)
            | FocusTarget::BackToLibrary => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiEffect {
    None,
    RetryLibrary,
    ImportFiles,
    CancelImport,
    RetryImport(u64),
    SaveRenderedCopy(PhotoId),
}

#[cfg(test)]
mod tests {
    use rusttable_core::FiniteF64;

    use super::{UiEffect, UiState};
    use crate::input::BasicEditIntent;
    use crate::{InputIntent, LibraryState, UiMessage};

    #[test]
    fn default_state_has_visible_sidebar_and_library_route() {
        let state = UiState::default();

        assert!(state.sidebar_visible());
        assert_eq!(state.route(), crate::WorkspaceRoute::Library);
        assert_eq!(state.library_state(), &LibraryState::Empty);
    }

    #[test]
    fn input_effects_are_reduced_inside_ui_state() {
        let mut state = UiState::default();

        assert_eq!(state.handle(UiMessage::ToggleSidebar), UiEffect::None);
        assert!(!state.sidebar_visible());
        assert_eq!(
            state.handle(UiMessage::Input(InputIntent::Activate)),
            UiEffect::None
        );
        assert!(state.sidebar_visible());
    }

    #[test]
    fn preview_focus_returns_to_the_selected_catalog_card_on_escape() {
        let photo_id = rusttable_core::PhotoId::new(1).expect("test photo ID is non-zero");
        let workspace = crate::PhotoWorkspaceViewModel::new(
            vec![crate::PhotoCardViewModel::new(
                photo_id,
                crate::PresentationText::new("Photo 1").expect("test text is valid"),
                None,
            )],
            vec![crate::PhotoDetailViewModel::new(
                photo_id,
                crate::PresentationText::new("Photo 1").expect("test text is valid"),
                Vec::new(),
            )],
        )
        .expect("test workspace is valid");
        let mut state = UiState::with_photo_workspace(workspace);

        assert_eq!(
            state.handle(UiMessage::Navigate(crate::NavigationIntent::ShowPhoto(
                photo_id
            ),)),
            UiEffect::None
        );
        assert!(state.is_focused(crate::FocusTarget::Preview(photo_id)));

        assert_eq!(
            state.handle(UiMessage::Input(crate::InputIntent::Escape)),
            UiEffect::None
        );
        assert_eq!(state.route(), crate::WorkspaceRoute::Library);
        assert!(state.is_focused(crate::FocusTarget::PhotoCard(photo_id)));
    }

    #[test]
    fn export_status_is_scoped_to_the_selected_photo_detail() {
        let photo_id = rusttable_core::PhotoId::new(1).expect("test photo ID is non-zero");
        let workspace = crate::PhotoWorkspaceViewModel::new(
            vec![crate::PhotoCardViewModel::new(
                photo_id,
                crate::PresentationText::new("Photo 1").expect("test text is valid"),
                None,
            )],
            vec![crate::PhotoDetailViewModel::new(
                photo_id,
                crate::PresentationText::new("Photo 1").expect("test text is valid"),
                Vec::new(),
            )],
        )
        .expect("test workspace is valid");
        let mut state = UiState::with_photo_workspace(workspace);

        state.set_export_status(photo_id, "Ignored before selection".to_owned());
        assert!(state.export_status(photo_id).is_none());
        let _ = state.handle(UiMessage::Navigate(crate::NavigationIntent::ShowPhoto(
            photo_id,
        )));
        state.set_export_status(photo_id, "Saved rendered.png".to_owned());

        assert_eq!(
            state
                .export_status(photo_id)
                .map(crate::PresentationText::as_str),
            Some("Saved rendered.png")
        );
    }

    #[test]
    fn png_export_controls_project_bounded_settings_and_safe_cancellation() {
        let photo_id = rusttable_core::PhotoId::new(1).expect("test photo ID is non-zero");
        let workspace = crate::PhotoWorkspaceViewModel::new(
            vec![crate::PhotoCardViewModel::new(
                photo_id,
                crate::PresentationText::new("Photo 1").expect("test text is valid"),
                None,
            )],
            vec![crate::PhotoDetailViewModel::new(
                photo_id,
                crate::PresentationText::new("Photo 1").expect("test text is valid"),
                Vec::new(),
            )],
        )
        .expect("test workspace is valid");
        let mut state = UiState::with_photo_workspace(workspace);

        assert_eq!(
            state.handle(UiMessage::Input(InputIntent::Export(
                crate::input::ExportIntent::SelectSize(crate::input::ExportSize::Fit2048),
            ))),
            UiEffect::None
        );
        assert_eq!(state.export_size(), crate::input::ExportSize::Original);

        let _ = state.handle(UiMessage::Navigate(crate::NavigationIntent::ShowPhoto(
            photo_id,
        )));
        assert_eq!(
            state.handle(UiMessage::Input(InputIntent::Export(
                crate::input::ExportIntent::SelectSize(crate::input::ExportSize::Fit2048),
            ))),
            UiEffect::None
        );
        assert_eq!(state.export_size(), crate::input::ExportSize::Fit2048);
        assert_eq!(
            state.handle(UiMessage::Input(InputIntent::Export(
                crate::input::ExportIntent::Start,
            ))),
            UiEffect::SaveRenderedCopy(photo_id)
        );
        assert!(state.export_in_progress(photo_id));
        assert_eq!(
            state.handle(UiMessage::Input(InputIntent::Export(
                crate::input::ExportIntent::SelectSize(crate::input::ExportSize::Fit4096),
            ))),
            UiEffect::None
        );
        assert_eq!(state.export_size(), crate::input::ExportSize::Fit2048);

        assert_eq!(
            state.handle(UiMessage::Input(InputIntent::Export(
                crate::input::ExportIntent::Cancel,
            ))),
            UiEffect::None
        );
        assert!(state.export_cancellation_requested(photo_id));
        assert_eq!(
            state
                .export_status(photo_id)
                .map(crate::PresentationText::as_str),
            Some("Cancelling PNG export…")
        );
    }

    #[test]
    fn basic_edit_intents_update_only_the_selected_photo_inspector() {
        let photo_id = rusttable_core::PhotoId::new(1).expect("test photo ID is non-zero");
        let workspace = crate::PhotoWorkspaceViewModel::new(
            vec![crate::PhotoCardViewModel::new(
                photo_id,
                crate::PresentationText::new("Photo 1").expect("test text is valid"),
                None,
            )],
            vec![crate::PhotoDetailViewModel::new(
                photo_id,
                crate::PresentationText::new("Photo 1").expect("test text is valid"),
                Vec::new(),
            )],
        )
        .expect("test workspace is valid");
        let mut state = UiState::with_photo_workspace(workspace);
        let _ = state.handle(UiMessage::Navigate(crate::NavigationIntent::ShowPhoto(
            photo_id,
        )));

        let _ = state.handle(UiMessage::Input(InputIntent::BasicEdit(
            BasicEditIntent::Increment(crate::presentation::BasicEditField::Exposure),
        )));
        let inspector = state.basic_edit().expect("selected photo has inspector");
        assert_eq!(
            inspector
                .values()
                .value(crate::presentation::BasicEditField::Exposure),
            FiniteF64::new(0.01).expect("edit step is finite")
        );
        assert_eq!(
            inspector.save_state(),
            crate::presentation::BasicEditSaveState::Unsaved
        );

        let _ = state.handle(UiMessage::Input(InputIntent::BasicEdit(
            BasicEditIntent::Reset,
        )));
        assert_eq!(
            state
                .basic_edit()
                .expect("selected photo has inspector")
                .save_state(),
            crate::presentation::BasicEditSaveState::Clean
        );
        let _ = state.handle(UiMessage::Input(InputIntent::BasicEdit(
            BasicEditIntent::Commit,
        )));
        assert_eq!(
            state
                .basic_edit()
                .expect("selected photo has inspector")
                .save_state(),
            crate::presentation::BasicEditSaveState::Saving
        );
    }

    #[test]
    fn failed_basic_edit_save_keeps_values_until_successful_projection() {
        let photo_id = rusttable_core::PhotoId::new(1).expect("test photo ID is non-zero");
        let workspace = crate::PhotoWorkspaceViewModel::new(
            vec![crate::PhotoCardViewModel::new(
                photo_id,
                crate::PresentationText::new("Photo 1").expect("test text is valid"),
                None,
            )],
            vec![crate::PhotoDetailViewModel::new(
                photo_id,
                crate::PresentationText::new("Photo 1").expect("test text is valid"),
                Vec::new(),
            )],
        )
        .expect("test workspace is valid");
        let mut state = UiState::with_photo_workspace(workspace);
        let _ = state.handle(UiMessage::Navigate(crate::NavigationIntent::ShowPhoto(
            photo_id,
        )));

        let values = crate::presentation::BasicEditValues::defaults();
        let mut inspector =
            crate::presentation::BasicEditInspectorViewModel::with_values(photo_id, values);
        inspector.increment(crate::presentation::BasicEditField::Exposure);
        let changed = inspector.values();
        state.set_basic_edit_draft_values(photo_id, changed);
        state.begin_basic_edit_save(photo_id);
        state.fail_basic_edit_save(photo_id);

        let failed = state.basic_edit().expect("selected photo has inspector");
        assert_eq!(
            failed.save_state(),
            crate::presentation::BasicEditSaveState::Failed
        );
        assert_eq!(failed.values(), changed);

        state.set_basic_edit_values(photo_id, changed);
        let clean = state.basic_edit().expect("selected photo has inspector");
        assert_eq!(
            clean.save_state(),
            crate::presentation::BasicEditSaveState::Clean
        );
        assert_eq!(clean.values(), changed);
    }
}
