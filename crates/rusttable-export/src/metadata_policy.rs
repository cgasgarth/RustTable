// Source lineage: src/common/metadata_export.h and src/common/metadata_export.c.
// This is the native configuration codec only. EXIF/IPTC/XMP and tag inclusion
// remain separate retained-native serializer responsibilities.

pub const NATIVE_METADATA_FLAGS_KEY: &str = "plugins/lighttable/export/metadata_flags";
pub const NATIVE_METADATA_FORMULA_KEY: &str = "plugins/lighttable/export/metadata_formula";

/// The inclusion bits from Darktable's `dt_metadata_id` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct NativeMetadataFlags(u32);

impl NativeMetadataFlags {
    pub const NONE: Self = Self(0);
    pub const EXIF: Self = Self(1 << 0);
    pub const METADATA: Self = Self(1 << 1);
    pub const GEOTAG: Self = Self(1 << 2);
    pub const TAG: Self = Self(1 << 3);
    pub const HIERARCHICAL_TAG: Self = Self(1 << 4);
    pub const DT_HISTORY: Self = Self(1 << 5);
    pub const PRIVATE_TAG: Self = Self(1 << 16);
    pub const SYNONYMS_TAG: Self = Self(1 << 17);
    pub const OMIT_HIERARCHY: Self = Self(1 << 18);
    pub const CALCULATED: Self = Self(1 << 19);

    /// Returns the native default (`DT_META_EXIF | ... | DT_META_DT_HISTORY`).
    #[must_use]
    pub const fn default_flags() -> Self {
        Self(Self::EXIF.0 | Self::METADATA.0 | Self::GEOTAG.0 | Self::TAG.0 | Self::DT_HISTORY.0)
    }

    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Returns whether a non-empty inclusion bit is enabled.
    #[must_use]
    pub const fn enabled(self, flag: Self) -> bool {
        flag.0 != 0 && (self.0 & flag.0) != 0
    }

    /// Parses the native hexadecimal `strtol` result and its signed-to-`u32` cast.
    #[must_use]
    pub fn from_hex_text(raw: &str) -> Self {
        let parsed = native_strtol_hex(raw);
        // `metadata_export.c` assigns the platform `long` to int32_t and then
        // returns it as uint32_t. Reducing modulo 2^32 preserves that native
        // two's-complement narrowing without a second interpretation of bits.
        let bits = u32::try_from(parsed.rem_euclid(1_i64 << 32)).unwrap_or_default();
        Self(bits)
    }
}

#[cfg(target_pointer_width = "32")]
const NATIVE_LONG_MAX: i64 = i32::MAX as i64;
#[cfg(target_pointer_width = "32")]
const NATIVE_LONG_MIN: i64 = i32::MIN as i64;
#[cfg(target_pointer_width = "32")]
const NATIVE_LONG_NEGATIVE_LIMIT: u64 = 1_u64 << 31;

#[cfg(target_pointer_width = "64")]
const NATIVE_LONG_MAX: i64 = i64::MAX;
#[cfg(target_pointer_width = "64")]
const NATIVE_LONG_MIN: i64 = i64::MIN;
#[cfg(target_pointer_width = "64")]
const NATIVE_LONG_NEGATIVE_LIMIT: u64 = 1_u64 << 63;

