use crate::{
    CombinationMode, GeometryAncestry, MaskExecutionError, MaskGeometry, MaskIdentity,
    MaskModifier, MaskRaster, MaskReference, MaskRoi, MaskSource,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

const MAX_NODES: usize = 4096;
const GRAPH_ENCODING_PREFIX: &[u8] = b"rusttable.mask-graph.v1";

/// A node with dense authored/raster coverage. Generated values are resolved
/// through the publication store before graph evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskNode {
    identity: MaskIdentity,
    name: String,
    source: MaskSource,
    geometry: MaskGeometry,
    values: Option<MaskRaster>,
    modifiers: Vec<MaskModifier>,
}

impl MaskNode {
    pub fn new(
        identity: MaskIdentity,
        name: impl Into<String>,
        source: MaskSource,
        geometry: MaskGeometry,
        values: Option<MaskRaster>,
        modifiers: impl IntoIterator<Item = MaskModifier>,
    ) -> Result<Self, GraphBuildError> {
        let name = name.into();
        if name.len() > 256 || name.chars().any(char::is_control) {
            return Err(GraphBuildError::InvalidName);
        }
        if source.is_opaque() && values.is_some() {
            return Err(GraphBuildError::OpaqueHasValue);
        }
        Ok(Self {
            identity,
            name,
            source,
            geometry,
            values,
            modifiers: modifiers.into_iter().collect(),
        })
    }
    #[must_use]
    pub const fn identity(&self) -> MaskIdentity {
        self.identity
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub const fn source(&self) -> &MaskSource {
        &self.source
    }
    #[must_use]
    pub const fn geometry(&self) -> &MaskGeometry {
        &self.geometry
    }
    #[must_use]
    pub fn modifiers(&self) -> &[MaskModifier] {
        &self.modifiers
    }
    #[must_use]
    pub const fn values(&self) -> Option<&MaskRaster> {
        self.values.as_ref()
    }
}

/// An immutable ordered group of mask references.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskGroup {
    identity: MaskIdentity,
    name: String,
    members: Vec<MaskReference>,
    combination: CombinationMode,
    modifiers: Vec<MaskModifier>,
}

