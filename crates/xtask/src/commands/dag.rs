use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{Result, report};
use crate::process::ProcessRunner;
use crate::root::RepositoryRoot;

#[path = "dag_metadata.rs"]
mod metadata;

const CONTRACT_PATH: &str = "architecture/workspace-dag.toml";

pub(super) fn run(root: &RepositoryRoot, runner: &ProcessRunner) -> Result {
    let contract = load_contract(root)?;
    contract.validate()?;
    let contexts = load_contexts(root, &contract, runner)?;
    let verification = verify(&contract, &contexts);
    let serialized = serde_json::to_value(&verification).map_err(|error| error.to_string())?;
    if verification.violations.is_empty() {
        Ok(report(root, "repo.verify-dag", serialized))
    } else {
        Err(format!(
            "repo.verify-dag failed: {}",
            serde_json::to_string(&serialized).map_err(|error| error.to_string())?
        ))
    }
}

fn load_contract(root: &RepositoryRoot) -> Result<Contract> {
    let path = root.join(CONTRACT_PATH);
    let source = fs::read_to_string(&path)
        .map_err(|error| format!("{CONTRACT_PATH}: cannot read contract: {error}"))?;
    toml::from_str(&source).map_err(|error| format!("{CONTRACT_PATH}: invalid TOML: {error}"))
}

fn load_contexts(
    root: &RepositoryRoot,
    contract: &Contract,
    runner: &ProcessRunner,
) -> Result<Vec<MetadataContext>> {
    let mut contexts = Vec::new();
    contexts.push(metadata::run(
        root,
        runner,
        "default",
        &["metadata", "--locked", "--format-version", "1"],
        None,
    )?);
    contexts.push(metadata::run(
        root,
        runner,
        "all-features",
        &[
            "metadata",
            "--locked",
            "--all-features",
            "--format-version",
            "1",
        ],
        None,
    )?);

    let all = contexts
        .iter()
        .find(|context| context.name == "all-features")
        .ok_or_else(|| "workspace DAG: all-features metadata context missing".to_owned())?;
    let workspace_root = all.metadata.workspace_root.clone();
    let workspace_ids = all
        .metadata
        .workspace_members
        .iter()
        .collect::<BTreeSet<_>>();
    let mut feature_packages = all
        .metadata
        .packages
        .iter()
        .filter(|package| workspace_ids.contains(&package.id))
        .filter(|package| !package.features.is_empty())
        .map(|package| (package.name.clone(), package.manifest_path.clone()))
        .collect::<Vec<_>>();
    feature_packages.sort();
    for target in &contract.target_platforms {
        let context_name = format!("all-features@target:{target}");
        let args = [
            "metadata".to_owned(),
            "--locked".to_owned(),
            "--all-features".to_owned(),
            "--filter-platform".to_owned(),
            target.clone(),
            "--format-version".to_owned(),
            "1".to_owned(),
        ];
        let references = args.iter().map(String::as_str).collect::<Vec<_>>();
        contexts.push(metadata::run(
            root,
            runner,
            &context_name,
            &references,
            None,
        )?);
    }
    for (package, manifest) in feature_packages {
        let context_name = format!("no-default-features:{package}");
        let manifest_arg = relative_manifest(&workspace_root, &manifest);
        let args = [
            "metadata".to_owned(),
            "--locked".to_owned(),
            "--no-default-features".to_owned(),
            "--manifest-path".to_owned(),
            manifest_arg,
            "--format-version".to_owned(),
            "1".to_owned(),
        ];
        let references = args.iter().map(String::as_str).collect::<Vec<_>>();
        contexts.push(metadata::run(
            root,
            runner,
            &context_name,
            &references,
            Some(package),
        )?);
    }

    let declared_contexts = contract
        .edges
        .iter()
        .flat_map(|edge| edge.contexts.iter())
        .collect::<BTreeSet<_>>();
    for context in &contexts {
        if !declared_contexts.is_empty()
            && context.name.starts_with("no-default-features:")
            && !declared_contexts.contains(&context.name)
        {
            return Err(format!(
                "{CONTRACT_PATH}: feature context {} is not declared by any edge",
                context.name
            ));
        }
    }
    Ok(contexts)
}

