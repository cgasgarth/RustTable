// This contract is intentionally independent of architecture/operation metadata.
// It is a checked-in trust anchor: do not regenerate it from mutable inputs during checks.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::model::{
    AbiLayout, Evidence, Operation, OperationManifest, OperationOverride, ParameterCodec,
    ParameterMigration, ParameterVersion, ReferenceIdentity,
};
use crate::ScanError;

pub const NATIVE_SOURCE_COMMIT: &str = "cfe57f3bbf5269bfacf31e832267279caa6938ad";
pub const AUDITED_SOURCE_SNAPSHOT: &str = "d8628e8103989bc4ef06dbfb9fd01f3809f884bf";

const MANIFEST_LENS_NAME: &str = "lens";
const TRUSTED_LENS_NAME: &str = "lenscorrection";
const MANIFEST_LINEAR_OFFSET_NAME: &str = "linear_offset";
const GENERATED_LINEAR_OFFSET_ALIAS: &str = "linear-offset";
const MANIFEST_RGB_GAIN_NAME: &str = "rgb_gain";
const GENERATED_RGB_GAIN_ALIAS: &str = "rgbgain";

const EXPECTED_ARCHITECTURE_RECORDS: usize = 93;
const EXPECTED_TRUSTED_RECORDS: usize = 57;
const EXPECTED_MANIFEST_TRUSTED_RECORDS: usize = 55;
const EXPECTED_REGISTRY_ONLY_TRUSTED_RECORDS: usize = 2;
const EXPECTED_MANIFEST_UNANCHORED_RECORDS: usize = 38;
// This is a cross-domain summary only: the two registry-only trusted contracts are included in
// the 57-contract population but do not replace native manifest rows. Keep it separate from the
// 38 ordinary unanchored manifest records above.
const EXPECTED_CROSS_DOMAIN_UNANCHORED_SUMMARY: usize = 36;

const GENERATED_COMPATIBILITY_ALIASES: &[(&str, &str)] = &[
    (MANIFEST_LENS_NAME, TRUSTED_LENS_NAME),
    (MANIFEST_LINEAR_OFFSET_NAME, GENERATED_LINEAR_OFFSET_ALIAS),
    (MANIFEST_RGB_GAIN_NAME, GENERATED_RGB_GAIN_ALIAS),
];

/// Manifest records without an independent trust contract are explicitly reference-only.
/// Keep this list independent from the generated architecture artifacts so a missing or newly
/// introduced record cannot silently acquire an Implemented status.
const OPAQUE_DEFERRED_MANIFEST_NAMES: &[&str] = &[
    "atrous",
    "bilat",
    "bilateral",
    "blurs",
    "cacorrect",
    "cacorrectrgb",
    "channelmixerrgb",
    "colorbalance",
    "colorbalancergb",
    "colorchecker",
    "colorequal",
    "colorharmonizer",
    "colorize",
    "demosaic",
    "denoiseprofile",
    "diffuse",
    "equalizer",
    "filmic",
    "filmicrgb",
    "gamma",
    "globaltonemap",
    "hazeremoval",
    "hotpixels",
    "lowlight",
    "lowpass",
    "lut3d",
    "monochrome",
    "negadoctor",
    "nlmeans",
    "overexposed",
    "rawdenoise",
    "rawoverexposed",
    "rawprepare",
    "rgbcurve",
    "sigmoid",
    "toneequal",
    "tonemap",
    "zonesystem",
];

#[derive(Debug, Clone, Copy)]
pub struct TrustedOperationContract {
    pub(crate) compatibility_name: &'static str,
    pub(crate) rust_id: &'static str,
    pub(crate) descriptor_version: u16,
    pub(crate) parameter_version: u16,
    pub(crate) implementation_version: u16,
    pub(crate) native: TrustedRecord,
    pub(crate) audited: TrustedRecord,
}