impl MaskGroup {
    pub fn new(
        identity: MaskIdentity,
        name: impl Into<String>,
        members: impl IntoIterator<Item = MaskReference>,
        combination: CombinationMode,
        modifiers: impl IntoIterator<Item = MaskModifier>,
    ) -> Result<Self, GraphBuildError> {
        let name = name.into();
        if name.len() > 256 || name.chars().any(char::is_control) {
            return Err(GraphBuildError::InvalidName);
        }
        Ok(Self {
            identity,
            name,
            members: members.into_iter().collect(),
            combination,
            modifiers: modifiers.into_iter().collect(),
        })
    }
    #[must_use]
    pub const fn identity(&self) -> MaskIdentity {
        self.identity
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn members(&self) -> &[MaskReference] {
        &self.members
    }
    #[must_use]
    pub const fn combination(&self) -> CombinationMode {
        self.combination
    }
    #[must_use]
    pub fn modifiers(&self) -> &[MaskModifier] {
        &self.modifiers
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GraphNode {
    Mask(MaskNode),
    Group(MaskGroup),
}

impl GraphNode {
    #[must_use]
    pub const fn identity(&self) -> MaskIdentity {
        match self {
            Self::Mask(node) => node.identity(),
            Self::Group(group) => group.identity(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
struct MaskEdge {
    producer: MaskIdentity,
    consumer_operation: u128,
    consumer_mask_id: u128,
}

/// Builder for one immutable pipeline snapshot graph.
#[derive(Debug, Clone, Default)]
pub struct MaskGraphBuilder {
    nodes: Vec<GraphNode>,
    edges: Vec<MaskEdge>,
}

impl MaskGraphBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    #[must_use]
    pub fn add_mask(mut self, node: MaskNode) -> Self {
        self.nodes.push(GraphNode::Mask(node));
        self
    }
    #[must_use]
    pub fn add_group(mut self, group: MaskGroup) -> Self {
        self.nodes.push(GraphNode::Group(group));
        self
    }
    /// Adds an explicit producer-to-consumer edge.
    #[must_use]
    pub fn add_edge(
        mut self,
        producer: MaskIdentity,
        consumer_operation: u128,
        consumer_mask_id: u128,
    ) -> Self {
        self.edges.push(MaskEdge {
            producer,
            consumer_operation,
            consumer_mask_id,
        });
        self
    }

    /// Validates and publishes a deterministic immutable graph.
    pub fn build(self) -> Result<MaskGraph, GraphBuildError> {
        if self.nodes.len() > MAX_NODES {
            return Err(GraphBuildError::NodeLimit);
        }
        let mut nodes = BTreeMap::new();
        for node in self.nodes {
            let identity = node.identity();
            if nodes.insert(identity, node).is_some() {
                return Err(GraphBuildError::DuplicateIdentity(identity));
            }
        }
        let mut edges = self.edges;
        edges.sort_unstable();
        for edge in &edges {
            if !nodes.contains_key(&edge.producer) {
                return Err(GraphBuildError::MissingReference(edge.producer));
            }
        }
        for group in nodes.values().filter_map(|node| match node {
            GraphNode::Group(group) => Some(group),
            GraphNode::Mask(_) => None,
        }) {
            for member in group.members() {
                if !nodes.contains_key(&member.identity()) {
                    return Err(GraphBuildError::MissingReference(member.identity()));
                }
            }
        }
        let graph = MaskGraph {
            nodes,
            edges,
            order: Vec::new(),
            identity: [0; 32],
        };
        let order = graph.topological_order()?;
        let mut graph = graph;
        graph.order = order;
        graph.identity = Sha256::digest(graph.identity_bytes()).into();
        Ok(graph)
    }
}

/// Fully validated immutable graph with stable topological order and identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskGraph {
    nodes: BTreeMap<MaskIdentity, GraphNode>,
    edges: Vec<MaskEdge>,
    order: Vec<MaskIdentity>,
    identity: [u8; 32],
}

impl MaskGraph {
    #[must_use]
    pub fn nodes(&self) -> impl Iterator<Item = &GraphNode> {
        self.order
            .iter()
            .filter_map(|identity| self.nodes.get(identity))
    }
    #[must_use]
    pub fn node(&self, identity: MaskIdentity) -> Option<&GraphNode> {
        self.nodes.get(&identity)
    }
    #[must_use]
    pub fn order(&self) -> &[MaskIdentity] {
        &self.order
    }
    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = GRAPH_ENCODING_PREFIX.to_vec();
        bytes.extend_from_slice(&postcard::to_allocvec(self).expect("mask graph is serializable"));
        bytes
    }
    /// Decodes a canonical graph and verifies its stored identity before publication.
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self, GraphBuildError> {
        if !bytes.starts_with(GRAPH_ENCODING_PREFIX) {
            return Err(GraphBuildError::Serialization(
                "unknown mask graph encoding".to_owned(),
            ));
        }
        let graph: Self = postcard::from_bytes(&bytes[GRAPH_ENCODING_PREFIX.len()..])
            .map_err(|error| GraphBuildError::Serialization(error.to_string()))?;
        let expected = Sha256::digest(graph.identity_bytes());
        if graph.identity != expected[..] {
            return Err(GraphBuildError::IdentityMismatch);
        }
        Ok(graph)
    }

    fn identity_bytes(&self) -> Vec<u8> {
        let mut canonical = self.clone();
        canonical.identity = [0; 32];
        let mut bytes = GRAPH_ENCODING_PREFIX.to_vec();
        bytes.extend_from_slice(
            &postcard::to_allocvec(&canonical).expect("mask graph is serializable"),
        );
        bytes
    }
    #[must_use]
    pub fn consumer_use_count(&self, identity: MaskIdentity) -> usize {
        self.nodes
            .values()
            .filter_map(|node| match node {
                GraphNode::Group(group) => Some(group.members()),
                GraphNode::Mask(_) => None,
            })
            .flatten()
            .filter(|reference| reference.identity() == identity)
            .count()
            + self
                .edges
                .iter()
                .filter(|edge| edge.producer == identity)
                .count()
    }

    /// Evaluates a graph node using only values already present in the graph.
    /// Generated nodes are intentionally blocking until their exact raster is
    /// supplied by `evaluate_with_store`.
    pub fn evaluate(&self, identity: MaskIdentity) -> Result<MaskRaster, MaskExecutionError> {
        self.evaluate_inner(identity, None)
    }

    pub fn evaluate_with_store(
        &self,
        identity: MaskIdentity,
        store: &mut crate::RasterMaskStore,
    ) -> Result<MaskRaster, MaskExecutionError> {
        self.evaluate_inner(identity, Some(store))
    }

    fn evaluate_inner(
        &self,
        identity: MaskIdentity,
        mut store: Option<&mut crate::RasterMaskStore>,
    ) -> Result<MaskRaster, MaskExecutionError> {
        let node = self
            .nodes
            .get(&identity)
            .ok_or(MaskExecutionError::MissingPublishedRaster(
                identity_to_bytes(identity),
            ))?;
        match node {
            GraphNode::Mask(mask) => {
                let mut value = match (&mask.source, &mask.values) {
                    (MaskSource::Opaque { version, .. }, _) => {
                        return Err(MaskExecutionError::OpaqueSource { version: *version });
                    }
                    (MaskSource::Generated(descriptor), _) => store
                        .as_deref_mut()
                        .ok_or(MaskExecutionError::MissingPublishedRaster(
                            descriptor.cache_identity(),
                        ))?
                        .consume(descriptor)?
                        .raster()
                        .clone(),
                    (MaskSource::Raster, Some(raster)) => raster.clone(),
                    (MaskSource::Raster, None) => {
                        return Err(MaskExecutionError::MissingPublishedRaster(
                            identity_to_bytes(identity),
                        ));
                    }
                };
                for modifier in mask.modifiers.iter().copied() {
                    value = value.modified(modifier)?;
                }
                Ok(value)
            }
            GraphNode::Group(group) => {
                let mut members = group.members.iter();
                let first = members
                    .next()
                    .ok_or(MaskExecutionError::DimensionsMismatch {
                        expected: 1,
                        actual: 0,
                    })?;
                let mut value = self.evaluate_inner(first.identity(), store.as_deref_mut())?;
                for member in members {
                    value = value.combine(
                        &self.evaluate_inner(member.identity(), store.as_deref_mut())?,
                        group.combination,
                    )?;
                }
                for modifier in group.modifiers.iter().copied() {
                    value = value.modified(modifier)?;
                }
                Ok(value)
            }
        }
    }

    fn topological_order(&self) -> Result<Vec<MaskIdentity>, GraphBuildError> {
        let mut dependencies = BTreeMap::<MaskIdentity, BTreeSet<MaskIdentity>>::new();
        for identity in self.nodes.keys().copied() {
            dependencies.insert(identity, BTreeSet::new());
        }
        for node in self.nodes.values() {
            if let GraphNode::Group(group) = node {
                let members = dependencies
                    .get_mut(&group.identity())
                    .expect("group was inserted");
                members.extend(group.members.iter().map(|reference| reference.identity()));
            }
        }
        let mut order = Vec::with_capacity(self.nodes.len());
        while !dependencies.is_empty() {
            let ready = dependencies
                .iter()
                .find(|(_, deps)| deps.is_empty())
                .map(|(identity, _)| *identity);
            let Some(identity) = ready else {
                return Err(GraphBuildError::Cycle);
            };
            dependencies.remove(&identity);
            for deps in dependencies.values_mut() {
                deps.remove(&identity);
            }
            order.push(identity);
        }
        Ok(order)
    }
}

fn identity_to_bytes(identity: MaskIdentity) -> [u8; 32] {
    let mut bytes = Vec::with_capacity(40);
    bytes.extend_from_slice(&identity.photo_id().to_le_bytes());
    bytes.extend_from_slice(&identity.edit_revision().to_le_bytes());
    bytes.extend_from_slice(&identity.mask_id().to_le_bytes());
    bytes.extend_from_slice(&identity.mask_version().to_le_bytes());
    Sha256::digest(bytes).into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphBuildError {
    NodeLimit,
    DuplicateIdentity(MaskIdentity),
    MissingReference(MaskIdentity),
    Cycle,
    InvalidName,
    OpaqueHasValue,
    Serialization(String),
    IdentityMismatch,
}

impl fmt::Display for GraphBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NodeLimit => formatter.write_str("mask graph exceeds node limit"),
            Self::DuplicateIdentity(identity) => {
                write!(formatter, "duplicate mask identity {identity:?}")
            }
            Self::MissingReference(identity) => write!(
                formatter,
                "mask graph references missing identity {identity:?}"
            ),
            Self::Cycle => formatter.write_str("mask graph contains a cycle"),
            Self::InvalidName => formatter.write_str("mask name is invalid"),
            Self::OpaqueHasValue => {
                formatter.write_str("opaque mask source cannot carry evaluated values")
            }
            Self::Serialization(error) => {
                write!(formatter, "mask graph serialization failed: {error}")
            }
            Self::IdentityMismatch => {
                formatter.write_str("mask graph identity does not match canonical bytes")
            }
        }
    }
}

impl std::error::Error for GraphBuildError {}

// Keep the public geometry import in this module useful to callers composing a graph.
#[allow(dead_code)]
fn _geometry_type_is_typed(_: GeometryAncestry, _: MaskRoi) {}