#[derive(Debug, Deserialize)]
struct Contract {
    schema_version: u32,
    composition_root: String,
    #[serde(default)]
    tooling_packages: Vec<String>,
    #[serde(default)]
    target_platforms: Vec<String>,
    packages: Vec<DeclaredPackage>,
    edges: Vec<DeclaredEdge>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct DeclaredPackage {
    name: String,
    manifest: String,
    role: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct DeclaredEdge {
    from: String,
    to: String,
    #[serde(default = "default_kind")]
    kind: String,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    contexts: Vec<String>,
    #[serde(default = "default_required")]
    required: bool,
}

fn default_kind() -> String {
    "normal".to_owned()
}

const fn default_required() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct MetadataContext {
    name: String,
    package: Option<String>,
    args: Vec<String>,
    metadata: CargoMetadata,
}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    workspace_root: PathBuf,
    workspace_members: Vec<String>,
    packages: Vec<MetadataPackage>,
    resolve: Option<Resolve>,
}

#[derive(Debug, Deserialize)]
struct MetadataPackage {
    name: String,
    id: String,
    manifest_path: PathBuf,
    #[serde(default)]
    dependencies: Vec<MetadataDependency>,
    #[serde(default)]
    features: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct MetadataDependency {
    name: String,
    #[serde(default)]
    package: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    optional: bool,
}

#[derive(Debug, Deserialize)]
struct Resolve {
    nodes: Vec<ResolveNode>,
}

#[derive(Debug, Deserialize)]
struct ResolveNode {
    id: String,
    #[serde(default)]
    deps: Vec<ResolveDependency>,
}

#[derive(Debug, Deserialize)]
struct ResolveDependency {
    name: String,
    pkg: String,
    #[serde(default)]
    dep_kinds: Vec<ResolveDependencyKind>,
}

#[derive(Debug, Deserialize)]
struct ResolveDependencyKind {
    kind: Option<String>,
    target: Option<String>,
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd, Serialize)]
struct DiscoveredEdge {
    source: String,
    destination: String,
    dependency_kind: String,
    target: String,
    optional: bool,
    feature_context: String,
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd, Serialize)]
struct AllowedEdge {
    source: String,
    destination: String,
    dependency_kind: String,
    target: String,
    required: bool,
    feature_contexts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct PackageReceipt {
    name: String,
    manifest: String,
    role: String,
}

#[derive(Debug, Clone, Serialize)]
struct ContextReceipt {
    name: String,
    package: Option<String>,
    args: Vec<String>,
    discovered_edges: Vec<DiscoveredEdge>,
    topological_order: Vec<String>,
    violations: Vec<Violation>,
}

#[derive(Debug, Clone, Serialize)]
struct Violation {
    code: String,
    source: String,
    destination: String,
    dependency_kind: String,
    target: String,
    feature_context: String,
    message: String,
    allowed_alternatives: Vec<String>,
}

#[derive(Debug, Serialize)]
struct DagReport {
    schema_version: u32,
    contract: String,
    package_inventory: Vec<PackageReceipt>,
    allowed_edges: Vec<AllowedEdge>,
    discovered_edges: Vec<DiscoveredEdge>,
    feature_contexts: Vec<ContextReceipt>,
    topological_order: Vec<String>,
    violations: Vec<Violation>,
}

impl Contract {
    fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            return Err(format!(
                "{CONTRACT_PATH}: unsupported schema version {}",
                self.schema_version
            ));
        }
        let mut packages = BTreeSet::new();
        let mut roots = Vec::new();
        for package in &self.packages {
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
        }
        if roots != [self.composition_root.clone()] {
            return Err(format!(
                "{CONTRACT_PATH}: composition_root must name the only composition-root package"
            ));
        }
        if !packages.contains(&self.composition_root) {
            return Err(format!(
                "{CONTRACT_PATH}: composition_root {} is not declared",
                self.composition_root
            ));
        }
        let mut targets = BTreeSet::new();
        for target in &self.target_platforms {
            if target.is_empty() || !targets.insert(target) {
                return Err(format!(
                    "{CONTRACT_PATH}: target_platforms must be unique and non-empty"
                ));
            }
        }
        for tooling in &self.tooling_packages {
            if !packages.contains(tooling)
                || self
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
        let mut edges = BTreeSet::new();
        for edge in &self.edges {
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
            let key = edge_key(edge);
            if !edges.insert(key) {
                return Err(format!(
                    "{CONTRACT_PATH}: duplicate edge {} -> {}",
                    edge.from, edge.to
                ));
            }
        }
        Ok(())
    }
}