fn native_strtol_hex(raw: &str) -> i64 {
    let bytes = raw.as_bytes();
    let mut index = 0;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }

    let negative = match bytes.get(index) {
        Some(b'-') => {
            index += 1;
            true
        }
        Some(b'+') => {
            index += 1;
            false
        }
        _ => false,
    };

    if bytes.get(index) == Some(&b'0') && matches!(bytes.get(index + 1), Some(b'x' | b'X')) {
        index += 2;
    }

    let limit = if negative {
        NATIVE_LONG_NEGATIVE_LIMIT
    } else {
        NATIVE_LONG_MAX as u64
    };
    let mut value = 0_u64;
    let mut converted = false;

    while let Some(&byte) = bytes.get(index) {
        let Some(digit) = hex_digit(byte) else {
            break;
        };
        converted = true;
        let digit = u64::from(digit);
        if value > (limit - digit) / 16 {
            return if negative {
                NATIVE_LONG_MIN
            } else {
                NATIVE_LONG_MAX
            };
        }
        value = value * 16 + digit;
        index += 1;
    }

    if !converted {
        return 0;
    }
    if negative {
        if value == 1_u64 << 63 {
            i64::MIN
        } else {
            -i64::try_from(value).expect("negative value is within i64 range")
        }
    } else {
        i64::try_from(value).expect("positive value is within i64 range")
    }
}

const fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeMetadataExportConfig {
    flags: Option<String>,
    formula_slots: Vec<Option<String>>,
}

impl NativeMetadataExportConfig {
    /// Builds a configuration from key presence and contiguous formula slots.
    #[must_use]
    pub fn from_keys(flags: Option<&str>, formula_slots: &[Option<&str>]) -> Self {
        Self {
            flags: flags.map(str::to_owned),
            formula_slots: formula_slots
                .iter()
                .map(|slot| slot.map(str::to_owned))
                .collect(),
        }
    }

    /// Returns the native formula key for a zero-based formula slot.
    #[must_use]
    pub fn formula_key(index: usize) -> String {
        format!("{NATIVE_METADATA_FORMULA_KEY}{index}")
    }

    #[must_use]
    pub const fn flags_key_exists(&self) -> bool {
        self.flags.is_some()
    }

    #[must_use]
    pub fn raw_flags(&self) -> Option<&str> {
        self.flags.as_deref()
    }

    #[must_use]
    pub fn formula_slots(&self) -> &[Option<String>] {
        &self.formula_slots
    }

    /// Returns a formula slot, preserving `Some("")` as an existing key.
    #[must_use]
    pub fn formula_slot(&self, index: usize) -> Option<&str> {
        self.formula_slots.get(index).and_then(Option::as_deref)
    }

    /// Sets or removes a formula key without compacting later key indices.
    pub fn set_formula_slot(&mut self, index: usize, value: Option<String>) {
        if self.formula_slots.len() <= index {
            self.formula_slots.resize_with(index + 1, || None);
        }
        self.formula_slots[index] = value;
    }

    /// Implements `dt_lib_export_metadata_get_conf_flags`.
    ///
    /// Native string lookup inserts an empty value for a missing key. This
    /// method intentionally mutates a missing flags key for the same reason.
    pub fn get_conf_flags(&mut self) -> NativeMetadataFlags {
        let raw = self.flags.get_or_insert_default();
        NativeMetadataFlags::from_hex_text(raw)
    }

    /// Implements `dt_lib_export_metadata_get_conf` without exporting metadata.
    #[must_use]
    pub fn get_conf(&self) -> String {
        let Some(flags) = self.flags.as_deref() else {
            return format!("{:x}", NativeMetadataFlags::default_flags().bits());
        };

        let mut presets = flags.to_owned();
        for slot in &self.formula_slots {
            let Some(nameformula) = slot.as_deref() else {
                break;
            };
            if nameformula.is_empty() {
                continue;
            }
            let Some((name, formula)) = nameformula.split_once(';') else {
                continue;
            };
            presets.push('\x01');
            presets.push_str(name);
            presets.push('\x01');
            presets.push_str(formula);
        }
        presets
    }

