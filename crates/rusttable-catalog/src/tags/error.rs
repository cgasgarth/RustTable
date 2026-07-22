use std::fmt;

use rusttable_core::{PhotoId, Revision};

use super::TagId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagError {
    EmptyName,
    NameTooLong,
    AliasTooLong,
    InvalidControl,
    HierarchySeparator,
    EmptyPathSegment,
    TooManyAliases,
    DuplicateAlias,
    InvalidTagId,
    TooManyTags,
    TooManyTaggedPhotos,
    TooManyTagsForPhoto {
        photo_id: PhotoId,
    },
    UnknownParent {
        parent_id: TagId,
    },
    UnknownTag {
        tag_id: TagId,
    },
    UnknownPhoto {
        photo_id: PhotoId,
    },
    TagIdConflict {
        tag_id: TagId,
    },
    CanonicalPathConflict {
        path: String,
        existing_tag_id: TagId,
        conflicting_tag_id: TagId,
    },
    AliasConflict {
        alias: String,
        existing_tag_id: TagId,
        conflicting_tag_id: TagId,
    },
    HierarchyCycle {
        tag_id: TagId,
    },
    HierarchyTooDeep {
        tag_id: TagId,
    },
    EmptyAssignmentBatch,
    AssignmentBatchTooLarge,
    DuplicatePhotoInBatch {
        photo_id: PhotoId,
    },
    DuplicateTagInBatch {
        tag_id: TagId,
    },
    AssignmentExists {
        photo_id: PhotoId,
        tag_id: TagId,
    },
    AssignmentMissing {
        photo_id: PhotoId,
        tag_id: TagId,
    },
    RevisionConflict {
        expected: Revision,
        actual: Revision,
    },
    RevisionOverflow,
    UnsupportedSchema {
        version: u16,
    },
    CorruptPersistedData,
    Unavailable,
    CommitFailure,
}

impl fmt::Display for TagError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => formatter.write_str("tag name cannot be empty"),
            Self::NameTooLong => formatter.write_str("tag name exceeds its byte limit"),
            Self::AliasTooLong => formatter.write_str("tag alias exceeds its byte limit"),
            Self::InvalidControl => formatter.write_str("tag text cannot contain controls"),
            Self::HierarchySeparator => {
                formatter.write_str("tag name cannot contain the hierarchy separator")
            }
            Self::EmptyPathSegment => {
                formatter.write_str("tag path cannot contain an empty segment")
            }
            Self::TooManyAliases => formatter.write_str("tag alias count exceeds its limit"),
            Self::DuplicateAlias => {
                formatter.write_str("tag aliases must be unique after normalization")
            }
            Self::InvalidTagId => formatter.write_str("tag ID cannot be zero"),
            Self::TooManyTags => formatter.write_str("tag count exceeds its limit"),
            Self::TooManyTaggedPhotos => {
                formatter.write_str("tagged photo count exceeds its limit")
            }
            Self::TooManyTagsForPhoto { photo_id } => {
                write!(formatter, "photo {photo_id} has too many tags")
            }
            Self::UnknownParent { parent_id } => {
                write!(formatter, "tag parent {parent_id} does not exist")
            }
            Self::UnknownTag { tag_id } => write!(formatter, "tag {tag_id} does not exist"),
            Self::UnknownPhoto { photo_id } => write!(formatter, "photo {photo_id} does not exist"),
            Self::TagIdConflict { tag_id } => write!(formatter, "tag ID {tag_id} already exists"),
            Self::CanonicalPathConflict { path, .. } => {
                write!(formatter, "canonical tag path {path:?} conflicts")
            }
            Self::AliasConflict { alias, .. } => write!(formatter, "tag alias {alias:?} conflicts"),
            Self::HierarchyCycle { tag_id } => {
                write!(formatter, "tag hierarchy contains a cycle at {tag_id}")
            }
            Self::HierarchyTooDeep { tag_id } => write!(
                formatter,
                "tag hierarchy exceeds its depth limit at {tag_id}"
            ),
            Self::EmptyAssignmentBatch => {
                formatter.write_str("tag assignment batch cannot be empty")
            }
            Self::AssignmentBatchTooLarge => {
                formatter.write_str("tag assignment batch exceeds its limit")
            }
            Self::DuplicatePhotoInBatch { photo_id } => {
                write!(formatter, "tag assignment duplicates photo {photo_id}")
            }
            Self::DuplicateTagInBatch { tag_id } => {
                write!(formatter, "tag assignment duplicates tag {tag_id}")
            }
            Self::AssignmentExists { photo_id, tag_id } => {
                write!(formatter, "photo {photo_id} already has tag {tag_id}")
            }
            Self::AssignmentMissing { photo_id, tag_id } => {
                write!(formatter, "photo {photo_id} does not have tag {tag_id}")
            }
            Self::RevisionConflict { expected, actual } => write!(
                formatter,
                "tag revision conflict: expected {expected}, actual {actual}"
            ),
            Self::RevisionOverflow => formatter.write_str("tag revision cannot be incremented"),
            Self::UnsupportedSchema { version } => {
                write!(formatter, "unsupported tag schema {version}")
            }
            Self::CorruptPersistedData => formatter.write_str("persisted tag data is corrupt"),
            Self::Unavailable => formatter.write_str("tag repository is unavailable"),
            Self::CommitFailure => formatter.write_str("tag transaction did not commit"),
        }
    }
}

impl std::error::Error for TagError {}