#[derive(Debug, Clone, Copy)]
pub struct TrustedRecord {
    pub(crate) source_commit: &'static str,
    pub(crate) source_snapshot: &'static str,
    pub(crate) module_version: u32,
    pub(crate) parameter_size: usize,
    pub(crate) parameter_layout_hash: &'static str,
    pub(crate) native_order: Option<usize>,
    pub(crate) reference_path: Option<&'static str>,
    pub(crate) source_abi_model: &'static str,
    pub(crate) abi_identity: &'static str,
    pub(crate) codec_identity: &'static str,
    pub(crate) parameter_versions: &'static [TrustedVersion],
    pub(crate) evidence_identity: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct TrustedVersion {
    pub(crate) version: u32,
    pub(crate) byte_size: usize,
    pub(crate) layout_hash: &'static str,
    pub(crate) decoder: &'static str,
    pub(crate) opaque_blocking: bool,
    pub(crate) abi_identity: &'static str,
    pub(crate) codec_identity: &'static str,
}

pub const TRUSTED_OPERATIONS: &[TrustedOperationContract] = &[
    TrustedOperationContract {
        compatibility_name: "agx",
        rust_id: "rusttable.agx",
        descriptor_version: 7,
        parameter_version: 7,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 7,
            parameter_size: 144,
            parameter_layout_hash: "3aff44c0fabc743303ac96a0b818ba65c964555a41c89b9cfbf164748905443a",
            native_order: Some(155),
            reference_path: Some("src/iop/agx.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "475986c178e7160787064b604c1c16ddbb4cd40f73b22f32b40c67d4333bcd0b",
            codec_identity: "c860a78d823b76b9a78dc571ba7d53ac1eda286b6282e65fe82cc44a5b974daa",
            parameter_versions: &[TrustedVersion {
                version: 7,
                byte_size: 144,
                layout_hash: "db9bed41f60be7904f90a3ae9880026e591be352806c4c8a371c59de54cddc54",
                decoder: "generated.bytes.decode.v7",
                opaque_blocking: false,
                abi_identity: "475986c178e7160787064b604c1c16ddbb4cd40f73b22f32b40c67d4333bcd0b",
                codec_identity: "c860a78d823b76b9a78dc571ba7d53ac1eda286b6282e65fe82cc44a5b974daa",
            }],
            evidence_identity: "8c02b278defb2e8499cb81f27648b9c735f8bff15b6a088c4bf4a96c01894409",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 7,
            parameter_size: 144,
            parameter_layout_hash: "903a318cd6a5b889727c52038202b5b9db32ddd2b8e9ca8e994f07970aba2906",
            native_order: Some(155),
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 7,
                byte_size: 144,
                layout_hash: "db9bed41f60be7904f90a3ae9880026e591be352806c4c8a371c59de54cddc54",
                decoder: "rusttable.agx.decode.v7",
                opaque_blocking: false,
                abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            }],
            evidence_identity: "8c02b278defb2e8499cb81f27648b9c735f8bff15b6a088c4bf4a96c01894409",
        },
    },
    TrustedOperationContract {
        compatibility_name: "ashift",
        rust_id: "rusttable.ashift",
        descriptor_version: 1,
        parameter_version: 5,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 5,
            parameter_size: 892,
            parameter_layout_hash: "c9972d1921ae96daf63e138889a424dd29448db6b996cde6ad0c0414b67b36a3",
            native_order: Some(136),
            reference_path: Some("src/iop/ashift.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "c12fd0478d7cc1622da917bbf74e93418380011b12d90a8add26b8555cbd179d",
            codec_identity: "f02dbca90b5d09649da724f9b6dd90d15ead263e64f65afb5560b7f63e3d81bd",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 2520,
                    layout_hash: "50ad4f49e886c778a550a91ce99e784282c473292f972d7e6f2c2c1b43c66ab0",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 2696,
                    layout_hash: "be5d24b17e48423427b5845a01b9ce8a3706c3fc2a7e3734064d28209a399d67",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 2904,
                    layout_hash: "8e1d7c6b5ee8fcf6c891f6671e1581554388e71987b8a08474b2bb84497dc0d7",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 3104,
                    layout_hash: "cf7ec412c943a448ba8aacd19f683b7b6491b78bd0ab07f1eb3ec181852474d1",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 5,
                    byte_size: 892,
                    layout_hash: "c0ef08512c9f6a9327411572d346f88809120ea90a700e88535e3c43c1068a74",
                    decoder: "generated.bytes.decode.v5",
                    opaque_blocking: false,
                    abi_identity: "c12fd0478d7cc1622da917bbf74e93418380011b12d90a8add26b8555cbd179d",
                    codec_identity: "f02dbca90b5d09649da724f9b6dd90d15ead263e64f65afb5560b7f63e3d81bd",
                },
            ],
            evidence_identity: "620b3aef41075e77e6f46a4ade371f7333652e2d006af580092e3bf03c21d6e9",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 5,
            parameter_size: 892,
            parameter_layout_hash: "f0bcac516a619d7964412a226729f08389e97835addc76169ea06dfa18c5addc",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 2520,
                    layout_hash: "50ad4f49e886c778a550a91ce99e784282c473292f972d7e6f2c2c1b43c66ab0",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 2696,
                    layout_hash: "be5d24b17e48423427b5845a01b9ce8a3706c3fc2a7e3734064d28209a399d67",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 2904,
                    layout_hash: "8e1d7c6b5ee8fcf6c891f6671e1581554388e71987b8a08474b2bb84497dc0d7",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 3104,
                    layout_hash: "cf7ec412c943a448ba8aacd19f683b7b6491b78bd0ab07f1eb3ec181852474d1",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 5,
                    byte_size: 892,
                    layout_hash: "c0ef08512c9f6a9327411572d346f88809120ea90a700e88535e3c43c1068a74",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
            ],
            evidence_identity: "620b3aef41075e77e6f46a4ade371f7333652e2d006af580092e3bf03c21d6e9",
        },
    },
    TrustedOperationContract {
        compatibility_name: "basicadj",
        rust_id: "rusttable.basicadj",
        descriptor_version: 2,
        parameter_version: 2,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 2,
            parameter_size: 44,
            parameter_layout_hash: "de49aa2ecec5121928807906c03f83c4ea4e1f2e19dbc4d9af78038e92346e9a",
            native_order: Some(78),
            reference_path: Some("src/iop/basicadj.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "b6504035a9fe95727b0c3260e28912e7cd147de39328e4b411b1e4480d6a86ab",
            codec_identity: "ff44012507f5650db432fbbdfa3c13b9c0123177abc0b5c11120e2d6c562191e",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 40,
                    layout_hash: "011fee3ea6b617643143b34a46df799f10b06ae254c115f7666f1b7bf66253b6",
                    decoder: "rusttable.basicadj.decode.v1",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 44,
                    layout_hash: "63a7cb5b40837abfb60f9aeb968eda5b93924ee35e2c5fe526867e130b809da2",
                    decoder: "rusttable.basicadj.decode.v2",
                    opaque_blocking: false,
                    abi_identity: "b6504035a9fe95727b0c3260e28912e7cd147de39328e4b411b1e4480d6a86ab",
                    codec_identity: "ff44012507f5650db432fbbdfa3c13b9c0123177abc0b5c11120e2d6c562191e",
                },
            ],
            evidence_identity: "f91171523d9920d7ccf4a564cbe16c6fc7710add90b0d4e565192d29f8b92ed1",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 2,
            parameter_size: 44,
            parameter_layout_hash: "30e5761e04a86aac16e281cc170a4f21b099166a2a5f8527a9b01ee3aa0f5a2e",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 40,
                    layout_hash: "011fee3ea6b617643143b34a46df799f10b06ae254c115f7666f1b7bf66253b6",
                    decoder: "rusttable.basicadj.decode.v1",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 44,
                    layout_hash: "63a7cb5b40837abfb60f9aeb968eda5b93924ee35e2c5fe526867e130b809da2",
                    decoder: "rusttable.basicadj.decode.v2",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
            ],
            evidence_identity: "f91171523d9920d7ccf4a564cbe16c6fc7710add90b0d4e565192d29f8b92ed1",
        },
    },
    TrustedOperationContract {
        compatibility_name: "basecurve",
        rust_id: "rusttable.basecurve",
        descriptor_version: 6,
        parameter_version: 6,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 6,
            parameter_size: 520,
            parameter_layout_hash: "0881226e618e645a145b052def2dbd6a6ab11bee15ab944867475556e1141dd8",
            native_order: Some(92),
            reference_path: Some("src/iop/basecurve.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "ffc80e7d80332f183509039a2288d22fee2cfac55db8e8914b68d4a032ab3de3",
            codec_identity: "d094e1f7a0866843983ea47771c3a250837f7ff699365362800ad31d50f99b18",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 52,
                    layout_hash: "d63cecbb050f267481218175459f98ea7a9f05db1237f70cb3c6fc4d52c0987f",
                    decoder: "rusttable.basecurve.decode.v1",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 504,
                    layout_hash: "350774dd5d29c1c8eefa2d2c06a61d28063d709962cf3d52100a6958d8c49a3f",
                    decoder: "rusttable.basecurve.decode.v2",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 512,
                    layout_hash: "4c7e8161b209f41a8997c4bb70a273ea5e69f11299daa19774c672f6fbdc5d2d",
                    decoder: "rusttable.basecurve.decode.v3",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 512,
                    layout_hash: "4c7e8161b209f41a8997c4bb70a273ea5e69f11299daa19774c672f6fbdc5d2d",
                    decoder: "rusttable.basecurve.decode.v4",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 5,
                    byte_size: 516,
                    layout_hash: "6b7ca6858c6aa91e9f9b90b38828a9a66ec52ca43b31d429815491782ba12001",
                    decoder: "rusttable.basecurve.decode.v5",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 6,
                    byte_size: 520,
                    layout_hash: "0881226e618e645a145b052def2dbd6a6ab11bee15ab944867475556e1141dd8",
                    decoder: "rusttable.basecurve.decode.v6",
                    opaque_blocking: false,
                    abi_identity: "ffc80e7d80332f183509039a2288d22fee2cfac55db8e8914b68d4a032ab3de3",
                    codec_identity: "d094e1f7a0866843983ea47771c3a250837f7ff699365362800ad31d50f99b18",
                },
            ],
            evidence_identity: "0f03a0c515c10e087814d2765a6935696d3621bb567a4dff40fbe82313dbcfcc",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 6,
            parameter_size: 520,
            parameter_layout_hash: "0881226e618e645a145b052def2dbd6a6ab11bee15ab944867475556e1141dd8",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "ffc80e7d80332f183509039a2288d22fee2cfac55db8e8914b68d4a032ab3de3",
            codec_identity: "d094e1f7a0866843983ea47771c3a250837f7ff699365362800ad31d50f99b18",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 52,
                    layout_hash: "d63cecbb050f267481218175459f98ea7a9f05db1237f70cb3c6fc4d52c0987f",
                    decoder: "rusttable.basecurve.decode.v1",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 504,
                    layout_hash: "350774dd5d29c1c8eefa2d2c06a61d28063d709962cf3d52100a6958d8c49a3f",
                    decoder: "rusttable.basecurve.decode.v2",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 512,
                    layout_hash: "4c7e8161b209f41a8997c4bb70a273ea5e69f11299daa19774c672f6fbdc5d2d",
                    decoder: "rusttable.basecurve.decode.v3",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 512,
                    layout_hash: "4c7e8161b209f41a8997c4bb70a273ea5e69f11299daa19774c672f6fbdc5d2d",
                    decoder: "rusttable.basecurve.decode.v4",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 5,
                    byte_size: 516,
                    layout_hash: "6b7ca6858c6aa91e9f9b90b38828a9a66ec52ca43b31d429815491782ba12001",
                    decoder: "rusttable.basecurve.decode.v5",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 6,
                    byte_size: 520,
                    layout_hash: "0881226e618e645a145b052def2dbd6a6ab11bee15ab944867475556e1141dd8",
                    decoder: "rusttable.basecurve.decode.v6",
                    opaque_blocking: false,
                    abi_identity: "ffc80e7d80332f183509039a2288d22fee2cfac55db8e8914b68d4a032ab3de3",
                    codec_identity: "d094e1f7a0866843983ea47771c3a250837f7ff699365362800ad31d50f99b18",
                },
            ],
            evidence_identity: "0f03a0c515c10e087814d2765a6935696d3621bb567a4dff40fbe82313dbcfcc",
        },
    },
    TrustedOperationContract {
        compatibility_name: "bloom",
        rust_id: "rusttable.bloom",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 1,
            parameter_size: 12,
            parameter_layout_hash: "1cdf4b1c74dea5b674e66a56795656a7a01cca14c97fbf0834c0bfbc2f608ae4",
            native_order: Some(67),
            reference_path: Some("src/iop/bloom.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "ad658933041d7e0cae4758fe697dab72b9978945b23647129d67a74be2c861fd",
            codec_identity: "40224b3a3d844ceb0a4c1119e373d1f7733ea44adc6a6c1047a4c642511d1529",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 12,
                layout_hash: "b0880bf77001faed9ff5fe3c3978a0ea67f2b24d674fb1934228d383e5b0f87e",
                decoder: "generated.bytes.decode.v1",
                opaque_blocking: false,
                abi_identity: "ad658933041d7e0cae4758fe697dab72b9978945b23647129d67a74be2c861fd",
                codec_identity: "40224b3a3d844ceb0a4c1119e373d1f7733ea44adc6a6c1047a4c642511d1529",
            }],
            evidence_identity: "cf133d59e58d3d403f116ff2a526af93b3739c2dd812eb21298c0961d6c896c8",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 1,
            parameter_size: 12,
            parameter_layout_hash: "8dab7530b84521ba38cd18b5107129a5bec82a0cd667966484fc5317d0f267f0",
            native_order: Some(61),
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 12,
                layout_hash: "8dab7530b84521ba38cd18b5107129a5bec82a0cd667966484fc5317d0f267f0",
                decoder: "rusttable.bloom.decode.v1",
                opaque_blocking: false,
                abi_identity: "9d56cba77a15032c2eca34d856ab52d4f535581b0a7e252e7718d3c879cee0d9",
                codec_identity: "8e6454f32e8531d0687f7683b9d77ccdde17d86b10465038e9a3af477568fb71",
            }],
            evidence_identity: "dd475795277375365fb0331ff266499ada8c07d47c7217af1b59e430da2b1b37",
        },
    },
    TrustedOperationContract {
        compatibility_name: "borders",
        rust_id: "rusttable.borders",
        descriptor_version: 4,
        parameter_version: 4,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 4,
            parameter_size: 120,
            parameter_layout_hash: "de65eab5fa5c9e59ebb58458aa64d36e4a538384dc0fca28f743499b29ff5777",
            native_order: Some(122),
            reference_path: Some("src/iop/borders.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "9edf6e24c118c37b98268666e7943e14ad19d677a428d67a5eb7e02f3a9be961",
            codec_identity: "36634f7d7384374f72e2c0a1fa8f1ba59b54ec29f362f07bf9b1e59db5054d64",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 408,
                    layout_hash: "dd81f8fac04158b1b2d08f05d463fc0e8ed92d30a212efa2412ed0ff30361efe",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 600,
                    layout_hash: "3f52d7bc2a334329d4bd718ae035591ec70f315c535bf45b8efd795269c3f460",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 376,
                    layout_hash: "d6f924f72c1bb02e11f44ae4c50370b620ac0d3734dbbb4f6a86619d8c28f4be",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 120,
                    layout_hash: "8223182652d738d5add43ee04b1bb20de78d98c0a4057ca00378e48992652689",
                    decoder: "generated.bytes.decode.v4",
                    opaque_blocking: false,
                    abi_identity: "9edf6e24c118c37b98268666e7943e14ad19d677a428d67a5eb7e02f3a9be961",
                    codec_identity: "36634f7d7384374f72e2c0a1fa8f1ba59b54ec29f362f07bf9b1e59db5054d64",
                },
            ],
            evidence_identity: "04f27288be463627426b519e38bdd7de59115e645756ae98f1e20d614ebbcda0",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 4,
            parameter_size: 120,
            parameter_layout_hash: "623ae7d63fbae391d8d8181dc9ff4bf683f21ba11baf638b417666452a281c56",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 408,
                    layout_hash: "dd81f8fac04158b1b2d08f05d463fc0e8ed92d30a212efa2412ed0ff30361efe",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 600,
                    layout_hash: "3f52d7bc2a334329d4bd718ae035591ec70f315c535bf45b8efd795269c3f460",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 376,
                    layout_hash: "d6f924f72c1bb02e11f44ae4c50370b620ac0d3734dbbb4f6a86619d8c28f4be",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 120,
                    layout_hash: "8223182652d738d5add43ee04b1bb20de78d98c0a4057ca00378e48992652689",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
            ],
            evidence_identity: "04f27288be463627426b519e38bdd7de59115e645756ae98f1e20d614ebbcda0",
        },
    },
    TrustedOperationContract {
        compatibility_name: "censorize",
        rust_id: "rusttable.censorize",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 1,
            parameter_size: 16,
            parameter_layout_hash: "0c6cb8df7815de03ebd74a8e86766e37e0a4f129ac601f46e73eb6a2d303fbb1",
            native_order: Some(149),
            reference_path: Some("src/iop/censorize.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "ffa98e011e79692d70214aff96bb708b886c239151f3760a0a7fd4eb4374cf0f",
            codec_identity: "e7ce053b9b80badd634517649ca49734459fe92315edc285764d27adba44040f",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 16,
                layout_hash: "b3468f323572987bf4133f5987e8abd08e7e82948995fb4af3c3f73bd10f7d41",
                decoder: "generated.bytes.decode.v1",
                opaque_blocking: false,
                abi_identity: "ffa98e011e79692d70214aff96bb708b886c239151f3760a0a7fd4eb4374cf0f",
                codec_identity: "e7ce053b9b80badd634517649ca49734459fe92315edc285764d27adba44040f",
            }],
            evidence_identity: "c1d727c5452f102c3d84169b226f81c15564b5546bbf95f2f10290e7013ced31",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 1,
            parameter_size: 16,
            parameter_layout_hash: "130861a2e11e76234ae18c2f4a5627ccfad5e5cc568c8777151f018657fbad77",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 16,
                layout_hash: "b3468f323572987bf4133f5987e8abd08e7e82948995fb4af3c3f73bd10f7d41",
                decoder: "opaque",
                opaque_blocking: true,
                abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            }],
            evidence_identity: "c1d727c5452f102c3d84169b226f81c15564b5546bbf95f2f10290e7013ced31",
        },
    },
    TrustedOperationContract {
        compatibility_name: "channelmixer",
        rust_id: "rusttable.channelmixer",
        descriptor_version: 2,
        parameter_version: 2,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 2,
            parameter_size: 88,
            parameter_layout_hash: "a73fafce2ca050c9a793a65d30055bf2004c0415a0540d6c083343d0e8fe166a",
            native_order: Some(106),
            reference_path: Some("src/iop/channelmixer.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "1fe1190a9f9653b4e26333441f85fe1b705c23a1c8c6ea5b8b902f2994882b06",
            codec_identity: "c0cec59dc4e59d26ba84d123cdd0ffc5ad08ea1ad4dc3043ba493783d49693f6",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 84,
                    layout_hash: "bf475015610ac5744012623d0004140d04a8e20ce9903ab87502353c90e30b0c",
                    decoder: "rusttable.channelmixer.decode.v1",
                    opaque_blocking: false,
                    abi_identity: "ca13b34df0a2d1e84defb13e4be994bd0817549da9d62141ea2d2d1899d9848b",
                    codec_identity: "80e4e43162faf41bd07eaf2c25e826b129ea4907354ef0e32e35b0e528b024fb",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 88,
                    layout_hash: "a73fafce2ca050c9a793a65d30055bf2004c0415a0540d6c083343d0e8fe166a",
                    decoder: "rusttable.channelmixer.decode.v2",
                    opaque_blocking: false,
                    abi_identity: "1fe1190a9f9653b4e26333441f85fe1b705c23a1c8c6ea5b8b902f2994882b06",
                    codec_identity: "c0cec59dc4e59d26ba84d123cdd0ffc5ad08ea1ad4dc3043ba493783d49693f6",
                },
            ],
            evidence_identity: "719a32a45b94c8b26433e1ce56e632e82fde7958cdc13da5c43a72be20c79cf9",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 2,
            parameter_size: 88,
            parameter_layout_hash: "a73fafce2ca050c9a793a65d30055bf2004c0415a0540d6c083343d0e8fe166a",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 84,
                    layout_hash: "bf475015610ac5744012623d0004140d04a8e20ce9903ab87502353c90e30b0c",
                    decoder: "rusttable.channelmixer.decode.v1",
                    opaque_blocking: false,
                    abi_identity: "ca13b34df0a2d1e84defb13e4be994bd0817549da9d62141ea2d2d1899d9848b",
                    codec_identity: "80e4e43162faf41bd07eaf2c25e826b129ea4907354ef0e32e35b0e528b024fb",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 88,
                    layout_hash: "a73fafce2ca050c9a793a65d30055bf2004c0415a0540d6c083343d0e8fe166a",
                    decoder: "rusttable.channelmixer.decode.v2",
                    opaque_blocking: false,
                    abi_identity: "1fe1190a9f9653b4e26333441f85fe1b705c23a1c8c6ea5b8b902f2994882b06",
                    codec_identity: "c0cec59dc4e59d26ba84d123cdd0ffc5ad08ea1ad4dc3043ba493783d49693f6",
                },
            ],
            evidence_identity: "719a32a45b94c8b26433e1ce56e632e82fde7958cdc13da5c43a72be20c79cf9",
        },
    },
    TrustedOperationContract {
        compatibility_name: "clahe",
        rust_id: "rusttable.clahe",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 1,
            parameter_size: 0,
            parameter_layout_hash: "",
            native_order: Some(101),
            reference_path: Some("src/iop/clahe.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[],
            evidence_identity: "c9881aad12864cc95cbca2b0c92721acac213c02844bfb625c1e9d0ee492e554",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 1,
            parameter_size: 0,
            parameter_layout_hash: "",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[],
            evidence_identity: "c9881aad12864cc95cbca2b0c92721acac213c02844bfb625c1e9d0ee492e554",
        },
    },
    TrustedOperationContract {
        compatibility_name: "clipping",
        rust_id: "rusttable.clipping",
        descriptor_version: 5,
        parameter_version: 5,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 5,
            parameter_size: 84,
            parameter_layout_hash: "2b7e3636620a38a0eeee4c712c3bbe4e5d4eff358e051c0b008d5411191f6a06",
            native_order: Some(86),
            reference_path: Some("src/iop/clipping.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "eda69be5d454d86630315d1d4e2fcdb2b866df85457a519bceef7ccd0deb2f49",
            codec_identity: "cf9c027e322e6c9f5a9ffeaf61d157888cd37851cbb16d971f6051d333403d93",
            parameter_versions: &[TrustedVersion {
                version: 5,
                byte_size: 84,
                layout_hash: "fb3cc2fcd9ca21a31e9b15df996fde646d695e3b9622123ef900f98cd5770bdf",
                decoder: "generated.bytes.decode.v5",
                opaque_blocking: false,
                abi_identity: "eda69be5d454d86630315d1d4e2fcdb2b866df85457a519bceef7ccd0deb2f49",
                codec_identity: "cf9c027e322e6c9f5a9ffeaf61d157888cd37851cbb16d971f6051d333403d93",
            }],
            evidence_identity: "5c6a5bb860ec57a60834a5a464deb356a3f4856acf13018d2aac6c0e76437409",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 5,
            parameter_size: 84,
            parameter_layout_hash: "c4868c59f0d6e757e56dc310c647535e562d2ed1dbea67ece61ad40d9d975701",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 5,
                byte_size: 84,
                layout_hash: "fb3cc2fcd9ca21a31e9b15df996fde646d695e3b9622123ef900f98cd5770bdf",
                decoder: "opaque",
                opaque_blocking: true,
                abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            }],
            evidence_identity: "5c6a5bb860ec57a60834a5a464deb356a3f4856acf13018d2aac6c0e76437409",
        },
    },
    TrustedOperationContract {
        compatibility_name: "colisa",
        rust_id: "rusttable.colisa",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: NATIVE_SOURCE_COMMIT,
            source_snapshot: NATIVE_SOURCE_COMMIT,
            module_version: 1,
            parameter_size: 12,
            parameter_layout_hash: "1cdf4b1c74dea5b674e66a56795656a7a01cca14c97fbf0834c0bfbc2f608ae4",
            native_order: Some(74),
            reference_path: Some("src/iop/colisa.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "ad658933041d7e0cae4758fe697dab72b9978945b23647129d67a74be2c861fd",
            codec_identity: "40224b3a3d844ceb0a4c1119e373d1f7733ea44adc6a6c1047a4c642511d1529",
            parameter_versions: COLISA_NATIVE_VERSIONS,
            evidence_identity: "d146b2c06de778626b2f346a296d822ad1835053d1a03f7903a64fafdbb76bfc",
        },
        audited: TrustedRecord {
            source_commit: NATIVE_SOURCE_COMMIT,
            source_snapshot: AUDITED_SOURCE_SNAPSHOT,
            module_version: 1,
            parameter_size: 12,
            parameter_layout_hash: "fa36b37d71742a8882fac942653022dbf0209281bd67a16dceb116e93ff67ab8",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: COLISA_AUDITED_VERSIONS,
            evidence_identity: "d146b2c06de778626b2f346a296d822ad1835053d1a03f7903a64fafdbb76bfc",
        },
    },
    TrustedOperationContract {
        compatibility_name: "colorcontrast",
        rust_id: "rusttable.colorcontrast",
        descriptor_version: 2,
        parameter_version: 2,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 2,
            parameter_size: 20,
            parameter_layout_hash: "1384abf72e8a84a45fb18496ecc030c29a8008ea30d60b4790b94d5b5c4241ef",
            native_order: Some(124),
            reference_path: Some("src/iop/colorcontrast.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "77eaaf2a0d1a4f97b2fb82a93aa1bc755438621379fdb7a08d39c7c6d6333370",
            codec_identity: "c42e56bf5e8c09ee5d857f925ac6ac5842566e689644ea4716bc916bbaf3f7d3",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 16,
                    layout_hash: "80865022a8d2899440cb76b62f75d8dc4a51d433f835a14d0f93f0d6423b9f93",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 20,
                    layout_hash: "9c1d07186eed3ffadf05614c38601ea4da509e940d98ff12301a0e1295192749",
                    decoder: "generated.bytes.decode.v2",
                    opaque_blocking: false,
                    abi_identity: "77eaaf2a0d1a4f97b2fb82a93aa1bc755438621379fdb7a08d39c7c6d6333370",
                    codec_identity: "c42e56bf5e8c09ee5d857f925ac6ac5842566e689644ea4716bc916bbaf3f7d3",
                },
            ],
            evidence_identity: "e6fb7e9bab4f47c46374d47925d409067c027fbd8509526d0d221161e9722f09",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 2,
            parameter_size: 20,
            parameter_layout_hash: "efcfc84077a8c7d10001872b1a8ded1f1042e808eb74767d75e536690e73d00d",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 16,
                    layout_hash: "80865022a8d2899440cb76b62f75d8dc4a51d433f835a14d0f93f0d6423b9f93",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 20,
                    layout_hash: "9c1d07186eed3ffadf05614c38601ea4da509e940d98ff12301a0e1295192749",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
            ],
            evidence_identity: "e6fb7e9bab4f47c46374d47925d409067c027fbd8509526d0d221161e9722f09",
        },
    },
    TrustedOperationContract {
        compatibility_name: "colorcorrection",
        rust_id: "rusttable.colorcorrection",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 1,
            parameter_size: 20,
            parameter_layout_hash: "1384abf72e8a84a45fb18496ecc030c29a8008ea30d60b4790b94d5b5c4241ef",
            native_order: Some(77),
            reference_path: Some("src/iop/colorcorrection.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "77eaaf2a0d1a4f97b2fb82a93aa1bc755438621379fdb7a08d39c7c6d6333370",
            codec_identity: "fe5a0a3274116794cea935f9b95fd287d5fe33d0169712500baf63e313865a5b",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 20,
                layout_hash: "c06df7180f2b0334524d9a617a4aea3d2227b73286ea82d50ef59fa20de38b03",
                decoder: "generated.bytes.decode.v1",
                opaque_blocking: false,
                abi_identity: "77eaaf2a0d1a4f97b2fb82a93aa1bc755438621379fdb7a08d39c7c6d6333370",
                codec_identity: "fe5a0a3274116794cea935f9b95fd287d5fe33d0169712500baf63e313865a5b",
            }],
            evidence_identity: "3cd4598a8550ba61439c53c8fe4650c0fd4c1388e1a8e5f310198607c0a23cc1",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 1,
            parameter_size: 20,
            parameter_layout_hash: "4f4b77ae74328d541fe19522281e69dfb474dcd589b6c65b6c429345cb9fad6e",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 20,
                layout_hash: "c06df7180f2b0334524d9a617a4aea3d2227b73286ea82d50ef59fa20de38b03",
                decoder: "opaque",
                opaque_blocking: true,
                abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            }],
            evidence_identity: "3cd4598a8550ba61439c53c8fe4650c0fd4c1388e1a8e5f310198607c0a23cc1",
        },
    },
    TrustedOperationContract {
        compatibility_name: "colorin",
        rust_id: "rusttable.colorin",
        descriptor_version: 7,
        parameter_version: 7,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 7,
            parameter_size: 1044,
            parameter_layout_hash: "a47122d6214f2a4a9eed7d56707586bec841f38844779f66929fb79cb3c474d5",
            native_order: Some(83),
            reference_path: Some("src/iop/colorin.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "7e3f691bd95df924ec8e10a96c192728f5679f1fef4aef5508ab36e49e368792",
            codec_identity: "461d80728b4ac8152ffa51813643fa4577825a4e955e511abbdec8a65e500535",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 3464,
                    layout_hash: "cd2e92c9cea90c4b016ece7e088543a19e411445695cfa8bd3d6c191f8e7e284",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 3704,
                    layout_hash: "9b8154bbf9cd5681eae87eddb6b766f189559b50b7a09b8bbe1b68645c9e54ce",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 3960,
                    layout_hash: "098155f3734c86541d34da621995323d3b574ee050abd4d51f0bdc7f48af9176",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 4208,
                    layout_hash: "2cd9184dbfe76d93be69ccdd84a13569e04c56357c2257389dd7e7f534e103f8",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 5,
                    byte_size: 4520,
                    layout_hash: "3521cdde8b4b9028936ce5e2a0ccd82d39ff4d395f0b8aef6a1c512ede052e5b",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 6,
                    byte_size: 5656,
                    layout_hash: "c94c2bb2ab11b32d858a5e974b9543a90eb6420a32709aab1ecd01320473c2e2",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 7,
                    byte_size: 1044,
                    layout_hash: "3d395a1249d8b4a2166e25f6e323671d04df96678803980e2c6d143913e4d0ef",
                    decoder: "generated.bytes.decode.v7",
                    opaque_blocking: false,
                    abi_identity: "7e3f691bd95df924ec8e10a96c192728f5679f1fef4aef5508ab36e49e368792",
                    codec_identity: "461d80728b4ac8152ffa51813643fa4577825a4e955e511abbdec8a65e500535",
                },
            ],
            evidence_identity: "708d9329b446512b74146df9cde7d2f43b2dc9317854f43af4abcaa80728a0df",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 7,
            parameter_size: 1044,
            parameter_layout_hash: "ac2f16bb989ba014e80a9e5b4e23037334330a558c9ff31b14720ffa59b132fb",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 3464,
                    layout_hash: "cd2e92c9cea90c4b016ece7e088543a19e411445695cfa8bd3d6c191f8e7e284",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 3704,
                    layout_hash: "9b8154bbf9cd5681eae87eddb6b766f189559b50b7a09b8bbe1b68645c9e54ce",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 3960,
                    layout_hash: "098155f3734c86541d34da621995323d3b574ee050abd4d51f0bdc7f48af9176",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 4208,
                    layout_hash: "2cd9184dbfe76d93be69ccdd84a13569e04c56357c2257389dd7e7f534e103f8",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 5,
                    byte_size: 4520,
                    layout_hash: "3521cdde8b4b9028936ce5e2a0ccd82d39ff4d395f0b8aef6a1c512ede052e5b",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 6,
                    byte_size: 5656,
                    layout_hash: "c94c2bb2ab11b32d858a5e974b9543a90eb6420a32709aab1ecd01320473c2e2",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 7,
                    byte_size: 1044,
                    layout_hash: "3d395a1249d8b4a2166e25f6e323671d04df96678803980e2c6d143913e4d0ef",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
            ],
            evidence_identity: "708d9329b446512b74146df9cde7d2f43b2dc9317854f43af4abcaa80728a0df",
        },
    },
    TrustedOperationContract {
        compatibility_name: "colormapping",
        rust_id: "rusttable.colormapping",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 1,
            parameter_size: 16600,
            parameter_layout_hash: "59b200251af9c46804ed6e5c0d7fb20ef961036a6de2d0d9ec7fda9698da2bc6",
            native_order: Some(105),
            reference_path: Some("src/iop/colormapping.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "d52b8af7512f50c270a5c726cc0de2989bf416e385d9413231ce4016783549d7",
            codec_identity: "ba098309b84a42ab54d56604a23c99be63732cc268c96430d5c7607d6404b807",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 16600,
                layout_hash: "9a802e076f0fe0c83968eef69acee347d616e45c1adf37bfe18a15c016048780",
                decoder: "rusttable.colormapping.decode.v1",
                opaque_blocking: false,
                abi_identity: "d52b8af7512f50c270a5c726cc0de2989bf416e385d9413231ce4016783549d7",
                codec_identity: "ba098309b84a42ab54d56604a23c99be63732cc268c96430d5c7607d6404b807",
            }],
            evidence_identity: "bcd889f9f405def00d7f90c3f4c05a4b0c665a697acf47d847dbfada12e87e05",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 1,
            parameter_size: 16600,
            parameter_layout_hash: "9a802e076f0fe0c83968eef69acee347d616e45c1adf37bfe18a15c016048780",
            native_order: Some(105),
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 16600,
                layout_hash: "9a802e076f0fe0c83968eef69acee347d616e45c1adf37bfe18a15c016048780",
                decoder: "rusttable.colormapping.decode.v1",
                opaque_blocking: false,
                abi_identity: "d52b8af7512f50c270a5c726cc0de2989bf416e385d9413231ce4016783549d7",
                codec_identity: "ba098309b84a42ab54d56604a23c99be63732cc268c96430d5c7607d6404b807",
            }],
            evidence_identity: "bcd889f9f405def00d7f90c3f4c05a4b0c665a697acf47d847dbfada12e87e05",
        },
    },
    TrustedOperationContract {
        compatibility_name: "colorout",
        rust_id: "rusttable.colorout",
        descriptor_version: 7,
        parameter_version: 7,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 5,
            parameter_size: 592,
            parameter_layout_hash: "311be64f054274c8dff3e8e1a6e8a6471e3fb1fc6da0cffe09bc4f22313e2cc3",
            native_order: Some(84),
            reference_path: Some("src/iop/colorout.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "75660b1bef23bd129a4c086362066e1ad14243807f7bea90caab740fac9d7386",
            codec_identity: "cace5daeb998dc610481d8e684f4c7e589b49971e49c8020dfce78c737ed9caf",
            parameter_versions: &[
                TrustedVersion {
                    version: 3,
                    byte_size: 1520,
                    layout_hash: "9feeec2058a446c5f4ab301cc848026d2ad260d906b3e8e06164879a7d520346",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 1712,
                    layout_hash: "bff1627ec61054b332809c3c59c9dbd572bd288f52ac1836d0c2d7e75cf51db0",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 5,
                    byte_size: 592,
                    layout_hash: "2d9da03cf85b980dc343dbdb66713252d6b1444b49161090acb3c77654366e8d",
                    decoder: "generated.bytes.decode.v5",
                    opaque_blocking: false,
                    abi_identity: "75660b1bef23bd129a4c086362066e1ad14243807f7bea90caab740fac9d7386",
                    codec_identity: "cace5daeb998dc610481d8e684f4c7e589b49971e49c8020dfce78c737ed9caf",
                },
            ],
            evidence_identity: "cbb2f387e801b0991fd5a6a59fcfa3d900ce0bd2f26013816aa3a592013fa044",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 5,
            parameter_size: 592,
            parameter_layout_hash: "c9f00068acc0240940812fb7c5f01428019c0bf3a989f8548467f31ffaa299f9",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 3,
                    byte_size: 1520,
                    layout_hash: "9feeec2058a446c5f4ab301cc848026d2ad260d906b3e8e06164879a7d520346",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 1712,
                    layout_hash: "bff1627ec61054b332809c3c59c9dbd572bd288f52ac1836d0c2d7e75cf51db0",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 5,
                    byte_size: 592,
                    layout_hash: "2d9da03cf85b980dc343dbdb66713252d6b1444b49161090acb3c77654366e8d",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
            ],
            evidence_identity: "cbb2f387e801b0991fd5a6a59fcfa3d900ce0bd2f26013816aa3a592013fa044",
        },
    },
    TrustedOperationContract {
        compatibility_name: "colorreconstruct",
        rust_id: "rusttable.colorreconstruct",
        descriptor_version: 3,
        parameter_version: 3,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 3,
            parameter_size: 20,
            parameter_layout_hash: "3756c4815d337d3909424f2833160234eb848b3d538059a748efd4f6c8ae98b8",
            native_order: Some(69),
            reference_path: Some("src/iop/colorreconstruction.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "77eaaf2a0d1a4f97b2fb82a93aa1bc755438621379fdb7a08d39c7c6d6333370",
            codec_identity: "f4b245f93400b322d64b909abecdb586e0f77e8bb1e028a973646c23daa976d1",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 12,
                    layout_hash: "463e0b63a879e781180102784f3c4842db824f4854f7cb5dcf9e735cbc7d91e7",
                    decoder: "rusttable.colorreconstruct.decode.v1",
                    opaque_blocking: false,
                    abi_identity: "1a63ef0ca65b46f7185ef41c0ad167a78e0e3a351f763badfa4529352ef65296",
                    codec_identity: "9ab24e6c9a6ad31a00c9e1a082b305bbaa98814d1b6ad741c988cb1446593ccd",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 16,
                    layout_hash: "84445661bafeda4e6626b3d6b9b7b6947a0a05b29ab92741e9a3f90f5cf0361d",
                    decoder: "rusttable.colorreconstruct.decode.v2",
                    opaque_blocking: false,
                    abi_identity: "8a3bbadba62135c4fe4913390abceb7dfcc1a3520d29122c6978568de9ac0254",
                    codec_identity: "e858350f4f21995baeccec1118d92d0009c0cc5fc7805198f6a8025b02c1893a",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 20,
                    layout_hash: "3756c4815d337d3909424f2833160234eb848b3d538059a748efd4f6c8ae98b8",
                    decoder: "rusttable.colorreconstruct.decode.v3",
                    opaque_blocking: false,
                    abi_identity: "f97beb7c1061f9c7a00389f7dcbd3ce1dfa12e8555b1d52a6acf8ece9703bd47",
                    codec_identity: "d5e9309647abb5227e89b6049649f1216ecc2d9ff2c070fa464ecb19e141f78f",
                },
            ],
            evidence_identity: "ea6ba04c5a3df40fc72def37cf76d115da8732995f4a071082913cec409a5f5e",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 3,
            parameter_size: 20,
            parameter_layout_hash: "3756c4815d337d3909424f2833160234eb848b3d538059a748efd4f6c8ae98b8",
            native_order: Some(69),
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 12,
                    layout_hash: "463e0b63a879e781180102784f3c4842db824f4854f7cb5dcf9e735cbc7d91e7",
                    decoder: "rusttable.colorreconstruct.decode.v1",
                    opaque_blocking: false,
                    abi_identity: "1a63ef0ca65b46f7185ef41c0ad167a78e0e3a351f763badfa4529352ef65296",
                    codec_identity: "9ab24e6c9a6ad31a00c9e1a082b305bbaa98814d1b6ad741c988cb1446593ccd",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 16,
                    layout_hash: "84445661bafeda4e6626b3d6b9b7b6947a0a05b29ab92741e9a3f90f5cf0361d",
                    decoder: "rusttable.colorreconstruct.decode.v2",
                    opaque_blocking: false,
                    abi_identity: "8a3bbadba62135c4fe4913390abceb7dfcc1a3520d29122c6978568de9ac0254",
                    codec_identity: "e858350f4f21995baeccec1118d92d0009c0cc5fc7805198f6a8025b02c1893a",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 20,
                    layout_hash: "3756c4815d337d3909424f2833160234eb848b3d538059a748efd4f6c8ae98b8",
                    decoder: "rusttable.colorreconstruct.decode.v3",
                    opaque_blocking: false,
                    abi_identity: "f97beb7c1061f9c7a00389f7dcbd3ce1dfa12e8555b1d52a6acf8ece9703bd47",
                    codec_identity: "d5e9309647abb5227e89b6049649f1216ecc2d9ff2c070fa464ecb19e141f78f",
                },
            ],
            evidence_identity: "710520589637748ad01e5bb368c69eb8ba003ae73f885ff003424368c092dc76",
        },
    },
    TrustedOperationContract {
        compatibility_name: "colortransfer",
        rust_id: "rusttable.colortransfer",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 1,
            parameter_size: 8280,
            parameter_layout_hash: "6185c59d80b7cb58a6de9ba90f32f657e7dca69d44f415d0e8c3772997ef5832",
            native_order: Some(104),
            reference_path: Some("src/iop/colortransfer.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "98c8eedfde8f6e48b23a9407d6c96d1ba137055258dc12bbac8e28bd550968ff",
            codec_identity: "fc2e7828e6100aaea39f230c69f12cf25df29e992d2eac4ade742bf20bdef3bb",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 8280,
                layout_hash: "291eb0dc486255d733b5f8c68ae2d210e9889d8d0e96db37384b18754ef818dc",
                decoder: "rusttable.colortransfer.decode.v1",
                opaque_blocking: false,
                abi_identity: "98c8eedfde8f6e48b23a9407d6c96d1ba137055258dc12bbac8e28bd550968ff",
                codec_identity: "fc2e7828e6100aaea39f230c69f12cf25df29e992d2eac4ade742bf20bdef3bb",
            }],
            evidence_identity: "0fd0fd775a82b366ef0f550550b33fa06e2450ad0fb14fa00826489230328512",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 1,
            parameter_size: 8280,
            parameter_layout_hash: "291eb0dc486255d733b5f8c68ae2d210e9889d8d0e96db37384b18754ef818dc",
            native_order: Some(104),
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 8280,
                layout_hash: "291eb0dc486255d733b5f8c68ae2d210e9889d8d0e96db37384b18754ef818dc",
                decoder: "rusttable.colortransfer.decode.v1",
                opaque_blocking: false,
                abi_identity: "98c8eedfde8f6e48b23a9407d6c96d1ba137055258dc12bbac8e28bd550968ff",
                codec_identity: "fc2e7828e6100aaea39f230c69f12cf25df29e992d2eac4ade742bf20bdef3bb",
            }],
            evidence_identity: "0fd0fd775a82b366ef0f550550b33fa06e2450ad0fb14fa00826489230328512",
        },
    },
    TrustedOperationContract {
        compatibility_name: "colorzones",
        rust_id: "rusttable.colorzones",
        descriptor_version: 5,
        parameter_version: 5,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 5,
            parameter_size: 520,
            parameter_layout_hash: "6cfbf9771cfafa954a0e598005128c1185b2863487351577e5dcfc0882eddead",
            native_order: Some(93),
            reference_path: Some("src/iop/colorzones.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "bff243ea0326136775ca8280bd200a3853e06e9f3166cc5a2225478df4d89d3e",
            codec_identity: "03691123ad70d938ed85512d2c9492cfcb9de3b4d6862ddf4e14e053c039f793",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 148,
                    layout_hash: "4aad0d4b8de355275e5dafdd3d8f7d5c8627625b7bf10304bbfbafa5ebca331e",
                    decoder: "rusttable.colorzones.decode.v1",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 196,
                    layout_hash: "6db006a7fe3b1affed7aa2d446c4f0930e364156d5b90e1ebb8dbd3ca3a2755d",
                    decoder: "rusttable.colorzones.decode.v2",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 200,
                    layout_hash: "c89fbcba828e6a86d2f0acb7e5377377a5a8116c90832c9f88ee1b6f84e742de",
                    decoder: "rusttable.colorzones.decode.v3",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 516,
                    layout_hash: "bf883718fb270969f0efbdea2ea70faf9ee1fe2a10584e27b036027185b30ede",
                    decoder: "rusttable.colorzones.decode.v4",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 5,
                    byte_size: 520,
                    layout_hash: "6cfbf9771cfafa954a0e598005128c1185b2863487351577e5dcfc0882eddead",
                    decoder: "rusttable.colorzones.decode.v5",
                    opaque_blocking: false,
                    abi_identity: "bff243ea0326136775ca8280bd200a3853e06e9f3166cc5a2225478df4d89d3e",
                    codec_identity: "03691123ad70d938ed85512d2c9492cfcb9de3b4d6862ddf4e14e053c039f793",
                },
            ],
            evidence_identity: "048ab84e7e87083752be5d6f92e1aa29a9ab8869f47205aff60d4eb7476f85a0",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 5,
            parameter_size: 520,
            parameter_layout_hash: "6cfbf9771cfafa954a0e598005128c1185b2863487351577e5dcfc0882eddead",
            native_order: Some(60),
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 148,
                    layout_hash: "4aad0d4b8de355275e5dafdd3d8f7d5c8627625b7bf10304bbfbafa5ebca331e",
                    decoder: "rusttable.colorzones.decode.v1",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 196,
                    layout_hash: "6db006a7fe3b1affed7aa2d446c4f0930e364156d5b90e1ebb8dbd3ca3a2755d",
                    decoder: "rusttable.colorzones.decode.v2",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 200,
                    layout_hash: "c89fbcba828e6a86d2f0acb7e5377377a5a8116c90832c9f88ee1b6f84e742de",
                    decoder: "rusttable.colorzones.decode.v3",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 516,
                    layout_hash: "bf883718fb270969f0efbdea2ea70faf9ee1fe2a10584e27b036027185b30ede",
                    decoder: "rusttable.colorzones.decode.v4",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 5,
                    byte_size: 520,
                    layout_hash: "6cfbf9771cfafa954a0e598005128c1185b2863487351577e5dcfc0882eddead",
                    decoder: "rusttable.colorzones.decode.v5",
                    opaque_blocking: false,
                    abi_identity: "bff243ea0326136775ca8280bd200a3853e06e9f3166cc5a2225478df4d89d3e",
                    codec_identity: "03691123ad70d938ed85512d2c9492cfcb9de3b4d6862ddf4e14e053c039f793",
                },
            ],
            evidence_identity: "048ab84e7e87083752be5d6f92e1aa29a9ab8869f47205aff60d4eb7476f85a0",
        },
    },
    TrustedOperationContract {
        compatibility_name: "crop",
        rust_id: "rusttable.crop",
        descriptor_version: 1,
        parameter_version: 3,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 3,
            parameter_size: 32,
            parameter_layout_hash: "02bf7e8cfb69be6a9f9ff866e03456df5c7664c99674cbe8419048574f7ec299",
            native_order: Some(88),
            reference_path: Some("src/iop/crop.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "7537bb07233f4f21d961758ba08465093188c4f5652209bcd6fa0353dc03204a",
            codec_identity: "dcbc495f3044c6df8e7e8fca5848b31dd2f214a144a322f119887fafaefcc8e4",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 184,
                    layout_hash: "8279fc52af3e00b8c08586a8f55b81ab159a79a0952c27a0c3528a264504004b",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 208,
                    layout_hash: "7fd63ae665306d6e510e54306954c60a3dd6125eccdfe5254e06b9c484d0e899",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 32,
                    layout_hash: "3d85c2b67f2f4770ed6a2fe860674a2613332c2f8cca6a45b67eccd8f30b6359",
                    decoder: "generated.bytes.decode.v3",
                    opaque_blocking: false,
                    abi_identity: "7537bb07233f4f21d961758ba08465093188c4f5652209bcd6fa0353dc03204a",
                    codec_identity: "dcbc495f3044c6df8e7e8fca5848b31dd2f214a144a322f119887fafaefcc8e4",
                },
            ],
            evidence_identity: "c6b1b95ebd8892dff1e79e851efaead7ac98423582be24e7505eacba88717c76",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 3,
            parameter_size: 32,
            parameter_layout_hash: "02fcefdf6432c20b0f6a3213ff479120a5708935da45fa1280ce0023aa8bff40",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 184,
                    layout_hash: "8279fc52af3e00b8c08586a8f55b81ab159a79a0952c27a0c3528a264504004b",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 208,
                    layout_hash: "7fd63ae665306d6e510e54306954c60a3dd6125eccdfe5254e06b9c484d0e899",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 32,
                    layout_hash: "3d85c2b67f2f4770ed6a2fe860674a2613332c2f8cca6a45b67eccd8f30b6359",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
            ],
            evidence_identity: "c6b1b95ebd8892dff1e79e851efaead7ac98423582be24e7505eacba88717c76",
        },
    },
    TrustedOperationContract {
        compatibility_name: "defringe",
        rust_id: "rusttable.defringe",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 1,
            parameter_size: 12,
            parameter_layout_hash: "1cdf4b1c74dea5b674e66a56795656a7a01cca14c97fbf0834c0bfbc2f608ae4",
            native_order: Some(135),
            reference_path: Some("src/iop/defringe.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "ad658933041d7e0cae4758fe697dab72b9978945b23647129d67a74be2c861fd",
            codec_identity: "40224b3a3d844ceb0a4c1119e373d1f7733ea44adc6a6c1047a4c642511d1529",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 12,
                layout_hash: "45f8915ceb9c2b9dc01aab16ec2967871392b47899c0543610a232f34c3908c3",
                decoder: "generated.bytes.decode.v1",
                opaque_blocking: false,
                abi_identity: "ad658933041d7e0cae4758fe697dab72b9978945b23647129d67a74be2c861fd",
                codec_identity: "40224b3a3d844ceb0a4c1119e373d1f7733ea44adc6a6c1047a4c642511d1529",
            }],
            evidence_identity: "006a588ec1b08cb297b29ed5e8c548f61b73eb57699cae443519f8f1535455ff",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 1,
            parameter_size: 12,
            parameter_layout_hash: "2e69569f949d9974092231e8fc2fdc487dafc3dbad907fcd74ebd5c324881aaa",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 12,
                layout_hash: "45f8915ceb9c2b9dc01aab16ec2967871392b47899c0543610a232f34c3908c3",
                decoder: "opaque",
                opaque_blocking: true,
                abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            }],
            evidence_identity: "006a588ec1b08cb297b29ed5e8c548f61b73eb57699cae443519f8f1535455ff",
        },
    },
    TrustedOperationContract {
        compatibility_name: "dither",
        rust_id: "rusttable.dither",
        descriptor_version: 2,
        parameter_version: 2,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 2,
            parameter_size: 36,
            parameter_layout_hash: "1181b2314586f87c4b06f1ea940a3a65f8799f80bc6721ed705bf24ab5e4346a",
            native_order: Some(90),
            reference_path: Some("src/iop/dither.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "27ec376424217bc6405d2ef93b7642b74ae9d0a3ffa3d0123863a18ff95adc16",
            codec_identity: "0768cea5275a9a529afdeb3e69bf9a65b4d4e87fa649eed37d32468d6fff47dd",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 168,
                    layout_hash: "c3293417bac6348e7f66d2ebf192a87c10383ce12ee485df0cc6d5ffc7f24c67",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 36,
                    layout_hash: "6bbd405408b0cf8fb0dd24c8e67e480bde75af91e79ff212cbc66c774258e4c8",
                    decoder: "generated.bytes.decode.v2",
                    opaque_blocking: false,
                    abi_identity: "27ec376424217bc6405d2ef93b7642b74ae9d0a3ffa3d0123863a18ff95adc16",
                    codec_identity: "0768cea5275a9a529afdeb3e69bf9a65b4d4e87fa649eed37d32468d6fff47dd",
                },
            ],
            evidence_identity: "caaa03041dc1861e2fc4801f7d55beb603b4b16e68cf6e493d3cdc65b258a46e",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 2,
            parameter_size: 36,
            parameter_layout_hash: "d09e955c9d5b9b1cc68287a453bbd24f657254b43969a1875b27f23bfeb71755",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 168,
                    layout_hash: "c3293417bac6348e7f66d2ebf192a87c10383ce12ee485df0cc6d5ffc7f24c67",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 36,
                    layout_hash: "6bbd405408b0cf8fb0dd24c8e67e480bde75af91e79ff212cbc66c774258e4c8",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
            ],
            evidence_identity: "caaa03041dc1861e2fc4801f7d55beb603b4b16e68cf6e493d3cdc65b258a46e",
        },
    },
    TrustedOperationContract {
        compatibility_name: "enlargecanvas",
        rust_id: "rusttable.enlargecanvas",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 1,
            parameter_size: 20,
            parameter_layout_hash: "6e309af2482880a1c4aa9a01951329046c5187d67f1448b498f3f5ce8d4f8c97",
            native_order: Some(87),
            reference_path: Some("src/iop/enlargecanvas.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "77eaaf2a0d1a4f97b2fb82a93aa1bc755438621379fdb7a08d39c7c6d6333370",
            codec_identity: "fe5a0a3274116794cea935f9b95fd287d5fe33d0169712500baf63e313865a5b",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 20,
                layout_hash: "6e309af2482880a1c4aa9a01951329046c5187d67f1448b498f3f5ce8d4f8c97",
                decoder: "rusttable.enlargecanvas.decode.v1",
                opaque_blocking: false,
                abi_identity: "61417638b366cd93b94845f977468b393da83854750f4c33af992701e9ede832",
                codec_identity: "94eab37bc6f615d32e679b8ab23ec1a0ab82d43c938f641059febfce4febb65e",
            }],
            evidence_identity: "9f136d5914b5dd0bcec7cb36dfd906ca8430082f02c03cf4b7da2bf1717c3993",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 1,
            parameter_size: 20,
            parameter_layout_hash: "6e309af2482880a1c4aa9a01951329046c5187d67f1448b498f3f5ce8d4f8c97",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 20,
                layout_hash: "6e309af2482880a1c4aa9a01951329046c5187d67f1448b498f3f5ce8d4f8c97",
                decoder: "rusttable.enlargecanvas.decode.v1",
                opaque_blocking: false,
                abi_identity: "61417638b366cd93b94845f977468b393da83854750f4c33af992701e9ede832",
                codec_identity: "94eab37bc6f615d32e679b8ab23ec1a0ab82d43c938f641059febfce4febb65e",
            }],
            evidence_identity: "9f136d5914b5dd0bcec7cb36dfd906ca8430082f02c03cf4b7da2bf1717c3993",
        },
    },
    TrustedOperationContract {
        compatibility_name: "exposure",
        rust_id: "rusttable.exposure",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 7,
            parameter_size: 28,
            parameter_layout_hash: "b74d61e97c5f8d4d7053c1712e9f040290af18ae81a137ccf8639e2f724594af",
            native_order: Some(79),
            reference_path: Some("src/iop/exposure.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "d0c15b0308dd1b8e46337fbbcd9b626eaf869c002b6d07e0c5b4f3fb982312da",
            codec_identity: "b70d5e09fb4dab85d5e77a604681700bfa538db708fdd306a3a599bda71ea20a",
            parameter_versions: &[
                TrustedVersion {
                    version: 2,
                    byte_size: 320,
                    layout_hash: "d92a05ff72ab1777ced1c6a0b7a8044d5d67d473146bd0ed27cc80927d86e34b",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 416,
                    layout_hash: "1d8c76f0038bc324cb6c52c34a6087fbbe76fb535d3b13c1f5a331d8bdebbc3e",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 512,
                    layout_hash: "32841e2d12a80e34962f44b8a9f46bf565158010d1edc464f27007777b8cc2ff",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 5,
                    byte_size: 608,
                    layout_hash: "09e5b0d573d9380f281d9d03f630ca52a97e0e9c88f45616022f13a63db9218a",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 6,
                    byte_size: 288,
                    layout_hash: "98511744689415295929efd0f901276542d3b70443119257902daf73cb801979",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 7,
                    byte_size: 28,
                    layout_hash: "75d6ed5a57578d2eea9c0c93d0bbc56be2e01ddd35c120c2c6b989f9c4df770a",
                    decoder: "generated.bytes.decode.v7",
                    opaque_blocking: false,
                    abi_identity: "d0c15b0308dd1b8e46337fbbcd9b626eaf869c002b6d07e0c5b4f3fb982312da",
                    codec_identity: "b70d5e09fb4dab85d5e77a604681700bfa538db708fdd306a3a599bda71ea20a",
                },
            ],
            evidence_identity: "01bb86106d7833134ec6fde18faea2b6c5e6bee5dbe457ac511cf44a65a1c6d8",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 7,
            parameter_size: 28,
            parameter_layout_hash: "a7bb98f28018c4a80062e7006e0b255d21c880a1ebaba19a488dcf75adeba1e6",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 2,
                    byte_size: 320,
                    layout_hash: "d92a05ff72ab1777ced1c6a0b7a8044d5d67d473146bd0ed27cc80927d86e34b",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 416,
                    layout_hash: "1d8c76f0038bc324cb6c52c34a6087fbbe76fb535d3b13c1f5a331d8bdebbc3e",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 512,
                    layout_hash: "32841e2d12a80e34962f44b8a9f46bf565158010d1edc464f27007777b8cc2ff",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 5,
                    byte_size: 608,
                    layout_hash: "09e5b0d573d9380f281d9d03f630ca52a97e0e9c88f45616022f13a63db9218a",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 6,
                    byte_size: 288,
                    layout_hash: "98511744689415295929efd0f901276542d3b70443119257902daf73cb801979",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 7,
                    byte_size: 28,
                    layout_hash: "75d6ed5a57578d2eea9c0c93d0bbc56be2e01ddd35c120c2c6b989f9c4df770a",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
            ],
            evidence_identity: "01bb86106d7833134ec6fde18faea2b6c5e6bee5dbe457ac511cf44a65a1c6d8",
        },
    },
    TrustedOperationContract {
        compatibility_name: "finalscale",
        rust_id: "rusttable.finalscale",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 1,
            parameter_size: 4,
            parameter_layout_hash: "389be5cfd30b667cc11cdd0cf2d1bcc2ed33a423e5ae9043bcd31ff451192627",
            native_order: Some(131),
            reference_path: Some("src/iop/finalscale.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "42a719556434cc07fb5a9c299664f5c5e0645ade6436ab15d9161638c7c553fc",
            codec_identity: "71604b673da1a1e1ad40aec5259e7bdb3ccdb68cb869907cc48c97e29380ff23",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 4,
                layout_hash: "0e4dd01deab1c6890d46577f1f5bdcdbc17f6b6c281dd9c25edafb3b25fcf87f",
                decoder: "generated.bytes.decode.v1",
                opaque_blocking: false,
                abi_identity: "42a719556434cc07fb5a9c299664f5c5e0645ade6436ab15d9161638c7c553fc",
                codec_identity: "71604b673da1a1e1ad40aec5259e7bdb3ccdb68cb869907cc48c97e29380ff23",
            }],
            evidence_identity: "03712f9dfa0612481c5005b9ad814abfb268844d056b3e85e300a3979d48d3ee",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 1,
            parameter_size: 4,
            parameter_layout_hash: "d504dd2944afc71c4b4f60b6c95be03ccf462c00bbe9274ea1e92a77b37d12fb",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 4,
                layout_hash: "0e4dd01deab1c6890d46577f1f5bdcdbc17f6b6c281dd9c25edafb3b25fcf87f",
                decoder: "opaque",
                opaque_blocking: true,
                abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            }],
            evidence_identity: "03712f9dfa0612481c5005b9ad814abfb268844d056b3e85e300a3979d48d3ee",
        },
    },
    TrustedOperationContract {
        compatibility_name: "flip",
        rust_id: "rusttable.flip",
        descriptor_version: 1,
        parameter_version: 2,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 2,
            parameter_size: 4,
            parameter_layout_hash: "389be5cfd30b667cc11cdd0cf2d1bcc2ed33a423e5ae9043bcd31ff451192627",
            native_order: Some(130),
            reference_path: Some("src/iop/flip.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "42a719556434cc07fb5a9c299664f5c5e0645ade6436ab15d9161638c7c553fc",
            codec_identity: "b4ed946cccea685e722fb2535da5aec1f2dcfc480df7a424d2e82aa38f2cd7f0",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 168,
                    layout_hash: "1410e2fdf558de0d4b45233aa0741986f24af4ce291232beb67e7b5cf1c5a6cc",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 4,
                    layout_hash: "6028c58c578a148b3c81eba7123dade3725a382a75fe78f9e2c274abeb78fb23",
                    decoder: "generated.bytes.decode.v2",
                    opaque_blocking: false,
                    abi_identity: "42a719556434cc07fb5a9c299664f5c5e0645ade6436ab15d9161638c7c553fc",
                    codec_identity: "b4ed946cccea685e722fb2535da5aec1f2dcfc480df7a424d2e82aa38f2cd7f0",
                },
            ],
            evidence_identity: "a2933cfeebba0fc92d2096fa53b8b925fc049789d226d151bebefd61ef65ee46",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 2,
            parameter_size: 4,
            parameter_layout_hash: "877275314b31b86d36dcfc3f4f3de67bb60f46ebdd3340b77894fcf4288171e8",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 168,
                    layout_hash: "1410e2fdf558de0d4b45233aa0741986f24af4ce291232beb67e7b5cf1c5a6cc",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 4,
                    layout_hash: "6028c58c578a148b3c81eba7123dade3725a382a75fe78f9e2c274abeb78fb23",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
            ],
            evidence_identity: "a2933cfeebba0fc92d2096fa53b8b925fc049789d226d151bebefd61ef65ee46",
        },
    },
    TrustedOperationContract {
        compatibility_name: "graduatednd",
        rust_id: "rusttable.graduatednd",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 1,
            parameter_size: 24,
            parameter_layout_hash: "a3ec8e1f316560821b03f356a29f68a9b23a3646d64a2a081af6c9c4eed65f0c",
            native_order: Some(107),
            reference_path: Some("src/iop/graduatednd.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "06a897ae899047267365b955cc5b7b98806702110a0c4ee1d0e96968f962756e",
            codec_identity: "0788afc91702d878d06476a48f95a3dc9ea2fe563667c48c6e6bdb3d57836ead",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 24,
                layout_hash: "76f46e1764e033ec97bd700c8625d6cfe1d47fc304bfcd8119058c0977ad7a64",
                decoder: "generated.bytes.decode.v1",
                opaque_blocking: false,
                abi_identity: "06a897ae899047267365b955cc5b7b98806702110a0c4ee1d0e96968f962756e",
                codec_identity: "0788afc91702d878d06476a48f95a3dc9ea2fe563667c48c6e6bdb3d57836ead",
            }],
            evidence_identity: "8f24e2db15e54ff30e09625c541c49656cbbacb469b1b04504d8f9009ff26df2",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 1,
            parameter_size: 24,
            parameter_layout_hash: "98ca7ad4bcc4cb83683611be6f2dcedafbf66cec2dad28fc6f4f171b86c8202b",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 24,
                layout_hash: "76f46e1764e033ec97bd700c8625d6cfe1d47fc304bfcd8119058c0977ad7a64",
                decoder: "opaque",
                opaque_blocking: true,
                abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            }],
            evidence_identity: "8f24e2db15e54ff30e09625c541c49656cbbacb469b1b04504d8f9009ff26df2",
        },
    },
    TrustedOperationContract {
        compatibility_name: "grain",
        rust_id: "rusttable.grain",
        descriptor_version: 2,
        parameter_version: 2,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 2,
            parameter_size: 16,
            parameter_layout_hash: "0c6cb8df7815de03ebd74a8e86766e37e0a4f129ac601f46e73eb6a2d303fbb1",
            native_order: Some(100),
            reference_path: Some("src/iop/grain.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "ffa98e011e79692d70214aff96bb708b886c239151f3760a0a7fd4eb4374cf0f",
            codec_identity: "348407041101a60959f06f519558fd53856defb2a8ec88ba61d4842b014c3c70",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 65616,
                    layout_hash: "2935d7dccc0c1239d45e7d1286f037dd9929d580a3758b473a31abbebd2441f7",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 16,
                    layout_hash: "d0dc5297894971f9f9d842278638e0ea0bab7744840fc3731baeae3c28772425",
                    decoder: "generated.bytes.decode.v2",
                    opaque_blocking: false,
                    abi_identity: "ffa98e011e79692d70214aff96bb708b886c239151f3760a0a7fd4eb4374cf0f",
                    codec_identity: "348407041101a60959f06f519558fd53856defb2a8ec88ba61d4842b014c3c70",
                },
            ],
            evidence_identity: "a35c9f67ec905156a3f5b7bf0904f2cd35ee568430a1a52a45b2115fd7430db4",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 2,
            parameter_size: 16,
            parameter_layout_hash: "8790838641a33cc9014066071eff05133e54f8c046d35111e0fabcdca2f88e0d",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 65616,
                    layout_hash: "2935d7dccc0c1239d45e7d1286f037dd9929d580a3758b473a31abbebd2441f7",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 16,
                    layout_hash: "d0dc5297894971f9f9d842278638e0ea0bab7744840fc3731baeae3c28772425",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
            ],
            evidence_identity: "a35c9f67ec905156a3f5b7bf0904f2cd35ee568430a1a52a45b2115fd7430db4",
        },
    },
    TrustedOperationContract {
        compatibility_name: "highlights",
        rust_id: "rusttable.highlights",
        descriptor_version: 4,
        parameter_version: 4,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 4,
            parameter_size: 48,
            parameter_layout_hash: "bef1898313272008979391a01d624b21860930d7fe13149f5239e2fdbfc5845c",
            native_order: Some(94),
            reference_path: Some("src/iop/highlights.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "0fd8070451115c782c9065859d15c822da8c9baf88d5184ac369619145439749",
            codec_identity: "35caba032a9383a87a7929fe9cef0434170e5e065d351721d322915ac7f23b83",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 304,
                    layout_hash: "9b6eec87eef18f79b34650367cd81513951d724ab6f0fa0530872d5dbc4a9bc4",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 408,
                    layout_hash: "cfa2700cfe5656e10084ee5bac8f34dbcc638475d44b73a7fdcf3a7737081d37",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 528,
                    layout_hash: "e4a44e356e450ac08c38ca40129aaf8c0a181bc9cafbb8d3a442a212c77cb893",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 48,
                    layout_hash: "b0e073d174d54b6d7bb662bbdf9cf3f3af62ad243e583ca4c423d43ea5fef68f",
                    decoder: "generated.bytes.decode.v4",
                    opaque_blocking: false,
                    abi_identity: "0fd8070451115c782c9065859d15c822da8c9baf88d5184ac369619145439749",
                    codec_identity: "35caba032a9383a87a7929fe9cef0434170e5e065d351721d322915ac7f23b83",
                },
            ],
            evidence_identity: "d6267be48d18c844f178dfc61bb8cfeaa89538e1194bbae7d5c07e2172230f8c",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 4,
            parameter_size: 48,
            parameter_layout_hash: "ec41c54e8835c1396d556afd1a341ee078d372f49fdfb7a7675620c00579ffa5",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 304,
                    layout_hash: "9b6eec87eef18f79b34650367cd81513951d724ab6f0fa0530872d5dbc4a9bc4",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 408,
                    layout_hash: "cfa2700cfe5656e10084ee5bac8f34dbcc638475d44b73a7fdcf3a7737081d37",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 528,
                    layout_hash: "e4a44e356e450ac08c38ca40129aaf8c0a181bc9cafbb8d3a442a212c77cb893",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 48,
                    layout_hash: "b0e073d174d54b6d7bb662bbdf9cf3f3af62ad243e583ca4c423d43ea5fef68f",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
            ],
            evidence_identity: "d6267be48d18c844f178dfc61bb8cfeaa89538e1194bbae7d5c07e2172230f8c",
        },
    },
    TrustedOperationContract {
        compatibility_name: "highpass",
        rust_id: "rusttable.highpass",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 1,
            parameter_size: 8,
            parameter_layout_hash: "63d81ca88d8255603f54e7e660198dc3f59d43d8f94d0a3564005c4f58bf4f0c",
            native_order: Some(68),
            reference_path: Some("src/iop/highpass.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "b89aceff81335954d0d4a170ea77a486407e84d1b8a9eae913bd7784299b47d3",
            codec_identity: "5a3da642766fedbc22359f8aa1aff025d2d251b4071f10a78b5ddcb6c2d62e5c",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 8,
                layout_hash: "d28caef95b9206275ded40c6fbad0f2c25aca47dca471562e85e72f59f024a0d",
                decoder: "generated.bytes.decode.v1",
                opaque_blocking: false,
                abi_identity: "b89aceff81335954d0d4a170ea77a486407e84d1b8a9eae913bd7784299b47d3",
                codec_identity: "5a3da642766fedbc22359f8aa1aff025d2d251b4071f10a78b5ddcb6c2d62e5c",
            }],
            evidence_identity: "f6d0fe2d411ce77b473c9a23947aa6aa8998ec801715ce8a2ef9926c0c3d0da7",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 1,
            parameter_size: 8,
            parameter_layout_hash: "16e017dcdff0e8e3c63ac96055fbf1c4d283a7f640600cb94495fa8e8c65451d",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 8,
                layout_hash: "d28caef95b9206275ded40c6fbad0f2c25aca47dca471562e85e72f59f024a0d",
                decoder: "opaque",
                opaque_blocking: true,
                abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            }],
            evidence_identity: "f6d0fe2d411ce77b473c9a23947aa6aa8998ec801715ce8a2ef9926c0c3d0da7",
        },
    },
    TrustedOperationContract {
        compatibility_name: "invert",
        rust_id: "rusttable.invert",
        descriptor_version: 2,
        parameter_version: 2,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 2,
            parameter_size: 16,
            parameter_layout_hash: "0c6cb8df7815de03ebd74a8e86766e37e0a4f129ac601f46e73eb6a2d303fbb1",
            native_order: Some(128),
            reference_path: Some("src/iop/invert.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "ffa98e011e79692d70214aff96bb708b886c239151f3760a0a7fd4eb4374cf0f",
            codec_identity: "348407041101a60959f06f519558fd53856defb2a8ec88ba61d4842b014c3c70",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 296,
                    layout_hash: "65304a75424eb28197d9c7472376a093227a2db4669d21325b1b4f3c5f9a93ec",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 16,
                    layout_hash: "289ad04154d876ebbdfb887d1b08726ca664e1c8c7ec06bbd0c3ab6db6e37f03",
                    decoder: "generated.bytes.decode.v2",
                    opaque_blocking: false,
                    abi_identity: "ffa98e011e79692d70214aff96bb708b886c239151f3760a0a7fd4eb4374cf0f",
                    codec_identity: "348407041101a60959f06f519558fd53856defb2a8ec88ba61d4842b014c3c70",
                },
            ],
            evidence_identity: "44dfc53ef822d81d580bc707db03eb0d6daf8bc6b697cd320e99b1646283e412",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 2,
            parameter_size: 16,
            parameter_layout_hash: "bf6ffefc71786aeea8002ca8218db646369f6435fa147ef91959422bb6c57811",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 296,
                    layout_hash: "65304a75424eb28197d9c7472376a093227a2db4669d21325b1b4f3c5f9a93ec",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 16,
                    layout_hash: "289ad04154d876ebbdfb887d1b08726ca664e1c8c7ec06bbd0c3ab6db6e37f03",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
            ],
            evidence_identity: "44dfc53ef822d81d580bc707db03eb0d6daf8bc6b697cd320e99b1646283e412",
        },
    },
    TrustedOperationContract {
        compatibility_name: "lenscorrection",
        rust_id: "rusttable.lenscorrection",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 10,
            parameter_size: 356,
            parameter_layout_hash: "c19e6ad92c9a17cbfee573fa80f84e70f77bb6d58d24d2de0da33efb4badb867",
            native_order: Some(164),
            reference_path: Some("src/iop/lens.cc"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "d572830b888e359abb5f148663122de82bab7efc56d9f431785c9c5abc41b5e3",
            codec_identity: "a6f2bdff325914a2acabcfdca2ca971f082cc9f8280a2cd480673a32b2aadb7c",
            parameter_versions: &[
                TrustedVersion {
                    version: 2,
                    byte_size: 3752,
                    layout_hash: "086bb80c1c85e447ba42f46a937b710f4b7ecb4b395fe9238d3fe3c291986629",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 4200,
                    layout_hash: "23e1c36bc206398832c7a3bf35aafa9b12b2664be278f60264bfd5ba76547c02",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 4648,
                    layout_hash: "d0c66533df591f1573d5b70c0b52efba4136ec38696c0d594783c63443e11d4d",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 5,
                    byte_size: 5104,
                    layout_hash: "6a687d3d230718683e5f667d7de2d66ce7e3399596257a94ee0467036dbd9ca8",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 6,
                    byte_size: 5576,
                    layout_hash: "73888379248f8af029e3289bb8b0d796bf701fdea2942694b254d9cf943931f5",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 7,
                    byte_size: 6056,
                    layout_hash: "1310be20b101cd770ced8bf04b60058689fd2f948fe3cee69aaca58a795c26eb",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 8,
                    byte_size: 6552,
                    layout_hash: "6a930081be64c776c134f55af6dc5b899ddb29ccce63feedd2b9134f94be8948",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 9,
                    byte_size: 7048,
                    layout_hash: "6a4074428be0ba51cd9be73554775f72f5e18e742a53c09f60edd996783b4a19",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 10,
                    byte_size: 356,
                    layout_hash: "3a6f027819ab5e5953f46e603132dbfd3657feb72da72cef594540cb6b8a1c39",
                    decoder: "generated.bytes.decode.v10",
                    opaque_blocking: false,
                    abi_identity: "d572830b888e359abb5f148663122de82bab7efc56d9f431785c9c5abc41b5e3",
                    codec_identity: "a6f2bdff325914a2acabcfdca2ca971f082cc9f8280a2cd480673a32b2aadb7c",
                },
            ],
            evidence_identity: "49a77a9b4f7e0fe1f52f481a63c7ee7e1cb2c89d7974aa50cd5c2e968c77639f",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 10,
            parameter_size: 356,
            parameter_layout_hash: "c1721ee3b7633b8eec8d3a07835b7e8364ac8dc6df7190d31cbe6cee1869ce81",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 2,
                    byte_size: 3752,
                    layout_hash: "086bb80c1c85e447ba42f46a937b710f4b7ecb4b395fe9238d3fe3c291986629",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 4200,
                    layout_hash: "23e1c36bc206398832c7a3bf35aafa9b12b2664be278f60264bfd5ba76547c02",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 4648,
                    layout_hash: "d0c66533df591f1573d5b70c0b52efba4136ec38696c0d594783c63443e11d4d",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 5,
                    byte_size: 5104,
                    layout_hash: "6a687d3d230718683e5f667d7de2d66ce7e3399596257a94ee0467036dbd9ca8",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 6,
                    byte_size: 5576,
                    layout_hash: "73888379248f8af029e3289bb8b0d796bf701fdea2942694b254d9cf943931f5",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 7,
                    byte_size: 6056,
                    layout_hash: "1310be20b101cd770ced8bf04b60058689fd2f948fe3cee69aaca58a795c26eb",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 8,
                    byte_size: 6552,
                    layout_hash: "6a930081be64c776c134f55af6dc5b899ddb29ccce63feedd2b9134f94be8948",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 9,
                    byte_size: 7048,
                    layout_hash: "6a4074428be0ba51cd9be73554775f72f5e18e742a53c09f60edd996783b4a19",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 10,
                    byte_size: 356,
                    layout_hash: "3a6f027819ab5e5953f46e603132dbfd3657feb72da72cef594540cb6b8a1c39",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
            ],
            evidence_identity: "49a77a9b4f7e0fe1f52f481a63c7ee7e1cb2c89d7974aa50cd5c2e968c77639f",
        },
    },
    TrustedOperationContract {
        compatibility_name: "levels",
        rust_id: "rusttable.levels",
        descriptor_version: 2,
        parameter_version: 2,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 2,
            parameter_size: 28,
            parameter_layout_hash: "b74d61e97c5f8d4d7053c1712e9f040290af18ae81a137ccf8639e2f724594af",
            native_order: Some(125),
            reference_path: Some("src/iop/levels.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "d0c15b0308dd1b8e46337fbbcd9b626eaf869c002b6d07e0c5b4f3fb982312da",
            codec_identity: "ad8f4f733495c9fcd885b12a75f9617600d3a6e3fcea2bffa141e308f07baf5a",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 16,
                    layout_hash: "9024b04b5791ef52ba24caa9a6a92b8927ec0239b917cab9a0502b0675cf4cb5",
                    decoder: "rusttable.levels.decode.v1",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 28,
                    layout_hash: "55c5d03eee92b554ed23e9937410826573cd52d56aa3434a7d7d6de3697bc8b8",
                    decoder: "generated.bytes.decode.v2",
                    opaque_blocking: false,
                    abi_identity: "d0c15b0308dd1b8e46337fbbcd9b626eaf869c002b6d07e0c5b4f3fb982312da",
                    codec_identity: "ad8f4f733495c9fcd885b12a75f9617600d3a6e3fcea2bffa141e308f07baf5a",
                },
            ],
            evidence_identity: "24a7a307f62192007336a11debe8c216dfb2077bbd9fcee0c898255d73eef29c",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 2,
            parameter_size: 28,
            parameter_layout_hash: "b7dcce3cb8910c7dbebb57300b34d1903b509f53093e2f588a029bcf04014d6d",
            native_order: Some(125),
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 16,
                    layout_hash: "9024b04b5791ef52ba24caa9a6a92b8927ec0239b917cab9a0502b0675cf4cb5",
                    decoder: "rusttable.levels.decode.v1",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 28,
                    layout_hash: "55c5d03eee92b554ed23e9937410826573cd52d56aa3434a7d7d6de3697bc8b8",
                    decoder: "rusttable.levels.decode.v2",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
            ],
            evidence_identity: "24a7a307f62192007336a11debe8c216dfb2077bbd9fcee0c898255d73eef29c",
        },
    },
    TrustedOperationContract {
        compatibility_name: "linear-offset",
        rust_id: "rusttable.linear_offset",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 1,
            parameter_size: 4,
            parameter_layout_hash: "9746ae3a59383a479e165af015962c436f0d731b60692efcdeec7149c744b980",
            native_order: Some(30),
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "8c841cb6066b74c40aeb8727990caa4e410e98d973b7f753990e868b0dbc21d0",
            codec_identity: "59615633b15fb24f635f5e3535ed9356f2e47b0db66707c6f98e13e765567ba2",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 4,
                layout_hash: "9746ae3a59383a479e165af015962c436f0d731b60692efcdeec7149c744b980",
                decoder: "rusttable.linear-offset.decode.v1",
                opaque_blocking: false,
                abi_identity: "8c841cb6066b74c40aeb8727990caa4e410e98d973b7f753990e868b0dbc21d0",
                codec_identity: "59615633b15fb24f635f5e3535ed9356f2e47b0db66707c6f98e13e765567ba2",
            }],
            evidence_identity: "953d9c567ac6ef04c87511a156b8438b31ac3dffcc0d30d0ada1c76f76f6af98",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 1,
            parameter_size: 4,
            parameter_layout_hash: "9746ae3a59383a479e165af015962c436f0d731b60692efcdeec7149c744b980",
            native_order: Some(30),
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "8c841cb6066b74c40aeb8727990caa4e410e98d973b7f753990e868b0dbc21d0",
            codec_identity: "59615633b15fb24f635f5e3535ed9356f2e47b0db66707c6f98e13e765567ba2",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 4,
                layout_hash: "9746ae3a59383a479e165af015962c436f0d731b60692efcdeec7149c744b980",
                decoder: "rusttable.linear-offset.decode.v1",
                opaque_blocking: false,
                abi_identity: "8c841cb6066b74c40aeb8727990caa4e410e98d973b7f753990e868b0dbc21d0",
                codec_identity: "59615633b15fb24f635f5e3535ed9356f2e47b0db66707c6f98e13e765567ba2",
            }],
            evidence_identity: "953d9c567ac6ef04c87511a156b8438b31ac3dffcc0d30d0ada1c76f76f6af98",
        },
    },
    TrustedOperationContract {
        compatibility_name: "liquify",
        rust_id: "rusttable.liquify",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 1,
            parameter_size: 144,
            parameter_layout_hash: "3aff44c0fabc743303ac96a0b818ba65c964555a41c89b9cfbf164748905443a",
            native_order: Some(120),
            reference_path: Some("src/iop/liquify.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "475986c178e7160787064b604c1c16ddbb4cd40f73b22f32b40c67d4333bcd0b",
            codec_identity: "10db24e9f5a2a8335339f1337cd89fe5e74d31c189c3c8dd8b5ea4f566137f70",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 144,
                layout_hash: "c9bf37054854b46b5024c22e478726dcf2015c268b66b509f3d833b83aab268a",
                decoder: "generated.bytes.decode.v1",
                opaque_blocking: false,
                abi_identity: "475986c178e7160787064b604c1c16ddbb4cd40f73b22f32b40c67d4333bcd0b",
                codec_identity: "10db24e9f5a2a8335339f1337cd89fe5e74d31c189c3c8dd8b5ea4f566137f70",
            }],
            evidence_identity: "127f14bac247a812dc50071b18ce82b010c92f6a2b335fa165a7d8e233d3455b",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 1,
            parameter_size: 144,
            parameter_layout_hash: "debe1d807d738fd64071ae12f78540bd5aeaabbacae61534b41f3b5a2a7567f6",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 144,
                layout_hash: "c9bf37054854b46b5024c22e478726dcf2015c268b66b509f3d833b83aab268a",
                decoder: "opaque",
                opaque_blocking: true,
                abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            }],
            evidence_identity: "127f14bac247a812dc50071b18ce82b010c92f6a2b335fa165a7d8e233d3455b",
        },
    },
    TrustedOperationContract {
        compatibility_name: "mask_manager",
        rust_id: "rusttable.mask_manager",
        descriptor_version: 2,
        parameter_version: 2,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 2,
            parameter_size: 4,
            parameter_layout_hash: "389be5cfd30b667cc11cdd0cf2d1bcc2ed33a423e5ae9043bcd31ff451192627",
            native_order: Some(139),
            reference_path: Some("src/iop/mask_manager.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "42a719556434cc07fb5a9c299664f5c5e0645ade6436ab15d9161638c7c553fc",
            codec_identity: "b4ed946cccea685e722fb2535da5aec1f2dcfc480df7a424d2e82aa38f2cd7f0",
            parameter_versions: &[TrustedVersion {
                version: 2,
                byte_size: 4,
                layout_hash: "bf6c32d6aefe4eba261707a4c2bd1ee4d4677a798c7e367b547ae566e1ee6e07",
                decoder: "generated.bytes.decode.v2",
                opaque_blocking: false,
                abi_identity: "42a719556434cc07fb5a9c299664f5c5e0645ade6436ab15d9161638c7c553fc",
                codec_identity: "b4ed946cccea685e722fb2535da5aec1f2dcfc480df7a424d2e82aa38f2cd7f0",
            }],
            evidence_identity: "a0ec19006c0ee12ed83bbc70a44fb4562f241acf90683fb1490fd6169a81d30a",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 2,
            parameter_size: 4,
            parameter_layout_hash: "13006e12dedfcd2dea21dba519fe2535b55facae246d82762530f969e9da66ce",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 2,
                byte_size: 4,
                layout_hash: "bf6c32d6aefe4eba261707a4c2bd1ee4d4677a798c7e367b547ae566e1ee6e07",
                decoder: "opaque",
                opaque_blocking: true,
                abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            }],
            evidence_identity: "a0ec19006c0ee12ed83bbc70a44fb4562f241acf90683fb1490fd6169a81d30a",
        },
    },
    TrustedOperationContract {
        compatibility_name: "overlay",
        rust_id: "rusttable.overlay",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 1,
            parameter_size: 1088,
            parameter_layout_hash: "d0633409112da9bedaf50e225f26d904bd28ab41723fed4d1bcf55b3c0b8db14",
            native_order: Some(95),
            reference_path: Some("src/iop/overlay.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "deb67933cbda6679e6ec13ee1b879f8b7bdc13f0a83d9e5a1901a1bfa702f95b",
            codec_identity: "c595e39dc16a14e5805ecc3968a83448c1f5047a7b52c9407f157044886738df",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 1088,
                layout_hash: "e7e38a671e3d057891cfb76b3920b893239128360006c7d7934801d856f9528f",
                decoder: "generated.bytes.decode.v1",
                opaque_blocking: false,
                abi_identity: "deb67933cbda6679e6ec13ee1b879f8b7bdc13f0a83d9e5a1901a1bfa702f95b",
                codec_identity: "c595e39dc16a14e5805ecc3968a83448c1f5047a7b52c9407f157044886738df",
            }],
            evidence_identity: "4e81af27c48a8f3fe580fa387e704d5d644cbd22b7ec01fdc34a3b7cb6ac8509",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 1,
            parameter_size: 1088,
            parameter_layout_hash: "d4d0471f44f31151b9bddb5c606af195a59898a9ed2ad3b4ffb8e025e45012ce",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 1088,
                layout_hash: "e7e38a671e3d057891cfb76b3920b893239128360006c7d7934801d856f9528f",
                decoder: "opaque",
                opaque_blocking: true,
                abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            }],
            evidence_identity: "4e81af27c48a8f3fe580fa387e704d5d644cbd22b7ec01fdc34a3b7cb6ac8509",
        },
    },
    TrustedOperationContract {
        compatibility_name: "primaries",
        rust_id: "rusttable.primaries",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 1,
            parameter_size: 32,
            parameter_layout_hash: "02bf7e8cfb69be6a9f9ff866e03456df5c7664c99674cbe8419048574f7ec299",
            native_order: Some(156),
            reference_path: Some("src/iop/primaries.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "7537bb07233f4f21d961758ba08465093188c4f5652209bcd6fa0353dc03204a",
            codec_identity: "07b1ad0680f1a3f7f430cf5119a4dc468c4c0a069bdbe676ddb2c918866d6b7a",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 32,
                layout_hash: "83b8fbd4303988a05e0a35a5761fbd3f4dc42669faceb1e0f62cc1a3a937a2ec",
                decoder: "generated.bytes.decode.v1",
                opaque_blocking: false,
                abi_identity: "7537bb07233f4f21d961758ba08465093188c4f5652209bcd6fa0353dc03204a",
                codec_identity: "07b1ad0680f1a3f7f430cf5119a4dc468c4c0a069bdbe676ddb2c918866d6b7a",
            }],
            evidence_identity: "7e76e4b682eef0c91b8cb74fa05da31d4a014dfe1763298c8d6528ba6164240a",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 1,
            parameter_size: 32,
            parameter_layout_hash: "148d589cdc35fa4c52ebd61f0ba78ff79c5ddc4f03d740071d6a6d06b308f005",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 32,
                layout_hash: "83b8fbd4303988a05e0a35a5761fbd3f4dc42669faceb1e0f62cc1a3a937a2ec",
                decoder: "opaque",
                opaque_blocking: true,
                abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            }],
            evidence_identity: "7e76e4b682eef0c91b8cb74fa05da31d4a014dfe1763298c8d6528ba6164240a",
        },
    },
    TrustedOperationContract {
        compatibility_name: "rasterfile",
        rust_id: "rusttable.rasterfile",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 1,
            parameter_size: 4100,
            parameter_layout_hash: "7346d0e1cd4ba9be2a6597c29c6a6e373f3705f219162db1dd642e88190babaa",
            native_order: Some(158),
            reference_path: Some("src/iop/rasterfile.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "6b3993b1c4e59496604089f1c7abc57873f397a06070630a6833503d1ea9db5f",
            codec_identity: "b62f01e4c13a59af876a2a491475118c6c05df5c9da5af8a7bce3db4bb58358c",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 4100,
                layout_hash: "87325e51ccf94cdf2ecafec26ef5c90cfb2713b9a577a3a913c1ee40ac4acf57",
                decoder: "generated.bytes.decode.v1",
                opaque_blocking: false,
                abi_identity: "6b3993b1c4e59496604089f1c7abc57873f397a06070630a6833503d1ea9db5f",
                codec_identity: "b62f01e4c13a59af876a2a491475118c6c05df5c9da5af8a7bce3db4bb58358c",
            }],
            evidence_identity: "687f0fd6b50af288b25a860004d664c69c218fd19af2c627da2248acd0cc9833",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 1,
            parameter_size: 4100,
            parameter_layout_hash: "dcb36173d1ca8f37e676500fe6bf76f9c51d3d0d7bd67011e650d6bc1023ed34",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 4100,
                layout_hash: "87325e51ccf94cdf2ecafec26ef5c90cfb2713b9a577a3a913c1ee40ac4acf57",
                decoder: "opaque",
                opaque_blocking: true,
                abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            }],
            evidence_identity: "687f0fd6b50af288b25a860004d664c69c218fd19af2c627da2248acd0cc9833",
        },
    },
    TrustedOperationContract {
        compatibility_name: "relight",
        rust_id: "rusttable.relight",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 1,
            parameter_size: 12,
            parameter_layout_hash: "1cdf4b1c74dea5b674e66a56795656a7a01cca14c97fbf0834c0bfbc2f608ae4",
            native_order: Some(108),
            reference_path: Some("src/iop/relight.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "ad658933041d7e0cae4758fe697dab72b9978945b23647129d67a74be2c861fd",
            codec_identity: "40224b3a3d844ceb0a4c1119e373d1f7733ea44adc6a6c1047a4c642511d1529",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 12,
                layout_hash: "24dffe0bc95f20f36d93b1d2460c515b59fcc8e61d0177f6639fb13a34e83c21",
                decoder: "generated.bytes.decode.v1",
                opaque_blocking: false,
                abi_identity: "ad658933041d7e0cae4758fe697dab72b9978945b23647129d67a74be2c861fd",
                codec_identity: "40224b3a3d844ceb0a4c1119e373d1f7733ea44adc6a6c1047a4c642511d1529",
            }],
            evidence_identity: "71ab8fb2e5fbc76253d4b8756451ca91a066ff3e668bb885a2edc6633d8e5276",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 1,
            parameter_size: 12,
            parameter_layout_hash: "50796e6df004aec83c5c4361c7db45a06605bed4b704ee3a4aef9d77b2ea24b4",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 12,
                layout_hash: "24dffe0bc95f20f36d93b1d2460c515b59fcc8e61d0177f6639fb13a34e83c21",
                decoder: "opaque",
                opaque_blocking: true,
                abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            }],
            evidence_identity: "71ab8fb2e5fbc76253d4b8756451ca91a066ff3e668bb885a2edc6633d8e5276",
        },
    },
    TrustedOperationContract {
        compatibility_name: "retouch",
        rust_id: "rusttable.retouch",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 3,
            parameter_size: 136,
            parameter_layout_hash: "ab5ba3b3fcfe85a51d57217a5cfaff6c36e897d8b36d9d89cae3ac550cca5933",
            native_order: Some(119),
            reference_path: Some("src/iop/retouch.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "3e081e0f1a8a4cfba44ce490b0cf5b20f1007bc1e3cdf353165cb2561f12e6d5",
            codec_identity: "769ef214ee36d1318c7967a9b70a5c26edcf0d9f2d930c08165b8dba9dde530c",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 3048,
                    layout_hash: "b0481bc916844a530a2ac4e1975ea35af00921cf1db705864e6446c465a87e8d",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 4504,
                    layout_hash: "85b05e9b83a2966b721bbbd57a92b886134e20faa586d41860c2bdc4b8917ed0",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 136,
                    layout_hash: "e3e0e330a71546b0caa074aeef6d247199226f91558037230835b1122af3c475",
                    decoder: "generated.bytes.decode.v3",
                    opaque_blocking: false,
                    abi_identity: "3e081e0f1a8a4cfba44ce490b0cf5b20f1007bc1e3cdf353165cb2561f12e6d5",
                    codec_identity: "769ef214ee36d1318c7967a9b70a5c26edcf0d9f2d930c08165b8dba9dde530c",
                },
            ],
            evidence_identity: "60feec01ffda67811d9cf231bb77612edb26a16dca38669154691d222417ec80",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 3,
            parameter_size: 136,
            parameter_layout_hash: "546546ebeda8d549b1f5efd6f2bc539ba194e3f8e02c790a53131347b7602c7b",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 3048,
                    layout_hash: "b0481bc916844a530a2ac4e1975ea35af00921cf1db705864e6446c465a87e8d",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 4504,
                    layout_hash: "85b05e9b83a2966b721bbbd57a92b886134e20faa586d41860c2bdc4b8917ed0",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 136,
                    layout_hash: "e3e0e330a71546b0caa074aeef6d247199226f91558037230835b1122af3c475",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
            ],
            evidence_identity: "60feec01ffda67811d9cf231bb77612edb26a16dca38669154691d222417ec80",
        },
    },
    TrustedOperationContract {
        compatibility_name: "rgbgain",
        rust_id: "rusttable.rgb_gain",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 1,
            parameter_size: 12,
            parameter_layout_hash: "d550640c14231e5a6c1758fb85e5adf84f8c21a57dbed0a10eb98243e72c441f",
            native_order: Some(38),
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "c27dd0f1a2639dfcd2e83855605c221550ba800236f79b367af4383bf1bf4188",
            codec_identity: "969a48bfff41f837be26131de67a073cdc03c3cf0378e3ef39c3edbad7e4a3ca",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 12,
                layout_hash: "d550640c14231e5a6c1758fb85e5adf84f8c21a57dbed0a10eb98243e72c441f",
                decoder: "rusttable.rgbgain.decode.v1",
                opaque_blocking: false,
                abi_identity: "c27dd0f1a2639dfcd2e83855605c221550ba800236f79b367af4383bf1bf4188",
                codec_identity: "969a48bfff41f837be26131de67a073cdc03c3cf0378e3ef39c3edbad7e4a3ca",
            }],
            evidence_identity: "e7a14bfc303f8de262c727bc286175dba5148a98de68bfe701270b3e7d16615d",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 1,
            parameter_size: 12,
            parameter_layout_hash: "d550640c14231e5a6c1758fb85e5adf84f8c21a57dbed0a10eb98243e72c441f",
            native_order: Some(38),
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "c27dd0f1a2639dfcd2e83855605c221550ba800236f79b367af4383bf1bf4188",
            codec_identity: "969a48bfff41f837be26131de67a073cdc03c3cf0378e3ef39c3edbad7e4a3ca",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 12,
                layout_hash: "d550640c14231e5a6c1758fb85e5adf84f8c21a57dbed0a10eb98243e72c441f",
                decoder: "rusttable.rgbgain.decode.v1",
                opaque_blocking: false,
                abi_identity: "c27dd0f1a2639dfcd2e83855605c221550ba800236f79b367af4383bf1bf4188",
                codec_identity: "969a48bfff41f837be26131de67a073cdc03c3cf0378e3ef39c3edbad7e4a3ca",
            }],
            evidence_identity: "e7a14bfc303f8de262c727bc286175dba5148a98de68bfe701270b3e7d16615d",
        },
    },
    TrustedOperationContract {
        compatibility_name: "rgblevels",
        rust_id: "rusttable.rgblevels",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 1,
            parameter_size: 44,
            parameter_layout_hash: "de49aa2ecec5121928807906c03f83c4ea4e1f2e19dbc4d9af78038e92346e9a",
            native_order: Some(126),
            reference_path: Some("src/iop/rgblevels.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "b6504035a9fe95727b0c3260e28912e7cd147de39328e4b411b1e4480d6a86ab",
            codec_identity: "70efc7df809575b1a9fe572801259b13b24e0f10a361cc3aff7b2c3624e5f068",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 44,
                layout_hash: "78367ee700aeaf7d397d57e9a61fef57ed1a5a367dffce8d705681e4e0e94a76",
                decoder: "generated.bytes.decode.v1",
                opaque_blocking: false,
                abi_identity: "b6504035a9fe95727b0c3260e28912e7cd147de39328e4b411b1e4480d6a86ab",
                codec_identity: "70efc7df809575b1a9fe572801259b13b24e0f10a361cc3aff7b2c3624e5f068",
            }],
            evidence_identity: "1adaceae92550e2a445b1ded7e1e9287401eddeb91e6f8c173842b15540822d0",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 1,
            parameter_size: 44,
            parameter_layout_hash: "4b91591263ca988f727d3b6a7eb17ca59f8224009ffaed418eedd38ea0ee3a1b",
            native_order: Some(126),
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 44,
                layout_hash: "78367ee700aeaf7d397d57e9a61fef57ed1a5a367dffce8d705681e4e0e94a76",
                decoder: "rusttable.rgblevels.decode.v1",
                opaque_blocking: false,
                abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            }],
            evidence_identity: "1adaceae92550e2a445b1ded7e1e9287401eddeb91e6f8c173842b15540822d0",
        },
    },
    TrustedOperationContract {
        compatibility_name: "rotatepixels",
        rust_id: "rusttable.rotatepixels",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 1,
            parameter_size: 8,
            parameter_layout_hash: "63d81ca88d8255603f54e7e660198dc3f59d43d8f94d0a3564005c4f58bf4f0c",
            native_order: Some(111),
            reference_path: Some("src/iop/rotatepixels.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "b89aceff81335954d0d4a170ea77a486407e84d1b8a9eae913bd7784299b47d3",
            codec_identity: "5a3da642766fedbc22359f8aa1aff025d2d251b4071f10a78b5ddcb6c2d62e5c",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 8,
                layout_hash: "e0fbc0badc4c552aad69d2f7392a3cd16b156af227224c05cc84afcbed00b1b0",
                decoder: "generated.bytes.decode.v1",
                opaque_blocking: false,
                abi_identity: "b89aceff81335954d0d4a170ea77a486407e84d1b8a9eae913bd7784299b47d3",
                codec_identity: "5a3da642766fedbc22359f8aa1aff025d2d251b4071f10a78b5ddcb6c2d62e5c",
            }],
            evidence_identity: "1ff67ab957230c8cc56d4ce2a5911a6e680a3ad7de8a236af35c0f74c4811933",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 1,
            parameter_size: 8,
            parameter_layout_hash: "055bf6398e866d3a4a16d1701ab425f560fa5e61a7bca5a44413946f40711859",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 8,
                layout_hash: "e0fbc0badc4c552aad69d2f7392a3cd16b156af227224c05cc84afcbed00b1b0",
                decoder: "opaque",
                opaque_blocking: true,
                abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            }],
            evidence_identity: "1ff67ab957230c8cc56d4ce2a5911a6e680a3ad7de8a236af35c0f74c4811933",
        },
    },
    TrustedOperationContract {
        compatibility_name: "scalepixels",
        rust_id: "rusttable.scalepixels",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 1,
            parameter_size: 4,
            parameter_layout_hash: "389be5cfd30b667cc11cdd0cf2d1bcc2ed33a423e5ae9043bcd31ff451192627",
            native_order: Some(112),
            reference_path: Some("src/iop/scalepixels.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "42a719556434cc07fb5a9c299664f5c5e0645ade6436ab15d9161638c7c553fc",
            codec_identity: "71604b673da1a1e1ad40aec5259e7bdb3ccdb68cb869907cc48c97e29380ff23",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 4,
                layout_hash: "0c49017dd3adf00cd5dd85bad727707b070f1bb545f0d6d4bfbb5eb7790c3bf1",
                decoder: "generated.bytes.decode.v1",
                opaque_blocking: false,
                abi_identity: "42a719556434cc07fb5a9c299664f5c5e0645ade6436ab15d9161638c7c553fc",
                codec_identity: "71604b673da1a1e1ad40aec5259e7bdb3ccdb68cb869907cc48c97e29380ff23",
            }],
            evidence_identity: "d35deadb91bdb222f8e979e636946428e99a60135643f5c246ac6d30c9b91655",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 1,
            parameter_size: 4,
            parameter_layout_hash: "fbd5dfc5567ed4b4f3621df64455f71848ce2e360160cc6afe183426174b7c2b",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 4,
                layout_hash: "0c49017dd3adf00cd5dd85bad727707b070f1bb545f0d6d4bfbb5eb7790c3bf1",
                decoder: "opaque",
                opaque_blocking: true,
                abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            }],
            evidence_identity: "d35deadb91bdb222f8e979e636946428e99a60135643f5c246ac6d30c9b91655",
        },
    },
    TrustedOperationContract {
        compatibility_name: "shadhi",
        rust_id: "rusttable.shadhi",
        descriptor_version: 5,
        parameter_version: 5,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 5,
            parameter_size: 48,
            parameter_layout_hash: "bef1898313272008979391a01d624b21860930d7fe13149f5239e2fdbfc5845c",
            native_order: Some(70),
            reference_path: Some("src/iop/shadhi.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "0fd8070451115c782c9065859d15c822da8c9baf88d5184ac369619145439749",
            codec_identity: "eeecc56a4bc39ee5d61fc5b6f6b2a8f3ff6cce3cda498daba39f38c86fda3865",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 280,
                    layout_hash: "1e9c19a9f8f396d4e5ebd11fa1849bb307d8df9421c8e025cab520b13895f4c4",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 416,
                    layout_hash: "4c94b80ee216a83d9f6a8eedc49519cf0fb7f9277da51e3174362dfcc967632b",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 560,
                    layout_hash: "95a99d3777d9a1a3554fab8af19da9737fe0620f0a3303cba1893613cb76177b",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 696,
                    layout_hash: "0130f4272a8acdffce1f1cb4ae9064042be30bfcc746b5b611e0cb4c0e51195b",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 5,
                    byte_size: 48,
                    layout_hash: "17c83e86596167c503e9fa832df93c152410f28357a5c8681d7a6e20a7c007ab",
                    decoder: "generated.bytes.decode.v5",
                    opaque_blocking: false,
                    abi_identity: "0fd8070451115c782c9065859d15c822da8c9baf88d5184ac369619145439749",
                    codec_identity: "eeecc56a4bc39ee5d61fc5b6f6b2a8f3ff6cce3cda498daba39f38c86fda3865",
                },
            ],
            evidence_identity: "3e04d622d7894a8ed2554dc447488bca85ed2e661dc4f8a2250425031e2aff19",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 5,
            parameter_size: 48,
            parameter_layout_hash: "fb5732b1d9b034815488bd2cc779f2caa580553cfcadedf97f227de20e151370",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 280,
                    layout_hash: "1e9c19a9f8f396d4e5ebd11fa1849bb307d8df9421c8e025cab520b13895f4c4",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 416,
                    layout_hash: "4c94b80ee216a83d9f6a8eedc49519cf0fb7f9277da51e3174362dfcc967632b",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 560,
                    layout_hash: "95a99d3777d9a1a3554fab8af19da9737fe0620f0a3303cba1893613cb76177b",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 696,
                    layout_hash: "0130f4272a8acdffce1f1cb4ae9064042be30bfcc746b5b611e0cb4c0e51195b",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 5,
                    byte_size: 48,
                    layout_hash: "17c83e86596167c503e9fa832df93c152410f28357a5c8681d7a6e20a7c007ab",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
            ],
            evidence_identity: "3e04d622d7894a8ed2554dc447488bca85ed2e661dc4f8a2250425031e2aff19",
        },
    },
    TrustedOperationContract {
        compatibility_name: "sharpen",
        rust_id: "rusttable.sharpen",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 1,
            parameter_size: 12,
            parameter_layout_hash: "1cdf4b1c74dea5b674e66a56795656a7a01cca14c97fbf0834c0bfbc2f608ae4",
            native_order: Some(89),
            reference_path: Some("src/iop/sharpen.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "ad658933041d7e0cae4758fe697dab72b9978945b23647129d67a74be2c861fd",
            codec_identity: "40224b3a3d844ceb0a4c1119e373d1f7733ea44adc6a6c1047a4c642511d1529",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 12,
                layout_hash: "4cc50c742a7dc36f1abddd7df0723a3dc2013faf80c645ef0e8eec28d5d10391",
                decoder: "generated.bytes.decode.v1",
                opaque_blocking: false,
                abi_identity: "ad658933041d7e0cae4758fe697dab72b9978945b23647129d67a74be2c861fd",
                codec_identity: "40224b3a3d844ceb0a4c1119e373d1f7733ea44adc6a6c1047a4c642511d1529",
            }],
            evidence_identity: "1e77267ac35ede9a6c3ba033283e64b125a8b38428768bada2d887f76a40d52d",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 1,
            parameter_size: 12,
            parameter_layout_hash: "66c221e746aed16747ee334d5f226aa2587eb187f29e612cc255762a13b169b9",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 12,
                layout_hash: "4cc50c742a7dc36f1abddd7df0723a3dc2013faf80c645ef0e8eec28d5d10391",
                decoder: "rusttable.sharpen.decode.v1",
                opaque_blocking: false,
                abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            }],
            evidence_identity: "1e77267ac35ede9a6c3ba033283e64b125a8b38428768bada2d887f76a40d52d",
        },
    },
    TrustedOperationContract {
        compatibility_name: "soften",
        rust_id: "rusttable.soften",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 1,
            parameter_size: 16,
            parameter_layout_hash: "0c6cb8df7815de03ebd74a8e86766e37e0a4f129ac601f46e73eb6a2d303fbb1",
            native_order: Some(66),
            reference_path: Some("src/iop/soften.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "ffa98e011e79692d70214aff96bb708b886c239151f3760a0a7fd4eb4374cf0f",
            codec_identity: "e7ce053b9b80badd634517649ca49734459fe92315edc285764d27adba44040f",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 16,
                layout_hash: "98678e672852f4b1212c4e5e6587760a7b7c04b26fa1b5aaf77b36ef5b7bc9bc",
                decoder: "generated.bytes.decode.v1",
                opaque_blocking: false,
                abi_identity: "ffa98e011e79692d70214aff96bb708b886c239151f3760a0a7fd4eb4374cf0f",
                codec_identity: "e7ce053b9b80badd634517649ca49734459fe92315edc285764d27adba44040f",
            }],
            evidence_identity: "dffcccd256881b17efb2b91d40581a1cdcb7a4bbf1dc072fad3662731aea54af",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 1,
            parameter_size: 16,
            parameter_layout_hash: "86bb7f22c42680e71cd74fdad4741008e367c0be579660a59b6c7cbfe57aa262",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 1,
                byte_size: 16,
                layout_hash: "98678e672852f4b1212c4e5e6587760a7b7c04b26fa1b5aaf77b36ef5b7bc9bc",
                decoder: "opaque",
                opaque_blocking: true,
                abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            }],
            evidence_identity: "dffcccd256881b17efb2b91d40581a1cdcb7a4bbf1dc072fad3662731aea54af",
        },
    },
    TrustedOperationContract {
        compatibility_name: "spots",
        rust_id: "rusttable.spots",
        descriptor_version: 2,
        parameter_version: 2,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 2,
            parameter_size: 512,
            parameter_layout_hash: "0ae185bec54295757bce892b9016e68bef8db4e87af0130eed14fe19c05ce2be",
            native_order: Some(118),
            reference_path: Some("src/iop/spots.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "878bd4356f6d19788bfd5ef5d579a61357369c4ebcfc6b24701d146d47a851f7",
            codec_identity: "a4b0898ec9aae5d594a31a754abb43c7679aa262feb68930a4c09a4ea28a4364",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 1472,
                    layout_hash: "a565af88cc22682e3c4e35175b586587ffdf74d4fda7be194f0aac9a0675b07f",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 512,
                    layout_hash: "a9522dc2bd102127538992d595a43070e6e82d545579b451370fc1936db6be71",
                    decoder: "generated.bytes.decode.v2",
                    opaque_blocking: false,
                    abi_identity: "878bd4356f6d19788bfd5ef5d579a61357369c4ebcfc6b24701d146d47a851f7",
                    codec_identity: "a4b0898ec9aae5d594a31a754abb43c7679aa262feb68930a4c09a4ea28a4364",
                },
            ],
            evidence_identity: "318a4ed7ebe5036e4b0dd19a1b3304dc3ee0cc7adda81d8db1fbd3803ba2eb32",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 2,
            parameter_size: 512,
            parameter_layout_hash: "972feb3c642b6b4af43fc328cf239fdf7183de8789168fd0781a567aa6a6aa5d",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 1472,
                    layout_hash: "a565af88cc22682e3c4e35175b586587ffdf74d4fda7be194f0aac9a0675b07f",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 512,
                    layout_hash: "a9522dc2bd102127538992d595a43070e6e82d545579b451370fc1936db6be71",
                    decoder: "generated.bytes.decode.v2",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
            ],
            evidence_identity: "318a4ed7ebe5036e4b0dd19a1b3304dc3ee0cc7adda81d8db1fbd3803ba2eb32",
        },
    },
    TrustedOperationContract {
        compatibility_name: "temperature",
        rust_id: "rusttable.temperature",
        descriptor_version: 1,
        parameter_version: 4,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 4,
            parameter_size: 20,
            parameter_layout_hash: "1384abf72e8a84a45fb18496ecc030c29a8008ea30d60b4790b94d5b5c4241ef",
            native_order: Some(76),
            reference_path: Some("src/iop/temperature.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "77eaaf2a0d1a4f97b2fb82a93aa1bc755438621379fdb7a08d39c7c6d6333370",
            codec_identity: "847b46d92960422f6bedf77174d1e7969eaf2cf0544c302404d2eb7be53a1c2d",
            parameter_versions: &[
                TrustedVersion {
                    version: 2,
                    byte_size: 1640,
                    layout_hash: "5a1cf60a36dcd60182b38705c30b427b7d24e1ec7fc142b7a90018371366b1b2",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 1600,
                    layout_hash: "8034e3b891b8dcd9409c0cc2809fb2eb0868a73357709975be3a147fdaabe2e4",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 20,
                    layout_hash: "9b43f2fe9ad9a61181b247e349864add13170bff6af1c3d7d9724052779bdb8f",
                    decoder: "generated.bytes.decode.v4",
                    opaque_blocking: false,
                    abi_identity: "77eaaf2a0d1a4f97b2fb82a93aa1bc755438621379fdb7a08d39c7c6d6333370",
                    codec_identity: "847b46d92960422f6bedf77174d1e7969eaf2cf0544c302404d2eb7be53a1c2d",
                },
            ],
            evidence_identity: "a6ac0d5dd65a583779971895c85ef244837900eb2ada3920563675e433020743",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 4,
            parameter_size: 20,
            parameter_layout_hash: "07e8c77331268d066af26bb9aca75dc99eaae19d9764bf235a3a07e05beb04b3",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 2,
                    byte_size: 1640,
                    layout_hash: "5a1cf60a36dcd60182b38705c30b427b7d24e1ec7fc142b7a90018371366b1b2",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 1600,
                    layout_hash: "8034e3b891b8dcd9409c0cc2809fb2eb0868a73357709975be3a147fdaabe2e4",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 20,
                    layout_hash: "9b43f2fe9ad9a61181b247e349864add13170bff6af1c3d7d9724052779bdb8f",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
            ],
            evidence_identity: "a6ac0d5dd65a583779971895c85ef244837900eb2ada3920563675e433020743",
        },
    },
    TrustedOperationContract {
        compatibility_name: "tonecurve",
        rust_id: "rusttable.tonecurve",
        descriptor_version: 5,
        parameter_version: 5,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 5,
            parameter_size: 520,
            parameter_layout_hash: "c6b53ef5e90c1c9fa80fc00bdf85fc4b539a11a64d9674cfdfbaeb85335f4332",
            native_order: Some(73),
            reference_path: Some("src/iop/tonecurve.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "bd47e62aef28f7f325af80385c944aed1cfa7515c7e137da7bf0963d0f26d85c",
            codec_identity: "61256607a96be3dd832c9dbc1ce160a436b8f08ffca415024572d9851fcc4dad",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 52,
                    layout_hash: "d63cecbb050f267481218175459f98ea7a9f05db1237f70cb3c6fc4d52c0987f",
                    decoder: "rusttable.tonecurve.decode.v1",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 512,
                    layout_hash: "d52e3e3aa3c5f8c3fa1afd20dc36a5382b4b20bc6ccda90fad496c32a79c93ac",
                    decoder: "rusttable.tonecurve.decode.v3",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 516,
                    layout_hash: "8257087257a3dc8981ce6ea4a7a24f0a7d1c16805f10a6aae4b2c714cfd2b31a",
                    decoder: "rusttable.tonecurve.decode.v4",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 5,
                    byte_size: 520,
                    layout_hash: "c6b53ef5e90c1c9fa80fc00bdf85fc4b539a11a64d9674cfdfbaeb85335f4332",
                    decoder: "rusttable.tonecurve.decode.v5",
                    opaque_blocking: false,
                    abi_identity: "bd47e62aef28f7f325af80385c944aed1cfa7515c7e137da7bf0963d0f26d85c",
                    codec_identity: "61256607a96be3dd832c9dbc1ce160a436b8f08ffca415024572d9851fcc4dad",
                },
            ],
            evidence_identity: "6f4198c4c2fb4d637e62431748900ec23e3967f4e93a72c07a07acb3833e17f8",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 5,
            parameter_size: 520,
            parameter_layout_hash: "c6b53ef5e90c1c9fa80fc00bdf85fc4b539a11a64d9674cfdfbaeb85335f4332",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "bd47e62aef28f7f325af80385c944aed1cfa7515c7e137da7bf0963d0f26d85c",
            codec_identity: "61256607a96be3dd832c9dbc1ce160a436b8f08ffca415024572d9851fcc4dad",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 52,
                    layout_hash: "d63cecbb050f267481218175459f98ea7a9f05db1237f70cb3c6fc4d52c0987f",
                    decoder: "rusttable.tonecurve.decode.v1",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 512,
                    layout_hash: "d52e3e3aa3c5f8c3fa1afd20dc36a5382b4b20bc6ccda90fad496c32a79c93ac",
                    decoder: "rusttable.tonecurve.decode.v3",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 516,
                    layout_hash: "8257087257a3dc8981ce6ea4a7a24f0a7d1c16805f10a6aae4b2c714cfd2b31a",
                    decoder: "rusttable.tonecurve.decode.v4",
                    opaque_blocking: false,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 5,
                    byte_size: 520,
                    layout_hash: "c6b53ef5e90c1c9fa80fc00bdf85fc4b539a11a64d9674cfdfbaeb85335f4332",
                    decoder: "rusttable.tonecurve.decode.v5",
                    opaque_blocking: false,
                    abi_identity: "bd47e62aef28f7f325af80385c944aed1cfa7515c7e137da7bf0963d0f26d85c",
                    codec_identity: "61256607a96be3dd832c9dbc1ce160a436b8f08ffca415024572d9851fcc4dad",
                },
            ],
            evidence_identity: "6f4198c4c2fb4d637e62431748900ec23e3967f4e93a72c07a07acb3833e17f8",
        },
    },
    TrustedOperationContract {
        compatibility_name: "velvia",
        rust_id: "rusttable.velvia",
        descriptor_version: 2,
        parameter_version: 2,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 2,
            parameter_size: 8,
            parameter_layout_hash: "63d81ca88d8255603f54e7e660198dc3f59d43d8f94d0a3564005c4f58bf4f0c",
            native_order: Some(97),
            reference_path: Some("src/iop/velvia.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "b89aceff81335954d0d4a170ea77a486407e84d1b8a9eae913bd7784299b47d3",
            codec_identity: "9e0045b1da2258790ca1348a0ad6579c824679607ff98fe0a4e26748cec5f408",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 112,
                    layout_hash: "ef32a71302905d28d0a215271ec6959235d8f8be67002455cc2fb14b8791dd7e",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 8,
                    layout_hash: "8a55b103deae5e96abcdecd7fcc223cf3c26ed0fb726fcd34e1fa64a06374c4f",
                    decoder: "generated.bytes.decode.v2",
                    opaque_blocking: false,
                    abi_identity: "b89aceff81335954d0d4a170ea77a486407e84d1b8a9eae913bd7784299b47d3",
                    codec_identity: "9e0045b1da2258790ca1348a0ad6579c824679607ff98fe0a4e26748cec5f408",
                },
            ],
            evidence_identity: "1971f386711f2129b19cfe5b1f8436153352d178374e503b6e6bffc4ae4d8a5b",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 2,
            parameter_size: 8,
            parameter_layout_hash: "df9cfd38e941dc987ec1a97b1154e17034d67faeece349930ba8949de8758f95",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 112,
                    layout_hash: "ef32a71302905d28d0a215271ec6959235d8f8be67002455cc2fb14b8791dd7e",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 8,
                    layout_hash: "8a55b103deae5e96abcdecd7fcc223cf3c26ed0fb726fcd34e1fa64a06374c4f",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
            ],
            evidence_identity: "1971f386711f2129b19cfe5b1f8436153352d178374e503b6e6bffc4ae4d8a5b",
        },
    },
    TrustedOperationContract {
        compatibility_name: "vibrance",
        rust_id: "rusttable.vibrance",
        descriptor_version: 2,
        parameter_version: 2,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 2,
            parameter_size: 4,
            parameter_layout_hash: "389be5cfd30b667cc11cdd0cf2d1bcc2ed33a423e5ae9043bcd31ff451192627",
            native_order: Some(129),
            reference_path: Some("src/iop/vibrance.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "42a719556434cc07fb5a9c299664f5c5e0645ade6436ab15d9161638c7c553fc",
            codec_identity: "b4ed946cccea685e722fb2535da5aec1f2dcfc480df7a424d2e82aa38f2cd7f0",
            parameter_versions: &[TrustedVersion {
                version: 2,
                byte_size: 4,
                layout_hash: "8355b81fc6ce8b3aa2f20b2027098d44c35d3d76d7cedb8d5b494fc36d43ea2b",
                decoder: "generated.bytes.decode.v2",
                opaque_blocking: false,
                abi_identity: "42a719556434cc07fb5a9c299664f5c5e0645ade6436ab15d9161638c7c553fc",
                codec_identity: "b4ed946cccea685e722fb2535da5aec1f2dcfc480df7a424d2e82aa38f2cd7f0",
            }],
            evidence_identity: "3a2ee6fb81bd385bf66576462f137432a9e5cdab6711ad4372ba87ebc705ff6f",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 2,
            parameter_size: 4,
            parameter_layout_hash: "952f27a2986a4c2b1a9d9b4c3d243c6b42f7cb498f1bcef2e58f17fecec1c8cc",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[TrustedVersion {
                version: 2,
                byte_size: 4,
                layout_hash: "8355b81fc6ce8b3aa2f20b2027098d44c35d3d76d7cedb8d5b494fc36d43ea2b",
                decoder: "opaque",
                opaque_blocking: true,
                abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            }],
            evidence_identity: "3a2ee6fb81bd385bf66576462f137432a9e5cdab6711ad4372ba87ebc705ff6f",
        },
    },
    TrustedOperationContract {
        compatibility_name: "vignette",
        rust_id: "rusttable.vignette",
        descriptor_version: 4,
        parameter_version: 4,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 4,
            parameter_size: 64,
            parameter_layout_hash: "ad201c61dd062b8f4812fea521c479b4486812028135f673f6c3f3aee2fb50cc",
            native_order: Some(98),
            reference_path: Some("src/iop/vignette.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "411903f8e630aae0f2829f3d73715787e9e50c9f17cfcd77ceb94a6c4ba80dc6",
            codec_identity: "0f44f5048269bb2b75e87e64f4b6f4d45b95d55b2c93490bde8b079485490d08",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 320,
                    layout_hash: "c49f232d336779704fd7e07458d7a44f4c25114b984760256e3df8f6610c086a",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 464,
                    layout_hash: "4eb83b38b4ad70caadc2b1c5995d8495f16d76eb163a728d5a6515aa9a0a1835",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 592,
                    layout_hash: "2ccd6b1742f964fbcd6b47fef3c1e6b9932c2ea89ac4afd500c0175f2d86dcfd",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 64,
                    layout_hash: "f9d461c3cc65bb6ef6bdeca2b0bea044b29e666d84466544766e0929800267a8",
                    decoder: "generated.bytes.decode.v4",
                    opaque_blocking: false,
                    abi_identity: "411903f8e630aae0f2829f3d73715787e9e50c9f17cfcd77ceb94a6c4ba80dc6",
                    codec_identity: "0f44f5048269bb2b75e87e64f4b6f4d45b95d55b2c93490bde8b079485490d08",
                },
            ],
            evidence_identity: "1765582183b1463bd3001762462c71cf195e5652ba5bfdea0468e9148b3e96f7",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 4,
            parameter_size: 64,
            parameter_layout_hash: "8c9b9551d4ebc838d1f59cf44203d2523bba2f99e718012c30c15d1bb6d92757",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 320,
                    layout_hash: "c49f232d336779704fd7e07458d7a44f4c25114b984760256e3df8f6610c086a",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 464,
                    layout_hash: "4eb83b38b4ad70caadc2b1c5995d8495f16d76eb163a728d5a6515aa9a0a1835",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 592,
                    layout_hash: "2ccd6b1742f964fbcd6b47fef3c1e6b9932c2ea89ac4afd500c0175f2d86dcfd",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 64,
                    layout_hash: "f9d461c3cc65bb6ef6bdeca2b0bea044b29e666d84466544766e0929800267a8",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
            ],
            evidence_identity: "1765582183b1463bd3001762462c71cf195e5652ba5bfdea0468e9148b3e96f7",
        },
    },
    TrustedOperationContract {
        compatibility_name: "watermark",
        rust_id: "rusttable.watermark",
        descriptor_version: 1,
        parameter_version: 7,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            module_version: 7,
            parameter_size: 628,
            parameter_layout_hash: "df82b766af09900bbe7d9f0c843eeed4364b1d46aea36b126f928fab3a28aeed",
            native_order: Some(161),
            reference_path: Some("src/iop/watermark.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "a794bcf2d39ec40e2eb3dc1f0fc88b23cf9bb9329d82ac648b085bd279b73573",
            codec_identity: "8f3a46df919a00533a5218db3c3dcb8a53fd7bcfa905d77e6db418857f8da3d8",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 2840,
                    layout_hash: "041d3ea1040138fc2df089b5ad573614c6e1918bc16829ee9be9b8465d8e7e29",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 3040,
                    layout_hash: "f7e113bc2d9d41dcb215374d54fc421ffdd72f129a605e4229a6d11faa392048",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 3232,
                    layout_hash: "0d7e7ba74b44f36f829e893b088d2f22dd1c62dc56f5f02583a5242b8462cc15",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 3576,
                    layout_hash: "f54766b9d00a1b126160ea011448052ae81ba941c39ea3ca591c8d289c77db12",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 5,
                    byte_size: 4376,
                    layout_hash: "6cd78cca03d074770616f400792d4a831da65b28a9de4b02c9c75386b5250432",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 6,
                    byte_size: 2752,
                    layout_hash: "1f4fb428b367449d672fd4bd39bcc29d5cc823b78f9f367831e1caa6fe82da09",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 7,
                    byte_size: 628,
                    layout_hash: "cd91b832eae489d73b960f700ce8ccf804559762b42eebaeb3bf535e1f57b6e9",
                    decoder: "generated.bytes.decode.v7",
                    opaque_blocking: false,
                    abi_identity: "a794bcf2d39ec40e2eb3dc1f0fc88b23cf9bb9329d82ac648b085bd279b73573",
                    codec_identity: "8f3a46df919a00533a5218db3c3dcb8a53fd7bcfa905d77e6db418857f8da3d8",
                },
            ],
            evidence_identity: "03a8bbb88abb3b7bbe5ade2aac10bbde6a29df9867c9e08f736d4ff6a9b3f3f3",
        },
        audited: TrustedRecord {
            source_commit: "cfe57f3bbf5269bfacf31e832267279caa6938ad",
            source_snapshot: "d8628e8103989bc4ef06dbfb9fd01f3809f884bf",
            module_version: 7,
            parameter_size: 628,
            parameter_layout_hash: "0bc1e7e0ed13d4e5673223dfc63c77206455e63c72fac2af2e20e307c5f8b863",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: &[
                TrustedVersion {
                    version: 1,
                    byte_size: 2840,
                    layout_hash: "041d3ea1040138fc2df089b5ad573614c6e1918bc16829ee9be9b8465d8e7e29",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 2,
                    byte_size: 3040,
                    layout_hash: "f7e113bc2d9d41dcb215374d54fc421ffdd72f129a605e4229a6d11faa392048",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 3,
                    byte_size: 3232,
                    layout_hash: "0d7e7ba74b44f36f829e893b088d2f22dd1c62dc56f5f02583a5242b8462cc15",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 4,
                    byte_size: 3576,
                    layout_hash: "f54766b9d00a1b126160ea011448052ae81ba941c39ea3ca591c8d289c77db12",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 5,
                    byte_size: 4376,
                    layout_hash: "6cd78cca03d074770616f400792d4a831da65b28a9de4b02c9c75386b5250432",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 6,
                    byte_size: 2752,
                    layout_hash: "1f4fb428b367449d672fd4bd39bcc29d5cc823b78f9f367831e1caa6fe82da09",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
                TrustedVersion {
                    version: 7,
                    byte_size: 628,
                    layout_hash: "cd91b832eae489d73b960f700ce8ccf804559762b42eebaeb3bf535e1f57b6e9",
                    decoder: "opaque",
                    opaque_blocking: true,
                    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
                    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
                },
            ],
            evidence_identity: "03a8bbb88abb3b7bbe5ade2aac10bbde6a29df9867c9e08f736d4ff6a9b3f3f3",
        },
    },
];

