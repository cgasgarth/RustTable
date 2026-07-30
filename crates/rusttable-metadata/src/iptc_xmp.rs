//! Bounded canonicalization for standalone IPTC-IIM and RDF/XMP packets.

use std::collections::{BTreeMap, BTreeSet};

use roxmltree::{Document, Node, ParsingOptions};

use crate::domain::{
    DomainValue, HierarchicalKeywords, LanguageAlternative, LanguageTag, MetadataDocument,
    MetadataKey, MetadataNamespace, MetadataProvenance, MetadataRecord, PrivacyClass,
    RawRepresentation, StructuredValue,
};
use crate::error::MetadataPacketError;
use crate::limits::MetadataPacketLimits;
use crate::policy::MetadataSource;

const RDF_URI: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const DC_URI: &str = "http://purl.org/dc/elements/1.1/";
const DC_TERMS_URI: &str = "http://purl.org/dc/terms/";
const PHOTOSHOP_URI: &str = "http://ns.adobe.com/photoshop/1.0/";
const XMP_URI: &str = "http://ns.adobe.com/xap/1.0/";
const LR_URI: &str = "http://ns.adobe.com/lightroom/1.0/";
const XML_URI: &str = "http://www.w3.org/XML/1998/namespace";

/// Bounded reader for an RDF/XMP packet.
#[derive(Debug, Clone, Copy)]
pub struct XmpMetadataInput {
    limits: MetadataPacketLimits,
}

impl XmpMetadataInput {
    #[must_use]
    pub const fn new(limits: MetadataPacketLimits) -> Self {
        Self { limits }
    }

    /// Extracts one bounded XMP packet and canonicalizes its RDF properties.
    ///
    /// DTDs, entity references, external resolvers, excessive nesting, and excessive
    /// collections are rejected before values enter the canonical domain.
    ///
    /// # Errors
    ///
    /// Returns [`MetadataPacketError`] when extraction, XML decoding, XML limits, or domain
    /// validation fails.
    pub fn read_domain(&self, source: &[u8]) -> Result<MetadataDocument, MetadataPacketError> {
        let raw_source_len =
            u64::try_from(source.len()).map_err(|_| MetadataPacketError::SourceTooLarge {
                limit: self.limits.source_bytes,
                actual: u64::MAX,
            })?;
        if raw_source_len > self.limits.source_bytes {
            return Err(MetadataPacketError::SourceTooLarge {
                limit: self.limits.source_bytes,
                actual: raw_source_len,
            });
        }
        let decoded = decode_xml(source)?;
        preflight_xml(decoded.as_bytes(), self.limits)?;
        let packet = extract_packet_text(&decoded, self.limits.packet_bytes)?;
        let packet_bytes = packet.as_bytes();
        preflight_xml(packet_bytes, self.limits)?;
        let document = Document::parse_with_options(
            packet,
            ParsingOptions {
                allow_dtd: false,
                nodes_limit: self.limits.xml_nodes,
                entity_resolver: None,
            },
        )
        .map_err(|error| match error {
            roxmltree::Error::NodesLimitReached => MetadataPacketError::XmlNodeLimit {
                limit: self.limits.xml_nodes,
            },
            _ => MetadataPacketError::MalformedXml,
        })?;

        let mut state = ParseState::new(self.limits);
        let mut values: BTreeMap<MetadataKey, Vec<DomainValue>> = BTreeMap::new();
        for description in document.descendants().filter(|node| {
            node.is_element()
                && node.tag_name().name() == "Description"
                && node.tag_name().namespace() == Some(RDF_URI)
        }) {
            parse_description(description, &mut state, &mut values)?;
        }
        if values.is_empty() {
            return Err(MetadataPacketError::MalformedXml);
        }
        finish_document(
            values,
            MetadataSource::Xmp,
            "application/rdf+xml",
            packet_bytes,
            state,
        )
    }
}

/// Bounded reader for an IPTC-IIM binary packet.
#[derive(Debug, Clone, Copy)]
pub struct IptcMetadataInput {
    limits: MetadataPacketLimits,
}

impl IptcMetadataInput {
    #[must_use]
    pub const fn new(limits: MetadataPacketLimits) -> Self {
        Self { limits }
    }

