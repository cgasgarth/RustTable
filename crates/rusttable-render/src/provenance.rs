use std::fmt;

use rusttable_core::{AssetId, ByteLength, ContentHash, Edit, PhotoId};
use rusttable_image::{ColorEncoding, ImageDimensions, ImageProbe};
use rusttable_processing::GamutClipReport;
use sha2::{Digest, Sha256};

use crate::{RenderError, RenderPlan, RenderProvenance, SourceColorDecision, SourceColorPolicy};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderSourceProvenance {
    photo_id: PhotoId,
    primary_asset_id: AssetId,
    content_hash: ContentHash,
    byte_length: ByteLength,
    probe: ImageProbe,
}

impl RenderSourceProvenance {
    #[must_use]
    pub const fn new(
        photo_id: PhotoId,
        primary_asset_id: AssetId,
        content_hash: ContentHash,
        byte_length: ByteLength,
        probe: ImageProbe,
    ) -> Self {
        Self {
            photo_id,
            primary_asset_id,
            content_hash,
            byte_length,
            probe,
        }
    }

    #[must_use]
    pub const fn photo_id(self) -> PhotoId {
        self.photo_id
    }

    #[must_use]
    pub const fn primary_asset_id(self) -> AssetId {
        self.primary_asset_id
    }

    #[must_use]
    pub const fn content_hash(self) -> ContentHash {
        self.content_hash
    }

    #[must_use]
    pub const fn byte_length(self) -> ByteLength {
        self.byte_length
    }

    #[must_use]
    pub const fn probe(self) -> ImageProbe {
        self.probe
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderRequestContext {
    source: RenderSourceProvenance,
    edit: RenderProvenance,
    policy: SourceColorPolicy,
    plan: RenderPlan,
}

impl RenderRequestContext {
    #[must_use]
    pub const fn new(
        source: RenderSourceProvenance,
        edit: &Edit,
        policy: SourceColorPolicy,
        plan: RenderPlan,
    ) -> Self {
        Self {
            source,
            edit: RenderProvenance::new(
                edit.id(),
                edit.photo_id(),
                edit.base_photo_revision(),
                edit.revision(),
            ),
            policy,
            plan,
        }
    }

    #[must_use]
    pub const fn source(self) -> RenderSourceProvenance {
        self.source
    }

    #[must_use]
    pub const fn edit(self) -> RenderProvenance {
        self.edit
    }

    #[must_use]
    pub const fn policy(self) -> SourceColorPolicy {
        self.policy
    }

    #[must_use]
    pub const fn plan(self) -> RenderPlan {
        self.plan
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderFailureStage {
    SourcePhoto,
    SourceDimensions,
    Plan,
    SourceColor,
    Pipeline,
    Evaluation,
    Image,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvenancedRenderErrorKind {
    SourcePhoto {
        source_photo_id: PhotoId,
        edit_photo_id: PhotoId,
    },
    SourceDimensions {
        probed: ImageDimensions,
        decoded: ImageDimensions,
    },
    Render {
        source: Box<RenderError>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvenancedRenderError {
    context: Box<RenderRequestContext>,
    stage: RenderFailureStage,
    kind: ProvenancedRenderErrorKind,
}

impl ProvenancedRenderError {
    #[must_use]
    pub const fn context(&self) -> RenderRequestContext {
        *self.context
    }

    #[must_use]
    pub const fn stage(&self) -> RenderFailureStage {
        self.stage
    }

    #[must_use]
    pub const fn kind(&self) -> &ProvenancedRenderErrorKind {
        &self.kind
    }

    pub(crate) fn new(
        context: RenderRequestContext,
        stage: RenderFailureStage,
        kind: ProvenancedRenderErrorKind,
    ) -> Self {
        Self {
            context: Box::new(context),
            stage,
            kind,
        }
    }
}

impl fmt::Display for ProvenancedRenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "provenanced render failed at {:?}", self.stage)
    }
}

impl std::error::Error for ProvenancedRenderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            ProvenancedRenderErrorKind::Render { source } => Some(source.as_ref()),
            ProvenancedRenderErrorKind::SourcePhoto { .. }
            | ProvenancedRenderErrorKind::SourceDimensions { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderReceipt {
    context: RenderRequestContext,
    source_color_decision: SourceColorDecision,
    render_provenance: RenderProvenance,
    clipping: GamutClipReport,
    output_dimensions: ImageDimensions,
    output_encoding: ColorEncoding,
}

impl RenderReceipt {
    pub(crate) const fn new(context: RenderRequestContext, output: &crate::RenderOutput) -> Self {
        Self {
            context,
            source_color_decision: output.source_color_decision(),
            render_provenance: output.provenance(),
            clipping: output.clipping(),
            output_dimensions: output.image().dimensions(),
            output_encoding: output.image().color_encoding(),
        }
    }

    #[must_use]
    pub const fn context(&self) -> RenderRequestContext {
        self.context
    }

    #[must_use]
    pub const fn source(&self) -> RenderSourceProvenance {
        self.context.source()
    }

    #[must_use]
    pub const fn source_color_decision(&self) -> SourceColorDecision {
        self.source_color_decision
    }

    #[must_use]
    pub const fn render_provenance(&self) -> RenderProvenance {
        self.render_provenance
    }

    #[must_use]
    pub const fn clipping(&self) -> GamutClipReport {
        self.clipping
    }

    #[must_use]
    pub const fn output_dimensions(&self) -> ImageDimensions {
        self.output_dimensions
    }

    #[must_use]
    pub const fn output_encoding(&self) -> ColorEncoding {
        self.output_encoding
    }

    /// Returns the stable receipt representation used by export artifacts.
    #[must_use]
    pub fn canonical_encoding(&self) -> String {
        format!(
            "render-receipt-v1\nsource={:?}\nedit={:?}\npolicy={:?}\nplan={:?}\nsource-color={:?}\nrender={:?}\nclip={:?}\noutput={:?}|{:?}\n",
            self.context.source,
            self.context.edit,
            self.context.policy,
            self.context.plan,
            self.source_color_decision,
            self.render_provenance,
            self.clipping,
            self.output_dimensions,
            self.output_encoding,
        )
    }

    #[must_use]
    pub fn identity_hash(&self) -> [u8; 32] {
        Sha256::digest(self.canonical_encoding().as_bytes()).into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvenancedRenderOutput {
    output: crate::RenderOutput,
    receipt: RenderReceipt,
}

impl ProvenancedRenderOutput {
    pub(crate) const fn new(output: crate::RenderOutput, receipt: RenderReceipt) -> Self {
        Self { output, receipt }
    }

    #[must_use]
    pub const fn output(&self) -> &crate::RenderOutput {
        &self.output
    }

    #[must_use]
    pub const fn receipt(&self) -> &RenderReceipt {
        &self.receipt
    }
}