const COLISA_NATIVE_VERSIONS: &[TrustedVersion] = &[TrustedVersion {
    version: 1,
    byte_size: 12,
    layout_hash: "1786080f69acc48bae2e4b0c07107f8902aa2bf43e6aa6900bed388cc7d0f359",
    decoder: "generated.bytes.decode.v1",
    opaque_blocking: false,
    abi_identity: "ad658933041d7e0cae4758fe697dab72b9978945b23647129d67a74be2c861fd",
    codec_identity: "40224b3a3d844ceb0a4c1119e373d1f7733ea44adc6a6c1047a4c642511d1529",
}];

const COLISA_AUDITED_VERSIONS: &[TrustedVersion] = &[TrustedVersion {
    version: 1,
    byte_size: 12,
    layout_hash: "1786080f69acc48bae2e4b0c07107f8902aa2bf43e6aa6900bed388cc7d0f359",
    decoder: "opaque",
    opaque_blocking: true,
    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
}];

const PROFILE_GAMMA_NATIVE_VERSIONS: &[TrustedVersion] = &[
    TrustedVersion {
        version: 1,
        byte_size: 8,
        layout_hash: "63d81ca88d8255603f54e7e660198dc3f59d43d8f94d0a3564005c4f58bf4f0c",
        decoder: "generated.bytes.decode.v1",
        opaque_blocking: false,
        abi_identity: "b89aceff81335954d0d4a170ea77a486407e84d1b8a9eae913bd7784299b47d3",
        codec_identity: "5a3da642766fedbc22359f8aa1aff025d2d251b4071f10a78b5ddcb6c2d62e5c",
    },
    TrustedVersion {
        version: 2,
        byte_size: 28,
        layout_hash: "dca15454c85b9040af17fc29bcd93818b4e66eef714addab1841075fe5388d14",
        decoder: "generated.bytes.decode.v2",
        opaque_blocking: false,
        abi_identity: "d0c15b0308dd1b8e46337fbbcd9b626eaf869c002b6d07e0c5b4f3fb982312da",
        codec_identity: "ad8f4f733495c9fcd885b12a75f9617600d3a6e3fcea2bffa141e308f07baf5a",
    },
];