    /// Parses IPTC-IIM datasets into the canonical metadata domain.
    ///
    /// # Errors
    ///
    /// Returns [`MetadataPacketError`] when the packet is malformed, exceeds a configured
    /// limit, uses unsupported text encoding, or cannot become canonical metadata.
    pub fn read_domain(&self, source: &[u8]) -> Result<MetadataDocument, MetadataPacketError> {
        let actual =
            u64::try_from(source.len()).map_err(|_| MetadataPacketError::PacketTooLarge {
                limit: self.limits.packet_bytes,
                actual: u64::MAX,
            })?;
        if actual > self.limits.source_bytes {
            return Err(MetadataPacketError::SourceTooLarge {
                limit: self.limits.source_bytes,
                actual,
            });
        }
        if actual > self.limits.packet_bytes {
            return Err(MetadataPacketError::PacketTooLarge {
                limit: self.limits.packet_bytes,
                actual,
            });
        }

        let mut cursor = 0;
        let mut charset_utf8 = false;
        let mut datasets = 0_u32;
        let mut state = ParseState::new(self.limits);
        let mut values: BTreeMap<MetadataKey, Vec<DomainValue>> = BTreeMap::new();
        while cursor < source.len() {
            if source.get(cursor) != Some(&0x1c) {
                return Err(MetadataPacketError::MalformedIptc);
            }
            let record = *source
                .get(cursor + 1)
                .ok_or(MetadataPacketError::MalformedIptc)?;
            let dataset = *source
                .get(cursor + 2)
                .ok_or(MetadataPacketError::MalformedIptc)?;
            cursor += 3;
            let length = read_iptc_length(source, &mut cursor)?;
            let end = cursor
                .checked_add(length)
                .ok_or(MetadataPacketError::MalformedIptc)?;
            let value = source
                .get(cursor..end)
                .ok_or(MetadataPacketError::MalformedIptc)?;
            cursor = end;
            datasets = datasets
                .checked_add(1)
                .ok_or(MetadataPacketError::IptcDatasetLimit {
                    limit: self.limits.properties,
                })?;
            if datasets > self.limits.properties {
                return Err(MetadataPacketError::IptcDatasetLimit {
                    limit: self.limits.properties,
                });
            }
            if record == 1 && dataset == 90 {
                charset_utf8 = value == b"\x1b%G";
                if !charset_utf8 && !value.is_empty() {
                    return Err(MetadataPacketError::UnsupportedIptcEncoding);
                }
                continue;
            }
            let key_name = iptc_key(record, dataset);
            let Some(key_name) = key_name else {
                let key = MetadataKey::new(
                    MetadataNamespace::Iptc,
                    format!("dataset-{record:03}-{dataset:03}"),
                )?;
                add_value(
                    &mut values,
                    key,
                    DomainValue::Opaque(RawRepresentation::try_new(
                        "application/octet-stream",
                        value.to_vec(),
                    )?),
                    &mut state,
                )?;
                continue;
            };
            let text = decode_iptc_text(value, charset_utf8)?;
            if text.is_empty() {
                continue;
            }
            let key = MetadataKey::new(MetadataNamespace::Iptc, key_name)?;
            let domain_value = if matches!(key_name, "keywords" | "creator") {
                DomainValue::List(vec![DomainValue::Text(text)])
            } else {
                DomainValue::Text(text)
            };
            add_value(&mut values, key, domain_value, &mut state)?;
        }
        if values.is_empty() {
            return Err(MetadataPacketError::MalformedIptc);
        }
        finish_document(
            values,
            MetadataSource::Iptc,
            "application/iptc",
            source,
            state,
        )
    }
}

#[derive(Debug)]
struct ParseState {
    limits: MetadataPacketLimits,
    properties: u32,
    items: u32,
    warnings: BTreeSet<crate::NormalizationWarning>,
}

impl ParseState {
    const fn new(limits: MetadataPacketLimits) -> Self {
        Self {
            limits,
            properties: 0,
            items: 0,
            warnings: BTreeSet::new(),
        }
    }

    fn property(&mut self) -> Result<(), MetadataPacketError> {
        self.properties =
            self.properties
                .checked_add(1)
                .ok_or(MetadataPacketError::PropertyLimit {
                    limit: self.limits.properties,
                })?;
        if self.properties > self.limits.properties {
            return Err(MetadataPacketError::PropertyLimit {
                limit: self.limits.properties,
            });
        }
        Ok(())
    }

