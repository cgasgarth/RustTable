//! Source-to-Rust responsibility map for the bounded Color Correction CPU leaf.

#![forbid(unsafe_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorCorrectionSourceMapEntry {
    pub native_symbol: &'static str,
    pub native_file: &'static str,
    pub rust_symbol: &'static str,
    pub status: ColorCorrectionPortStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorCorrectionPortStatus {
    Ported,
    SourceEvidenceOnly,
    RustAdaptation,
    ExplicitlyDeferred,
}

/// The root retained `CMakeLists.txt` selects this build type when none is
/// supplied. The profile-specific flags below are the additions made by retained
/// `src/CMakeLists.txt`; toolchain-provided defaults remain compiler dependent.
pub const COLORCORRECTION_NATIVE_DEFAULT_BUILD_TYPE: &str = "RelWithDebInfo";
pub const COLORCORRECTION_NATIVE_COMMON_NON_CUSTOM_C_FLAG: &str = "-g";
pub const COLORCORRECTION_NATIVE_DEBUG_NON_CUSTOM_C_FLAGS: [&str; 1] = ["-O0"];
pub const COLORCORRECTION_NATIVE_DEBUG_DEFINE: &str = "-D_DEBUG";
pub const COLORCORRECTION_NATIVE_GNU_DEBUG_EXTRA: [&str; 2] = ["-g3", "-ggdb3"];
pub const COLORCORRECTION_NATIVE_RELWITHDEBINFO_C_FLAGS: [&str; 2] = ["-O2", "-ftree-vectorize"];
pub const COLORCORRECTION_NATIVE_RELEASE_C_FLAGS: [&str; 3] =
    ["-O3", concat!("-ffast", "-math"), "-fno-finite-math-only"];
pub const COLORCORRECTION_NATIVE_GNU_RELEASE_EXTRA: &str = "-fexpensive-optimizations";

/// Release fast-math expressly permits both transformations. Native Debug,
/// default `RelWithDebInfo`, and Release output bits are nevertheless not fixed by
/// this source map: compiler defaults, target selection, and contraction choices
/// remain part of each native binary's profile.
pub const COLORCORRECTION_NATIVE_RELEASE_PERMITS_FMA: bool = true;
pub const COLORCORRECTION_NATIVE_RELEASE_PERMITS_REASSOCIATION: bool = true;
pub const COLORCORRECTION_NATIVE_PROFILE_BITS_PORTABLE: bool = false;

/// Native CPU storage and scheduling facts used by `process`. `pixelpipe_hb.c`
/// derives a 16-byte four-f32 raster payload per pixel, obtains input/output
/// cachelines from `pixelpipe_cache.c`, and rejects buffers that are not aligned
/// to `DT_CACHELINE_BYTES` before calling the module. Color Correction then
/// applies the same cache-line assumption to both pointers and uses
/// `DT_OMP_FOR()` (static OpenMP scheduling when enabled) for its pixel loop.
pub const COLORCORRECTION_NATIVE_CACHELINE_BYTES_APPLE_AARCH64: usize = 128;
pub const COLORCORRECTION_NATIVE_CACHELINE_BYTES_OTHER: usize = 64;
pub const COLORCORRECTION_NATIVE_CACHELINE_PIXELS_APPLE_AARCH64: usize = 8;
pub const COLORCORRECTION_NATIVE_CACHELINE_PIXELS_OTHER: usize = 4;
pub const COLORCORRECTION_NATIVE_RASTER_PIXEL_BYTES: usize = 16;
pub const COLORCORRECTION_NATIVE_ALIGNED_PIXEL_ALIGNMENT_BYTES: usize = 16;
pub const COLORCORRECTION_NATIVE_OPENMP_STATIC_SCHEDULE: bool = true;
pub const COLORCORRECTION_NATIVE_CACHE_OWNS_ALIGNED_BUFFERS: bool = true;
pub const COLORCORRECTION_NATIVE_CACHE_TRACKS_REQUESTED_PAYLOAD_BYTES: bool = true;

/// Safe-Rust adaptation: the leaf uses a serial loop and an ordinary `Vec` whose
/// required alignment is only `align_of::<ColorCorrectionPixel>()`. Its element
/// payload is still 16 bytes, but it makes neither the separate native 16-byte
/// aligned-pixel promise nor the stronger 64/128-byte cache-line promise and
/// applies no aligned-pointer assumption.
pub const COLORCORRECTION_RUST_PIXEL_BYTES: usize =
    std::mem::size_of::<super::execution::ColorCorrectionPixel>();
