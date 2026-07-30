# ICC profile parsing contract

`rusttable-color` owns the RustTable ICC v2/v4 parser. The parser is pure Rust,
offline, bounded by `IccProfileLimits`, and contains no native colour-library
handle. Parsing and transform execution are deliberately separate: an
`IccProfile` is validated data and has no API that creates a `TransformPlan`.

## Supported profile surface

- ICC v2 and v4 `scnr`, `mntr`, `prtr`, `spac`, and selected `link` classes.
- RGB and gray device spaces. Non-device-link PCS values are D50 XYZ or Lab;
  selected device links may target RGB, gray, XYZ, or Lab.
- RGB matrix/TRC and gray TRC profiles, plus `mft1`, `mft2`, `mAB`, and `mBA`
  LUT elements with one or three input/output channels.
- `XYZ `, `curv`, parametric `para` functions 0–4, `sf32` chromatic
  adaptation, v2 `desc`, v4 `mluc`, `cicp`, and `view` values.
- Unknown tag signatures are retained as bounded owned opaque bytes after the
  common type signature and reserved bytes are validated.

The parser rejects unsupported classes, non-RGB/gray device spaces, LUT channel
counts outside the selected one/three-channel surface, and unsupported types on
known transform tags. These are typed separately from malformed data and
resource-limit failures through `IccParseErrorKind`.

## Validation and limits

Validation covers declared size, supported version, profile/file signatures,
reserved header fields, calendar date, class/space/PCS compatibility, rendering
intent, D50 PCS illuminant, optional ICC MD5 profile ID, tag count/table bounds,
four-byte offsets, duplicate signatures, and partial overlaps. Exact shared tag
elements are allowed. Curves, text, localization records, LUT grids, element
offsets, and copied opaque data are checked against explicit limits before
allocation. Matrix validation reuses `Matrix3`; fixed values reuse `FiniteF32`;
parametric-domain division uses the core checked finite numerical policy.

## Identity semantics

`IccProfileIdentity::byte` is SHA-256 plus the declared byte length over the
complete profile. Equal bytes therefore always have equal byte identity. The
legacy product `ProfileId` remains a parser-versioned projection of that exact
content identity.

`IccProfileIdentity::semantic` is a separate SHA-256 namespace. It hashes the
ICC version, class, device/connection spaces, rendering intent, D50 illuminant,
and every complete tag payload in tag-signature order. It excludes tag-table
order, offsets, alignment padding, the embedded MD5 field, creation time, and
vendor/device provenance. It includes descriptions and opaque tags, so it is a
canonical profile-content identity, not proof that two future executable
transforms are equivalent.

## Deliberate limitations

This slice does not execute transforms, select or discover OS profiles, cache
profiles, generate SIMD/WGPU work, or parse iccMAX/v5. Named-colour, abstract,
CMYK, n-channel, spectral, `mpet`, and vendor-private executable processing
elements remain explicitly unsupported. Multi-stage LUTs are retained as typed
data only; no interpolation or evaluation policy is implied.
