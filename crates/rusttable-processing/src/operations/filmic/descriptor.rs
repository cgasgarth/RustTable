//! Bounded capability metadata for legacy `filmic`.
//!
//! This is deliberately operation-local.  The shared registry and descriptor
//! hubs must not advertise this leaf until history, routing, blending, and UI
//! integration are owned by a later milestone.

/// Fail-closed capability projection for the isolated CPU leaf.
#[allow(
    clippy::struct_excessive_bools,
    reason = "the fail-closed capability projection mirrors independent native flags"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilmicDescriptor {
    pub compatibility_id: &'static str,
    pub rust_id: &'static str,
    pub schema_version: u16,
    pub default_enabled: bool,
    pub input_stage: &'static str,
    pub output_stage: &'static str,
    pub identity_roi: bool,
    pub overlap_pixels: u32,
    pub cpu_supported: bool,
    pub gpu_tier: Option<u8>,
    pub deterministic_cpu: bool,
    pub deterministic_gpu: bool,
    pub fallback_to_cpu: bool,
    pub consumes_operation_mask: bool,
    pub publishes_operation_mask: bool,
    pub supports_external_blending: bool,
    pub ui: Option<&'static str>,
}

#[must_use]
pub const fn descriptor() -> FilmicDescriptor {
    FilmicDescriptor {
        compatibility_id: "filmic",
        rust_id: "rusttable.filmic",
        schema_version: 3,
        default_enabled: false,
        input_stage: "lab-d50",
        output_stage: "lab-d50",
        identity_roi: true,
        overlap_pixels: 0,
        cpu_supported: true,
        gpu_tier: None,
        deterministic_cpu: true,
        deterministic_gpu: false,
        fallback_to_cpu: false,
        consumes_operation_mask: false,
        publishes_operation_mask: false,
        supports_external_blending: true,
        ui: None,
    }
}
