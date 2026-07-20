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
        Self {
            exif: MetadataAction::Include,
            iptc: MetadataAction::Include,
            xmp: MetadataAction::Include,
            gps: MetadataAction::Redact,
            faces_and_regions: MetadataAction::Redact,
            ratings_labels_tags: MetadataAction::Include,
            history: MetadataAction::Exclude,
            thumbnail: MetadataAction::Include,
            icc_and_cicp: MetadataAction::Include,
            software_and_version: MetadataAction::Include,
            user_fields: MetadataAction::Redact,
        }
    }
}

impl MetadataPolicy {
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
}

fn action(value: MetadataAction, include: CanonicalAction) -> CanonicalAction {
    match value {
        MetadataAction::Include => include,
        MetadataAction::Exclude => CanonicalAction::Exclude,
        MetadataAction::Redact => CanonicalAction::Redact,
    }
}