const PROFILE_GAMMA_AUDITED_VERSIONS: &[TrustedVersion] = &[
    TrustedVersion {
        version: 1,
        byte_size: 8,
        layout_hash: "63d81ca88d8255603f54e7e660198dc3f59d43d8f94d0a3564005c4f58bf4f0c",
        decoder: "opaque",
        opaque_blocking: true,
        abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
        codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
    },
    TrustedVersion {
        version: 2,
        byte_size: 28,
        layout_hash: "dca15454c85b9040af17fc29bcd93818b4e66eef714addab1841075fe5388d14",
        decoder: "opaque",
        opaque_blocking: true,
        abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
        codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
    },
];

const SPLITTONING_NATIVE_VERSIONS: &[TrustedVersion] = &[TrustedVersion {
    version: 1,
    byte_size: 24,
    layout_hash: "ad022576bd0e6a567c93eae6d042ccf6f10242fac44587388ce2aa6efebc3d41",
    decoder: "generated.bytes.decode.v1",
    opaque_blocking: false,
    abi_identity: "06a897ae899047267365b955cc5b7b98806702110a0c4ee1d0e96968f962756e",
    codec_identity: "0788afc91702d878d06476a48f95a3dc9ea2fe563667c48c6e6bdb3d57836ead",
}];

