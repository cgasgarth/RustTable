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
provenance. They intentionally do not select precedence, pair sidecars, or
write packets back to image containers.

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

Parsing, precedence selection, catalog writes, and file write-back are
intentionally outside this domain boundary and remain consumers of it.
