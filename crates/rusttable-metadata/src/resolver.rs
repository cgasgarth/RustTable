use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use sha2::{Digest, Sha256};

use crate::{
    DatePrecision, DomainValue, HierarchicalKeywords, LanguageTag, MAX_METADATA_RECORDS,
    MetadataDocument, MetadataDomainError, MetadataKey, MetadataProvenance, MetadataRecord,
    MetadataSource, MetadataSourceClass, PrivacyClass,
};

const MAX_ASSERTIONS: usize = MAX_METADATA_RECORDS * 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetadataMergeStrategy {
    SourcePriority,
    SetUnion,
    HierarchicalUnion,
    LanguageSelection,
    DatePrecision,
    RatingMapping,
    LabelMapping,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataPrecedencePolicy {
    preferred_languages: Vec<LanguageTag>,
}

impl Default for MetadataPrecedencePolicy {
    fn default() -> Self {
        Self {
            preferred_languages: vec![
                LanguageTag::new("x-default").expect("static language tag"),
                LanguageTag::new("en").expect("static language tag"),
            ],
        }
    }
}

impl MetadataPrecedencePolicy {
    #[must_use]
    pub fn with_preferred_languages(mut self, languages: Vec<LanguageTag>) -> Self {
        self.preferred_languages.clear();
        for language in languages {
            if !self.preferred_languages.contains(&language) {
                self.preferred_languages.push(language);
            }
        }
        self
    }

    #[must_use]
    pub fn preferred_languages(&self) -> &[LanguageTag] {
        &self.preferred_languages
    }

    #[must_use]
    pub fn strategy(&self, key: &MetadataKey) -> MetadataMergeStrategy {
        match key.name().to_ascii_lowercase().as_str() {
            "keywords" | "subject" | "people" | "creator" => MetadataMergeStrategy::SetUnion,
            "keywords.hierarchical" | "hierarchicalsubject" => {
                MetadataMergeStrategy::HierarchicalUnion
            }
            "caption" | "description" | "rights" | "copyright" => {
                MetadataMergeStrategy::LanguageSelection
            }
            "capture.datetime" | "datetimeoriginal" | "datecreated" => {
                MetadataMergeStrategy::DatePrecision
            }
            "rating" => MetadataMergeStrategy::RatingMapping,
            "label" | "colorlabel" | "color-label" => MetadataMergeStrategy::LabelMapping,
            _ => MetadataMergeStrategy::SourcePriority,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataAssertion {
    Value(MetadataRecord),
    Clear {
        key: MetadataKey,
        provenance: MetadataProvenance,
    },
}

impl MetadataAssertion {
    #[must_use]
    pub fn value(record: MetadataRecord) -> Self {
        Self::Value(record)
    }

    #[must_use]
    pub fn clear(key: MetadataKey, provenance: MetadataProvenance) -> Self {
        Self::Clear { key, provenance }
    }

    #[must_use]
    pub fn key(&self) -> &MetadataKey {
        match self {
            Self::Value(record) => record.key(),
            Self::Clear { key, .. } => key,
        }
    }
}

impl From<MetadataRecord> for MetadataAssertion {
    fn from(record: MetadataRecord) -> Self {
        Self::Value(record)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DecisionRule {
    SourcePriority,
    SetUnion,
    HierarchicalUnion,
    LanguagePreference,
    DatePrecision,
    RatingMapping,
    LabelMapping,
    ClearRevealsLowerPriority,
    InvalidValueIgnored,
    NoValidValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EvidenceDisposition {
    Selected,
    Merged,
    LowerPriority,
    Cleared,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiptValue {
    Public(DomainValue),
    Sha256([u8; 32]),
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionEvidence {
    source: MetadataSource,
    source_class: MetadataSourceClass,
    privacy: PrivacyClass,
    value: ReceiptValue,
    disposition: EvidenceDisposition,
}

impl DecisionEvidence {
    #[must_use]
    pub const fn source(&self) -> MetadataSource {
        self.source
    }

    #[must_use]
    pub const fn source_class(&self) -> MetadataSourceClass {
        self.source_class
    }

    #[must_use]
    pub const fn privacy(&self) -> PrivacyClass {
        self.privacy
    }

    #[must_use]
    pub const fn value(&self) -> &ReceiptValue {
        &self.value
    }

    #[must_use]
    pub const fn disposition(&self) -> EvidenceDisposition {
        self.disposition
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDecision {
    key: MetadataKey,
    rules: Vec<DecisionRule>,
    effective_source: Option<MetadataSource>,
    conflict: bool,
    evidence: Vec<DecisionEvidence>,
}

impl FieldDecision {
    #[must_use]
    pub const fn key(&self) -> &MetadataKey {
        &self.key
    }

    #[must_use]
    pub fn rules(&self) -> &[DecisionRule] {
        &self.rules
    }

    #[must_use]
    pub const fn effective_source(&self) -> Option<MetadataSource> {
        self.effective_source
    }

    #[must_use]
    pub const fn is_conflict(&self) -> bool {
        self.conflict
    }

    #[must_use]
    pub fn evidence(&self) -> &[DecisionEvidence] {
        &self.evidence
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MetadataResolutionReceipt {
    decisions: Vec<FieldDecision>,
}

impl MetadataResolutionReceipt {
    #[must_use]
    pub fn decisions(&self) -> impl ExactSizeIterator<Item = &FieldDecision> {
        self.decisions.iter()
    }

    pub fn conflicts(&self) -> impl Iterator<Item = &FieldDecision> {
        self.decisions.iter().filter(|decision| decision.conflict)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedMetadata {
    effective: MetadataDocument,
    retained: BTreeMap<MetadataKey, Vec<MetadataRecord>>,
    receipt: MetadataResolutionReceipt,
}

impl ResolvedMetadata {
    #[must_use]
    pub const fn effective(&self) -> &MetadataDocument {
        &self.effective
    }

    #[must_use]
    pub fn retained(&self, key: &MetadataKey) -> &[MetadataRecord] {
        self.retained.get(key).map_or(&[], Vec::as_slice)
    }

    #[must_use]
    pub const fn receipt(&self) -> &MetadataResolutionReceipt {
        &self.receipt
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataResolutionError {
    TooManyAssertions,
    Domain(MetadataDomainError),
}

impl fmt::Display for MetadataResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "metadata resolution failed: {self:?}")
    }
}

impl std::error::Error for MetadataResolutionError {}

impl From<MetadataDomainError> for MetadataResolutionError {
    fn from(error: MetadataDomainError) -> Self {
        Self::Domain(error)
    }
}

#[derive(Debug, Clone)]
pub struct MetadataResolver {
    policy: MetadataPrecedencePolicy,
}

impl Default for MetadataResolver {
    fn default() -> Self {
        Self::new(MetadataPrecedencePolicy::default())
    }
}

impl MetadataResolver {
    #[must_use]
    pub const fn new(policy: MetadataPrecedencePolicy) -> Self {
        Self { policy }
    }

    /// Resolves canonical records into one effective document and an auditable receipt.
    ///
    /// Clear assertions remove only their own logical source layer. This makes removing a
    /// user or export override reveal the next valid catalog or extracted value.
    ///
    /// # Errors
    ///
    /// Returns an error when the assertion bound or canonical effective-document bounds are
    /// exceeded. Invalid individual source values are retained in the receipt as rejected
    /// evidence and do not prevent a lower-priority valid value from being selected.
    pub fn resolve<I, A>(&self, assertions: I) -> Result<ResolvedMetadata, MetadataResolutionError>
    where
        I: IntoIterator<Item = A>,
        A: Into<MetadataAssertion>,
    {
        let assertions = assertions
            .into_iter()
            .map(Into::into)
            .collect::<Vec<MetadataAssertion>>();
        if assertions.len() > MAX_ASSERTIONS {
            return Err(MetadataResolutionError::TooManyAssertions);
        }

        let mut grouped = BTreeMap::<MetadataKey, Vec<MetadataAssertion>>::new();
        for assertion in assertions {
            grouped
                .entry(assertion.key().clone())
                .or_default()
                .push(assertion);
        }

        let mut effective = Vec::new();
        let mut retained = BTreeMap::new();
        let mut decisions = Vec::with_capacity(grouped.len());
        for (key, assertions) in grouped {
            let resolution = self.resolve_field(&key, assertions);
            if !resolution.retained.is_empty() {
                retained.insert(key.clone(), resolution.retained);
            }
            if let Some(record) = resolution.effective {
                effective.push(record);
            }
            decisions.push(resolution.decision);
        }

        Ok(ResolvedMetadata {
            effective: MetadataDocument::from_records(effective)?,
            retained,
            receipt: MetadataResolutionReceipt { decisions },
        })
    }

    fn resolve_field(
        &self,
        key: &MetadataKey,
        assertions: Vec<MetadataAssertion>,
    ) -> FieldResolution {
        let strategy = self.policy.strategy(key);
        let (mut records, mut invalid, mut clears) = partition_assertions(assertions);
        records.sort_by(compare_records);
        let retained = records.clone();
        clears.sort_by_key(|provenance| {
            (
                provenance.source().precedence(),
                provenance.source(),
                provenance.confidence(),
            )
        });
        clears.dedup_by(|left, right| left.source() == right.source());

        let mut active = Vec::new();
        let mut cleared = Vec::new();
        for record in records {
            if clears
                .iter()
                .any(|clear| same_logical_source(clear.source(), record.provenance().source()))
            {
                cleared.push(record);
            } else {
                active.push(record);
            }
        }

        let candidates = canonical_candidates(strategy, active);
        invalid.extend(
            candidates
                .rejected
                .iter()
                .map(|record| evidence_for_record(record, EvidenceDisposition::Invalid)),
        );
        let selected = select_effective(strategy, &self.policy, candidates.accepted);
        let conflict = has_conflict(&selected, &cleared, &invalid, &clears);

        let mut rules = vec![strategy_rule(strategy)];
        if !clears.is_empty() {
            rules.push(DecisionRule::ClearRevealsLowerPriority);
        }
        if !invalid.is_empty() {
            rules.push(DecisionRule::InvalidValueIgnored);
        }
        if selected.effective.is_none() {
            rules.push(DecisionRule::NoValidValue);
        }

        let selected_digest = selected
            .effective
            .as_ref()
            .map(|record| value_digest(record.value()));
        let merged = matches!(
            strategy,
            MetadataMergeStrategy::SetUnion | MetadataMergeStrategy::HierarchicalUnion
        );
        let mut evidence = selected
            .considered
            .iter()
            .map(|record| {
                let disposition = if merged {
                    EvidenceDisposition::Merged
                } else if selected_digest == Some(value_digest(record.value()))
                    && selected.effective.as_ref().is_some_and(|winner| {
                        winner.provenance().source() == record.provenance().source()
                    })
                {
                    EvidenceDisposition::Selected
                } else {
                    EvidenceDisposition::LowerPriority
                };
                evidence_for_record(record, disposition)
            })
            .chain(
                cleared
                    .iter()
                    .map(|record| evidence_for_record(record, EvidenceDisposition::Cleared)),
            )
            .chain(
                clears
                    .iter()
                    .map(|clear| evidence_for_clear(clear, EvidenceDisposition::Cleared)),
            )
            .chain(invalid)
            .collect::<Vec<_>>();
        evidence.sort_by(compare_evidence);
        evidence.dedup();

        let effective_source = selected
            .effective
            .as_ref()
            .map(|record| record.provenance().source());
        FieldResolution {
            effective: selected.effective,
            retained,
            decision: FieldDecision {
                key: key.clone(),
                rules,
                effective_source,
                conflict,
                evidence,
            },
        }
    }
}

struct FieldResolution {
    effective: Option<MetadataRecord>,
    retained: Vec<MetadataRecord>,
    decision: FieldDecision,
}

struct CanonicalCandidates {
    accepted: Vec<MetadataRecord>,
    rejected: Vec<MetadataRecord>,
}

struct Selection {
    effective: Option<MetadataRecord>,
    considered: Vec<MetadataRecord>,
}

fn has_conflict(
    selected: &Selection,
    cleared: &[MetadataRecord],
    invalid: &[DecisionEvidence],
    clears: &[MetadataProvenance],
) -> bool {
    let distinct_values = selected
        .considered
        .iter()
        .map(|record| value_digest(record.value()))
        .collect::<BTreeSet<_>>();
    distinct_values.len() > 1 || !cleared.is_empty() || !invalid.is_empty() || !clears.is_empty()
}

fn partition_assertions(
    assertions: Vec<MetadataAssertion>,
) -> (
    Vec<MetadataRecord>,
    Vec<DecisionEvidence>,
    Vec<MetadataProvenance>,
) {
    let mut records = Vec::new();
    let mut invalid = Vec::new();
    let mut clears = Vec::new();
    for assertion in assertions {
        match assertion {
            MetadataAssertion::Value(record) => match normalize_record(record.clone()) {
                Ok(record) => records.push(record),
                Err(()) => {
                    invalid.push(evidence_for_record(&record, EvidenceDisposition::Invalid));
                }
            },
            MetadataAssertion::Clear { key, provenance } => {
                if valid_provenance(&key, &provenance) {
                    clears.push(provenance);
                } else {
                    invalid.push(evidence_for_clear(
                        &provenance,
                        EvidenceDisposition::Invalid,
                    ));
                }
            }
        }
    }
    (records, invalid, clears)
}

fn normalize_record(record: MetadataRecord) -> Result<MetadataRecord, ()> {
    MetadataDocument::from_records([record])
        .ok()
        .and_then(|document| document.records().next().cloned())
        .ok_or(())
}

fn valid_provenance(key: &MetadataKey, provenance: &MetadataProvenance) -> bool {
    normalize_record(MetadataRecord::new(
        key.clone(),
        DomainValue::Boolean(false),
        provenance.clone(),
    ))
    .is_ok()
}

fn canonical_candidates(
    strategy: MetadataMergeStrategy,
    records: Vec<MetadataRecord>,
) -> CanonicalCandidates {
    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    for record in records {
        let value = canonical_value(strategy, record.value());
        if let Some(value) = value {
            accepted.push(MetadataRecord::new(
                record.key().clone(),
                value,
                record.provenance().clone(),
            ));
        } else {
            rejected.push(record);
        }
    }
    CanonicalCandidates { accepted, rejected }
}

fn canonical_value(strategy: MetadataMergeStrategy, value: &DomainValue) -> Option<DomainValue> {
    match strategy {
        MetadataMergeStrategy::SourcePriority => Some(value.clone()),
        MetadataMergeStrategy::SetUnion => match value {
            DomainValue::List(_) | DomainValue::Text(_) => Some(value.clone()),
            _ => None,
        },
        MetadataMergeStrategy::HierarchicalUnion => {
            matches!(value, DomainValue::Keywords(_)).then(|| value.clone())
        }
        MetadataMergeStrategy::LanguageSelection => matches!(
            value,
            DomainValue::Text(_) | DomainValue::LanguageAlternative(_)
        )
        .then(|| value.clone()),
        MetadataMergeStrategy::DatePrecision => {
            matches!(value, DomainValue::DateTime(_)).then(|| value.clone())
        }
        MetadataMergeStrategy::RatingMapping => canonical_rating(value),
        MetadataMergeStrategy::LabelMapping => canonical_label(value),
    }
}

fn canonical_rating(value: &DomainValue) -> Option<DomainValue> {
    let rating = match value {
        DomainValue::Unsigned(value) => u8::try_from(*value).ok(),
        DomainValue::Integer(value) => u8::try_from(*value).ok(),
        DomainValue::Text(value) => match value.trim().to_ascii_lowercase().as_str() {
            "reject" | "rejected" | "unrated" => Some(0),
            stars if !stars.is_empty() && stars.bytes().all(|byte| byte == b'*') => {
                u8::try_from(stars.len()).ok()
            }
            number => number.parse::<u8>().ok(),
        },
        _ => None,
    }?;
    (rating <= 5).then_some(DomainValue::Unsigned(u64::from(rating)))
}

fn canonical_label(value: &DomainValue) -> Option<DomainValue> {
    let label = match value {
        DomainValue::Unsigned(value) => match value {
            0 => "none",
            1 => "red",
            2 => "yellow",
            3 => "green",
            4 => "blue",
            5 => "purple",
            _ => return None,
        },
        DomainValue::Integer(value) => match value {
            0 => "none",
            1 => "red",
            2 => "yellow",
            3 => "green",
            4 => "blue",
            5 => "purple",
            _ => return None,
        },
        DomainValue::Text(value) => match value.trim().to_ascii_lowercase().as_str() {
            "none" | "red" | "yellow" | "green" | "blue" | "purple" => {
                return Some(DomainValue::Text(value.trim().to_ascii_lowercase()));
            }
            _ => return None,
        },
        _ => return None,
    };
    Some(DomainValue::Text(label.to_owned()))
}

fn select_effective(
    strategy: MetadataMergeStrategy,
    policy: &MetadataPrecedencePolicy,
    records: Vec<MetadataRecord>,
) -> Selection {
    match strategy {
        MetadataMergeStrategy::SetUnion => select_set_union(records),
        MetadataMergeStrategy::HierarchicalUnion => select_hierarchical_union(records),
        MetadataMergeStrategy::LanguageSelection => select_language(policy, records),
        MetadataMergeStrategy::DatePrecision => select_date(records),
        MetadataMergeStrategy::SourcePriority
        | MetadataMergeStrategy::RatingMapping
        | MetadataMergeStrategy::LabelMapping => select_priority(records),
    }
}

fn select_priority(mut records: Vec<MetadataRecord>) -> Selection {
    records.sort_by(compare_records);
    let effective = records.last().cloned();
    Selection {
        effective,
        considered: records,
    }
}

fn select_date(mut records: Vec<MetadataRecord>) -> Selection {
    records.sort_by(|left, right| {
        let left_precision = date_precision(left.value());
        let right_precision = date_precision(right.value());
        (
            left.provenance().source().precedence(),
            left_precision,
            left.provenance().confidence(),
            left.provenance().source(),
            value_digest(left.value()),
        )
            .cmp(&(
                right.provenance().source().precedence(),
                right_precision,
                right.provenance().confidence(),
                right.provenance().source(),
                value_digest(right.value()),
            ))
    });
    let effective = records.last().cloned();
    Selection {
        effective,
        considered: records,
    }
}

fn date_precision(value: &DomainValue) -> DatePrecision {
    match value {
        DomainValue::DateTime(value) => value.precision(),
        _ => unreachable!("date candidates are canonicalized"),
    }
}

fn select_set_union(mut records: Vec<MetadataRecord>) -> Selection {
    records.sort_by(compare_records);
    let Some(provenance) = records.last().map(|record| record.provenance().clone()) else {
        return Selection {
            effective: None,
            considered: records,
        };
    };
    let key = records
        .last()
        .expect("nonempty records have a key")
        .key()
        .clone();
    let mut values = records
        .iter()
        .flat_map(|record| match record.value() {
            DomainValue::List(values) => values.clone(),
            value => vec![value.clone()],
        })
        .collect::<Vec<_>>();
    sort_and_dedup_values(&mut values);
    Selection {
        effective: Some(MetadataRecord::new(
            key,
            DomainValue::List(values),
            provenance,
        )),
        considered: records,
    }
}

fn select_hierarchical_union(mut records: Vec<MetadataRecord>) -> Selection {
    records.sort_by(compare_records);
    let Some(provenance) = records.last().map(|record| record.provenance().clone()) else {
        return Selection {
            effective: None,
            considered: records,
        };
    };
    let key = records
        .last()
        .expect("nonempty records have a key")
        .key()
        .clone();
    let paths = records
        .iter()
        .flat_map(|record| match record.value() {
            DomainValue::Keywords(keywords) => keywords.paths().to_vec(),
            _ => unreachable!("hierarchical candidates are canonicalized"),
        })
        .collect();
    let effective = HierarchicalKeywords::new(paths)
        .ok()
        .map(|keywords| MetadataRecord::new(key, DomainValue::Keywords(keywords), provenance));
    Selection {
        effective,
        considered: records,
    }
}

fn select_language(
    policy: &MetadataPrecedencePolicy,
    mut records: Vec<MetadataRecord>,
) -> Selection {
    records.sort_by(compare_records);
    let mut alternatives = Vec::<(LanguageTag, String, MetadataRecord)>::new();
    for record in &records {
        match record.value() {
            DomainValue::Text(value) => alternatives.push((
                LanguageTag::new("x-default").expect("static language tag"),
                value.clone(),
                record.clone(),
            )),
            DomainValue::LanguageAlternative(values) => {
                alternatives.extend(
                    values.iter().map(|(language, value)| {
                        (language.clone(), value.to_owned(), record.clone())
                    }),
                );
            }
            _ => unreachable!("language candidates are canonicalized"),
        }
    }

    let selected = policy
        .preferred_languages()
        .iter()
        .find_map(|preferred| {
            best_language_match(
                alternatives
                    .iter()
                    .filter(|(language, _, _)| language == preferred),
            )
        })
        .or_else(|| {
            alternatives.sort_by(|left, right| {
                left.0
                    .cmp(&right.0)
                    .then_with(|| compare_records(&left.2, &right.2))
                    .then_with(|| left.1.cmp(&right.1))
            });
            alternatives.last().cloned()
        });
    let effective = selected.map(|(_, value, record)| {
        MetadataRecord::new(
            record.key().clone(),
            DomainValue::Text(value),
            record.provenance().clone(),
        )
    });
    Selection {
        effective,
        considered: records,
    }
}

fn best_language_match<'a>(
    alternatives: impl Iterator<Item = &'a (LanguageTag, String, MetadataRecord)>,
) -> Option<(LanguageTag, String, MetadataRecord)> {
    alternatives
        .cloned()
        .max_by(|left, right| compare_records(&left.2, &right.2).then_with(|| left.1.cmp(&right.1)))
}

fn compare_records(left: &MetadataRecord, right: &MetadataRecord) -> std::cmp::Ordering {
    (
        left.provenance().source().precedence(),
        left.provenance().confidence(),
        left.provenance().source(),
        value_digest(left.value()),
    )
        .cmp(&(
            right.provenance().source().precedence(),
            right.provenance().confidence(),
            right.provenance().source(),
            value_digest(right.value()),
        ))
}

fn same_logical_source(clear: MetadataSource, value: MetadataSource) -> bool {
    clear == value
        || matches!(
            (clear, value),
            (MetadataSource::Xmp, MetadataSource::EmbeddedXmp)
                | (MetadataSource::EmbeddedXmp, MetadataSource::Xmp)
                | (MetadataSource::Imported, MetadataSource::ImportDefault)
                | (MetadataSource::ImportDefault, MetadataSource::Imported)
                | (MetadataSource::CatalogEdit, MetadataSource::UserOverride)
                | (MetadataSource::UserOverride, MetadataSource::CatalogEdit)
                | (
                    MetadataSource::RecipeOverride,
                    MetadataSource::ExportOverride
                )
                | (
                    MetadataSource::ExportOverride,
                    MetadataSource::RecipeOverride
                )
        )
}

fn strategy_rule(strategy: MetadataMergeStrategy) -> DecisionRule {
    match strategy {
        MetadataMergeStrategy::SourcePriority => DecisionRule::SourcePriority,
        MetadataMergeStrategy::SetUnion => DecisionRule::SetUnion,
        MetadataMergeStrategy::HierarchicalUnion => DecisionRule::HierarchicalUnion,
        MetadataMergeStrategy::LanguageSelection => DecisionRule::LanguagePreference,
        MetadataMergeStrategy::DatePrecision => DecisionRule::DatePrecision,
        MetadataMergeStrategy::RatingMapping => DecisionRule::RatingMapping,
        MetadataMergeStrategy::LabelMapping => DecisionRule::LabelMapping,
    }
}

fn sort_and_dedup_values(values: &mut Vec<DomainValue>) {
    values.sort_by_key(|value| (value_digest(value), format!("{value:?}")));
    values.dedup();
}

fn evidence_for_record(
    record: &MetadataRecord,
    disposition: EvidenceDisposition,
) -> DecisionEvidence {
    DecisionEvidence {
        source: record.provenance().source(),
        source_class: record.provenance().source().class(),
        privacy: record.provenance().privacy(),
        value: receipt_value(record.value(), record.provenance().privacy()),
        disposition,
    }
}

fn evidence_for_clear(
    provenance: &MetadataProvenance,
    disposition: EvidenceDisposition,
) -> DecisionEvidence {
    DecisionEvidence {
        source: provenance.source(),
        source_class: provenance.source().class(),
        privacy: provenance.privacy(),
        value: ReceiptValue::Clear,
        disposition,
    }
}

fn receipt_value(value: &DomainValue, privacy: PrivacyClass) -> ReceiptValue {
    if privacy == PrivacyClass::Public {
        ReceiptValue::Public(value.clone())
    } else {
        ReceiptValue::Sha256(value_digest(value))
    }
}

fn value_digest(value: &DomainValue) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"RustTable metadata receipt value v1\0");
    digest.update(format!("{value:?}").as_bytes());
    digest.finalize().into()
}

fn compare_evidence(left: &DecisionEvidence, right: &DecisionEvidence) -> std::cmp::Ordering {
    (
        left.source.precedence(),
        left.source,
        left.disposition,
        receipt_digest(&left.value),
    )
        .cmp(&(
            right.source.precedence(),
            right.source,
            right.disposition,
            receipt_digest(&right.value),
        ))
}

fn receipt_digest(value: &ReceiptValue) -> [u8; 32] {
    match value {
        ReceiptValue::Public(value) => value_digest(value),
        ReceiptValue::Sha256(value) => *value,
        ReceiptValue::Clear => [0; 32],
    }
}