const SPLITTONING_AUDITED_VERSIONS: &[TrustedVersion] = &[TrustedVersion {
    version: 1,
    byte_size: 24,
    layout_hash: "ad022576bd0e6a567c93eae6d042ccf6f10242fac44587388ce2aa6efebc3d41",
    decoder: "opaque",
    opaque_blocking: true,
    abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
    codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
}];

/// Literal contracts for bounded leaves whose shared production seams remain deferred.
/// They are checked against both generated native records and audited overrides, but are
/// intentionally excluded from the production registry closure until their shared seams land.
pub const TRUSTED_DEFERRED_OPERATIONS: &[TrustedOperationContract] = &[
    TrustedOperationContract {
        compatibility_name: "profile_gamma",
        rust_id: "rusttable.profile_gamma",
        descriptor_version: 2,
        parameter_version: 2,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: NATIVE_SOURCE_COMMIT,
            source_snapshot: NATIVE_SOURCE_COMMIT,
            module_version: 2,
            parameter_size: 28,
            parameter_layout_hash: "b74d61e97c5f8d4d7053c1712e9f040290af18ae81a137ccf8639e2f724594af",
            native_order: Some(103),
            reference_path: Some("src/iop/profile_gamma.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "d0c15b0308dd1b8e46337fbbcd9b626eaf869c002b6d07e0c5b4f3fb982312da",
            codec_identity: "ad8f4f733495c9fcd885b12a75f9617600d3a6e3fcea2bffa141e308f07baf5a",
            parameter_versions: PROFILE_GAMMA_NATIVE_VERSIONS,
            evidence_identity: "f9105fab6add69c9134487b5d44c91b4de69d4c7471bcc7e24345f4208908d44",
        },
        audited: TrustedRecord {
            source_commit: NATIVE_SOURCE_COMMIT,
            source_snapshot: AUDITED_SOURCE_SNAPSHOT,
            module_version: 2,
            parameter_size: 28,
            parameter_layout_hash: "d5d718e7465e42bfb11216bfdd04abb62e5ffe061396aabf703137fc4bb1a760",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: PROFILE_GAMMA_AUDITED_VERSIONS,
            evidence_identity: "f9105fab6add69c9134487b5d44c91b4de69d4c7471bcc7e24345f4208908d44",
        },
    },
    TrustedOperationContract {
        compatibility_name: "splittoning",
        rust_id: "rusttable.splittoning",
        descriptor_version: 1,
        parameter_version: 1,
        implementation_version: 1,
        native: TrustedRecord {
            source_commit: NATIVE_SOURCE_COMMIT,
            source_snapshot: NATIVE_SOURCE_COMMIT,
            module_version: 1,
            parameter_size: 24,
            parameter_layout_hash: "a3ec8e1f316560821b03f356a29f68a9b23a3646d64a2a081af6c9c4eed65f0c",
            native_order: Some(99),
            reference_path: Some("src/iop/splittoning.c"),
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "06a897ae899047267365b955cc5b7b98806702110a0c4ee1d0e96968f962756e",
            codec_identity: "0788afc91702d878d06476a48f95a3dc9ea2fe563667c48c6e6bdb3d57836ead",
            parameter_versions: SPLITTONING_NATIVE_VERSIONS,
            evidence_identity: "15f7c97e9910236ef9e29dbf298683b9da7afae39e8656dff3053bcdf6f3b70d",
        },
        audited: TrustedRecord {
            source_commit: NATIVE_SOURCE_COMMIT,
            source_snapshot: AUDITED_SOURCE_SNAPSHOT,
            module_version: 1,
            parameter_size: 24,
            parameter_layout_hash: "73b60ac848fb3a6400e615e71aed321289cbe3790a5edf025dc29aaf27b536d0",
            native_order: None,
            reference_path: None,
            source_abi_model: "x86_64-unknown-linux-gnu",
            abi_identity: "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            codec_identity: "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b",
            parameter_versions: SPLITTONING_AUDITED_VERSIONS,
            evidence_identity: "15f7c97e9910236ef9e29dbf298683b9da7afae39e8656dff3053bcdf6f3b70d",
        },
    },
];

