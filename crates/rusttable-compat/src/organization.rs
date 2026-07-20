use std::collections::{BTreeMap, BTreeSet};

use rusttable_sqlite_native::{DarktableSchema, OrganizationRows, RawImageRow};

const RATING_MASK: u64 = 0x07;
const REJECTED_FLAG: u64 = 0x08;
const KNOWN_IMAGE_FLAGS: u64 = RATING_MASK | REJECTED_FLAG;
const KNOWN_TAG_FLAGS: u64 = 0x8000_0000 | 0x07;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceRowKey {
    table: &'static str,
    row: u64,
}

impl SourceRowKey {
    #[must_use]
    pub const fn new(table: &'static str, row: u64) -> Self {
        Self { table, row }
    }

    #[must_use]
    pub const fn table(self) -> &'static str {
        self.table
    }

    #[must_use]
    pub const fn row(self) -> u64 {
        self.row
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagComponent {
    pub ordinal: usize,
    pub raw: Vec<u8>,
    pub display: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct TagFlags {
    pub raw: i64,
    pub category: bool,
    pub private: bool,
    pub order_set: bool,
    pub descending: bool,
    pub unknown_bits: u64,
}

impl TagFlags {
    fn decode(raw: i64) -> Self {
        let bits = raw.cast_unsigned();
        Self {
            raw,
            category: bits & 1 != 0,
            private: bits & 2 != 0,
            order_set: bits & 4 != 0,
            descending: bits & 0x8000_0000 != 0,
            unknown_bits: bits & !KNOWN_TAG_FLAGS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SynonymRecord {
    pub source: SourceRowKey,
    pub ordinal: usize,
    pub raw: Vec<u8>,
    pub display: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagRecord {
    pub source: SourceRowKey,
    pub id: i64,
    pub literal_name: Vec<u8>,
    pub components: Vec<TagComponent>,
    pub canonical_path: Option<String>,
    pub flags: TagFlags,
    pub raw_synonyms: Option<Vec<u8>>,
    pub synonyms: Vec<SynonymRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagAssignmentRecord {
    pub source: SourceRowKey,
    pub image_id: i64,
    pub tag_id: i64,
    pub position: Option<i64>,
    pub orphan_image: bool,
    pub orphan_tag: bool,
    pub duplicate_key: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DarktableRating {
    Zero,
    One,
    Two,
    Three,
    Four,
    Five,
}

impl DarktableRating {
    #[must_use]
    pub const fn from_bits(bits: u8) -> Option<Self> {
        match bits {
            0 => Some(Self::Zero),
            1 => Some(Self::One),
            2 => Some(Self::Two),
            3 => Some(Self::Three),
            4 => Some(Self::Four),
            5 => Some(Self::Five),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Zero => 0,
            Self::One => 1,
            Self::Two => 2,
            Self::Three => 3,
            Self::Four => 4,
            Self::Five => 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ColorLabel {
    Red,
    Yellow,
    Green,
    Blue,
    Purple,
}

impl ColorLabel {
    #[must_use]
    pub const fn from_raw(raw: i64) -> Option<Self> {
        match raw {
            0 => Some(Self::Red),
            1 => Some(Self::Yellow),
            2 => Some(Self::Green),
            3 => Some(Self::Blue),
            4 => Some(Self::Purple),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageOrganizationRecord {
    pub source: SourceRowKey,
    pub image_id: i64,
    pub group_id: Option<i64>,
    pub raw_flags: i64,
    pub rating_bits: u8,
    pub rating: Option<DarktableRating>,
    pub rejected: bool,
    pub unknown_flag_bits: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorLabelRecord {
    pub source: SourceRowKey,
    pub image_id: i64,
    pub raw_color: i64,
    pub label: Option<ColorLabel>,
    pub unknown_value: bool,
    pub orphan_image: bool,
    pub duplicate_key: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupMemberRecord {
    pub source: SourceRowKey,
    pub image_id: i64,
    pub group_id: i64,
    pub is_representative: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupRecord {
    pub group_id: i64,
    pub members: Vec<GroupMemberRecord>,
    pub representative_sources: Vec<SourceRowKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Blocking,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum FindingCode {
    InvalidUtf8,
    EmptyTagComponent,
    TagDepthLimit,
    TagComponentLimit,
    DuplicateTagId,
    DuplicateCanonicalPath,
    DuplicateAssignment,
    OrphanAssignmentImage,
    OrphanAssignmentTag,
    InvalidRatingBits,
    UnknownImageFlags,
    UnknownColorLabel,
    DuplicateColorLabel,
    OrphanColorLabel,
    OrphanGroupMember,
    DuplicateImageId,
    MultipleRepresentatives,
    MissingRepresentative,
    CrossGroupConflict,
    GroupCycle,
    LimitTruncated,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    pub code: FindingCode,
    pub severity: Severity,
    pub source: Option<SourceRowKey>,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrganizationLimits {
    pub max_tags: usize,
    pub max_assignments: usize,
    pub max_labels: usize,
    pub max_images: usize,
    pub max_hierarchy_depth: usize,
    pub max_component_bytes: usize,
    pub max_graph_steps: usize,
}

impl Default for OrganizationLimits {
    fn default() -> Self {
        Self {
            max_tags: 100_000,
            max_assignments: 1_000_000,
            max_labels: 1_000_000,
            max_images: 1_000_000,
            max_hierarchy_depth: 128,
            max_component_bytes: 16 * 1024,
            max_graph_steps: 1_000_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DecodeOptions {
    pub limits: OrganizationLimits,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrganizationSnapshot {
    pub schema: DarktableSchema,
    pub tags: Vec<TagRecord>,
    pub assignments: Vec<TagAssignmentRecord>,
    pub images: Vec<ImageOrganizationRecord>,
    pub labels: Vec<ColorLabelRecord>,
    pub groups: Vec<GroupRecord>,
    pub findings: Vec<Finding>,
}

pub struct OrganizationDecoder {
    options: DecodeOptions,
}

impl OrganizationDecoder {
    #[must_use]
    pub const fn new(options: DecodeOptions) -> Self {
        Self { options }
    }

    #[must_use]
    pub fn decode(
        &self,
        schema: DarktableSchema,
        mut rows: OrganizationRows,
    ) -> OrganizationSnapshot {
        rows.tags.sort_by_key(|row| (row.id, row.source_row));
        rows.assignments
            .sort_by_key(|row| (row.image_id, row.tag_id, row.source_row));
        rows.labels
            .sort_by_key(|row| (row.image_id, row.source_row, row.color));
        rows.images
            .sort_by_key(|row| (row.image_id, row.source_row));
        let mut findings = Vec::new();
        let tags = decode_tags(&rows, &self.options.limits, &mut findings);
        let tag_ids = tags.iter().map(|tag| tag.id).collect::<BTreeSet<_>>();
        let assignments = decode_assignments(
            &rows,
            &self.options.limits,
            &tag_ids,
            &rows.images,
            &mut findings,
        );
        let images = decode_images(&rows, &self.options.limits, &mut findings);
        let image_ids = images
            .iter()
            .map(|image| image.image_id)
            .collect::<BTreeSet<_>>();
        let labels = decode_labels(&rows, &self.options.limits, &image_ids, &mut findings);
        let groups = decode_groups(&images, self.options.limits.max_graph_steps, &mut findings);
        if rows.tags.len() > limits_len(self.options.limits.max_tags, rows.tags.len())
            || rows.assignments.len() > self.options.limits.max_assignments
            || rows.labels.len() > self.options.limits.max_labels
            || rows.images.len() > self.options.limits.max_images
        {
            findings.push(Finding {
                code: FindingCode::LimitTruncated,
                severity: Severity::Blocking,
                source: None,
                detail: "organization rows exceeded configured bounds".to_owned(),
            });
        }
        findings.sort();
        OrganizationSnapshot {
            schema,
            tags,
            assignments,
            images,
            labels,
            groups,
            findings,
        }
    }
}

fn limits_len(limit: usize, actual: usize) -> usize {
    limit.min(actual)
}

fn decode_tags(
    rows: &OrganizationRows,
    limits: &OrganizationLimits,
    findings: &mut Vec<Finding>,
) -> Vec<TagRecord> {
    let mut seen_ids = BTreeSet::new();
    let mut seen_paths = BTreeSet::new();
    rows.tags
        .iter()
        .take(limits.max_tags)
        .map(|row| {
            let source = SourceRowKey::new("data.tags", row.source_row);
            let mut components = Vec::new();
            for (ordinal, raw) in row.name.split(|byte| *byte == b'|').enumerate() {
                if raw.is_empty() {
                    findings.push(finding(
                        FindingCode::EmptyTagComponent,
                        Severity::Blocking,
                        source,
                        format!("tag {} component {ordinal} is empty", row.id),
                    ));
                }
                if raw.len() > limits.max_component_bytes {
                    findings.push(finding(
                        FindingCode::TagComponentLimit,
                        Severity::Blocking,
                        source,
                        format!("tag {} component {ordinal} is {} bytes", row.id, raw.len()),
                    ));
                }
                let display = if let Ok(value) = String::from_utf8(raw.to_vec()) {
                    value
                } else {
                    findings.push(finding(
                        FindingCode::InvalidUtf8,
                        Severity::Warning,
                        source,
                        format!("tag {} component {ordinal} is not UTF-8", row.id),
                    ));
                    String::from_utf8_lossy(raw).into_owned()
                };
                components.push(TagComponent {
                    ordinal,
                    raw: raw.to_vec(),
                    display,
                });
            }
            if components.len() > limits.max_hierarchy_depth {
                findings.push(finding(
                    FindingCode::TagDepthLimit,
                    Severity::Blocking,
                    source,
                    format!(
                        "tag {} depth {} exceeds {}",
                        row.id,
                        components.len(),
                        limits.max_hierarchy_depth
                    ),
                ));
            }
            let canonical_path = (components.len() <= limits.max_hierarchy_depth).then(|| {
                components
                    .iter()
                    .map(|component| component.display.as_str())
                    .collect::<Vec<_>>()
                    .join("|")
            });
            if !seen_ids.insert(row.id) {
                findings.push(finding(
                    FindingCode::DuplicateTagId,
                    Severity::Blocking,
                    source,
                    format!("tag id {} appears more than once", row.id),
                ));
            }
            if let Some(path) = &canonical_path
                && !seen_paths.insert(path.clone())
            {
                findings.push(finding(
                    FindingCode::DuplicateCanonicalPath,
                    Severity::Warning,
                    source,
                    format!("canonical tag path {path:?} appears more than once"),
                ));
            }
            TagRecord {
                source,
                id: row.id,
                literal_name: row.name.clone(),
                components,
                canonical_path,
                flags: TagFlags::decode(row.flags),
                raw_synonyms: row.synonyms.clone(),
                synonyms: decode_synonyms(source, row.synonyms.as_deref()),
            }
        })
        .collect()
}

fn decode_synonyms(source: SourceRowKey, raw: Option<&[u8]>) -> Vec<SynonymRecord> {
    let Some(raw) = raw else { return Vec::new() };
    raw.split(|byte| matches!(byte, b',' | b';' | b'\n'))
        .enumerate()
        .map(|(ordinal, value)| SynonymRecord {
            source,
            ordinal,
            raw: value.to_vec(),
            display: String::from_utf8(value.to_vec())
                .ok()
                .map(|text| text.trim().to_owned()),
        })
        .collect()
}

fn decode_assignments(
    rows: &OrganizationRows,
    limits: &OrganizationLimits,
    tag_ids: &BTreeSet<i64>,
    images: &[RawImageRow],
    findings: &mut Vec<Finding>,
) -> Vec<TagAssignmentRecord> {
    let image_ids = images
        .iter()
        .map(|image| image.image_id)
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    rows.assignments
        .iter()
        .take(limits.max_assignments)
        .map(|row| {
            let source = SourceRowKey::new("main.tagged_images", row.source_row);
            let duplicate_key = !seen.insert((row.image_id, row.tag_id));
            let orphan_image = !image_ids.contains(&row.image_id);
            let orphan_tag = !tag_ids.contains(&row.tag_id);
            if duplicate_key {
                findings.push(finding(
                    FindingCode::DuplicateAssignment,
                    Severity::Warning,
                    source,
                    format!(
                        "assignment ({}, {}) is duplicated",
                        row.image_id, row.tag_id
                    ),
                ));
            }
            if orphan_image {
                findings.push(finding(
                    FindingCode::OrphanAssignmentImage,
                    Severity::Blocking,
                    source,
                    format!("assignment references missing image {}", row.image_id),
                ));
            }
            if orphan_tag {
                findings.push(finding(
                    FindingCode::OrphanAssignmentTag,
                    Severity::Blocking,
                    source,
                    format!("assignment references missing tag {}", row.tag_id),
                ));
            }
            TagAssignmentRecord {
                source,
                image_id: row.image_id,
                tag_id: row.tag_id,
                position: row.position,
                orphan_image,
                orphan_tag,
                duplicate_key,
            }
        })
        .collect()
}

fn decode_images(
    rows: &OrganizationRows,
    limits: &OrganizationLimits,
    findings: &mut Vec<Finding>,
) -> Vec<ImageOrganizationRecord> {
    let mut seen = BTreeSet::new();
    let mut groups = BTreeMap::new();
    rows.images
        .iter()
        .take(limits.max_images)
        .map(|row| {
            let source = SourceRowKey::new("main.images", row.source_row);
            let bits = row.flags.cast_unsigned();
            let rating_bits = (bits & RATING_MASK) as u8;
            let rating = DarktableRating::from_bits(rating_bits);
            if rating.is_none() {
                findings.push(finding(
                    FindingCode::InvalidRatingBits,
                    Severity::Blocking,
                    source,
                    format!("image {} has rating bits {rating_bits}", row.image_id),
                ));
            }
            if bits & !KNOWN_IMAGE_FLAGS != 0 {
                findings.push(finding(
                    FindingCode::UnknownImageFlags,
                    Severity::Warning,
                    source,
                    format!(
                        "image {} has unknown flag bits 0x{:x}",
                        row.image_id,
                        bits & !KNOWN_IMAGE_FLAGS
                    ),
                ));
            }
            if !seen.insert(row.image_id) {
                findings.push(finding(
                    FindingCode::DuplicateImageId,
                    Severity::Blocking,
                    source,
                    format!("image id {} appears more than once", row.image_id),
                ));
            }
            if let Some(group_id) = row.group_id
                && let Some(previous) = groups.insert(row.image_id, group_id)
                && previous != group_id
            {
                findings.push(finding(
                    FindingCode::CrossGroupConflict,
                    Severity::Blocking,
                    source,
                    format!(
                        "image {} belongs to groups {} and {}",
                        row.image_id, previous, group_id
                    ),
                ));
            }
            ImageOrganizationRecord {
                source,
                image_id: row.image_id,
                group_id: row.group_id,
                raw_flags: row.flags,
                rating_bits,
                rating,
                rejected: bits & REJECTED_FLAG != 0,
                unknown_flag_bits: bits & !KNOWN_IMAGE_FLAGS,
            }
        })
        .collect()
}

fn decode_labels(
    rows: &OrganizationRows,
    limits: &OrganizationLimits,
    image_ids: &BTreeSet<i64>,
    findings: &mut Vec<Finding>,
) -> Vec<ColorLabelRecord> {
    let mut seen = BTreeSet::new();
    rows.labels
        .iter()
        .take(limits.max_labels)
        .map(|row| {
            let source = SourceRowKey::new("main.color_labels", row.source_row);
            let duplicate_key = !seen.insert((row.image_id, row.color));
            let label = ColorLabel::from_raw(row.color);
            let orphan_image = !image_ids.contains(&row.image_id);
            if duplicate_key {
                findings.push(finding(
                    FindingCode::DuplicateColorLabel,
                    Severity::Warning,
                    source,
                    format!(
                        "color label ({}, {}) is duplicated",
                        row.image_id, row.color
                    ),
                ));
            }
            if label.is_none() {
                findings.push(finding(
                    FindingCode::UnknownColorLabel,
                    Severity::Warning,
                    source,
                    format!(
                        "image {} has unknown color label {}",
                        row.image_id, row.color
                    ),
                ));
            }
            if orphan_image {
                findings.push(finding(
                    FindingCode::OrphanColorLabel,
                    Severity::Blocking,
                    source,
                    format!("color label references missing image {}", row.image_id),
                ));
            }
            ColorLabelRecord {
                source,
                image_id: row.image_id,
                raw_color: row.color,
                label,
                unknown_value: label.is_none(),
                orphan_image,
                duplicate_key,
            }
        })
        .collect()
}

fn decode_groups(
    images: &[ImageOrganizationRecord],
    max_steps: usize,
    findings: &mut Vec<Finding>,
) -> Vec<GroupRecord> {
    let ids = images
        .iter()
        .map(|image| image.image_id)
        .collect::<BTreeSet<_>>();
    let mut groups = BTreeMap::<i64, Vec<GroupMemberRecord>>::new();
    for image in images {
        let Some(group_id) = image.group_id else {
            continue;
        };
        let member = GroupMemberRecord {
            source: image.source,
            image_id: image.image_id,
            group_id,
            is_representative: image.image_id == group_id,
        };
        if !ids.contains(&group_id) {
            findings.push(finding(
                FindingCode::OrphanGroupMember,
                Severity::Blocking,
                image.source,
                format!(
                    "image {} references missing group representative {}",
                    image.image_id, group_id
                ),
            ));
        }
        groups.entry(group_id).or_default().push(member);
    }
    let mut output = Vec::new();
    for (group_id, mut members) in groups {
        members.sort_by_key(|member| (member.image_id, member.source));
        let representatives = members
            .iter()
            .filter(|member| member.is_representative)
            .map(|member| member.source)
            .collect::<Vec<_>>();
        if representatives.is_empty() {
            findings.push(finding_without_source(
                FindingCode::MissingRepresentative,
                Severity::Warning,
                format!("group {group_id} has no representative row"),
            ));
        }
        if representatives.len() > 1 {
            findings.push(finding(
                FindingCode::MultipleRepresentatives,
                Severity::Blocking,
                representatives[0],
                format!(
                    "group {group_id} has {} representative rows",
                    representatives.len()
                ),
            ));
        }
        output.push(GroupRecord {
            group_id,
            members,
            representative_sources: representatives,
        });
    }
    detect_group_cycles(images, max_steps, findings);
    output
}

fn detect_group_cycles(
    images: &[ImageOrganizationRecord],
    max_steps: usize,
    findings: &mut Vec<Finding>,
) {
    let edges = images
        .iter()
        .filter_map(|image| image.group_id.map(|group| (image.image_id, group)))
        .collect::<BTreeMap<_, _>>();
    let mut steps = 0;
    for start in edges.keys().copied() {
        let mut path = Vec::new();
        let mut current = start;
        while let Some(next) = edges.get(&current).copied() {
            steps += 1;
            if steps > max_steps {
                findings.push(finding_without_source(
                    FindingCode::LimitTruncated,
                    Severity::Blocking,
                    format!("group graph traversal exceeded {max_steps} steps"),
                ));
                return;
            }
            if next == current {
                break;
            }
            if let Some(index) = path.iter().position(|id| *id == current) {
                let cycle = path[index..]
                    .iter()
                    .chain(std::iter::once(&current))
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("->");
                findings.push(finding_without_source(
                    FindingCode::GroupCycle,
                    Severity::Blocking,
                    cycle,
                ));
                break;
            }
            path.push(current);
            current = next;
        }
    }
}

fn finding(code: FindingCode, severity: Severity, source: SourceRowKey, detail: String) -> Finding {
    Finding {
        code,
        severity,
        source: Some(source),
        detail,
    }
}

fn finding_without_source(code: FindingCode, severity: Severity, detail: String) -> Finding {
    Finding {
        code,
        severity,
        source: None,
        detail,
    }
}
