#![forbid(unsafe_code)]
#![doc = "Compatibility boundary contracts for the `RustTable` rewrite."]

pub mod basicadj;
pub mod channelmixer;
pub mod purpose;
pub mod sharpen;
pub mod soften;

mod accounting;
mod history;
mod organization;

pub use accounting::{OrganizationAccountingEntry, SOURCE_ACCOUNTING};
pub use history::{
    CompatHistory, CompatHistoryHash, CompatHistoryStep, CompatModuleInstance, CompatModuleOrder,
    DARKTABLE_ORDER_RULES, DarktableOperationManifest, EnabledState, HistoryDecodeOptions,
    HistoryDecoder, HistoryLimits, HistoryOrderSource, HistorySelection, ModuleInstanceId,
    ModuleOrderEntry, ModuleOrderRule, ModuleOrderVersion, OpaquePayload, OperationCompatibility,
    OperationIdentity,
};
pub use organization::{
    ColorLabel, ColorLabelRecord, DarktableRating, DecodeOptions, Finding, FindingCode,
    GroupMemberRecord, GroupRecord, ImageOrganizationRecord, OrganizationDecoder,
    OrganizationLimits, OrganizationSnapshot, Severity, SourceRowKey, SynonymRecord,
    TagAssignmentRecord, TagComponent, TagFlags, TagRecord,
};
