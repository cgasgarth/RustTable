mod edit_commit;
mod edit_draft;
mod edit_history;
pub(crate) mod preview_loader;
mod raster_import;

pub use edit_commit::{BasicEditCommitError, commit_basic_edit, commit_basic_edit_at_path};
#[expect(
    unused_imports,
    reason = "this is the workspace boundary for future draft editing consumers"
)]
pub use edit_draft::{
    BasicEditCommand, BasicEditCommandError, BasicEditDraft, BasicEditDraftError,
    BasicEditDraftReplacementError, BasicEditOperation, BasicEditParameter, BasicEditValue,
    BasicEditValueError, BasicEditValues, ParameterValueType,
};
pub use edit_history::{BasicEditMutation, BasicEditSession};
pub(crate) use preview_loader::{SelectedPreview, load_selected_preview};
pub(crate) use raster_import::{pick_raster_files, run_raster_import};
