use rusttable_color::RenderingIntent;
use rusttable_core::PhotoId;
use rusttable_display_profile::{
    DisplayProfileId, DisplayProfileSnapshot, MonitorId, ProfileTransformError, SelectionStatus,
};
use rusttable_image::ImageDimensions;
use rusttable_ui::{PresentationMode, PresentationStatus, SdrFallbackReason};
use sha2::{Digest, Sha256};

use crate::workspace::SelectedPreview;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PresentationFallback {
    MissingProfile,
    UnusableProfile,
}

impl PresentationFallback {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::MissingProfile => "monitor profile unavailable",
            Self::UnusableProfile => "monitor profile unusable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PresentationReceipt {
    scene_identity: [u8; 32],
    edit_identity: [u8; 32],
    presentation_identity: [u8; 32],
    monitor: Option<MonitorId>,
    profile_id: Option<DisplayProfileId>,
    generation: u64,
    intent: RenderingIntent,
    fallback: Option<PresentationFallback>,
}

impl PresentationReceipt {
    pub(crate) const fn scene_identity(&self) -> [u8; 32] {
        self.scene_identity
    }

    pub(crate) const fn edit_identity(&self) -> [u8; 32] {
        self.edit_identity
    }

    pub(crate) const fn presentation_identity(&self) -> [u8; 32] {
        self.presentation_identity
    }

    pub(crate) const fn monitor(&self) -> Option<MonitorId> {
        self.monitor
    }

    pub(crate) const fn profile_id(&self) -> Option<DisplayProfileId> {
        self.profile_id
    }

    pub(crate) const fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) const fn intent(&self) -> RenderingIntent {
        self.intent
    }

    pub(crate) const fn fallback(&self) -> Option<PresentationFallback> {
        self.fallback
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PresentedPreview {
    photo_id: PhotoId,
    dimensions: ImageDimensions,
    pixels: Vec<u8>,
    receipt: crate::CatalogPreviewReceipt,
    presentation: PresentationReceipt,
    status: PresentationStatus,
}

impl PresentedPreview {
    pub(crate) const fn photo_id(&self) -> PhotoId {
        self.photo_id
    }

    pub(crate) const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }

    pub(crate) fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub(crate) const fn receipt(&self) -> &crate::CatalogPreviewReceipt {
        &self.receipt
    }

    pub(crate) const fn presentation_receipt(&self) -> &PresentationReceipt {
        &self.presentation
    }

    pub(crate) const fn status(&self) -> PresentationStatus {
        self.status
    }
}

pub(crate) fn present(
    preview: SelectedPreview,
    snapshot: Option<&DisplayProfileSnapshot>,
    intent: RenderingIntent,
) -> Result<PresentedPreview, ProfileTransformError> {
    let (photo_id, dimensions, source_pixels, receipt) = preview.into_render_parts();
    let (monitor, profile_id, generation, fallback, pixels) = match snapshot {
        Some(snapshot)
            if snapshot.status() == SelectionStatus::Active && snapshot.profile().is_some() =>
        {
            let profile = snapshot.profile().expect("checked profile presence");
            let (fallback, pixels) = match profile.presentation_plan(intent) {
                Ok(plan) => (None, transform_pixels(&source_pixels, &plan)?),
                Err(error) => {
                    tracing::warn!(
                        target: "rusttable.gtk.preview",
                        profile = %profile.id(),
                        cause = ?error,
                        "monitor profile cannot transform selected preview; using sRGB fallback"
                    );
                    (Some(PresentationFallback::UnusableProfile), source_pixels)
                }
            };
            (
                Some(snapshot.monitor()),
                Some(profile.id()),
                snapshot.generation(),
                fallback,
                pixels,
            )
        }
        Some(snapshot) => (
            Some(snapshot.monitor()),
            snapshot.profile_id(),
            snapshot.generation(),
            Some(PresentationFallback::MissingProfile),
            source_pixels,
        ),
        None => (
            None,
            None,
            0,
            Some(PresentationFallback::MissingProfile),
            source_pixels,
        ),
    };
    let scene_identity = receipt.identity_hash();
    let edit_identity = edit_identity(&receipt);
    let presentation_identity =
        presentation_identity(monitor, profile_id, generation, intent, fallback);
    let status = match fallback {
        Some(PresentationFallback::MissingProfile) => {
            PresentationStatus::SrgbFallback(SdrFallbackReason::MissingMonitorProfile)
        }
        Some(PresentationFallback::UnusableProfile) => {
            PresentationStatus::SrgbFallback(SdrFallbackReason::TransformUnavailable)
        }
        None => PresentationStatus::Ready {
            mode: PresentationMode::Sdr,
            profile: snapshot.map_or(
                rusttable_display_profile::ProfileSelection::Unprofiled,
                DisplayProfileSnapshot::selection,
            ),
        },
    };
    Ok(PresentedPreview {
        photo_id,
        dimensions,
        pixels,
        receipt,
        presentation: PresentationReceipt {
            scene_identity,
            edit_identity,
            presentation_identity,
            monitor,
            profile_id,
            generation,
            intent,
            fallback,
        },
        status,
    })
}

fn transform_pixels(
    source: &[u8],
    plan: &rusttable_color::TransformPlan,
) -> Result<Vec<u8>, ProfileTransformError> {
    let mut output = Vec::with_capacity(source.len());
    #[allow(clippy::chunks_exact_to_as_chunks)]
    for pixel in source.chunks_exact(4) {
        let rgb = plan
            .apply_rgb(
                [
                    f32::from(pixel[0]) / 255.0,
                    f32::from(pixel[1]) / 255.0,
                    f32::from(pixel[2]) / 255.0,
                ],
                || false,
            )
            .map_err(|_| ProfileTransformError::InvalidMatrix)?;
        output.extend(rgb.map(quantize));
        output.push(pixel[3]);
    }
    Ok(output)
}

fn quantize(value: f32) -> u8 {
    let rounded = (value.clamp(0.0, 1.0) * 255.0).round();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    u8::try_from(rounded as u32).expect("clamped RGB channel fits u8")
}

fn edit_identity(receipt: &crate::CatalogPreviewReceipt) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.preview.edit.v1");
    hasher.update(format!("{:?}", receipt.render().context().edit()).as_bytes());
    hasher.finalize().into()
}

fn presentation_identity(
    monitor: Option<MonitorId>,
    profile_id: Option<DisplayProfileId>,
    generation: u64,
    intent: RenderingIntent,
    fallback: Option<PresentationFallback>,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.preview.presentation.v1");
    hasher.update(monitor.map_or([0; 32], MonitorId::bytes));
    if let Some(profile) = profile_id {
        hasher.update(profile.sha256());
        hasher.update(profile.size().to_le_bytes());
    }
    hasher.update(generation.to_le_bytes());
    hasher.update(format!("{intent:?}").as_bytes());
    hasher.update([fallback.map_or(0, |reason| match reason {
        PresentationFallback::MissingProfile => 1,
        PresentationFallback::UnusableProfile => 2,
    })]);
    hasher.finalize().into()
}