pub const COLORCORRECTION_RUST_PIXEL_ALIGNMENT_BYTES: usize =
    std::mem::align_of::<super::execution::ColorCorrectionPixel>();
pub const COLORCORRECTION_RUST_EXECUTION_IS_SERIAL: bool = true;
pub const COLORCORRECTION_RUST_USES_NATIVE_ALIGNMENT_CONTRACT: bool = false;

/// Budget values describe raster payload bytes. Native host-fit and cache
/// accounting add no cache-line padding. Generic `dt_alloc_aligned` rounding can
/// add alignment-minus-one bytes, but a Color Correction raster request is a
/// multiple of its 16-byte pixel payload, tightening those maxima to 112 and 48.
/// The POSIX Debug branch requests one additional cache line. The Rust caller
/// budget likewise adds no alignment padding or allocator metadata around its
/// ordinary `Vec`.
pub const COLORCORRECTION_NATIVE_HOST_BUDGET_ALIGNMENT_PADDING_BYTES: usize = 0;
pub const COLORCORRECTION_NATIVE_ALLOCATOR_GENERIC_MAX_TAIL_PADDING_APPLE_AARCH64: usize = 127;
pub const COLORCORRECTION_NATIVE_ALLOCATOR_GENERIC_MAX_TAIL_PADDING_OTHER: usize = 63;
pub const COLORCORRECTION_NATIVE_RASTER_ALLOCATION_MAX_TAIL_PADDING_APPLE_AARCH64: usize = 112;
pub const COLORCORRECTION_NATIVE_RASTER_ALLOCATION_MAX_TAIL_PADDING_OTHER: usize = 48;
pub const COLORCORRECTION_NATIVE_POSIX_DEBUG_ALLOCATION_EXTRA_CACHELINES: usize = 1;
pub const COLORCORRECTION_RUST_BUDGETED_ALIGNMENT_PADDING_BYTES: usize = 0;
pub const COLORCORRECTION_RUST_BUDGETED_ALLOCATOR_METADATA_BYTES: usize = 0;

/// Native `dt_tiling_piece_fits_host_memory` evaluates
/// `factor * width * height * bpp + overhead` in f32 because `factor` is a
/// `float`, then converts that rounded result to `size_t`. The bounded Rust leaf
/// deliberately retains checked integer payload sizing instead. At 4097² pixels,
/// factor two, and 16 bytes per pixel, native source-language f32 evaluation is
/// 32 bytes below the exact integer payload; these constants make that adaptation
/// reviewable without changing execution arithmetic.
pub const COLORCORRECTION_BUDGET_BOUNDARY_EDGE: usize = 4_097;
pub const COLORCORRECTION_BUDGET_BOUNDARY_FACTOR: usize = 2;
pub const COLORCORRECTION_BUDGET_BOUNDARY_BYTES_PER_PIXEL: usize = 16;
pub const COLORCORRECTION_NATIVE_F32_FACTOR_TWO_BUDGET_BITS: u32 = 0x4e00_1000;
pub const COLORCORRECTION_NATIVE_F32_FACTOR_TWO_BUDGET_BYTES: usize = 537_133_056;
pub const COLORCORRECTION_RUST_CHECKED_FACTOR_TWO_BUDGET_BYTES: usize = 537_133_088;
pub const COLORCORRECTION_FACTOR_TWO_BUDGET_SHORTFALL_BYTES: usize = 32;

/// Distinguishes values read from the retained callback from additive choices
/// made by the bounded safe-Rust execution boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorCorrectionTilingProvenance {
    RetainedDefaultCallback,
    RustTransactionalPolicy,
}

/// Exact identity-ROI result of retained `default_tiling_callback`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorCorrectionNativeIdentityTiling {
    pub provenance: ColorCorrectionTilingProvenance,
    pub input_output_factor: f32,
    pub input_output_factor_cl: f32,
    pub maximum_buffer_factor: f32,
    pub maximum_buffer_factor_cl: f32,
    pub overhead_bytes: u32,
    pub overlap_pixels: u32,
    /// Upper-left tile-coordinate modulus from `dt_develop_tiling_t::align`;
    /// this is not a raster-pointer memory-alignment guarantee.
    pub alignment_pixels: u32,
}