fn edge_key(edge: &DeclaredEdge) -> (String, String, String, String) {
    (
        edge.from.clone(),
        edge.to.clone(),
        edge.kind.clone(),
        normalized_target(edge.target.as_deref()),
    )
}

fn normalized_target(target: Option<&str>) -> String {
    target.unwrap_or("all").to_owned()
}

fn normalized_kind(kind: Option<&str>) -> String {
    kind.unwrap_or("normal").to_owned()
}

fn verify(contract: &Contract, contexts: &[MetadataContext]) -> DagReport {
    let packages = declared_package_receipts(contract);
    let allowed_edges = allowed_edge_receipts(contract);
    let mut context_receipts = Vec::new();
    let mut all_discovered = BTreeSet::new();
    let mut all_violations = Vec::new();
    let mut selected_order = Vec::new();

    for context in contexts {
        let (discovered, mut violations) = discover_context(contract, context);
        let order = topological_order(
            &discovered,
            contract.packages.iter().map(|p| p.name.clone()),
        );
        if selected_order.is_empty() || context.name == "all-features" {
            selected_order.clone_from(&order);
        }
        all_discovered.extend(discovered.iter().cloned());
        all_violations.append(&mut violations);
        context_receipts.push(ContextReceipt {
            name: context.name.clone(),
            package: context.package.clone(),
            args: context.args.clone(),
            discovered_edges: discovered,
            topological_order: order,
            violations: Vec::new(),
        });
    }
    for (context, receipt) in contexts.iter().zip(context_receipts.iter_mut()) {
        receipt.violations = all_violations
            .iter()
            .filter(|violation| violation.feature_context == context.name)
            .cloned()
            .collect();
    }
    let mut package_violations = verify_package_inventory(contract, contexts);
    all_violations.append(&mut package_violations);
    all_violations.sort_by(violation_order);
    DagReport {
        schema_version: 1,
        contract: CONTRACT_PATH.to_owned(),
        package_inventory: packages,
        allowed_edges,
        discovered_edges: all_discovered.into_iter().collect(),
        feature_contexts: context_receipts,
        topological_order: selected_order,
        violations: all_violations,
    }
}

fn declared_package_receipts(contract: &Contract) -> Vec<PackageReceipt> {
    let mut result = contract
        .packages
        .iter()
        .map(|package| PackageReceipt {
            name: package.name.clone(),
            manifest: package.manifest.clone(),
            role: package.role.clone(),
        })
        .collect::<Vec<_>>();
    result.sort_by(|left, right| left.name.cmp(&right.name));
    result
}

fn allowed_edge_receipts(contract: &Contract) -> Vec<AllowedEdge> {
    let mut result = contract
        .edges
        .iter()
        .map(|edge| AllowedEdge {
            source: edge.from.clone(),
            destination: edge.to.clone(),
            dependency_kind: edge.kind.clone(),
            target: normalized_target(edge.target.as_deref()),
            required: edge.required,
            feature_contexts: sorted_contexts(&edge.contexts),
        })
        .collect::<Vec<_>>();
    result.sort();
    result
}

fn sorted_contexts(contexts: &[String]) -> Vec<String> {
    let mut result = contexts.to_vec();
    result.sort();
    result
}

fn discover_context(
    contract: &Contract,
    context: &MetadataContext,
) -> (Vec<DiscoveredEdge>, Vec<Violation>) {
    let workspace_ids = context
        .metadata
        .workspace_members
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let packages = context
        .metadata
        .packages
        .iter()
        .filter(|package| workspace_ids.contains(&package.id))
        .map(|package| (package.id.clone(), package))
        .collect::<BTreeMap<_, _>>();
    let discovered = match discover_edges(context, &packages) {
        Ok(edges) => edges,
        Err(error) => return (Vec::new(), vec![*error]),
    };
    let mut violations = validate_discovered_edges(contract, context, &discovered);
    violations.extend(validate_required_edges(contract, context, &discovered));
    violations.extend(validate_cycles(contract, context, &discovered));
    (discovered, violations)
}

