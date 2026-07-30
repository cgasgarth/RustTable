# Canonical metadata domain

`rusttable-metadata` exposes a parser-independent metadata domain for values
that can be shared by import, catalog, sidecar, export, search, and scripting
layers. Its bounded EXIF input adapter maps supported EXIF values into that
domain without exposing parser-library types to consumers.

## IPTC/XMP packet slice

`XmpMetadataInput` and `IptcMetadataInput` accept standalone packets with an
explicit `MetadataPacketLimits` budget. XMP extraction accepts UTF-8/UTF-16
packets, disables DTDs and external entity resolution, and preflights entity,
node, depth, property, collection, packet, and text limits before creating the
read-only XML tree. IPTC-IIM datasets are length-checked and decode the UTF-8
marker or bounded single-byte text without treating any value as a path or
network resource.

The adapters map RDF `Alt`, `Bag`, and `Seq` values to language alternatives
and lists, map Lightroom `hierarchicalSubject` values to keyword paths, retain
unknown namespaces/properties and qualifiers as bounded structures or opaque
bytes, and attach the canonical packet representation to every record's
provenance. They do not pair sidecars or write packets back to image
containers.

## Effective metadata and precedence

`MetadataResolver` accepts `MetadataRecord` values plus explicit
`MetadataAssertion::Clear` markers and returns `ResolvedMetadata`. The result
contains:

- an effective `MetadataDocument`;
- every valid lower-priority or cleared source record, grouped by canonical
  key through `ResolvedMetadata::retained`; and
- an order-stable `MetadataResolutionReceipt` with one field decision per key.

Source priority, from lowest to highest, is import default, container, EXIF,
maker note, IPTC, embedded XMP, sidecar XMP, catalog value, user override,
export-only override, and generated technical value. The legacy `Imported`,
`Xmp`, `CatalogEdit`, and `RecipeOverride` variants map to their corresponding
explicit layers. `MetadataSourceClass` keeps extracted, import-default,
catalog, user, export-only, and generated values distinct.

Fields use explicit strategies:

- scalar and opaque fields select source priority, then confidence, source
  identity, and a canonical value digest for deterministic ties;
- flat tags/people/creator lists and hierarchical tags use sorted set union;
- caption, description, rights, and copyright select the first configured
  preferred language, then source priority within that language;
- capture dates select source priority first and greater declared precision
  for equal-priority evidence;
- ratings normalize rejected/unrated to zero and stars or integers to `0..=5`;
- labels normalize names or `0..=5` to none, red, yellow, green, blue, or
  purple.

A clear applies only to its own logical source layer. Clearing a user override,
for example, reveals the catalog, sidecar, or extracted value instead of
deleting those records. Null input is represented by this explicit clear
assertion, never by a synthetic domain value.

Every decision names its sources, source classes, applied rules, and evidence
disposition. Public evidence carries its canonical value. Personal, sensitive,
and location evidence carries a domain-separated SHA-256 value hash instead.
Invalid source values are marked as ignored evidence and cannot hide a valid
lower-priority value.

## Contract

- Keys and namespaces are non-empty, NFC-normalized UTF-8 identifiers with
  explicit byte bounds. Vendor namespaces are retained as `Unknown` values.
- Values are typed as scalars, exact rationals, dates, binary data, lists,
  language alternatives, structures, GPS coordinates, orientations, keyword
  paths, or opaque source representations.
- Text rejects NUL and control characters. Lists, structures, raw values,
  records, and encoded documents have explicit count or byte limits.
- Rationals are reduced without floating-point conversion. Date values retain
  their declared precision and an absent timezone remains absent; no timezone
  is inferred.
- Documents use sorted keys and reject duplicate keys. `CanonicalCodec` uses
  length-delimited fields, a versioned header, bounded decoding, and stable
  tags so equivalent documents have identical bytes.
- Each record retains source, confidence, privacy classification, optional raw
  representation, and normalization warnings.
- Resolution is independent of input enumeration order. Conflict receipts are
  sorted by key and stable evidence rank.

Catalog persistence, UI presentation, sidecar pairing, and file write-back are
outside this domain boundary and remain consumers of it.