    fn item(&mut self) -> Result<(), MetadataPacketError> {
        self.items = self
            .items
            .checked_add(1)
            .ok_or(MetadataPacketError::CollectionLimit {
                limit: self.limits.collection_items,
            })?;
        if self.items > self.limits.collection_items {
            return Err(MetadataPacketError::CollectionLimit {
                limit: self.limits.collection_items,
            });
        }
        Ok(())
    }
}

fn parse_description(
    description: Node<'_, '_>,
    state: &mut ParseState,
    values: &mut BTreeMap<MetadataKey, Vec<DomainValue>>,
) -> Result<(), MetadataPacketError> {
    for attribute in description.attributes() {
        if attribute.namespace() == Some(RDF_URI)
            || attribute.namespace() == Some(XML_URI)
            || attribute.namespace().is_none()
        {
            continue;
        }
        let key = property_key(attribute.namespace(), attribute.name())?;
        add_value(values, key, text_value(attribute.value(), state)?, state)?;
    }
    for property in description.children().filter(Node::is_element) {
        state.property()?;
        let key = property_key(property.tag_name().namespace(), property.tag_name().name())?;
        let value = parse_property(property, state, 0)?;
        add_value(
            values,
            key,
            canonical_property_value(property, value)?,
            state,
        )?;
    }
    Ok(())
}

fn parse_property(
    property: Node<'_, '_>,
    state: &mut ParseState,
    depth: u32,
) -> Result<DomainValue, MetadataPacketError> {
    if depth >= state.limits.xml_depth {
        return Err(MetadataPacketError::XmlDepthLimit {
            limit: state.limits.xml_depth,
        });
    }
    let children: Vec<_> = property.children().filter(Node::is_element).collect();
    let value = if children.is_empty() {
        let text = property
            .attribute((RDF_URI, "resource"))
            .or_else(|| property.text())
            .unwrap_or_default();
        if let Some(language) = property.attribute((XML_URI, "lang")) {
            let tag = LanguageTag::new(language)?;
            let mut alternatives = BTreeMap::new();
            alternatives.insert(tag, canonical_text(text, state)?);
            DomainValue::LanguageAlternative(LanguageAlternative::new(alternatives)?)
        } else {
            text_value(text, state)?
        }
    } else if children.len() == 1 && children[0].tag_name().namespace() == Some(RDF_URI) {
        match children[0].tag_name().name() {
            "Alt" => parse_alt(children[0], state, depth + 1)?,
            "Bag" | "Seq" => parse_list(children[0], state, depth + 1)?,
            "Description" => parse_structure(children[0], state, depth + 1)?,
            _ => parse_structure(property, state, depth + 1)?,
        }
    } else {
        parse_structure(property, state, depth + 1)?
    };
    with_qualifiers(property, value, state)
}

fn parse_alt(
    array: Node<'_, '_>,
    state: &mut ParseState,
    depth: u32,
) -> Result<DomainValue, MetadataPacketError> {
    let mut alternatives = BTreeMap::new();
    for item in array.children().filter(Node::is_element) {
        state.item()?;
        let language = item.attribute((XML_URI, "lang")).unwrap_or("x-default");
        let value = if item.children().any(|child| child.is_element()) {
            match parse_property(item, state, depth)? {
                DomainValue::Text(text) => text,
                value => format!("{value:?}"),
            }
        } else {
            canonical_text(item.text().unwrap_or_default(), state)?
        };
        alternatives.insert(LanguageTag::new(language)?, value);
    }
    Ok(DomainValue::LanguageAlternative(LanguageAlternative::new(
        alternatives,
    )?))
}

fn parse_list(
    array: Node<'_, '_>,
    state: &mut ParseState,
    depth: u32,
) -> Result<DomainValue, MetadataPacketError> {
    let mut values = Vec::new();
    for item in array.children().filter(Node::is_element) {
        state.item()?;
        values.push(parse_property(item, state, depth)?);
    }
    Ok(DomainValue::List(values))
}