fn discover_edges(
    context: &MetadataContext,
    packages: &BTreeMap<String, &MetadataPackage>,
) -> std::result::Result<Vec<DiscoveredEdge>, Box<Violation>> {
    let Some(resolve) = &context.metadata.resolve else {
        return Err(Box::new(violation(
            "missing-resolve",
            "<workspace>",
            "<resolve>",
            "metadata",
            context,
            "cargo metadata omitted the resolved dependency graph",
            Vec::new(),
        )));
    };
    let mut discovered = BTreeSet::new();
    for node in &resolve.nodes {
        let Some(source_package) = packages.get(&node.id) else {
            continue;
        };
        for dependency in &node.deps {
            let Some(destination_package) = packages.get(&dependency.pkg) else {
                continue;
            };
            let metadata_dependency = source_package.dependencies.iter().find(|candidate| {
                candidate.name == dependency.name
                    && candidate
                        .package
                        .as_deref()
                        .is_none_or(|package| package == destination_package.name)
                    && (dependency.dep_kinds.is_empty()
                        || dependency.dep_kinds.iter().any(|kind| {
                            normalized_kind(candidate.kind.as_deref())
                                == normalized_kind(kind.kind.as_deref())
                                && normalized_target(candidate.target.as_deref())
                                    == normalized_target(kind.target.as_deref())
                        }))
            });
            let optional = metadata_dependency.is_some_and(|dependency| dependency.optional);
            let dependency_kinds: Vec<(Option<&str>, Option<&str>)> =
                if dependency.dep_kinds.is_empty() {
                    vec![(None, None)]
                } else {
                    dependency
                        .dep_kinds
                        .iter()
                        .map(|kind| (kind.kind.as_deref(), kind.target.as_deref()))
                        .collect()
                };
            for (kind, target) in dependency_kinds {
                discovered.insert(DiscoveredEdge {
                    source: source_package.name.clone(),
                    destination: destination_package.name.clone(),
                    dependency_kind: normalized_kind(kind),
                    target: normalized_target(target),
                    optional,
                    feature_context: context.name.clone(),
                });
            }
        }
    }
    Ok(discovered.into_iter().collect())
}

fn validate_discovered_edges(
    contract: &Contract,
    context: &MetadataContext,
    discovered: &[DiscoveredEdge],
) -> Vec<Violation> {
    let roles = contract
        .packages
        .iter()
        .map(|package| (package.name.clone(), package.role.as_str()))
        .collect::<BTreeMap<_, _>>();
    let declared = contract
        .edges
        .iter()
        .filter(|edge| context_applies(edge, &context.name))
        .collect::<Vec<_>>();
    let mut violations = Vec::new();
    for edge in discovered {
        let alternatives = allowed_destinations(contract, &edge.source);
        let source_role = roles.get(&edge.source).copied();
        let destination_role = roles.get(&edge.destination).copied();
        if source_role != Some("tooling") && destination_role == Some("tooling") {
            violations.push(edge_violation(
                "tooling-leak",
                edge,
                context,
                "product packages may not depend on repository tooling",
                alternatives.clone(),
            ));
        }
        if destination_role == Some("composition-root") && edge.source != contract.composition_root
        {
            violations.push(edge_violation(
                "composition-root-reverse-edge",
                edge,
                context,
                "only the declared composition root may be the application root",
                alternatives.clone(),
            ));
        }
        let key = discovered_key(edge);
        if !declared
            .iter()
            .any(|candidate| edge_matches(candidate, &key))
        {
            let reverse = declared.iter().any(|candidate| {
                candidate.from == edge.destination
                    && candidate.to == edge.source
                    && candidate.kind == edge.dependency_kind
            });
            violations.push(edge_violation(
                if reverse {
                    "forbidden-reverse-edge"
                } else {
                    "undeclared-edge"
                },
                edge,
                context,
                if reverse {
                    "dependency reverses an explicitly declared architecture edge"
                } else {
                    "dependency is absent from the authoritative workspace DAG"
                },
                alternatives,
            ));
        }
    }
    violations
}

