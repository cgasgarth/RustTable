use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest path"));
    let locales = root.join("../../locales");
    let english = locales.join("en-US/messages.ftl");
    let source = read(&english);
    let source_ids = ids(&source, &english);
    assert!(
        !source_ids.is_empty(),
        "English catalog must contain messages"
    );

    let entries = fs::read_dir(&locales).expect("read locales directory");
    for entry in entries {
        let entry = entry.expect("read locale entry");
        let path = entry.path().join("messages.ftl");
        if !path.is_file() {
            continue;
        }
        println!("cargo:rerun-if-changed={}", path.display());
        let translated = read(&path);
        let translated_ids = ids(&translated, &path);
        let missing = source_ids
            .difference(&translated_ids)
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            missing.is_empty(),
            "{} is missing message IDs: {missing:?}",
            path.display()
        );
        let source_args = arguments(&source);
        let translated_args = arguments(&translated);
        for id in &source_ids {
            let expected = source_args.get(id).cloned().unwrap_or_default();
            let actual = translated_args.get(id).cloned().unwrap_or_default();
            assert_eq!(
                expected,
                actual,
                "{} changes typed arguments for {id}",
                path.display()
            );
        }
    }
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn ids(source: &str, path: &Path) -> BTreeSet<String> {
    let resource = fluent_syntax::parser::parse(source)
        .unwrap_or_else(|errors| panic!("{}: Fluent parse errors: {errors:?}", path.display()));
    let mut ids = BTreeSet::new();
    for entry in resource.body {
        if let fluent_syntax::ast::Entry::Message(message) = entry {
            let id = message.id.name.to_owned();
            assert!(ids.insert(id.clone()), "{} duplicates {id}", path.display());
        }
    }
    ids
}

fn arguments(source: &str) -> std::collections::BTreeMap<String, BTreeSet<String>> {
    let resource = fluent_syntax::parser::parse(source).expect("catalog was parsed above");
    resource
        .body
        .into_iter()
        .filter_map(|entry| match entry {
            fluent_syntax::ast::Entry::Message(message) => Some((
                message.id.name.to_owned(),
                variable_names(&format!("{message:?}")),
            )),
            _ => None,
        })
        .collect()
}

fn variable_names(source: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut current = String::new();
    let mut in_name = false;
    for character in source.chars() {
        if in_name && (character.is_ascii_alphanumeric() || character == '_') {
            current.push(character);
        } else {
            if in_name && !current.is_empty() {
                names.insert(current.clone());
            }
            current.clear();
            in_name = character == '$';
        }
    }
    if in_name && !current.is_empty() {
        names.insert(current);
    }
    names
}
