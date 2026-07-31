//! Lossless Darktable collection configuration codecs.
//!
//! Direct source mapping: `src/common/collection.c` functions
//! `dt_collection_serialize`, `dt_collection_deserialize`,
//! `dt_collection_sort_serialize`, `dt_collection_sort_deserialize`, and
//! `dt_collection_checksum`. This leaf does not update configuration or rebuild
//! an application query.

use md5::Context;
use rusttable_catalog::{
    NativeCollectionRule, NativeCollectionRules, NativeCollectionSortRule, NativeCollectionSorts,
};

const MAX_SERIALIZED_STRING_BYTES: usize = 399;

/// Serializes collect or filtering rules using Darktable's delimiter format.
#[must_use]
pub fn encode_rules(rules: &NativeCollectionRules) -> Vec<u8> {
    let mut encoded = rules.num_rules().to_string().into_bytes();
    encoded.push(b':');
    let count = usize::try_from(rules.num_rules().max(0)).unwrap_or_default();
    for rule in rules.rules().iter().take(count) {
        encoded.extend_from_slice(rule.mode().to_string().as_bytes());
        encoded.push(b':');
        encoded.extend_from_slice(rule.item().to_string().as_bytes());
        encoded.push(b':');
        if rules.filtering_mode() {
            encoded.extend_from_slice(rule.off().to_string().as_bytes());
            encoded.push(b':');
            encoded.extend_from_slice(rule.top().to_string().as_bytes());
            encoded.push(b':');
        }
        if rule.value().is_empty() {
            encoded.extend_from_slice(b"%$");
        } else {
            encoded.extend_from_slice(rule.value());
            encoded.push(b'$');
        }
    }
    encoded
}

/// Deserializes collect or filtering rules, truncating at the first malformed record.
#[must_use]
pub fn decode_rules(filtering: bool, bytes: &[u8]) -> NativeCollectionRules {
    let mut cursor = 0_usize;
    let declared = parse_integer(bytes, &mut cursor).unwrap_or(0);
    while cursor < bytes.len() && bytes[cursor] != b':' {
        cursor += 1;
    }
    if cursor < bytes.len() {
        cursor += 1;
    }

    if declared == 0 && !filtering {
        return NativeCollectionRules::from_parts(
            false,
            1,
            vec![NativeCollectionRule::collect(0, 0, b"%".to_vec())],
        )
        .expect("native collect default is a valid rule prefix");
    }

    let mut rules = Vec::new();
    let mut parsed_count = declared;
    for index in 0..declared {
        let Some(rule) = parse_rule(filtering, bytes, &mut cursor) else {
            parsed_count = index;
            break;
        };
        rules.push(rule);
        skip_to_delimiter(bytes, &mut cursor);
    }

    if !filtering && declared == 1 && rules.is_empty() && parsed_count == 0 {
        return NativeCollectionRules::from_parts(
            false,
            1,
            vec![NativeCollectionRule::collect(0, 0, b"%".to_vec())],
        )
        .expect("native collect default is a valid rule prefix");
    }

    NativeCollectionRules::from_parts(filtering, parsed_count, rules)
        .expect("codec parser only creates valid rule prefixes")
}

/// Serializes native sort identifiers and directions in source order.
#[must_use]
pub fn encode_sorts(sorts: &NativeCollectionSorts) -> Vec<u8> {
    let mut encoded = sorts.num_sort().to_string().into_bytes();
    encoded.push(b':');
    let count = usize::try_from(sorts.num_sort().max(0)).unwrap_or_default();
    for rule in sorts.rules().iter().take(count) {
        encoded.extend_from_slice(rule.sort_id().to_string().as_bytes());
        encoded.push(b':');
        encoded.extend_from_slice(rule.sort_order().to_string().as_bytes());
        encoded.push(b'$');
    }
    encoded
}