fn validate_required_edges(
    contract: &Contract,
    context: &MetadataContext,
    discovered: &[DiscoveredEdge],
) -> Vec<Violation> {
    let declared = contract
        .edges
        .iter()
        .filter(|edge| context_applies(edge, &context.name))
        .filter(|edge| edge.required);
    declared
        .filter_map(|edge| {
            let key = edge_key(edge);
            (!discovered
                .iter()
                .any(|candidate| discovered_key(candidate) == key))
            .then(|| {
                violation(
                    "missing-edge",
                    &edge.from,
                    &edge.to,
                    &edge.kind,
                    context,
                    "required architecture edge is absent from cargo metadata",
                    allowed_destinations(contract, &edge.from),
                )
            })
        })
        .collect()
}

fn validate_cycles(
    contract: &Contract,
    context: &MetadataContext,
    discovered: &[DiscoveredEdge],
) -> Vec<Violation> {
    let order = topological_order(
        discovered,
        contract.packages.iter().map(|package| package.name.clone()),
    );
    cycle_edges(discovered)
        .into_iter()
        .filter(|_| order.len() != contract.packages.len())
        .map(|(source, destination)| {
            violation(
                "cycle",
                &source,
                &destination,
                "cycle",
                context,
                "workspace dependency graph contains a cycle",
                allowed_destinations(contract, &source),
            )
        })
        .collect()
}

fn context_applies(edge: &DeclaredEdge, context: &str) -> bool {
    let feature_context = context
        .split_once("@target:")
        .map_or(context, |(feature, _)| feature);
    edge.contexts.is_empty()
        || edge
            .contexts
            .iter()
            .any(|candidate| candidate == feature_context)
}

fn edge_matches(declared: &DeclaredEdge, discovered: &(String, String, String, String)) -> bool {
    edge_key(declared) == *discovered
}

fn discovered_key(edge: &DiscoveredEdge) -> (String, String, String, String) {
    (
        edge.source.clone(),
        edge.destination.clone(),
        edge.dependency_kind.clone(),
        edge.target.clone(),
    )
}

fn allowed_destinations(contract: &Contract, source: &str) -> Vec<String> {
    let mut destinations = contract
        .edges
        .iter()
        .filter(|edge| edge.from == source)
        .map(|edge| edge.to.clone())
        .collect::<BTreeSet<_>>();
    destinations.remove(source);
    destinations.into_iter().collect()
}

fn violation(
    code: &str,
    source: &str,
    destination: &str,
    dependency_kind: &str,
    context: &MetadataContext,
    message: &str,
    allowed_alternatives: Vec<String>,
) -> Violation {
    Violation {
        code: code.to_owned(),
        source: source.to_owned(),
        destination: destination.to_owned(),
        dependency_kind: dependency_kind.to_owned(),
        target: "all".to_owned(),
        feature_context: context.name.clone(),
        message: message.to_owned(),
        allowed_alternatives,
    }
}

fn edge_violation(
    code: &str,
    edge: &DiscoveredEdge,
    context: &MetadataContext,
    message: &str,
    allowed_alternatives: Vec<String>,
) -> Violation {
    let mut result = violation(
        code,
        &edge.source,
        &edge.destination,
        &edge.dependency_kind,
        context,
        message,
        allowed_alternatives,
    );
    result.target.clone_from(&edge.target);
    result
}