pub const COLORCORRECTION_NATIVE_IDENTITY_TILING: ColorCorrectionNativeIdentityTiling =
    ColorCorrectionNativeIdentityTiling {
        provenance: ColorCorrectionTilingProvenance::RetainedDefaultCallback,
        input_output_factor: 2.0,
        input_output_factor_cl: 2.0,
        maximum_buffer_factor: 1.0,
        maximum_buffer_factor_cl: 1.0,
        overhead_bytes: 0,
        overlap_pixels: 0,
        alignment_pixels: 1,
    };

/// Operation-local tiling policy. The input and caller destination reproduce
/// the native factor of two; the temporary multiplier is the additional private
/// candidate raster required for transactional Rust publication. The native
/// callback supplies no tile-edge preference, so 256 is explicitly Rust policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorCorrectionRustTransactionalTiling {
    pub provenance: ColorCorrectionTilingProvenance,
    pub overlap_pixels: u32,
    /// Upper-left tile-coordinate modulus, kept separate from memory alignment.
    pub alignment_pixels: u32,
    pub minimum_tile_edge: u32,
    pub preferred_tile_edge: u32,
    pub input_multiplier_milli: u32,
    pub output_multiplier_milli: u32,
    pub temporary_multiplier_milli: u32,
}

pub const COLORCORRECTION_RUST_TRANSACTIONAL_TILING: ColorCorrectionRustTransactionalTiling =
    ColorCorrectionRustTransactionalTiling {
        provenance: ColorCorrectionTilingProvenance::RustTransactionalPolicy,
        overlap_pixels: 0,
        alignment_pixels: 1,
        minimum_tile_edge: 1,
        preferred_tile_edge: 256,
        input_multiplier_milli: 1_000,
        output_multiplier_milli: 1_000,
        temporary_multiplier_milli: 1_000,
    };

/// Maps the native two-raster host-memory factor to the bounded Rust leaf.
/// Borrowed input and caller destination are already allocated; only the extra
/// private staging raster's exact pixel payload is charged to
/// `ColorCorrectionPlan`'s caller budget. Neither this mapping nor native
/// `dt_tiling_piece_fits_host_memory` adds allocator metadata or alignment
/// padding; actual native aligned allocation rounding is inventoried separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorCorrectionBudgetMapping {
    /// All fields are raster multipliers in thousandths.
    pub native_input_output: u32,
    pub preallocated_input: u32,
    pub preallocated_destination: u32,
    pub budgeted_staging: u32,
    pub rust_peak: u32,
}

pub const COLORCORRECTION_BUDGET_MAPPING: ColorCorrectionBudgetMapping =
    ColorCorrectionBudgetMapping {
        native_input_output: 2_000,
        preallocated_input: 1_000,
        preallocated_destination: 1_000,
        budgeted_staging: 1_000,
        rust_peak: 3_000,
    };

