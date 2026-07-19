use std::collections::BTreeSet;

use super::metadata;
use super::model::relative_manifest;
use super::{Contract, MetadataContext, PlatformContract, Result};
use crate::process::ProcessRunner;
use crate::root::RepositoryRoot;

#[derive(Clone)]
struct ContextSpec {
    name: String,
    package: Option<String>,
    feature_set: Vec<String>,
    all_features: bool,
    no_default_features: bool,
    features: Vec<String>,
    manifest: Option<String>,
}

pub fn load_contexts(
    root: &RepositoryRoot,
    contract: &Contract,
    platform: &PlatformContract,
    runner: &ProcessRunner,
) -> Result<Vec<MetadataContext>> {
    let all = metadata::run(
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
    )?;
    let mut specs = base_specs();
    add_feature_specs(contract, &all, &mut specs);

    let mut names = BTreeSet::new();
    specs.retain(|spec| names.insert(spec.name.clone()));
    let mut contexts = Vec::new();
    for spec in &specs {
        contexts.push(run_spec(root, runner, spec, None)?);
    }
    for target in platform.targets() {
        for spec in &specs {
            let name = format!("{}@target:{}", spec.name, target.triple);
            contexts.push(run_spec(
                root,
                runner,
                spec,
                Some((name, target.triple.clone())),
            )?);
        }
    }
    Ok(contexts)
}

fn base_specs() -> Vec<ContextSpec> {
    vec![
        ContextSpec {
            name: "default".to_owned(),
            package: None,
            feature_set: Vec::new(),
            all_features: false,
            no_default_features: false,
            features: Vec::new(),
            manifest: None,
        },
        ContextSpec {
            name: "all-features".to_owned(),
            package: None,
            feature_set: Vec::new(),
            all_features: true,
            no_default_features: false,
            features: Vec::new(),
            manifest: None,
        },
    ]
}

fn add_feature_specs(contract: &Contract, all: &MetadataContext, specs: &mut Vec<ContextSpec>) {
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
        .filter_map(|package| {
            contract
                .packages
                .iter()
                .find(|declared| declared.name == package.name)
                .map(|declared| (declared, package))
        })
        .filter(|(declared, package)| !declared.features.is_empty() || !package.features.is_empty())
        .map(|(declared, package)| {
            (
                declared.name.clone(),
                declared.features.clone(),
                declared.feature_sets.clone(),
                relative_manifest(&workspace_root, &package.manifest_path),
            )
        })
        .collect::<Vec<_>>();
    feature_packages.sort_by(|left, right| left.0.cmp(&right.0));
    for (package, features, feature_sets, manifest) in feature_packages {
        add_package_feature_specs(specs, &package, &features, &feature_sets, &manifest);
    }
}

fn add_package_feature_specs(
    specs: &mut Vec<ContextSpec>,
    package: &str,
    features: &[String],
    feature_sets: &[Vec<String>],
    manifest: &str,
) {
    specs.push(ContextSpec {
        name: format!("no-default-features:{package}"),
        package: Some(package.to_owned()),
        feature_set: Vec::new(),
        all_features: false,
        no_default_features: true,
        features: Vec::new(),
        manifest: Some(manifest.to_owned()),
    });
    for feature in features {
        specs.push(ContextSpec {
            name: format!("feature:{package}={feature}"),
            package: Some(package.to_owned()),
            feature_set: vec![feature.clone()],
            all_features: false,
            no_default_features: true,
            features: vec![feature.clone()],
            manifest: Some(manifest.to_owned()),
        });
    }
    for feature_set in feature_sets {
        let mut sorted = feature_set.clone();
        sorted.sort();
        specs.push(ContextSpec {
            name: format!("feature:{package}={}", sorted.join("+")),
            package: Some(package.to_owned()),
            feature_set: sorted.clone(),
            all_features: false,
            no_default_features: true,
            features: sorted,
            manifest: Some(manifest.to_owned()),
        });
    }
}

fn run_spec(
    root: &RepositoryRoot,
    runner: &ProcessRunner,
    spec: &ContextSpec,
    target: Option<(String, String)>,
) -> Result<MetadataContext> {
    let name = target
        .as_ref()
        .map_or_else(|| spec.name.clone(), |(name, _)| name.clone());
    let target_triple = target.map(|(_, triple)| triple);
    let mut args = vec!["metadata".to_owned(), "--locked".to_owned()];
    if spec.all_features {
        args.push("--all-features".to_owned());
    }
    if spec.no_default_features {
        args.push("--no-default-features".to_owned());
    }
    if !spec.features.is_empty() {
        args.extend(["--features".to_owned(), spec.features.join(",")]);
    }
    if let Some(manifest) = &spec.manifest {
        args.extend(["--manifest-path".to_owned(), manifest.clone()]);
    }
    if let Some(triple) = &target_triple {
        args.extend(["--filter-platform".to_owned(), triple.clone()]);
    }
    args.extend(["--format-version".to_owned(), "1".to_owned()]);
    let references = args.iter().map(String::as_str).collect::<Vec<_>>();
    metadata::run(root, runner, &name, &references, spec.package.clone()).map(|mut context| {
        context.target = target_triple;
        context.feature_set.clone_from(&spec.feature_set);
        context
    })
}