fn verify_package_inventory(contract: &Contract, contexts: &[MetadataContext]) -> Vec<Violation> {
    let mut violations = Vec::new();
    let declared = contract
        .packages
        .iter()
        .map(|package| package.name.as_str())
        .collect::<BTreeSet<_>>();
    for context in contexts {
        let workspace_ids = context
            .metadata
            .workspace_members
            .iter()
            .collect::<BTreeSet<_>>();
        let actual = context
            .metadata
            .packages
            .iter()
            .filter(|package| workspace_ids.contains(&package.id))
            .collect::<Vec<_>>();
        let actual_names = actual
            .iter()
            .map(|package| package.name.as_str())
            .collect::<BTreeSet<_>>();
        for package in actual
            .iter()
            .filter(|package| !declared.contains(package.name.as_str()))
        {
            violations.push(violation(
                "extra-package",
                "<workspace>",
                &package.name,
                "package",
                context,
                "cargo metadata discovered a workspace package absent from the authoritative DAG",
                declared.iter().map(|name| (*name).to_owned()).collect(),
            ));
        }
        for package in contract
            .packages
            .iter()
            .filter(|package| !actual_names.contains(package.name.as_str()))
        {
            violations.push(violation(
                "missing-package",
                "<workspace>",
                &package.name,
                "package",
                context,
                "authoritative DAG declares a workspace package absent from cargo metadata",
                actual_names.iter().map(|name| (*name).to_owned()).collect(),
            ));
        }
        for package in actual {
            let Some(declared_package) = contract
                .packages
                .iter()
                .find(|candidate| candidate.name == package.name)
            else {
                continue;
            };
            let actual_manifest =
                relative_manifest(&context.metadata.workspace_root, &package.manifest_path);
            if actual_manifest != declared_package.manifest {
                violations.push(violation(
                    "manifest-mismatch",
                    &package.name,
                    &package.name,
                    "package",
                    context,
                    &format!(
                        "manifest path differs: metadata={} contract={}",
                        actual_manifest, declared_package.manifest
                    ),
                    vec![declared_package.manifest.clone()],
                ));
            }
        }
    }
    violations
}

fn relative_manifest(root: &Path, manifest: &Path) -> String {
    manifest
        .strip_prefix(root)
        .unwrap_or(manifest)
        .to_string_lossy()
        .replace('\\', "/")
}

fn violation_order(left: &Violation, right: &Violation) -> std::cmp::Ordering {
    (
        &left.feature_context,
        &left.code,
        &left.source,
        &left.destination,
        &left.dependency_kind,
        &left.target,
    )
        .cmp(&(
            &right.feature_context,
            &right.code,
            &right.source,
            &right.destination,
            &right.dependency_kind,
            &right.target,
        ))
}

fn topological_order<I>(edges: &[DiscoveredEdge], packages: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let package_set = packages.into_iter().collect::<BTreeSet<_>>();
    let mut outgoing = package_set
        .iter()
        .map(|package| (package.clone(), BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    let mut incoming = package_set
        .iter()
        .map(|package| (package.clone(), 0usize))
        .collect::<BTreeMap<_, _>>();
    for edge in edges {
        if package_set.contains(&edge.source)
            && package_set.contains(&edge.destination)
            && outgoing
                .get_mut(&edge.source)
                .is_some_and(|destinations| destinations.insert(edge.destination.clone()))
            && let Some(count) = incoming.get_mut(&edge.destination)
        {
            *count += 1;
        }
    }
    let mut ready = incoming
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(package, _)| package.clone())
        .collect::<BTreeSet<_>>();
    let mut order = Vec::with_capacity(package_set.len());
    while let Some(package) = ready.pop_first() {
        order.push(package.clone());
        if let Some(destinations) = outgoing.get(&package) {
            for destination in destinations {
                let count = incoming
                    .get_mut(destination)
                    .expect("outgoing package has an incoming count");
                *count -= 1;
                if *count == 0 {
                    ready.insert(destination.clone());
                }
            }
        }
    }
    order
}

fn cycle_edges(edges: &[DiscoveredEdge]) -> Vec<(String, String)> {
    let nodes = edges
        .iter()
        .flat_map(|edge| [edge.source.clone(), edge.destination.clone()])
        .collect::<BTreeSet<_>>();
    let order = topological_order(edges, nodes.iter().cloned());
    let residual = nodes
        .difference(&order.iter().cloned().collect::<BTreeSet<_>>())
        .cloned()
        .collect::<BTreeSet<_>>();
    edges
        .iter()
        .filter(|edge| residual.contains(&edge.source) && residual.contains(&edge.destination))
        .map(|edge| (edge.source.clone(), edge.destination.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
#[path = "dag_tests.rs"]
mod tests;