/// Deserializes native sort identifiers, truncating at the first malformed entry.
#[must_use]
pub fn decode_sorts(bytes: &[u8]) -> NativeCollectionSorts {
    let mut cursor = 0_usize;
    let declared = parse_integer(bytes, &mut cursor).unwrap_or(0);
    while cursor < bytes.len() && bytes[cursor] != b':' {
        cursor += 1;
    }
    if cursor < bytes.len() {
        cursor += 1;
    }

    let mut rules = Vec::new();
    let mut parsed_count = declared;
    for index in 0..declared {
        let Some(sort_id) = parse_integer_then_colon(bytes, &mut cursor) else {
            parsed_count = index;
            break;
        };
        let Some(sort_order) = parse_integer(bytes, &mut cursor) else {
            parsed_count = index;
            break;
        };
        rules.push(NativeCollectionSortRule::new(sort_id, sort_order));
        skip_to_delimiter(bytes, &mut cursor);
    }

    NativeCollectionSorts::from_parts(parsed_count, rules)
        .expect("codec parser only creates valid sort prefixes")
}

/// Computes Darktable's native-endian MD5 checksum for a rule set.
#[must_use]
pub fn checksum(rules: &NativeCollectionRules) -> [u8; 16] {
    let mut digest = Context::new();
    digest.consume(rules.num_rules().to_ne_bytes());
    let count = usize::try_from(rules.num_rules().max(0)).unwrap_or_default();
    for rule in rules.rules().iter().take(count) {
        digest.consume(rule.mode().to_ne_bytes());
        digest.consume(rule.item().to_ne_bytes());
        if rules.filtering_mode() {
            digest.consume(rule.off().to_ne_bytes());
            digest.consume(rule.top().to_ne_bytes());
        }
        digest.consume(rule.value());
    }
    digest.finalize().0
}

/// Returns the lowercase hexadecimal form of [`checksum`].
#[must_use]
pub fn checksum_hex(rules: &NativeCollectionRules) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = checksum(rules);
    let mut output = String::with_capacity(32);
    for byte in digest {
        output.push(HEX[usize::from(byte >> 4)] as char);
        output.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    output
}

fn parse_rule(filtering: bool, bytes: &[u8], cursor: &mut usize) -> Option<NativeCollectionRule> {
    let mode = parse_integer_then_colon(bytes, cursor)?;
    let item = parse_integer_then_colon(bytes, cursor)?;
    if filtering {
        let off = parse_integer_then_colon(bytes, cursor)?;
        let top = parse_integer_then_colon(bytes, cursor)?;
        let value = parse_value(bytes, cursor)?;
        Some(NativeCollectionRule::filtering(mode, item, off, top, value))
    } else {
        let value = parse_value(bytes, cursor)?;
        Some(NativeCollectionRule::collect(mode, item, value))
    }
}

fn parse_integer_then_colon(bytes: &[u8], cursor: &mut usize) -> Option<i32> {
    let value = parse_integer(bytes, cursor)?;
    if bytes.get(*cursor) != Some(&b':') {
        return None;
    }
    *cursor += 1;
    Some(value)
}

fn parse_integer(bytes: &[u8], cursor: &mut usize) -> Option<i32> {
    while bytes.get(*cursor).is_some_and(u8::is_ascii_whitespace) {
        *cursor += 1;
    }
    let start = *cursor;
    if matches!(bytes.get(*cursor), Some(b'+' | b'-')) {
        *cursor += 1;
    }
    let digits = *cursor;
    while bytes.get(*cursor).is_some_and(u8::is_ascii_digit) {
        *cursor += 1;
    }
    if *cursor == digits {
        *cursor = start;
        return None;
    }
    std::str::from_utf8(&bytes[start..*cursor])
        .ok()?
        .parse::<i32>()
        .ok()
}

fn parse_value(bytes: &[u8], cursor: &mut usize) -> Option<Vec<u8>> {
    let start = *cursor;
    while bytes.get(*cursor).is_some_and(|byte| *byte != b'$') {
        *cursor += 1;
    }
    if start == *cursor {
        return None;
    }
    let end = start
        .saturating_add(MAX_SERIALIZED_STRING_BYTES)
        .min(*cursor);
    Some(bytes[start..end].to_vec())
}

fn skip_to_delimiter(bytes: &[u8], cursor: &mut usize) {
    while bytes.get(*cursor).is_some_and(|byte| *byte != b'$') {
        *cursor += 1;
    }
    if bytes.get(*cursor) == Some(&b'$') {
        *cursor += 1;
    }
}
