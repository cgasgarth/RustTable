#![forbid(unsafe_code)]
#![doc = "Compatibility boundary contracts for the `RustTable` rewrite."]

pub mod purpose;

mod accounting;
mod organization;

pub use accounting::{OrganizationAccountingEntry, SOURCE_ACCOUNTING};
pub use organization::{
    ColorLabel, ColorLabelRecord, DarktableRating, DecodeOptions, Finding, FindingCode,
    GroupMemberRecord, GroupRecord, ImageOrganizationRecord, OrganizationDecoder,
    OrganizationLimits, OrganizationSnapshot, Severity, SourceRowKey, SynonymRecord,
    TagAssignmentRecord, TagComponent, TagFlags, TagRecord,
};
