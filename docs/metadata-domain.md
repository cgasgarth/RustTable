# Canonical metadata domain

`rusttable-metadata` exposes a parser-independent metadata domain for values
that can be shared by import, catalog, sidecar, export, search, and scripting
layers. The domain deliberately does not parse a container or expose EXIF,
IPTC, or XMP parser types.

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