pub const COLORCORRECTION_SOURCE_MAP: &[ColorCorrectionSourceMapEntry] = &[
    ColorCorrectionSourceMapEntry {
        native_symbol: "DT_MODULE_INTROSPECTION / dt_iop_colorcorrection_params_t",
        native_file: "src/iop/colorcorrection.c",
        rust_symbol: "codec::ColorCorrectionParametersV1 / ColorCorrectionHistory",
        status: ColorCorrectionPortStatus::Ported,
    },
    ColorCorrectionSourceMapEntry {
        native_symbol: "absence of legacy_params",
        native_file: "src/iop/colorcorrection.c; src/iop/iop_api.h",
        rust_symbol: "codec::COLORCORRECTION_MIGRATION_EDGES / migrate_to_current",
        status: ColorCorrectionPortStatus::Ported,
    },
    ColorCorrectionSourceMapEntry {
        native_symbol: "commit_params",
        native_file: "src/iop/colorcorrection.c",
        rust_symbol: "execution::ColorCorrectionPlan::new",
        status: ColorCorrectionPortStatus::Ported,
    },
    ColorCorrectionSourceMapEntry {
        native_symbol: "init_pipe / cleanup_pipe piece->data allocation and lifetime",
        native_file: "src/iop/colorcorrection.c; src/iop/iop_api.h",
        rust_symbol: "execution::ColorCorrectionPlan owned committed coefficients / automatic drop",
        status: ColorCorrectionPortStatus::Ported,
    },
    ColorCorrectionSourceMapEntry {
        native_symbol: "process",
        native_file: "src/iop/colorcorrection.c",
        rust_symbol: "execution::ColorCorrectionPlan::execute_with_cancel",
        status: ColorCorrectionPortStatus::Ported,
    },
    ColorCorrectionSourceMapEntry {
        native_symbol: "DT_IS_ALIGNED / dt_check_aligned / DT_OMP_FOR process pointer-alignment and static-schedule providers",
        native_file: "src/iop/colorcorrection.c; src/develop/pixelpipe_hb.c; src/common/darktable.h; src/common/dttypes.h",
        rust_symbol: "ordinary Vec<ColorCorrectionPixel> staging / serial execute_with_cancel loop",
        status: ColorCorrectionPortStatus::RustAdaptation,
    },
    ColorCorrectionSourceMapEntry {
        native_symbol: "pixelpipe_hb bpp*width*height cache request / pixelpipe_cache dt_alloc_aligned ownership / 16-byte raster payload / 64-or-128-byte allocation alignment and constrained tail padding",
        native_file: "src/develop/pixelpipe_hb.c; src/develop/pixelpipe_cache.c; src/develop/format.c; src/common/dttypes.h; src/common/darktable.h; src/common/darktable.c",
        rust_symbol: "source_map buffer-ownership, alignment, payload, and ordinary-Vec adaptation constants",
        status: ColorCorrectionPortStatus::RustAdaptation,
    },
    ColorCorrectionSourceMapEntry {
        native_symbol: concat!(
            "Debug -O0/-D_DEBUG, default RelWithDebInfo -O2/-ftree-vectorize, and ",
            "Release -O3/-ffast",
            "-math/-fno-finite-math-only compiler-dependent process profiles"
        ),
        native_file: "CMakeLists.txt; src/CMakeLists.txt; src/iop/colorcorrection.c",
        rust_symbol: "execution::COLORCORRECTION_CPU_ARITHMETIC_PROFILE / separate_rounding_opponent",
        status: ColorCorrectionPortStatus::RustAdaptation,
    },
    ColorCorrectionSourceMapEntry {
        native_symbol: "DEFAULT callback fallback assignment for omitted module symbols",
        native_file: "src/common/module_api.h; src/iop/iop_api.h",
        rust_symbol: "operation-local typed format and tiling evidence; shared callback dispatch deferred",
        status: ColorCorrectionPortStatus::SourceEvidenceOnly,
    },
    ColorCorrectionSourceMapEntry {
        native_symbol: "default_input_format / default_output_format four-channel float Lab contract",
        native_file: "src/common/module_api.h; src/iop/iop_api.h; src/develop/format.c; src/iop/colorcorrection.c",
        rust_symbol: "ColorCorrectionPixel / colorcorrection_leaf_descriptor lab-f32x4 IO",
        status: ColorCorrectionPortStatus::Ported,
    },
    ColorCorrectionSourceMapEntry {
        native_symbol: "default_tiling_callback identity-ROI factor/factor_cl, maxbuf/maxbuf_cl, overhead, overlap and upper-left coordinate align",
        native_file: "src/develop/tiling.c; src/develop/tiling.h; src/iop/iop_api.h",
        rust_symbol: "source_map::COLORCORRECTION_NATIVE_IDENTITY_TILING",
        status: ColorCorrectionPortStatus::SourceEvidenceOnly,
    },
    ColorCorrectionSourceMapEntry {
        native_symbol: "dt_tiling_piece_fits_host_memory f32 factor*width*height*bpp+overhead before size_t conversion / pixelpipe factor-two payload versus checked Rust transactional staging budget",
        native_file: "src/develop/tiling.c; src/develop/pixelpipe_hb.c; src/develop/pixelpipe_cache.c; src/iop/colorcorrection.c; src/common/darktable.c; src/common/dttypes.h",
        rust_symbol: "source_map native-f32 boundary constants / COLORCORRECTION_RUST_TRANSACTIONAL_TILING / COLORCORRECTION_BUDGET_MAPPING; execution::ColorCorrectionPlan::with_memory_budget",
        status: ColorCorrectionPortStatus::RustAdaptation,
    },
    ColorCorrectionSourceMapEntry {
        native_symbol: "name / description / flags / default_group / default_colorspace",
        native_file: "src/iop/colorcorrection.c; src/develop/imageop.c; src/develop/imageop.h; src/common/colorspaces.h",
        rust_symbol: "colorcorrection_leaf_presentation / colorcorrection_leaf_descriptor",
        status: ColorCorrectionPortStatus::Ported,
    },
    ColorCorrectionSourceMapEntry {
        native_symbol: "darktable-bench-3.4 Color Correction v1 history payload and order",
        native_file: "src/tests/benchmark/darktable-bench-3.4.xmp",
        rust_symbol: "tests/fixtures/colorcorrection-v1-benchmark.hex",
        status: ColorCorrectionPortStatus::SourceEvidenceOnly,
    },
    ColorCorrectionSourceMapEntry {
        native_symbol: "dt_iop_have_required_input_format / dt_iop_copy_image_roi exact-size copy, ROI-offset crop, and zero padding",
        native_file: "src/develop/imageop.h; src/develop/imageop.c; src/common/imagebuf.c",
        rust_symbol: "typed four-channel leaf; shared ROI-aware copy-through and trouble adapter deferred",
        status: ColorCorrectionPortStatus::ExplicitlyDeferred,
    },
    ColorCorrectionSourceMapEntry {
        native_symbol: "process_cl / init_global / cleanup_global / colorcorrection kernel",
        native_file: "src/iop/colorcorrection.c; data/kernels/basic.cl; data/kernels/common.h",
        rust_symbol: "ColorCorrectionCapabilities::require_gpu",
        status: ColorCorrectionPortStatus::ExplicitlyDeferred,
    },
    ColorCorrectionSourceMapEntry {
        native_symbol: "init_presets tuple values, insertion order, enabled flag and blend colorspace",
        native_file: "src/iop/colorcorrection.c; src/develop/blend.h",
        rust_symbol: "COLORCORRECTION_PRESET_EVIDENCE",
        status: ColorCorrectionPortStatus::SourceEvidenceOnly,
    },
    ColorCorrectionSourceMapEntry {
        native_symbol: "dt_gui_presets_add_generic registration and translation",
        native_file: "src/iop/colorcorrection.c; src/gui/presets.h",
        rust_symbol: "ColorCorrectionCapabilities::require_preset_registration",
        status: ColorCorrectionPortStatus::ExplicitlyDeferred,
    },
    ColorCorrectionSourceMapEntry {
        native_symbol: "custom-grid endpoint fields have no numeric precision / dt_bauhaus_slider_from_params saturation digits formula yields two",
        native_file: "src/iop/colorcorrection.c; src/develop/imageop_gui.c",
        rust_symbol: "COLORCORRECTION_ENDPOINT_NATIVE_UI_PRECISION / COLORCORRECTION_ENDPOINT_DEFERRED_DESCRIPTOR_PRECISION / COLORCORRECTION_SATURATION_NATIVE_UI_PRECISION",
        status: ColorCorrectionPortStatus::RustAdaptation,
    },
    ColorCorrectionSourceMapEntry {
        native_symbol: "gui_update / gui_init / gui_cleanup / draw and input callbacks",
        native_file: "src/iop/colorcorrection.c; src/develop/imageop_gui.c; src/common/colorspaces.h",
        rust_symbol: "ColorCorrectionCapabilities::require_gtk",
        status: ColorCorrectionPortStatus::ExplicitlyDeferred,
    },
    ColorCorrectionSourceMapEntry {
        native_symbol: "history import, descriptor export, registry, reconstruction and stack order",
        native_file: "Rust shared integration hubs",
        rust_symbol: "ColorCorrectionCapabilities::require_production_routing",
        status: ColorCorrectionPortStatus::ExplicitlyDeferred,
    },
    ColorCorrectionSourceMapEntry {
        native_symbol: "pixelpipe dispatch, masks, outer blending, snapshot, GPU and publication",
        native_file: "Rust shared integration hubs",
        rust_symbol: "ColorCorrectionCapabilities::require_production_routing",
        status: ColorCorrectionPortStatus::ExplicitlyDeferred,
    },
];
