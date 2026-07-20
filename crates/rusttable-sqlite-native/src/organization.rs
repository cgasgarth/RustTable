/// Raw rows are deliberately lossless and do not validate foreign keys. The
/// compatibility layer owns interpretation and findings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawTagRow {
    pub source_row: u64,
    pub id: i64,
    pub name: Vec<u8>,
    pub synonyms: Option<Vec<u8>>,
    pub flags: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawTagAssignmentRow {
    pub source_row: u64,
    pub image_id: i64,
    pub tag_id: i64,
    pub position: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawColorLabelRow {
    pub source_row: u64,
    pub image_id: i64,
    pub color: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawImageRow {
    pub source_row: u64,
    pub image_id: i64,
    pub group_id: Option<i64>,
    pub flags: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrganizationRows {
    pub tags: Vec<RawTagRow>,
    pub assignments: Vec<RawTagAssignmentRow>,
    pub labels: Vec<RawColorLabelRow>,
    pub images: Vec<RawImageRow>,
}

impl OrganizationRows {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
            && self.assignments.is_empty()
            && self.labels.is_empty()
            && self.images.is_empty()
    }
}