pub fn trusted_operation(name: &str) -> Option<&'static TrustedOperationContract> {
    TRUSTED_OPERATIONS
        .iter()
        .find(|contract| contract.compatibility_name == name)
}

pub fn trusted_names() -> impl Iterator<Item = &'static str> {
    TRUSTED_OPERATIONS
        .iter()
        .map(|contract| contract.compatibility_name)
}

/// The registry facts checked before a closure or capability artifact is generated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedRegistryEntry {
    pub compatibility_name: String,
    pub rust_id: String,
    pub descriptor_version: u16,
    pub parameter_version: u16,
    pub implementation_version: u16,
}

#[derive(Debug, Serialize)]
struct LayoutProjection<'a> {
    target: &'a str,
    c_abi_model: &'a str,
    endianness: &'a str,
    pointer_width: u16,
    fields: Vec<FieldProjection<'a>>,
    padding: Vec<PaddingProjection<'a>>,
    total_size: usize,
    alignment: usize,
    layout_hash: &'a str,
    difference_from: &'a [String],
}

#[derive(Debug, Serialize)]
struct FieldProjection<'a> {
    name: &'a str,
    type_name: &'a str,
    enum_identity: Option<&'a str>,
    enum_value: Option<i64>,
    array_extent: Option<usize>,
    offset: usize,
    size: usize,
    alignment: usize,
}

