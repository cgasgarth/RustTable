use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::super::Result;

const CONTRACT_PATH: &str = "architecture/workspace-dag.toml";
const PLATFORM_CONTRACT_PATH: &str = "architecture/platform-support.toml";

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PlatformContract {
    pub(super) schema_version: u32,
    pub(super) targets: Vec<PlatformTarget>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PlatformTarget {
    pub(super) triple: String,
    pub(super) os: String,
    pub(super) architecture: String,
    pub(super) runner: String,
}

impl PlatformContract {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            return Err(format!(
                "{PLATFORM_CONTRACT_PATH}: unsupported schema version {}",
                self.schema_version
            ));
        }
        let mut triples = BTreeSet::new();
        for target in &self.targets {
            if target.triple.is_empty()
                || target.os.is_empty()
                || target.architecture.is_empty()
                || target.runner.is_empty()
                || !triples.insert(&target.triple)
            {
                return Err(format!(
                    "{PLATFORM_CONTRACT_PATH}: targets must have unique non-empty triple, os, and architecture"
                ));
            }
        }
        if self.targets.is_empty() {
            return Err(format!(
                "{PLATFORM_CONTRACT_PATH}: at least one supported target is required"
            ));
        }
        Ok(())
    }

    pub fn targets(&self) -> &[PlatformTarget] {
        &self.targets
    }
}

