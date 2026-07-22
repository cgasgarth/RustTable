use rusttable_metadata::{CanonicalMetadataPolicy, MetadataAction as CanonicalAction};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetadataAction {
    Include,
    Exclude,
    Redact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MetadataPolicy {
    pub exif: MetadataAction,
    pub iptc: MetadataAction,
    pub xmp: MetadataAction,
    pub gps: MetadataAction,
    pub faces_and_regions: MetadataAction,
    pub ratings_labels_tags: MetadataAction,
    pub history: MetadataAction,
    pub thumbnail: MetadataAction,
    pub icc_and_cicp: MetadataAction,
    pub software_and_version: MetadataAction,
    pub user_fields: MetadataAction,
}

impl Default for MetadataPolicy {
    fn default() -> Self {
        Self::standard()
    }
}

impl MetadataPolicy {
    /// The product default, available in constant export request builders.
    /// It mirrors Darktable's EXIF, metadata, geotag, tag, and history flags
    /// for the field groups `RustTable` currently represents.
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            exif: MetadataAction::Include,
            iptc: MetadataAction::Include,
            xmp: MetadataAction::Include,
            gps: MetadataAction::Include,
            faces_and_regions: MetadataAction::Redact,
            ratings_labels_tags: MetadataAction::Include,
            history: MetadataAction::Include,
            thumbnail: MetadataAction::Include,
            icc_and_cicp: MetadataAction::Include,
            software_and_version: MetadataAction::Include,
            user_fields: MetadataAction::Redact,
        }
    }

    #[must_use]
    pub fn canonical(self) -> CanonicalMetadataPolicy {
        CanonicalMetadataPolicy {
            camera_exposure_lens: action(self.exif, CanonicalAction::Include),
            capture_date_time: action(self.exif, CanonicalAction::Include),
            description_rights: action(self.iptc, CanonicalAction::Include),
            keywords_rating: action(self.ratings_labels_tags, CanonicalAction::Include),
            gps_location: action(self.gps, CanonicalAction::Exclude),
            people_regions: action(self.faces_and_regions, CanonicalAction::Exclude),
            edit_history: action(self.history, CanonicalAction::Exclude),
            thumbnail: action(self.thumbnail, CanonicalAction::Exclude),
            technical: action(self.icc_and_cicp, CanonicalAction::Include),
            software_version: action(self.software_and_version, CanonicalAction::Include),
            unknown_imported: action(self.user_fields, CanonicalAction::Exclude),
            ..Default::default()
        }
    }

    /// Returns the stable identity of the immutable policy carried by an export.
    #[must_use]
    pub fn identity(self) -> String {
        format!(
            "rusttable.export-metadata-policy.v1:{:x}",
            policy_bits(self)
        )
    }

    /// Returns the concrete field groups selected for serialization.
    #[must_use]
    pub fn included_groups(self) -> Vec<String> {
        groups(self, false)
    }

    /// Returns the concrete field groups excluded from serialization.
    #[must_use]
    pub fn excluded_groups(self) -> Vec<String> {
        groups(self, true)
    }
}

fn policy_bits(policy: MetadataPolicy) -> u32 {
    [
        policy.exif,
        policy.iptc,
        policy.xmp,
        policy.gps,
        policy.faces_and_regions,
        policy.ratings_labels_tags,
        policy.history,
        policy.thumbnail,
        policy.icc_and_cicp,
        policy.software_and_version,
        policy.user_fields,
    ]
    .into_iter()
    .enumerate()
    .fold(0, |bits, (index, action)| {
        bits | (action as u32) << (index * 2)
    })
}

fn groups(policy: MetadataPolicy, excluded: bool) -> Vec<String> {
    [
        ("exif", policy.exif),
        ("iptc", policy.iptc),
        ("xmp", policy.xmp),
        ("gps", policy.gps),
        ("faces-and-regions", policy.faces_and_regions),
        ("ratings-labels-tags", policy.ratings_labels_tags),
        ("history", policy.history),
        ("thumbnail", policy.thumbnail),
        ("icc-and-cicp", policy.icc_and_cicp),
        ("software-and-version", policy.software_and_version),
        ("user-fields", policy.user_fields),
    ]
    .into_iter()
    .filter_map(|(name, action)| {
        (if excluded {
            action != MetadataAction::Include
        } else {
            action == MetadataAction::Include
        })
        .then_some(name.to_owned())
    })
    .collect()
}

fn action(value: MetadataAction, include: CanonicalAction) -> CanonicalAction {
    match value {
        MetadataAction::Include => include,
        MetadataAction::Exclude => CanonicalAction::Exclude,
        MetadataAction::Redact => CanonicalAction::Redact,
    }
}