fn parse_structure(
    resource: Node<'_, '_>,
    state: &mut ParseState,
    depth: u32,
) -> Result<DomainValue, MetadataPacketError> {
    let mut fields = BTreeMap::new();
    for attribute in resource.attributes() {
        if attribute.namespace() == Some(RDF_URI)
            || attribute.namespace() == Some(XML_URI)
            || attribute.namespace().is_none()
        {
            continue;
        }
        fields.insert(
            format!(
                "@{}:{}",
                attribute.namespace().unwrap_or_default(),
                attribute.name()
            ),
            text_value(attribute.value(), state)?,
        );
    }
    for child in resource.children().filter(Node::is_element) {
        state.property()?;
        let key = child.tag_name().name().to_owned();
        fields.insert(key, parse_property(child, state, depth)?);
    }
    Ok(DomainValue::Structure(StructuredValue::new(fields)?))
}

fn with_qualifiers(
    property: Node<'_, '_>,
    value: DomainValue,
    state: &mut ParseState,
) -> Result<DomainValue, MetadataPacketError> {
    let qualifiers: Vec<_> = property
        .attributes()
        .filter(|attribute| {
            attribute.namespace() != Some(XML_URI)
                && attribute.namespace() != Some(RDF_URI)
                && attribute.namespace().is_some()
        })
        .collect();
    if qualifiers.is_empty() {
        return Ok(value);
    }
    let mut fields = BTreeMap::new();
    fields.insert("_value".to_owned(), value);
    for qualifier in qualifiers {
        fields.insert(
            format!(
                "@{}:{}",
                qualifier.namespace().unwrap_or_default(),
                qualifier.name()
            ),
            text_value(qualifier.value(), state)?,
        );
    }
    Ok(DomainValue::Structure(StructuredValue::new(fields)?))
}

fn canonical_property_value(
    property: Node<'_, '_>,
    value: DomainValue,
) -> Result<DomainValue, MetadataPacketError> {
    let local = property.tag_name().name();
    if local == "hierarchicalSubject" {
        let DomainValue::List(values) = value else {
            return Ok(value);
        };
        let paths = values
            .into_iter()
            .filter_map(|value| match value {
                DomainValue::Text(value) => {
                    Some(value.split('|').map(str::to_owned).collect::<Vec<_>>())
                }
                _ => None,
            })
            .collect();
        return Ok(DomainValue::Keywords(HierarchicalKeywords::new(paths)?));
    }
    if matches!(local, "Rating" | "RatingPercent" | "ColorMode")
        && let DomainValue::Text(text) = &value
        && let Ok(number) = text.parse::<i64>()
    {
        return Ok(DomainValue::Integer(number));
    }
    Ok(value)
}

fn add_value(
    values: &mut BTreeMap<MetadataKey, Vec<DomainValue>>,
    key: MetadataKey,
    value: DomainValue,
    state: &mut ParseState,
) -> Result<(), MetadataPacketError> {
    state.item()?;
    values.entry(key).or_default().push(value);
    Ok(())
}

fn finish_document(
    values: BTreeMap<MetadataKey, Vec<DomainValue>>,
    source: MetadataSource,
    media_type: &str,
    raw: &[u8],
    state: ParseState,
) -> Result<MetadataDocument, MetadataPacketError> {
    let raw = RawRepresentation::try_new(media_type, raw.to_vec())?;
    let mut provenance =
        MetadataProvenance::new(source, crate::Confidence::new(100)?, PrivacyClass::Public)
            .with_raw(raw);
    for warning in state.warnings {
        provenance = provenance.with_warning(warning);
    }
    let records = values.into_iter().map(|(key, values)| {
        let value = merge_values(values);
        MetadataRecord::new(key, value, provenance.clone())
    });
    Ok(MetadataDocument::from_records(records)?)
}

fn merge_values(values: Vec<DomainValue>) -> DomainValue {
    if values.len() == 1 {
        return values
            .into_iter()
            .next()
            .unwrap_or(DomainValue::List(Vec::new()));
    }
    let mut merged = Vec::new();
    for value in values {
        match value {
            DomainValue::List(items) => merged.extend(items),
            value => merged.push(value),
        }
    }
    DomainValue::List(merged)
}