#[derive(Debug, Deserialize)]
pub struct Contract {
    pub(super) schema_version: u32,
    pub(super) composition_root: String,
    #[serde(default)]
    pub(super) tooling_packages: Vec<String>,
    pub(super) packages: Vec<DeclaredPackage>,
    #[serde(default)]
    pub(super) edges: Vec<DeclaredEdge>,
    #[serde(default, alias = "external_edges")]
    pub(super) external_dependencies: Vec<DeclaredExternal>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DeclaredPackage {
    pub(super) name: String,
    pub(super) manifest: String,
    pub(super) role: String,
    pub(super) integration_owner: String,
    #[serde(default)]
    pub(super) features: Vec<String>,
    #[serde(default)]
    pub(super) feature_sets: Vec<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DeclaredEdge {
    pub(super) from: String,
    pub(super) to: String,
    #[serde(default = "default_kind")]
    pub(super) kind: String,
    #[serde(default)]
    pub(super) target: Option<String>,
    #[serde(default)]
    pub(super) contexts: Vec<String>,
    #[serde(default = "default_required")]
    pub(super) required: bool,
    #[serde(default)]
    pub(super) tooling_only: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DeclaredExternal {
    pub(super) package: String,
    #[serde(default)]
    pub(super) source: Option<String>,
    #[serde(default)]
    pub(super) source_roles: Vec<String>,
    #[serde(default)]
    pub(super) source_packages: Vec<String>,
    #[serde(default, alias = "dependency_kinds")]
    pub(super) kinds: Vec<String>,
    #[serde(default)]
    pub(super) targets: Vec<String>,
    #[serde(default)]
    pub(super) contexts: Vec<String>,
    #[serde(default)]
    pub(super) optional: Option<bool>,
    #[serde(default)]
    pub(super) allow_transitive: bool,
}

fn default_kind() -> String {
    "normal".to_owned()
}

const fn default_required() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct MetadataContext {
    pub(super) name: String,
    pub(super) package: Option<String>,
    pub(super) args: Vec<String>,
    pub(super) target: Option<String>,
    pub(super) feature_set: Vec<String>,
    pub(super) metadata: CargoMetadata,
}

#[derive(Debug, Deserialize)]
pub struct CargoMetadata {
    pub(super) workspace_root: PathBuf,
    pub(super) workspace_members: Vec<String>,
    pub(super) packages: Vec<MetadataPackage>,
    pub(super) resolve: Option<Resolve>,
}

#[derive(Debug, Deserialize)]
pub struct MetadataPackage {
    pub(super) name: String,
    pub(super) id: String,
    pub(super) manifest_path: PathBuf,
    #[serde(default)]
    pub(super) source: Option<String>,
    #[serde(default)]
    pub(super) dependencies: Vec<MetadataDependency>,
    #[serde(default)]
    pub(super) features: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct MetadataDependency {
    pub(super) name: String,
    #[serde(default)]
    pub(super) package: Option<String>,
    #[serde(default)]
    pub(super) kind: Option<String>,
    #[serde(default)]
    pub(super) target: Option<String>,
    #[serde(default)]
    pub(super) optional: bool,
}

#[derive(Debug, Deserialize)]
pub struct Resolve {
    pub(super) nodes: Vec<ResolveNode>,
}

#[derive(Debug, Deserialize)]
pub struct ResolveNode {
    pub(super) id: String,
    #[serde(default)]
    pub(super) deps: Vec<ResolveDependency>,
}

#[derive(Debug, Deserialize)]
pub struct ResolveDependency {
    pub(super) name: String,
    pub(super) pkg: String,
    #[serde(default)]
    pub(super) dep_kinds: Vec<ResolveDependencyKind>,
    #[serde(default)]
    pub(super) transitive: bool,
}

#[derive(Debug, Deserialize)]
pub struct ResolveDependencyKind {
    pub(super) kind: Option<String>,
    pub(super) target: Option<String>,
}

// Cargo metadata exposes these independent receipt dimensions; keeping them
// as booleans preserves the machine-readable validation contract.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct DiscoveredEdge {
    pub(super) source: String,
    pub(super) destination: String,
    pub(super) destination_source: String,
    pub(super) source_role: String,
    pub(super) alias: String,
    pub(super) dependency_kind: String,
    pub(super) target: String,
    pub(super) platform: String,
    pub(super) optional: bool,
    pub(super) activated: bool,
    pub(super) transitive: bool,
    pub(super) external: bool,
    pub(super) feature_context: String,
    pub(super) rule: String,
}

impl Contract {
    pub fn parse(source: &str) -> Result<Self> {
        let contract: Self = toml::from_str(source)
            .map_err(|error| format!("{CONTRACT_PATH}: invalid TOML: {error}"))?;
        contract.validate()?;
        Ok(contract)
    }

    pub fn validate(&self) -> Result<()> {
        if ![1, 2, 3].contains(&self.schema_version) {
            return Err(format!(
                "{CONTRACT_PATH}: unsupported schema version {}",
                self.schema_version
            ));
        }
        let packages = validate_packages(self)?;
        validate_tooling_packages(self, &packages)?;
        validate_edges(self, &packages)?;
        validate_external_dependencies(self)
    }
}

fn validate_packages(contract: &Contract) -> Result<BTreeSet<String>> {
    let mut packages = BTreeSet::new();
    let mut roots = Vec::new();
    for package in &contract.packages {
        if package.name.is_empty() || !packages.insert(package.name.clone()) {
            return Err(format!(
                "{CONTRACT_PATH}: package names must be unique and non-empty: {}",
                package.name
            ));
        }
        if package.manifest.is_empty() {
            return Err(format!(
                "{CONTRACT_PATH}: package {} has no manifest",
                package.name
            ));
        }
        if !["product", "tooling", "composition-root"].contains(&package.role.as_str()) {
            return Err(format!(
                "{CONTRACT_PATH}: package {} has invalid role {}",
                package.name, package.role
            ));
        }
        if package.role == "composition-root" {
            roots.push(package.name.clone());
        }
        validate_feature_declarations(package)?;
    }
    if roots != [contract.composition_root.clone()] {
        return Err(format!(
            "{CONTRACT_PATH}: composition_root must name the only composition-root package"
        ));
    }
    if !packages.contains(&contract.composition_root) {
        return Err(format!(
            "{CONTRACT_PATH}: composition_root {} is not declared",
            contract.composition_root
        ));
    }
    for package in &contract.packages {
        if package.integration_owner.is_empty() || !packages.contains(&package.integration_owner) {
            return Err(format!(
                "{CONTRACT_PATH}: package {} has an unknown integration owner {}",
                package.name, package.integration_owner
            ));
        }
    }
    Ok(packages)
}

fn validate_tooling_packages(contract: &Contract, packages: &BTreeSet<String>) -> Result<()> {
    for tooling in &contract.tooling_packages {
        if !packages.contains(tooling)
            || contract
                .packages
                .iter()
                .find(|package| &package.name == tooling)
                .is_none_or(|package| package.role != "tooling")
        {
            return Err(format!(
                "{CONTRACT_PATH}: tooling package {tooling} is not declared with role tooling"
            ));
        }
    }
    Ok(())
}

fn validate_edges(contract: &Contract, packages: &BTreeSet<String>) -> Result<()> {
    let mut edges = BTreeSet::new();
    let roles = contract
        .packages
        .iter()
        .map(|package| (package.name.as_str(), package.role.as_str()))
        .collect::<BTreeMap<_, _>>();
    for edge in &contract.edges {
        if !packages.contains(&edge.from) || !packages.contains(&edge.to) {
            return Err(format!(
                "{CONTRACT_PATH}: edge {} -> {} references an undeclared package",
                edge.from, edge.to
            ));
        }
        if edge.kind.is_empty() || edge.target.as_deref() == Some("") {
            return Err(format!(
                "{CONTRACT_PATH}: edge {} -> {} has an incomplete dependency kind or target",
                edge.from, edge.to
            ));
        }
        if !edges.insert(edge_key(edge)) {
            return Err(format!(
                "{CONTRACT_PATH}: duplicate edge {} -> {}",
                edge.from, edge.to
            ));
        }
    }
    validate_acyclic_edges(&contract.packages, &contract.edges)?;
    for edge in &contract.edges {
        if edge.tooling_only && roles.get(edge.from.as_str()) != Some(&"tooling") {
            return Err(format!(
                "{CONTRACT_PATH}: tooling-only edge {} -> {} must originate in tooling",
                edge.from, edge.to
            ));
        }
        if roles.get(edge.from.as_str()) == Some(&"product")
            && roles.get(edge.to.as_str()) == Some(&"composition-root")
        {
            return Err(format!(
                "{CONTRACT_PATH}: product package {} cannot depend on composition root {}",
                edge.from, edge.to
            ));
        }
        if roles.get(edge.from.as_str()) != Some(&"tooling")
            && roles.get(edge.to.as_str()) == Some(&"tooling")
        {
            return Err(format!(
                "{CONTRACT_PATH}: package {} cannot depend on tooling package {}",
                edge.from, edge.to
            ));
        }
    }
    Ok(())
}

fn validate_external_dependencies(contract: &Contract) -> Result<()> {
    let mut external = BTreeSet::new();
    for rule in &contract.external_dependencies {
        if rule.package.is_empty() || !external.insert(external_key(rule)) {
            return Err(format!(
                "{CONTRACT_PATH}: external dependency rules must have unique package/source/kind/target identities"
            ));
        }
        for role in &rule.source_roles {
            if !["product", "tooling", "composition-root"].contains(&role.as_str()) {
                return Err(format!(
                    "{CONTRACT_PATH}: external rule {} has invalid source role {}",
                    rule.package, role
                ));
            }
        }
    }
    Ok(())
}

fn validate_acyclic_edges(packages: &[DeclaredPackage], edges: &[DeclaredEdge]) -> Result<()> {
    let mut graph = packages
        .iter()
        .map(|package| (package.name.clone(), BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    for edge in edges {
        graph
            .get_mut(&edge.from)
            .expect("validated edge source")
            .insert(edge.to.clone());
    }

    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for package in graph.keys() {
        if let Some(cycle) = visit_cycle(
            package,
            &graph,
            &mut visiting,
            &mut visited,
            &mut Vec::new(),
        ) {
            return Err(format!(
                "{CONTRACT_PATH}: dependency graph contains a cycle: {}",
                cycle.join(" -> ")
            ));
        }
    }
    Ok(())
}

fn visit_cycle(
    node: &str,
    graph: &BTreeMap<String, BTreeSet<String>>,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
    path: &mut Vec<String>,
) -> Option<Vec<String>> {
    if visiting.contains(node) {
        let start = path.iter().position(|item| item == node).unwrap_or(0);
        let mut cycle = path[start..].to_vec();
        cycle.push(node.to_owned());
        return Some(cycle);
    }
    if !visited.insert(node.to_owned()) {
        return None;
    }
    visiting.insert(node.to_owned());
    path.push(node.to_owned());
    if let Some(destinations) = graph.get(node) {
        for destination in destinations {
            if let Some(cycle) = visit_cycle(destination, graph, visiting, visited, path) {
                return Some(cycle);
            }
        }
    }
    path.pop();
    visiting.remove(node);
    None
}

fn validate_feature_declarations(package: &DeclaredPackage) -> Result<()> {
    let mut features = BTreeSet::new();
    for feature in &package.features {
        if feature.is_empty() || !features.insert(feature) {
            return Err(format!(
                "{CONTRACT_PATH}: package {} has duplicate or empty feature declarations",
                package.name
            ));
        }
    }
    for feature_set in &package.feature_sets {
        let mut set = BTreeSet::new();
        for feature in feature_set {
            if !features.contains(feature) || !set.insert(feature) {
                return Err(format!(
                    "{CONTRACT_PATH}: package {} has a feature set containing an unknown or duplicate feature",
                    package.name
                ));
            }
        }
        if set.is_empty() {
            return Err(format!(
                "{CONTRACT_PATH}: package {} has an empty feature set",
                package.name
            ));
        }
    }
    Ok(())
}

pub fn edge_key(edge: &DeclaredEdge) -> (String, String, String, String) {
    (
        edge.from.clone(),
        edge.to.clone(),
        edge.kind.clone(),
        normalized_target(edge.target.as_deref()),
    )
}

pub fn external_key(rule: &DeclaredExternal) -> (String, String, String, String, String, String) {
    (
        rule.package.clone(),
        rule.source.clone().unwrap_or_default(),
        rule.kinds.join(","),
        rule.targets.join(","),
        rule.source_roles.join(","),
        rule.source_packages.join(","),
    )
}

pub fn normalized_target(target: Option<&str>) -> String {
    target.unwrap_or("all").to_owned()
}

pub fn normalized_kind(kind: Option<&str>) -> String {
    kind.unwrap_or("normal").to_owned()
}

pub fn relative_manifest(root: &Path, manifest: &Path) -> String {
    let root_text = root.to_string_lossy().replace('\\', "/");
    let manifest_text = manifest.to_string_lossy().replace('\\', "/");
    let relative = manifest.strip_prefix(root).map_or_else(
        |_| manifest_text.clone(),
        |path| path.to_string_lossy().into_owned(),
    );
    let relative = relative.replace('\\', "/");
    if relative == manifest_text && manifest_text.starts_with(&(root_text.clone() + "/")) {
        manifest_text[root_text.len() + 1..].to_owned()
    } else {
        relative.trim_start_matches("./").to_owned()
    }
}