#[derive(Debug, Serialize)]
struct PaddingProjection<'a> {
    offset: usize,
    size: usize,
    kind: &'a str,
}

#[derive(Debug, Serialize)]
struct CodecProjection<'a> {
    byte_size: usize,
    decoder: &'a str,
    encoder: &'a str,
    byte_order: &'a str,
    fields: Vec<CodecFieldProjection<'a>>,
    preserves_padding: bool,
    format: &'a str,
}

#[derive(Debug, Serialize)]
struct CodecFieldProjection<'a> {
    name: &'a str,
    kind: &'a str,
    offset: usize,
    size: usize,
    array_extent: Option<usize>,
    enum_values: Vec<EnumProjection<'a>>,
}

#[derive(Debug, Serialize)]
struct EnumProjection<'a> {
    name: &'a str,
    value: i64,
}

#[derive(Debug, Serialize)]
struct EvidenceProjection<'a> {
    field: String,
    source_commit: &'a str,
    source_path: Option<&'a str>,
    line_start: Option<u32>,
    line_end: Option<u32>,
    fixture_id: Option<&'a str>,
    reason: &'a str,
    reviewer: &'a str,
    evidence_hash: &'a str,
}

fn digest<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).expect("trusted provenance projection serializes");
    let mut output = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing a digest cannot fail");
    }
    output
}

fn layout_projection(layout: &AbiLayout) -> LayoutProjection<'_> {
    LayoutProjection {
        target: &layout.target,
        c_abi_model: &layout.c_abi_model,
        endianness: &layout.endianness,
        pointer_width: layout.pointer_width,
        fields: layout
            .fields
            .iter()
            .map(|field| FieldProjection {
                name: &field.name,
                type_name: &field.type_name,
                enum_identity: field.enum_identity.as_deref(),
                enum_value: field.enum_value,
                array_extent: field.array_extent,
                offset: field.offset,
                size: field.size,
                alignment: field.alignment,
            })
            .collect(),
        padding: layout
            .padding
            .iter()
            .map(|padding| PaddingProjection {
                offset: padding.offset,
                size: padding.size,
                kind: &padding.kind,
            })
            .collect(),
        total_size: layout.total_size,
        alignment: layout.alignment,
        layout_hash: &layout.layout_hash,
        difference_from: &layout.difference_from,
    }
}

fn abi_identity(layouts: &[AbiLayout]) -> String {
    let projections: Vec<_> = layouts.iter().map(layout_projection).collect();
    digest(&projections)
}

fn codec_projection(codec: &ParameterCodec) -> CodecProjection<'_> {
    CodecProjection {
        byte_size: codec.byte_size,
        decoder: &codec.decoder,
        encoder: &codec.encoder,
        byte_order: &codec.byte_order,
        fields: codec
            .fields
            .iter()
            .map(|field| CodecFieldProjection {
                name: &field.name,
                kind: &field.kind,
                offset: field.offset,
                size: field.size,
                array_extent: field.array_extent,
                enum_values: field
                    .enum_values
                    .iter()
                    .map(|value| EnumProjection {
                        name: &value.name,
                        value: value.value,
                    })
                    .collect(),
            })
            .collect(),
        preserves_padding: codec.preserves_padding,
        format: &codec.format,
    }
}

fn codec_identity(codec: Option<&ParameterCodec>) -> String {
    digest(&codec.map(codec_projection))
}

fn evidence_projection(field: impl Into<String>, evidence: &Evidence) -> EvidenceProjection<'_> {
    EvidenceProjection {
        field: field.into(),
        source_commit: &evidence.source_commit,
        source_path: evidence.source_path.as_deref(),
        line_start: evidence.line_start,
        line_end: evidence.line_end,
        fixture_id: evidence.fixture_id.as_deref(),
        reason: &evidence.reason,
        reviewer: &evidence.reviewer,
        evidence_hash: &evidence.evidence_hash,
    }
}

fn evidence_identity(
    evidence: &[super::model::OperationEvidence],
    versions: &[ParameterVersion],
    migrations: &[ParameterMigration],
) -> String {
    let mut projections = Vec::with_capacity(evidence.len() + versions.len() + migrations.len());
    projections.extend(
        evidence
            .iter()
            .map(|item| evidence_projection(item.field.clone(), &item.evidence)),
    );
    projections.extend(versions.iter().map(|version| {
        evidence_projection(
            format!("parameter-version:{}", version.version),
            &version.evidence,
        )
    }));
    projections.extend(migrations.iter().map(|migration| {
        evidence_projection(
            format!(
                "migration:{}-{}",
                migration.from_version, migration.to_version
            ),
            &migration.evidence,
        )
    }));
    digest(&projections)
}

fn mismatch<T: fmt::Debug>(
    operation: &str,
    record: &str,
    field: &str,
    expected: &T,
    actual: &T,
) -> ScanError {
    ScanError::TrustedProvenance {
        operation: operation.to_owned(),
        record: record.to_owned(),
        field: field.to_owned(),
        expected: format!("{expected:?}"),
        actual: format!("{actual:?}"),
    }
}

fn require_equal<T: PartialEq + fmt::Debug>(
    operation: &str,
    record: &str,
    field: &str,
    expected: &T,
    actual: &T,
) -> Result<(), ScanError> {
    if expected == actual {
        Ok(())
    } else {
        Err(mismatch(operation, record, field, expected, actual))
    }
}

fn require_text(
    operation: &str,
    record: &str,
    field: &str,
    expected: &str,
    actual: &str,
) -> Result<(), ScanError> {
    if expected == actual {
        Ok(())
    } else {
        Err(mismatch(operation, record, field, &expected, &actual))
    }
}

fn trusted_for_source_name(name: &str) -> Option<&'static TrustedOperationContract> {
    let normalized = generated_compatibility_name(name);
    trusted_operation(normalized).or_else(|| {
        TRUSTED_DEFERRED_OPERATIONS
            .iter()
            .find(|contract| contract.compatibility_name == normalized)
    })
}

fn source_name(contract: &TrustedOperationContract) -> Option<&'static str> {
    match contract.compatibility_name {
        TRUSTED_LENS_NAME => Some(MANIFEST_LENS_NAME),
        GENERATED_LINEAR_OFFSET_ALIAS => Some(MANIFEST_LINEAR_OFFSET_NAME),
        GENERATED_RGB_GAIN_ALIAS => Some(MANIFEST_RGB_GAIN_NAME),
        _ => match contract.native.reference_path {
            Some(path) if !path.is_empty() => Some(contract.compatibility_name),
            _ => None,
        },
    }
}

/// Returns the registry compatibility name corresponding to a manifest name.
#[must_use]
pub fn generated_compatibility_name(manifest_name: &str) -> &str {
    GENERATED_COMPATIBILITY_ALIASES
        .iter()
        .find_map(|(source, generated)| (*source == manifest_name).then_some(*generated))
        .unwrap_or(manifest_name)
}

/// Returns whether a manifest record has an independent native or deferred trust contract.
#[must_use]
pub fn is_independently_trusted_manifest_name(manifest_name: &str) -> bool {
    source_name_for_name(manifest_name).is_some()
}

fn source_name_for_name(name: &str) -> Option<&'static str> {
    TRUSTED_OPERATIONS
        .iter()
        .chain(TRUSTED_DEFERRED_OPERATIONS)
        .find_map(|contract| {
            let source_name = source_name(contract)?;
            (source_name == name).then_some(source_name)
        })
}