fn property_key(namespace: Option<&str>, name: &str) -> Result<MetadataKey, MetadataPacketError> {
    let namespace = match namespace {
        Some(DC_URI | DC_TERMS_URI) => MetadataNamespace::DublinCore,
        Some(PHOTOSHOP_URI) => MetadataNamespace::Photoshop,
        Some(XMP_URI) => MetadataNamespace::Xmp,
        Some(LR_URI) => MetadataNamespace::unknown(LR_URI)?,
        Some(uri) => MetadataNamespace::unknown(uri)?,
        None => MetadataNamespace::unknown("urn:rusttable:xmp:unqualified")?,
    };
    Ok(MetadataKey::new(namespace, name)?)
}

fn text_value(text: &str, state: &mut ParseState) -> Result<DomainValue, MetadataPacketError> {
    Ok(DomainValue::Text(canonical_text(text, state)?))
}

fn canonical_text(text: &str, state: &mut ParseState) -> Result<String, MetadataPacketError> {
    let text = text.trim();
    let actual = u64::try_from(text.len()).map_err(|_| MetadataPacketError::TextTooLarge {
        limit: state.limits.text_bytes,
        actual: u64::MAX,
    })?;
    if actual > state.limits.text_bytes {
        return Err(MetadataPacketError::TextTooLarge {
            limit: state.limits.text_bytes,
            actual,
        });
    }
    Ok(text.to_owned())
}

fn decode_xml(source: &[u8]) -> Result<String, MetadataPacketError> {
    if source.starts_with(&[0xff, 0xfe]) {
        return decode_utf16(&source[2..], true);
    }
    if source.starts_with(&[0xfe, 0xff]) {
        return decode_utf16(&source[2..], false);
    }
    let source = source.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(source);
    String::from_utf8(source.to_vec()).map_err(|_| MetadataPacketError::InvalidUtf8)
}

fn decode_utf16(source: &[u8], little_endian: bool) -> Result<String, MetadataPacketError> {
    if !source.len().is_multiple_of(2) {
        return Err(MetadataPacketError::InvalidUtf16);
    }
    let chunks = source.chunks(2);
    let units = chunks
        .map(|chunk| {
            if little_endian {
                u16::from_le_bytes([chunk[0], chunk[1]])
            } else {
                u16::from_be_bytes([chunk[0], chunk[1]])
            }
        })
        .collect::<Vec<_>>();
    String::from_utf16(&units).map_err(|_| MetadataPacketError::InvalidUtf16)
}

fn extract_packet_text(text: &str, limit: u64) -> Result<&str, MetadataPacketError> {
    let start = ["<x:xmpmeta", "<xmpmeta", "<rdf:RDF"]
        .iter()
        .filter_map(|marker| text.find(marker))
        .min()
        .unwrap_or(0);
    let end = if text[start..].contains("</x:xmpmeta>") {
        start + text[start..].find("</x:xmpmeta>").unwrap_or(0) + "</x:xmpmeta>".len()
    } else if text[start..].contains("</xmpmeta>") {
        start + text[start..].find("</xmpmeta>").unwrap_or(0) + "</xmpmeta>".len()
    } else {
        text.len()
    };
    let packet = text[start..end].trim();
    let actual = u64::try_from(packet.len()).map_err(|_| MetadataPacketError::PacketTooLarge {
        limit,
        actual: u64::MAX,
    })?;
    if actual > limit {
        return Err(MetadataPacketError::PacketTooLarge { limit, actual });
    }
    Ok(packet)
}

