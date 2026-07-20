#[expect(
    dead_code,
    reason = "issue-536 defines the typed foundation without wiring UI or persistence"
)]
mod edit_draft;
mod preview_loader;

#[expect(
    unused_imports,
    reason = "this is the workspace boundary for future draft editing consumers"
)]
pub use edit_draft::{
    BasicEditDraft, BasicEditDraftError, BasicEditDraftReplacementError, BasicEditOperation,
    BasicEditParameter, BasicEditValue, BasicEditValueError, ParameterValueType,
};
pub(crate) use preview_loader::{SelectedPreview, load_selected_preview};