#[expect(
    clippy::too_many_lines,
    reason = "The provenance validator compares the complete native ABI, codec, version, and evidence contract in source order."
)]
fn validate_sources(
    operation: &str,
    record: &str,
    expected: &TrustedRecord,
    reference: &ReferenceIdentity,
    actual: &Operation,
) -> Result<(), ScanError> {
    let expected_snapshot = if record == "audited" {
        AUDITED_SOURCE_SNAPSHOT
    } else {
        NATIVE_SOURCE_COMMIT
    };
    require_text(
        operation,
        record,
        "source_snapshot",
        expected_snapshot,
        expected.source_snapshot,
    )?;
    require_text(
        operation,
        record,
        "source_commit",
        expected.source_commit,
        &reference.source_commit,
    )?;
    require_text(
        operation,
        record,
        "source_abi_model",
        expected.source_abi_model,
        &reference.c_abi_model,
    )?;
    require_equal(
        operation,
        record,
        "module_version",
        &expected.module_version,
        &actual.module_version,
    )?;
    require_equal(
        operation,
        record,
        "parameter_size",
        &expected.parameter_size,
        &actual.parameter_size,
    )?;
    require_text(
        operation,
        record,
        "parameter_layout_hash",
        expected.parameter_layout_hash,
        &actual.parameter_layout_hash,
    )?;
    if let Some(reference_path) = expected.reference_path {
        require_text(
            operation,
            record,
            "reference_path",
            reference_path,
            &actual.reference_path,
        )?;
    }
    if let Some(order) = expected.native_order {
        require_equal(
            operation,
            record,
            "native_order",
            &order,
            &actual.default_order,
        )?;
    }
    let actual_abi_identity = abi_identity(&actual.abi_layouts);
    require_text(
        operation,
        record,
        "abi_identity",
        expected.abi_identity,
        &actual_abi_identity,
    )?;
    let actual_codec_identity = codec_identity(actual.codec.as_ref());
    require_text(
        operation,
        record,
        "codec_identity",
        expected.codec_identity,
        &actual_codec_identity,
    )?;
    validate_versions(
        operation,
        record,
        expected.parameter_versions,
        &actual.parameter_versions,
    )?;
    validate_evidence_commits(
        operation,
        record,
        expected.source_commit,
        &actual.evidence,
        &actual.parameter_versions,
        &actual.migrations,
    )?;
    let actual_evidence_identity = evidence_identity(
        &actual.evidence,
        &actual.parameter_versions,
        &actual.migrations,
    );
    require_text(
        operation,
        record,
        "source_map_evidence_identity",
        expected.evidence_identity,
        &actual_evidence_identity,
    )
}

fn validate_versions(
    operation: &str,
    record: &str,
    expected: &[TrustedVersion],
    actual: &[ParameterVersion],
) -> Result<(), ScanError> {
    if expected.len() != actual.len() {
        return Err(mismatch(
            operation,
            record,
            "parameter_versions",
            &expected.len(),
            &actual.len(),
        ));
    }
    for (index, (expected, actual)) in expected.iter().zip(actual).enumerate() {
        let field = |name: &str| format!("parameter_versions[{index}].{name}");
        require_equal(
            operation,
            record,
            &field("version"),
            &expected.version,
            &actual.version,
        )?;
        require_equal(
            operation,
            record,
            &field("byte_size"),
            &expected.byte_size,
            &actual.byte_size,
        )?;
        require_text(
            operation,
            record,
            &field("layout_hash"),
            expected.layout_hash,
            &actual.layout_hash,
        )?;
        require_text(
            operation,
            record,
            &field("decoder"),
            expected.decoder,
            &actual.decoder,
        )?;
        require_equal(
            operation,
            record,
            &field("opaque_blocking"),
            &expected.opaque_blocking,
            &actual.opaque_blocking,
        )?;
        let actual_abi_identity = abi_identity(&actual.abi_layouts);
        require_text(
            operation,
            record,
            &field("abi_identity"),
            expected.abi_identity,
            &actual_abi_identity,
        )?;
        let actual_codec_identity = codec_identity(actual.codec.as_ref());
        require_text(
            operation,
            record,
            &field("codec_identity"),
            expected.codec_identity,
            &actual_codec_identity,
        )?;
    }
    Ok(())
}

fn validate_evidence_commits(
    operation: &str,
    record: &str,
    expected_commit: &str,
    evidence: &[super::model::OperationEvidence],
    versions: &[ParameterVersion],
    migrations: &[ParameterMigration],
) -> Result<(), ScanError> {
    for (index, item) in evidence.iter().enumerate() {
        require_text(
            operation,
            record,
            &format!("evidence[{index}].source_commit"),
            expected_commit,
            &item.evidence.source_commit,
        )?;
    }
    for (index, version) in versions.iter().enumerate() {
        require_text(
            operation,
            record,
            &format!("parameter_versions[{index}].evidence.source_commit"),
            expected_commit,
            &version.evidence.source_commit,
        )?;
    }
    for (index, migration) in migrations.iter().enumerate() {
        require_text(
            operation,
            record,
            &format!("migrations[{index}].evidence.source_commit"),
            expected_commit,
            &migration.evidence.source_commit,
        )?;
    }
    Ok(())
}

/// Validates one native manifest record against the checked-in contract.
pub fn validate_native_operation_provenance(
    reference: &ReferenceIdentity,
    operation: &Operation,
) -> Result<(), ScanError> {
    let Some(contract) = trusted_for_source_name(&operation.name) else {
        return Ok(());
    };
    validate_sources(
        &operation.name,
        "native",
        &contract.native,
        reference,
        operation,
    )
}

#[expect(
    clippy::too_many_lines,
    reason = "Audited override validation checks every optional ABI, decoder, migration, and evidence field before application."
)]
fn validate_override_fields(
    entry: &OperationOverride,
    expected: &TrustedRecord,
) -> Result<(), ScanError> {
    let operation = entry.name.as_str();
    let record = "audited";
    require_text(
        operation,
        record,
        "source_snapshot",
        AUDITED_SOURCE_SNAPSHOT,
        expected.source_snapshot,
    )?;
    let require_option = |field: &str, actual: Option<String>, expected: String| {
        let Some(actual) = actual else {
            let missing = "missing".to_owned();
            return Err(mismatch(operation, record, field, &expected, &missing));
        };
        require_equal(operation, record, field, &expected, &actual)
    };
    require_option(
        "module_version",
        entry.module_version.map(|value| value.to_string()),
        expected.module_version.to_string(),
    )?;
    require_option(
        "parameter_size",
        entry.parameter_size.map(|value| value.to_string()),
        expected.parameter_size.to_string(),
    )?;
    require_option(
        "parameter_layout_hash",
        entry.parameter_layout_hash.clone(),
        expected.parameter_layout_hash.to_owned(),
    )?;
    if let Some(order) = entry.default_order {
        let expected_order = expected.native_order.map(|value| value.to_string());
        require_equal(
            operation,
            record,
            "native_order",
            &expected_order,
            &Some(order.to_string()),
        )?;
    } else if expected.native_order.is_some() {
        return Err(mismatch(
            operation,
            record,
            "native_order",
            &expected.native_order,
            &None::<usize>,
        ));
    }
    if let Some(decoder) = &entry.parameter_decoder
        && let Some(version) = expected.parameter_versions.last()
    {
        require_text(
            operation,
            record,
            "parameter_decoder",
            version.decoder,
            decoder,
        )?;
    }
    match (
        &entry.parameter_versions,
        expected.parameter_versions.is_empty(),
    ) {
        (Some(actual), _) => {
            validate_versions(operation, record, expected.parameter_versions, actual)?;
        }
        (None, false) => {
            return Err(mismatch(
                operation,
                record,
                "parameter_versions",
                &"present",
                &"missing",
            ));
        }
        (None, true) => {}
    }
    if let Some(layouts) = &entry.abi_layouts {
        let actual = abi_identity(layouts);
        require_text(
            operation,
            record,
            "abi_identity",
            expected.abi_identity,
            &actual,
        )?;
    }
    if let Some(codec) = &entry.codec {
        let actual = codec_identity(Some(codec));
        require_text(
            operation,
            record,
            "codec_identity",
            expected.codec_identity,
            &actual,
        )?;
    }
    let Some(evidence) = &entry.evidence else {
        return Err(mismatch(
            operation,
            record,
            "source_map_evidence_identity",
            &expected.evidence_identity,
            &"missing",
        ));
    };
    let versions = entry.parameter_versions.as_deref().unwrap_or(&[]);
    let migrations = entry.migrations.as_deref().unwrap_or(&[]);
    validate_evidence_commits(
        operation,
        record,
        expected.source_commit,
        evidence,
        versions,
        migrations,
    )?;
    let actual = evidence_identity(evidence, versions, migrations);
    require_text(
        operation,
        record,
        "source_map_evidence_identity",
        expected.evidence_identity,
        &actual,
    )
}

/// Validates one override against the independent audited contract, when known.
pub fn validate_operation_override_provenance(entry: &OperationOverride) -> Result<(), ScanError> {
    let Some(contract) = trusted_for_source_name(&entry.name) else {
        return Ok(());
    };
    validate_override_fields(entry, &contract.audited)
}

pub fn validate_operation_override_names(
    overrides: &[OperationOverride],
    expected_names: &BTreeSet<String>,
) -> Result<(), ScanError> {
    let mut positions = BTreeMap::new();
    for (position, entry) in overrides.iter().enumerate() {
        let name = entry.name.trim();
        if name.is_empty() {
            return Err(ScanError::InvalidOverrides {
                message: format!("override entry {position} has an empty operation name"),
            });
        }
        if name != entry.name {
            return Err(ScanError::InvalidOverrides {
                message: format!(
                    "override entry {position} has leading or trailing whitespace in its operation name"
                ),
            });
        }
        if let Some(first_position) = positions.insert(name.to_owned(), position) {
            return Err(ScanError::InvalidOverrides {
                message: format!(
                    "duplicate override name {name:?}: entries {first_position} and {position}"
                ),
            });
        }
        if !expected_names.contains(name) {
            return Err(ScanError::InvalidOverrides {
                message: format!("override name {name:?} is absent from the architecture manifest"),
            });
        }
    }
    let missing = expected_names
        .iter()
        .filter(|name| !positions.contains_key(*name))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(ScanError::InvalidOverrides {
            message: format!(
                "manifest/override name mismatch: missing overrides for {}",
                missing.join(", ")
            ),
        });
    }
    Ok(())
}

pub fn validate_operation_overrides(
    overrides: &[OperationOverride],
    expected_names: &BTreeSet<String>,
) -> Result<(), ScanError> {
    validate_operation_override_names(overrides, expected_names)?;
    for entry in overrides {
        validate_operation_override_provenance(entry)?;
    }
    Ok(())
}

fn validate_generated_aliases() -> Result<(), ScanError> {
    let mut source_names = BTreeSet::new();
    let mut generated_names = BTreeSet::new();
    for (source_name, generated_name) in GENERATED_COMPATIBILITY_ALIASES {
        if !source_names.insert(*source_name) {
            return Err(mismatch(
                "registry",
                "aliases",
                "source_name",
                &"unique",
                source_name,
            ));
        }
        if !generated_names.insert(*generated_name) {
            return Err(mismatch(
                "registry",
                "aliases",
                "compatibility_name",
                &"unique",
                generated_name,
            ));
        }
        if trusted_operation(generated_name).is_none()
            && !TRUSTED_DEFERRED_OPERATIONS
                .iter()
                .any(|contract| contract.compatibility_name == *generated_name)
        {
            return Err(mismatch(
                "registry",
                "aliases",
                "compatibility_name",
                &"trusted contract",
                generated_name,
            ));
        }
    }
    Ok(())
}

fn expected_architecture_manifest_names() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for contract in TRUSTED_OPERATIONS.iter().chain(TRUSTED_DEFERRED_OPERATIONS) {
        // Registry-only contracts retain a normalized alias for direct lookup, but they do not
        // create a second native manifest record when their reference path is absent.
        if contract.native.reference_path.is_some()
            && let Some(name) = source_name(contract)
        {
            names.insert(name.to_owned());
        }
    }
    names.extend(
        OPAQUE_DEFERRED_MANIFEST_NAMES
            .iter()
            .map(|name| (*name).to_owned()),
    );
    names
}

fn validate_architecture_manifest_names(
    manifest_names: &BTreeSet<String>,
) -> Result<(), ScanError> {
    validate_generated_aliases()?;
    let expected = expected_architecture_manifest_names();
    let trusted_contract_count = TRUSTED_OPERATIONS.len() + TRUSTED_DEFERRED_OPERATIONS.len();
    let anchored_count = TRUSTED_OPERATIONS
        .iter()
        .chain(TRUSTED_DEFERRED_OPERATIONS)
        .filter(|contract| {
            contract.native.reference_path.is_some() && source_name(contract).is_some()
        })
        .count();
    // The two registry-only aliases are trusted contracts without native manifest rows. Keep them
    // in the trusted-contract total while counting only manifest-backed records here. The
    // cross-domain 57/36 summary is intentionally separate from the native 55/38 row split.
    let manifest_unanchored_count = expected.len().saturating_sub(anchored_count);
    let registry_only_count = trusted_contract_count.saturating_sub(anchored_count);
    let cross_domain_unanchored_summary = expected.len().saturating_sub(trusted_contract_count);
    if expected.len() != EXPECTED_ARCHITECTURE_RECORDS
        || trusted_contract_count != EXPECTED_TRUSTED_RECORDS
        || anchored_count != EXPECTED_MANIFEST_TRUSTED_RECORDS
        || registry_only_count != EXPECTED_REGISTRY_ONLY_TRUSTED_RECORDS
        || manifest_unanchored_count != EXPECTED_MANIFEST_UNANCHORED_RECORDS
        || cross_domain_unanchored_summary != EXPECTED_CROSS_DOMAIN_UNANCHORED_SUMMARY
    {
        return Err(mismatch(
            "registry",
            "architecture",
            "record_accounting",
            &(
                EXPECTED_ARCHITECTURE_RECORDS,
                EXPECTED_TRUSTED_RECORDS,
                EXPECTED_MANIFEST_TRUSTED_RECORDS,
                EXPECTED_REGISTRY_ONLY_TRUSTED_RECORDS,
                EXPECTED_MANIFEST_UNANCHORED_RECORDS,
                EXPECTED_CROSS_DOMAIN_UNANCHORED_SUMMARY,
            ),
            &(
                expected.len(),
                trusted_contract_count,
                anchored_count,
                registry_only_count,
                manifest_unanchored_count,
                cross_domain_unanchored_summary,
            ),
        ));
    }
    if manifest_names != &expected {
        return Err(mismatch(
            "manifest",
            "architecture",
            "operation_names",
            &expected,
            manifest_names,
        ));
    }
    Ok(())
}

/// Validates the generated status for every native manifest capability record.
///
/// # Errors
///
/// Returns a typed provenance error when a record is missing, duplicated, or has a status that
/// does not match its independent trust classification.
pub fn validate_manifest_capability_accounting(
    records: &[(String, String)],
) -> Result<(), ScanError> {
    validate_generated_aliases()?;
    let expected_names = expected_architecture_manifest_names();
    let actual_names = records
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<BTreeSet<_>>();
    if actual_names != expected_names {
        return Err(mismatch(
            "manifest",
            "capability",
            "operation_names",
            &expected_names,
            &actual_names,
        ));
    }
    let mut names = BTreeSet::new();
    let mut trusted_count = 0usize;
    let mut unanchored_count = 0usize;
    for (name, status) in records {
        if !names.insert(name.as_str()) {
            let expected = "unique".to_owned();
            return Err(mismatch(
                name,
                "capability",
                "compatibility_name",
                &expected,
                name,
            ));
        }
        let trusted = is_independently_trusted_manifest_name(name);
        let status_allowed = if trusted {
            trusted_count += 1;
            matches!(
                status.as_str(),
                "Implemented" | "DeprecatedImplemented" | "IntentionallyUnsupportedBlocking"
            )
        } else {
            unanchored_count += 1;
            matches!(
                status.as_str(),
                "ReferenceOnlyNonProduct" | "Opaque" | "Deferred"
            )
        };
        if !status_allowed {
            let expected_status = if trusted {
                "Implemented/DeprecatedImplemented/IntentionallyUnsupportedBlocking"
            } else {
                "ReferenceOnlyNonProduct/Opaque/Deferred"
            };
            let expected_status = expected_status.to_owned();
            return Err(mismatch(
                name,
                "capability",
                "status",
                &expected_status,
                status,
            ));
        }
    }
    if records.len() != EXPECTED_ARCHITECTURE_RECORDS
        || trusted_count != EXPECTED_MANIFEST_TRUSTED_RECORDS
        || unanchored_count != EXPECTED_MANIFEST_UNANCHORED_RECORDS
    {
        return Err(mismatch(
            "manifest",
            "capability",
            "record_accounting",
            &(
                EXPECTED_ARCHITECTURE_RECORDS,
                EXPECTED_MANIFEST_TRUSTED_RECORDS,
                EXPECTED_MANIFEST_UNANCHORED_RECORDS,
            ),
            &(records.len(), trusted_count, unanchored_count),
        ));
    }
    Ok(())
}

/// Validates the canonical manifest and its separate audited override records.
///
/// # Errors
///
/// Returns a typed provenance error when any native, audited, or source-map identity diverges.
pub fn validate_architecture_provenance(
    manifest: &OperationManifest,
    overrides: &[OperationOverride],
) -> Result<(), ScanError> {
    let manifest_names = manifest
        .operations
        .iter()
        .map(|operation| operation.name.clone())
        .collect::<BTreeSet<_>>();
    validate_architecture_manifest_names(&manifest_names)?;
    validate_operation_overrides(overrides, &manifest_names)?;
    super::validate::validate_operation_manifest(manifest)?;
    require_text(
        "reference",
        "native",
        "source_commit",
        NATIVE_SOURCE_COMMIT,
        &manifest.reference.source_commit,
    )?;
    let expected_sources = TRUSTED_OPERATIONS
        .iter()
        .chain(TRUSTED_DEFERRED_OPERATIONS)
        .filter(|contract| {
            contract.native.reference_path.is_some() && source_name(contract).is_some()
        })
        .count();
    let manifest_by_name = manifest
        .operations
        .iter()
        .map(|operation| (operation.name.as_str(), operation))
        .collect::<BTreeMap<_, _>>();
    let overrides_by_name = overrides
        .iter()
        .map(|entry| (entry.name.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let mut checked_sources = 0usize;
    for contract in TRUSTED_OPERATIONS.iter().chain(TRUSTED_DEFERRED_OPERATIONS) {
        let Some(name) = source_name(contract) else {
            continue;
        };
        if contract.native.reference_path.is_none() {
            // `linear-offset` and `rgbgain` are trusted registry contracts, not native source
            // records. Their normalized aliases are checked through the registry contract.
            continue;
        }
        checked_sources += 1;
        let Some(operation) = manifest_by_name.get(name).copied() else {
            return Err(mismatch(
                contract.compatibility_name,
                "native",
                "record",
                &"present",
                &"missing",
            ));
        };
        validate_sources(
            contract.compatibility_name,
            "native",
            &contract.native,
            &manifest.reference,
            operation,
        )?;
        let Some(entry) = overrides_by_name.get(name).copied() else {
            return Err(mismatch(
                contract.compatibility_name,
                "audited",
                "record",
                &"present",
                &"missing",
            ));
        };
        validate_override_fields(entry, &contract.audited)?;
    }
    if checked_sources != expected_sources {
        return Err(mismatch(
            "registry",
            "native",
            "source_record_count",
            &expected_sources,
            &checked_sources,
        ));
    }
    Ok(())
}

/// Validates registry descriptor identities before closure construction.
///
/// # Errors
///
/// Returns a typed provenance error when a built-in definition is absent or its identity changes.
pub fn validate_trusted_registry_entries(
    entries: &[TrustedRegistryEntry],
) -> Result<(), ScanError> {
    let expected_names = trusted_names().collect::<BTreeSet<_>>();
    let actual_names = entries
        .iter()
        .map(|entry| entry.compatibility_name.as_str())
        .collect::<BTreeSet<_>>();
    if entries.len() != expected_names.len() || actual_names != expected_names {
        return Err(mismatch(
            "registry",
            "registry",
            "operation_names",
            &expected_names,
            &actual_names,
        ));
    }
    for entry in entries {
        let Some(contract) = trusted_operation(&entry.compatibility_name) else {
            let expected = "trusted operation".to_owned();
            return Err(mismatch(
                &entry.compatibility_name,
                "registry",
                "compatibility_name",
                &expected,
                &entry.compatibility_name,
            ));
        };
        require_text(
            &entry.compatibility_name,
            "registry",
            "rust_id",
            contract.rust_id,
            &entry.rust_id,
        )?;
        require_equal(
            &entry.compatibility_name,
            "registry",
            "descriptor_version",
            &contract.descriptor_version,
            &entry.descriptor_version,
        )?;
        require_equal(
            &entry.compatibility_name,
            "registry",
            "parameter_version",
            &contract.parameter_version,
            &entry.parameter_version,
        )?;
        require_equal(
            &entry.compatibility_name,
            "registry",
            "implementation_version",
            &contract.implementation_version,
            &entry.implementation_version,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::trusted_operation;

    #[test]
    fn tonal_contracts_pin_native_and_legacy_parameter_sizes() {
        for (name, expected_sizes) in [
            ("basecurve", &[52, 504, 512, 512, 516, 520][..]),
            ("tonecurve", &[52, 512, 516, 520][..]),
        ] {
            let contract = trusted_operation(name).expect("tonal trust contract");
            assert_eq!(
                contract.native.parameter_size,
                *expected_sizes.last().unwrap()
            );
            assert_eq!(
                contract
                    .native
                    .parameter_versions
                    .iter()
                    .map(|version| version.byte_size)
                    .collect::<Vec<_>>(),
                expected_sizes
            );
            assert!(
                contract
                    .native
                    .parameter_versions
                    .iter()
                    .all(|version| !version.opaque_blocking)
            );
        }
    }
}