fn preflight_xml(source: &[u8], limits: MetadataPacketLimits) -> Result<(), MetadataPacketError> {
    let mut depth = 0_u32;
    let mut index = 0;
    while index < source.len() {
        if source[index] == b'&' {
            let end = source[index..]
                .iter()
                .position(|byte| *byte == b';')
                .map(|offset| index + offset)
                .ok_or(MetadataPacketError::MalformedXml)?;
            let entity = &source[index + 1..end];
            let safe = matches!(entity, b"amp" | b"lt" | b"gt" | b"quot" | b"apos")
                || entity.starts_with(b"#x")
                || entity.starts_with(b"#");
            if !safe || entity.len() > 16 {
                return Err(MetadataPacketError::MalformedXml);
            }
            index = end + 1;
            continue;
        }
        if source[index] != b'<' {
            index += 1;
            continue;
        }
        if source[index..].starts_with(b"<!--") {
            index = source[index + 4..]
                .windows(3)
                .position(|window| window == b"-->")
                .map(|end| index + 4 + end + 3)
                .ok_or(MetadataPacketError::MalformedXml)?;
            continue;
        }
        if source[index..].starts_with(b"<![CDATA[") {
            index = source[index + 9..]
                .windows(3)
                .position(|window| window == b"]]>")
                .map(|end| index + 9 + end + 3)
                .ok_or(MetadataPacketError::MalformedXml)?;
            continue;
        }
        let end = tag_end(source, index + 1).ok_or(MetadataPacketError::MalformedXml)?;
        let tag = &source[index + 1..end];
        let trimmed = tag.trim_ascii_start();
        if trimmed.starts_with(b"!DOCTYPE") || trimmed.starts_with(b"!ENTITY") {
            return Err(MetadataPacketError::MalformedXml);
        }
        if trimmed.starts_with(b"?") || trimmed.starts_with(b"!") {
            index = end + 1;
            continue;
        }
        if trimmed.starts_with(b"/") {
            depth = depth
                .checked_sub(1)
                .ok_or(MetadataPacketError::MalformedXml)?;
        } else if !trimmed.ends_with(b"/") {
            depth = depth
                .checked_add(1)
                .ok_or(MetadataPacketError::XmlDepthLimit {
                    limit: limits.xml_depth,
                })?;
            if depth > limits.xml_depth {
                return Err(MetadataPacketError::XmlDepthLimit {
                    limit: limits.xml_depth,
                });
            }
        }
        index = end + 1;
    }
    if depth != 0 {
        return Err(MetadataPacketError::MalformedXml);
    }
    Ok(())
}

fn tag_end(source: &[u8], mut index: usize) -> Option<usize> {
    let mut quote = None;
    while index < source.len() {
        match (quote, source[index]) {
            (None, b'\'' | b'"') => quote = Some(source[index]),
            (Some(current), byte) if byte == current => quote = None,
            (None, b'>') => return Some(index),
            _ => {}
        }
        index += 1;
    }
    None
}

fn read_iptc_length(source: &[u8], cursor: &mut usize) -> Result<usize, MetadataPacketError> {
    let first = *source
        .get(*cursor)
        .ok_or(MetadataPacketError::MalformedIptc)?;
    *cursor += 1;
    if first & 0x80 == 0 {
        let second = *source
            .get(*cursor)
            .ok_or(MetadataPacketError::MalformedIptc)?;
        *cursor += 1;
        return Ok(usize::from(u16::from_be_bytes([first, second])));
    }
    let count = usize::from(first & 0x7f);
    if count == 0 || count > 4 {
        return Err(MetadataPacketError::MalformedIptc);
    }
    let bytes = source
        .get(*cursor..*cursor + count)
        .ok_or(MetadataPacketError::MalformedIptc)?;
    *cursor += count;
    bytes.iter().try_fold(0_usize, |value, byte| {
        value
            .checked_shl(8)
            .and_then(|value| value.checked_add(usize::from(*byte)))
            .ok_or(MetadataPacketError::MalformedIptc)
    })
}

fn decode_iptc_text(value: &[u8], utf8: bool) -> Result<String, MetadataPacketError> {
    if utf8 {
        return String::from_utf8(value.to_vec())
            .map_err(|_| MetadataPacketError::UnsupportedIptcEncoding);
    }
    Ok(value.iter().map(|byte| char::from(*byte)).collect())
}

fn iptc_key(record: u8, dataset: u8) -> Option<&'static str> {
    match (record, dataset) {
        (2, 5) => Some("object-name"),
        (2, 15) => Some("category"),
        (2, 25) => Some("keywords"),
        (2, 55) => Some("date-created"),
        (2, 60) => Some("time-created"),
        (2, 80 | 122) => Some("creator"),
        (2, 85) => Some("creator-title"),
        (2, 90) => Some("city"),
        (2, 95) => Some("province-state"),
        (2, 100) => Some("country-code"),
        (2, 101) => Some("country"),
        (2, 105) => Some("headline"),
        (2, 110) => Some("credit"),
        (2, 115) => Some("source"),
        (2, 116) => Some("copyright"),
        (2, 120) => Some("caption"),
        _ => None,
    }
}
