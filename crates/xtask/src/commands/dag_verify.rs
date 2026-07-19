use super::graph::{cycle_edges, topological_order};
use super::model::PlatformTarget;
use super::model::{
    Contract, DeclaredEdge, DeclaredExternal, DiscoveredEdge, MetadataContext, MetadataPackage,
    PlatformContract, edge_key, external_key, normalized_kind, normalized_target,
    relative_manifest,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd, serde::Serialize)]
pub struct AllowedEdge {
    source: String,
    destination: String,
    dependency_kind: String,
    target: String,
    required: bool,
    feature_contexts: Vec<String>,
    tooling_only: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PackageReceipt {
    name: String,
    manifest: String,
    role: String,
    integration_owner: String,
    public_features: Vec<String>,
    feature_sets: Vec<Vec<String>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ExternalRuleReceipt {
    package: String,
    source: Option<String>,
    source_roles: Vec<String>,
    source_packages: Vec<String>,
    dependency_kinds: Vec<String>,
    targets: Vec<String>,
    feature_contexts: Vec<String>,
    optional: Option<bool>,
    allow_transitive: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ContextReceipt {
    pub(super) name: String,
    pub(super) package: Option<String>,
    pub(super) target: Option<String>,
    pub(super) feature_set: Vec<String>,
    pub(super) args: Vec<String>,
    pub(super) discovered_edges: Vec<DiscoveredEdge>,
    pub(super) topological_order: Vec<String>,
    pub(super) violations: Vec<Violation>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Violation {
    pub(super) code: String,
    pub(super) source: String,
    pub(super) source_role: String,
    pub(super) destination: String,
    pub(super) destination_source: String,
    pub(super) alias: String,
    pub(super) dependency_kind: String,
    pub(super) target: String,
    pub(super) platform: String,
    pub(super) optional: bool,
    pub(super) feature_context: String,
    pub(super) rule: String,
    pub(super) message: String,
    pub(super) allowed_alternatives: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct DagReport {
    pub(super) schema_version: u32,
    pub(super) contract: String,
    pub(super) first_violation: Option<Violation>,
    pub(super) platforms: Vec<PlatformTarget>,
    pub(super) package_inventory: Vec<PackageReceipt>,
    pub(super) allowed_edges: Vec<AllowedEdge>,
    pub(super) external_rules: Vec<ExternalRuleReceipt>,
    pub(super) discovered_edges: Vec<DiscoveredEdge>,
    pub(super) feature_contexts: Vec<ContextReceipt>,
    pub(super) topological_order: Vec<String>,
    pub(super) violations: Vec<Violation>,
}

pub fn verify(
    contract: &Contract,
    platform: &PlatformContract,
    contexts: &[MetadataContext],
) -> DagReport {
    let mut context_receipts = Vec::new();
    let mut all_discovered = BTreeSet::new();
    let mut all_violations = Vec::new();
    let mut selected_order = Vec::new();
    let mut ordered_contexts = contexts.iter().collect::<Vec<_>>();
    ordered_contexts.sort_by(|left, right| left.name.cmp(&right.name));
    for context in ordered_contexts {
        let (discovered, mut violations) = discover_context(contract, context);
        let order = topological_order(
            &discovered,
            contract.packages.iter().map(|package| package.name.clone()),
        );
        if selected_order.is_empty() || context.name == "all-features" {
            selected_order.clone_from(&order);
        }
        all_discovered.extend(discovered.iter().cloned());
        all_violations.append(&mut violations);
        context_receipts.push(ContextReceipt {
            name: context.name.clone(),
            package: context.package.clone(),
            target: context.target.clone(),
            feature_set: context.feature_set.clone(),
            args: context.args.clone(),
            discovered_edges: discovered,
            topological_order: order,
            violations: Vec::new(),
        });
    }
    all_violations.extend(verify_package_inventory(contract, contexts));
    all_violations.sort_by(violation_order);
    for receipt in &mut context_receipts {
        receipt.violations = all_violations
            .iter()
            .filter(|violation| violation.feature_context == receipt.name)
            .cloned()
            .collect();
    }
    DagReport {
        schema_version: contract.schema_version,
        contract: "architecture/workspace-dag.toml".to_owned(),
        first_violation: all_violations.first().cloned(),
        platforms: platform.targets().to_vec(),
        package_inventory: declared_package_receipts(contract),
        allowed_edges: allowed_edge_receipts(contract),
        external_rules: external_rule_receipts(contract),
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
            integration_owner: package.integration_owner.clone(),
            public_features: sorted_strings(&package.features),
            feature_sets: sorted_feature_sets(&package.feature_sets),
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
            feature_contexts: sorted_strings(&edge.contexts),
            tooling_only: edge.tooling_only,
        })
        .collect::<Vec<_>>();
    result.sort();
    result
}

fn external_rule_receipts(contract: &Contract) -> Vec<ExternalRuleReceipt> {
    let mut result = contract
        .external_dependencies
        .iter()
        .map(|rule| ExternalRuleReceipt {
            package: rule.package.clone(),
            source: rule.source.clone(),
            source_roles: sorted_strings(&rule.source_roles),
            source_packages: sorted_strings(&rule.source_packages),
            dependency_kinds: sorted_strings(&rule.kinds),
            targets: sorted_strings(&rule.targets),
            feature_contexts: sorted_strings(&rule.contexts),
            optional: rule.optional,
            allow_transitive: rule.allow_transitive,
        })
        .collect::<Vec<_>>();
    result.sort_by(|left, right| {
        (
            &left.package,
            &left.source,
            &left.source_packages,
            &left.dependency_kinds,
        )
            .cmp(&(
                &right.package,
                &right.source,
                &right.source_packages,
                &right.dependency_kinds,
            ))
    });
    result
}

fn sorted_strings(values: &[String]) -> Vec<String> {
    let mut result = values.to_vec();
    result.sort();
    result
}

fn sorted_feature_sets(values: &[Vec<String>]) -> Vec<Vec<String>> {
    let mut result = values.to_vec();
    for value in &mut result {
        value.sort();
    }
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
        .map(|package| (package.id.clone(), package))
        .collect::<BTreeMap<_, _>>();
    let roles = contract
        .packages
        .iter()
        .map(|package| (package.name.clone(), package.role.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut discovered = match discover_edges(context, &packages, &workspace_ids, &roles) {
        Ok(edges) => edges,
        Err(error) => return (Vec::new(), vec![*error]),
    };
    annotate_rules(contract, context, &mut discovered);
    let mut violations = validate_discovered_edges(contract, context, &discovered, &roles);
    violations.extend(validate_required_edges(contract, context, &discovered));
    violations.extend(validate_cycles(contract, context, &discovered));
    (discovered, violations)
}

fn discover_edges(
    context: &MetadataContext,
    packages: &BTreeMap<String, &MetadataPackage>,
    workspace_ids: &BTreeSet<String>,
    roles: &BTreeMap<String, String>,
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
        if !workspace_ids.contains(&node.id) {
            continue;
        }
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
            let external = !workspace_ids.contains(&destination_package.id);
            for (kind, target) in dependency_kinds {
                let destination_source = if external {
                    destination_package.source.clone().unwrap_or_else(|| {
                        format!(
                            "path:{}",
                            relative_manifest(
                                &context.metadata.workspace_root,
                                &destination_package.manifest_path
                            )
                        )
                    })
                } else {
                    "workspace".to_owned()
                };
                discovered.insert(DiscoveredEdge {
                    source: source_package.name.clone(),
                    destination: destination_package.name.clone(),
                    destination_source,
                    source_role: roles
                        .get(&source_package.name)
                        .cloned()
                        .unwrap_or_else(|| "<unknown>".to_owned()),
                    alias: dependency.name.clone(),
                    dependency_kind: normalized_kind(kind),
                    target: normalized_target(target),
                    platform: context.target.clone().unwrap_or_else(|| "all".to_owned()),
                    optional,
                    activated: true,
                    transitive: dependency.transitive,
                    external,
                    feature_context: context.name.clone(),
                    rule: String::new(),
                });
            }
        }
    }
    Ok(discovered.into_iter().collect())
}

fn annotate_rules(contract: &Contract, context: &MetadataContext, edges: &mut [DiscoveredEdge]) {
    for edge in edges {
        edge.rule = if edge.external {
            external_rule_for(contract, context, edge)
                .map_or_else(|| "<unmatched>".to_owned(), external_rule_id)
        } else {
            format!(
                "workspace:{}->{}:{}:{}",
                edge.source, edge.destination, edge.dependency_kind, edge.target
            )
        };
    }
}

fn validate_discovered_edges(
    contract: &Contract,
    context: &MetadataContext,
    discovered: &[DiscoveredEdge],
    roles: &BTreeMap<String, String>,
) -> Vec<Violation> {
    let declared = contract
        .edges
        .iter()
        .filter(|edge| context_applies(edge, context))
        .collect::<Vec<_>>();
    let mut violations = Vec::new();
    for edge in discovered {
        if edge.external {
            if external_rule_for(contract, context, edge).is_none() {
                let same_package = contract
                    .external_dependencies
                    .iter()
                    .any(|rule| rule.package == edge.destination);
                violations.push(edge_violation(
                    if same_package {
                        "external-rule-mismatch"
                    } else {
                        "undeclared-external-edge"
                    },
                    edge,
                    context,
                    if same_package {
                        "external dependency provenance, owner, kind, target, or feature context is not approved"
                    } else {
                        "direct external dependency is absent from the authoritative ownership contract"
                    },
                    allowed_external_alternatives(contract, &edge.destination),
                ));
            }
            continue;
        }
        let alternatives = allowed_destinations(contract, &edge.source);
        let source_role = roles.get(&edge.source).map(String::as_str);
        let destination_role = roles.get(&edge.destination).map(String::as_str);
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
        let key = (
            edge.source.clone(),
            edge.destination.clone(),
            edge.dependency_kind.clone(),
            edge.target.clone(),
        );
        if !declared.iter().any(|candidate| edge_key(candidate) == key) {
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
    contract
        .edges
        .iter()
        .filter(|edge| edge.required && context_applies(edge, context))
        .filter_map(|edge| {
            let key = edge_key(edge);
            (!discovered.iter().any(|candidate| {
                !candidate.external
                    && (
                        candidate.source.clone(),
                        candidate.destination.clone(),
                        candidate.dependency_kind.clone(),
                        candidate.target.clone(),
                    ) == key
            }))
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

fn context_applies(edge: &DeclaredEdge, context: &MetadataContext) -> bool {
    let base = base_context(&context.name);
    (edge.contexts.is_empty() || edge.contexts.iter().any(|candidate| candidate == base))
        && target_applies(edge.target.as_deref(), context.target.as_deref())
}

fn target_applies(target: Option<&str>, platform: Option<&str>) -> bool {
    let Some(target) = target.filter(|target| *target != "all") else {
        return true;
    };
    let Some(platform) = platform else {
        return true;
    };
    if target == platform {
        return true;
    }
    let architecture = platform.split('-').next().unwrap_or_default();
    match target {
        "cfg(unix)" => platform.contains("-linux-") || platform.contains("-apple-"),
        "cfg(windows)" => platform.contains("-windows-"),
        "cfg(target_os = \"windows\")" => platform.contains("windows"),
        "cfg(target_os = \"macos\")" => platform.contains("apple-darwin"),
        "cfg(target_os = \"linux\")" => platform.contains("-linux-"),
        "cfg(target_family = \"unix\")" => {
            platform.contains("-linux-") || platform.contains("-apple-")
        }
        "cfg(target_arch = \"x86_64\")" => architecture == "x86_64",
        "cfg(target_arch = \"aarch64\")" => architecture == "aarch64",
        _ => false,
    }
}

fn base_context(context: &str) -> &str {
    context
        .split_once("@target:")
        .map_or(context, |(base, _)| base)
}

fn external_rule_for<'a>(
    contract: &'a Contract,
    context: &MetadataContext,
    edge: &DiscoveredEdge,
) -> Option<&'a DeclaredExternal> {
    contract
        .external_dependencies
        .iter()
        .filter(|rule| rule_matches(rule, context, edge))
        .min_by_key(|rule| external_key(rule))
}

fn rule_matches(rule: &DeclaredExternal, context: &MetadataContext, edge: &DiscoveredEdge) -> bool {
    rule.package == edge.destination
        && rule
            .source
            .as_deref()
            .is_none_or(|source| source == edge.destination_source)
        && (rule.source_packages.is_empty() || rule.source_packages.contains(&edge.source))
        && (rule.source_roles.is_empty() || rule.source_roles.contains(&edge.source_role))
        && (rule.kinds.is_empty() || rule.kinds.contains(&edge.dependency_kind))
        && (rule.targets.is_empty()
            || rule.targets.iter().any(|target| {
                target == "all" || target == &edge.target || target == &edge.platform
            }))
        && (rule.contexts.is_empty()
            || rule
                .contexts
                .iter()
                .any(|candidate| candidate == base_context(&context.name)))
        && rule
            .optional
            .is_none_or(|optional| optional == edge.optional)
        && (!edge.transitive || rule.allow_transitive)
}

fn external_rule_id(rule: &DeclaredExternal) -> String {
    format!(
        "external:{}:{}:{}",
        rule.package,
        rule.source_packages.join("|"),
        rule.kinds.join("|")
    )
}

fn allowed_external_alternatives(contract: &Contract, destination: &str) -> Vec<String> {
    contract
        .external_dependencies
        .iter()
        .filter(|rule| rule.package == destination)
        .map(external_rule_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn allowed_destinations(contract: &Contract, source: &str) -> Vec<String> {
    contract
        .edges
        .iter()
        .filter(|edge| edge.from == source)
        .map(|edge| edge.to.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|destination| destination != source)
        .collect()
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
        source_role: "<unknown>".to_owned(),
        destination: destination.to_owned(),
        destination_source: "<unknown>".to_owned(),
        alias: destination.to_owned(),
        dependency_kind: dependency_kind.to_owned(),
        target: "all".to_owned(),
        platform: context.target.clone().unwrap_or_else(|| "all".to_owned()),
        optional: false,
        feature_context: context.name.clone(),
        rule: "<none>".to_owned(),
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
    Violation {
        code: code.to_owned(),
        source: edge.source.clone(),
        source_role: edge.source_role.clone(),
        destination: edge.destination.clone(),
        destination_source: edge.destination_source.clone(),
        alias: edge.alias.clone(),
        dependency_kind: edge.dependency_kind.clone(),
        target: edge.target.clone(),
        platform: edge.platform.clone(),
        optional: edge.optional,
        feature_context: context.name.clone(),
        rule: edge.rule.clone(),
        message: message.to_owned(),
        allowed_alternatives,
    }
}

fn verify_package_inventory(contract: &Contract, contexts: &[MetadataContext]) -> Vec<Violation> {
    let declared = contract
        .packages
        .iter()
        .map(|package| package.name.as_str())
        .collect::<BTreeSet<_>>();
    let mut violations = Vec::new();
    let mut seen = BTreeSet::new();
    let mut all_features = None;
    for context in contexts {
        if context.name == "all-features" {
            all_features = Some(context);
        }
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
            push_unique(
                &mut violations,
                &mut seen,
                violation(
                    "extra-package",
                    "<workspace>",
                    &package.name,
                    "package",
                    context,
                    "cargo metadata discovered a workspace package absent from the authoritative DAG",
                    declared.iter().map(|name| (*name).to_owned()).collect(),
                ),
            );
        }
        for package in contract
            .packages
            .iter()
            .filter(|package| !actual_names.contains(package.name.as_str()))
        {
            push_unique(
                &mut violations,
                &mut seen,
                violation(
                    "missing-package",
                    "<workspace>",
                    &package.name,
                    "package",
                    context,
                    "authoritative DAG declares a workspace package absent from cargo metadata",
                    actual_names.iter().map(|name| (*name).to_owned()).collect(),
                ),
            );
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
                push_unique(
                    &mut violations,
                    &mut seen,
                    violation(
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
                    ),
                );
            }
        }
    }
    if let Some(context) = all_features {
        verify_features(contract, contexts, context, &mut violations, &mut seen);
    }
    violations
}

fn verify_features(
    contract: &Contract,
    contexts: &[MetadataContext],
    context: &MetadataContext,
    violations: &mut Vec<Violation>,
    seen: &mut BTreeSet<String>,
) {
    let workspace_ids = context
        .metadata
        .workspace_members
        .iter()
        .collect::<BTreeSet<_>>();
    for package in context
        .metadata
        .packages
        .iter()
        .filter(|package| workspace_ids.contains(&package.id))
    {
        let Some(declared) = contract
            .packages
            .iter()
            .find(|item| item.name == package.name)
        else {
            continue;
        };
        let actual = package
            .features
            .keys()
            .filter(|feature| feature.as_str() != "default")
            .cloned()
            .collect::<BTreeSet<_>>();
        let expected = declared.features.iter().cloned().collect::<BTreeSet<_>>();
        verify_feature_names(&package.name, &actual, &expected, context, violations, seen);
        verify_feature_contexts(
            &package.name,
            &expected,
            &declared.feature_sets,
            contexts,
            context,
            violations,
            seen,
        );
    }
}

fn verify_feature_names(
    package: &str,
    actual: &BTreeSet<String>,
    expected: &BTreeSet<String>,
    context: &MetadataContext,
    violations: &mut Vec<Violation>,
    seen: &mut BTreeSet<String>,
) {
    for feature in actual.difference(expected) {
        push_unique(
            violations,
            seen,
            violation(
                "undeclared-feature",
                package,
                feature,
                "feature",
                context,
                "Cargo exposes a public feature absent from the architecture contract",
                expected.iter().cloned().collect(),
            ),
        );
    }
    for feature in expected.difference(actual) {
        push_unique(
            violations,
            seen,
            violation(
                "stale-feature",
                package,
                feature,
                "feature",
                context,
                "architecture contract declares a feature absent from Cargo metadata",
                actual.iter().cloned().collect(),
            ),
        );
    }
}

fn verify_feature_contexts(
    package: &str,
    expected: &BTreeSet<String>,
    feature_sets: &[Vec<String>],
    contexts: &[MetadataContext],
    context: &MetadataContext,
    violations: &mut Vec<Violation>,
    seen: &mut BTreeSet<String>,
) {
    for feature in expected {
        let tested = contexts.iter().any(|candidate| {
            candidate.package.as_deref() == Some(package)
                && candidate.feature_set == [feature.clone()]
        });
        if !tested {
            push_unique(
                violations,
                seen,
                violation(
                    "untested-feature",
                    package,
                    feature,
                    "feature",
                    context,
                    "declared public feature has no singleton validation context",
                    vec![format!("feature:{package}={feature}")],
                ),
            );
        }
    }
    for feature_set in feature_sets {
        let mut expected_set = feature_set.clone();
        expected_set.sort();
        let tested = contexts.iter().any(|candidate| {
            candidate.package.as_deref() == Some(package) && candidate.feature_set == expected_set
        });
        if !tested {
            push_unique(
                violations,
                seen,
                violation(
                    "untested-feature-set",
                    package,
                    &expected_set.join("+"),
                    "feature",
                    context,
                    "declared interacting feature set has no validation context",
                    vec![format!("feature:{package}={}", expected_set.join("+"))],
                ),
            );
        }
    }
}

fn push_unique(violations: &mut Vec<Violation>, seen: &mut BTreeSet<String>, violation: Violation) {
    let key = serde_json::to_string(&violation).expect("violation serializes");
    if seen.insert(key) {
        violations.push(violation);
    }
}

fn violation_order(left: &Violation, right: &Violation) -> std::cmp::Ordering {
    (
        &left.feature_context,
        &left.code,
        &left.source,
        &left.destination,
        &left.alias,
        &left.dependency_kind,
        &left.target,
        &left.platform,
    )
        .cmp(&(
            &right.feature_context,
            &right.code,
            &right.source,
            &right.destination,
            &right.alias,
            &right.dependency_kind,
            &right.target,
            &right.platform,
        ))
}