    /// Implements `dt_lib_export_metadata_set_conf` and stale-slot cleanup.
    pub fn set_conf(&mut self, metadata_presets: &str) {
        // The C API receives a NUL-terminated string.
        let metadata_presets = metadata_presets.split('\0').next().unwrap_or_default();
        let tokens: Vec<&str> = if metadata_presets.is_empty() {
            Vec::new()
        } else {
            metadata_presets.split('\x01').collect()
        };

        self.flags = Some(tokens.first().copied().unwrap_or_default().to_owned());
        let pairs = tokens.get(1..).map_or(0, |rest| rest.len() / 2);
        if self.formula_slots.len() < pairs {
            self.formula_slots.resize_with(pairs, || None);
        }
        if let Some(rest) = tokens.get(1..) {
            for (index, pair) in rest.chunks(2).take(pairs).enumerate() {
                self.formula_slots[index] = Some(format!("{};{}", pair[0], pair[1]));
            }
        }

        // Native cleanup stops at the first missing key and writes empty
        // values rather than deleting stale formula keys.
        let mut index = pairs;
        while self.formula_slots.get(index).is_some_and(Option::is_some) {
            self.formula_slots[index] = Some(String::new());
            index += 1;
        }
    }
}

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

const fn action(value: MetadataAction, include: CanonicalAction) -> CanonicalAction {
    match value {
        MetadataAction::Include => include,
        MetadataAction::Exclude => CanonicalAction::Exclude,
        MetadataAction::Redact => CanonicalAction::Redact,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_flags_match_the_export_metadata_enum() {
        assert_eq!(NativeMetadataFlags::NONE.bits(), 0);
        assert_eq!(NativeMetadataFlags::EXIF.bits(), 1 << 0);
        assert_eq!(NativeMetadataFlags::METADATA.bits(), 1 << 1);
        assert_eq!(NativeMetadataFlags::GEOTAG.bits(), 1 << 2);
        assert_eq!(NativeMetadataFlags::TAG.bits(), 1 << 3);
        assert_eq!(NativeMetadataFlags::HIERARCHICAL_TAG.bits(), 1 << 4);
        assert_eq!(NativeMetadataFlags::DT_HISTORY.bits(), 1 << 5);
        assert_eq!(NativeMetadataFlags::PRIVATE_TAG.bits(), 1 << 16);
        assert_eq!(NativeMetadataFlags::SYNONYMS_TAG.bits(), 1 << 17);
        assert_eq!(NativeMetadataFlags::OMIT_HIERARCHY.bits(), 1 << 18);
        assert_eq!(NativeMetadataFlags::CALCULATED.bits(), 1 << 19);
        assert_eq!(NativeMetadataFlags::default_flags().bits(), 0x2f);
        assert!(NativeMetadataFlags::default_flags().enabled(NativeMetadataFlags::EXIF));
        assert!(!NativeMetadataFlags::NONE.enabled(NativeMetadataFlags::EXIF));
        assert_eq!(
            NativeMetadataExportConfig::formula_key(3),
            "plugins/lighttable/export/metadata_formula3"
        );
        assert_eq!(
            NATIVE_METADATA_FLAGS_KEY,
            "plugins/lighttable/export/metadata_flags"
        );
    }

    #[test]
    fn native_hex_parsing_keeps_strtol_and_cast_behavior() {
        let cases = [
            ("", 0),
            ("not hexadecimal", 0),
            ("  +0x2Fsuffix", 0x2f),
            ("0X2f", 0x2f),
            ("1_2", 1),
            ("-1", u32::MAX),
            ("-0x80000000", 0x8000_0000),
        ];

        for (raw, expected) in cases {
            assert_eq!(
                NativeMetadataFlags::from_hex_text(raw).bits(),
                expected,
                "{raw:?}"
            );
        }

        #[cfg(target_pointer_width = "64")]
        {
            let cases = [
                ("ffffffff", u32::MAX),
                ("0x100000000", 0),
                ("-0x100000000", 0),
                ("9223372036854775808", u32::MAX),
                ("-9223372036854775809", 0),
            ];
            for (raw, expected) in cases {
                assert_eq!(
                    NativeMetadataFlags::from_hex_text(raw).bits(),
                    expected,
                    "{raw:?}"
                );
            }
        }

        #[cfg(target_pointer_width = "32")]
        {
            let cases = [
                ("ffffffff", 0x7fff_ffff),
                ("0x100000000", 0x7fff_ffff),
                ("-0x100000000", 0x8000_0000),
                ("9223372036854775808", 0x7fff_ffff),
                ("-9223372036854775809", 0x8000_0000),
            ];
            for (raw, expected) in cases {
                assert_eq!(
                    NativeMetadataFlags::from_hex_text(raw).bits(),
                    expected,
                    "{raw:?}"
                );
            }
        }
    }

    #[test]
    fn missing_and_present_empty_flags_follow_native_key_presence() {
        let mut missing = NativeMetadataExportConfig::default();
        assert!(!missing.flags_key_exists());
        assert_eq!(missing.get_conf(), "2f");
        assert_eq!(missing.get_conf_flags().bits(), 0);
        assert!(missing.flags_key_exists());
        assert_eq!(missing.raw_flags(), Some(""));
        assert_eq!(missing.get_conf(), "");

        let present_empty = NativeMetadataExportConfig::from_keys(Some(""), &[]);
        assert_eq!(present_empty.get_conf(), "");
    }

    #[test]
    fn get_conf_preserves_soh_order_empty_tokens_and_first_semicolon() {
        let config = NativeMetadataExportConfig::from_keys(
            Some("2f"),
            &[
                Some("without-separator"),
                Some("name;formula;later"),
                Some(""),
                Some(";"),
                Some("name;"),
            ],
        );

        assert_eq!(
            config.get_conf(),
            "2f\x01name\x01formula;later\x01\x01\x01name\x01"
        );

        let gap = NativeMetadataExportConfig::from_keys(
            Some("2f"),
            &[Some("first;one"), None, Some("later;two")],
        );
        assert_eq!(gap.get_conf(), "2f\x01first\x01one");
    }

    #[test]
    fn set_conf_writes_pairs_and_cleans_contiguous_stale_slots() {
        let mut config = NativeMetadataExportConfig::from_keys(
            Some("old"),
            &[Some("old;formula"), Some("stale"), Some("also-stale")],
        );

        config.set_conf("0042\x01tag\x01a;b\x01odd");

        assert_eq!(config.raw_flags(), Some("0042"));
        assert_eq!(config.formula_slot(0), Some("tag;a;b"));
        assert_eq!(config.formula_slot(1), Some(""));
        assert_eq!(config.formula_slot(2), Some(""));
        assert_eq!(config.get_conf(), "0042\x01tag\x01a;b");
    }

    #[test]
    fn set_conf_stops_cleanup_at_a_missing_formula_key() {
        let mut config = NativeMetadataExportConfig::from_keys(
            Some("old"),
            &[Some("old;formula"), None, Some("later;formula")],
        );

        config.set_conf("new");

        assert_eq!(config.raw_flags(), Some("new"));
        assert_eq!(config.formula_slot(0), Some(""));
        assert_eq!(config.formula_slot(1), None);
        assert_eq!(config.formula_slot(2), Some("later;formula"));
    }

    #[test]
    fn set_conf_round_trips_raw_flags_and_formula_text() {
        let presets = "0042\x01name\x01formula;with;semis\x01\x01";
        let mut config = NativeMetadataExportConfig::default();

        config.set_conf(presets);

        assert_eq!(config.get_conf(), presets);
        assert_eq!(config.formula_slot(0), Some("name;formula;with;semis"));
        assert_eq!(config.formula_slot(1), Some(";"));
    }

    #[test]
    fn set_empty_conf_keeps_existing_slots_as_empty_values() {
        let mut config =
            NativeMetadataExportConfig::from_keys(Some("old"), &[Some("first;formula"), Some("")]);

        config.set_conf("");

        assert_eq!(config.raw_flags(), Some(""));
        assert_eq!(config.formula_slot(0), Some(""));
        assert_eq!(config.formula_slot(1), Some(""));
        assert_eq!(config.get_conf(), "");
    }
}
