use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use super::model::{ShaderError, SourceSpanAlias};

const MAX_SOURCE_BYTES: usize = 64 * 1024;
const MAX_EXPANDED_BYTES: usize = 256 * 1024;
const MAX_INCLUDE_DEPTH: usize = 8;
const MAX_INCLUDE_COUNT: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedShaderSource {
    pub text: String,
    pub line_aliases: Vec<SourceSpanAlias>,
}

#[derive(Debug, Clone)]
pub(crate) struct SourceCatalog {
    sources: BTreeMap<String, String>,
}

impl SourceCatalog {
    pub(crate) fn checked_in() -> Self {
        let mut sources = BTreeMap::new();
        sources.insert(
            "shaders/point.wgsl".to_owned(),
            include_str!("../../shaders/point.wgsl").to_owned(),
        );
        sources.insert(
            "shaders/includes/point_common.wgsl".to_owned(),
            include_str!("../../shaders/includes/point_common.wgsl").to_owned(),
        );
        sources.insert(
            "shaders/fixtures/invalid_syntax.wgsl".to_owned(),
            include_str!("../../shaders/fixtures/invalid_syntax.wgsl").to_owned(),
        );
        Self { sources }
    }

    pub(crate) fn get(&self, alias: &str) -> Option<&str> {
        self.sources.get(alias).map(String::as_str)
    }

    pub(crate) fn aliases(&self) -> impl Iterator<Item = &str> {
        self.sources.keys().map(String::as_str)
    }

    pub(crate) fn source_tree_hash(&self, root: &str) -> Result<String, ShaderError> {
        let mut hasher = Sha256::new();
        for alias in self
            .aliases()
            .filter(|alias| *alias == root || alias.starts_with("shaders/includes/"))
        {
            let bytes = self
                .get(alias)
                .ok_or_else(|| ShaderError::SourceNotFound((*alias).to_owned()))?
                .as_bytes();
            hasher.update(alias.as_bytes());
            hasher.update([0]);
            hasher.update(bytes);
            hasher.update([0]);
        }
        Ok(hex_digest(hasher.finalize().into()))
    }

    pub(crate) fn expand(
        &self,
        root: &str,
        substitutions: &BTreeMap<String, String>,
    ) -> Result<ExpandedShaderSource, ShaderError> {
        let mut included = BTreeSet::new();
        expand_source(self, root, substitutions, &mut Vec::new(), &mut included, 0)
    }
}

pub fn expand_template(
    source: &str,
    includes: &BTreeMap<String, String>,
    substitutions: &BTreeMap<String, String>,
) -> Result<String, ShaderError> {
    let mut catalog = SourceCatalog {
        sources: includes.clone(),
    };
    catalog
        .sources
        .insert("shaders/__root__.wgsl".to_owned(), source.to_owned());
    Ok(catalog.expand("shaders/__root__.wgsl", substitutions)?.text)
}

pub fn validate_source_alias(alias: &str) -> Result<(), ShaderError> {
    if alias.is_empty()
        || alias.starts_with('/')
        || alias.contains('\\')
        || alias
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        || !alias.starts_with("shaders/")
    {
        return Err(ShaderError::InvalidSourceAlias(alias.to_owned()));
    }
    Ok(())
}

fn expand_source(
    catalog: &SourceCatalog,
    alias: &str,
    substitutions: &BTreeMap<String, String>,
    stack: &mut Vec<String>,
    included: &mut BTreeSet<String>,
    depth: usize,
) -> Result<ExpandedShaderSource, ShaderError> {
    validate_source_alias(alias)?;
    if depth > MAX_INCLUDE_DEPTH {
        return Err(ShaderError::IncludeDepth);
    }
    if stack.iter().any(|item| item == alias) {
        return Err(ShaderError::IncludeCycle(stack.join(" -> ")));
    }
    let source = catalog
        .get(alias)
        .ok_or_else(|| ShaderError::SourceNotFound(alias.to_owned()))?;
    if source.len() > MAX_SOURCE_BYTES {
        return Err(ShaderError::ExpansionTooLarge);
    }
    if source.contains("#include")
        || source.contains("include_str!")
        || source.contains("include_bytes!")
        || source.contains("http://")
        || source.contains("https://")
    {
        return Err(ShaderError::ForbiddenSourceConstruct(alias.to_owned()));
    }
    stack.push(alias.to_owned());
    let mut output = String::new();
    let mut line_aliases = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(include) = trimmed.strip_prefix("// rusttable:include ") {
            let include = include.trim();
            if !include.starts_with("shaders/includes/")
                || include.contains('\\')
                || include.contains("..")
                || include.split('/').any(str::is_empty)
            {
                return Err(ShaderError::IncludeTraversal(include.to_owned()));
            }
            if stack.iter().any(|item| item == include) {
                return Err(ShaderError::IncludeCycle(stack.join(" -> ")));
            }
            if included.len() >= MAX_INCLUDE_COUNT {
                return Err(ShaderError::IncludeCount);
            }
            if !included.insert(include.to_owned()) {
                return Err(ShaderError::IncludeCount);
            }
            let nested =
                expand_source(catalog, include, substitutions, stack, included, depth + 1)?;
            output.push_str(&nested.text);
            line_aliases.extend(nested.line_aliases);
        } else {
            let substituted = substitute_line(line, substitutions)?;
            output.push_str(&substituted);
            output.push('\n');
            let line = u32::try_from(line_aliases.len() + 1).unwrap_or(u32::MAX);
            line_aliases.push(SourceSpanAlias {
                source_alias: alias.to_owned(),
                line,
                column: 1,
            });
        }
        if output.len() > MAX_EXPANDED_BYTES {
            return Err(ShaderError::ExpansionTooLarge);
        }
    }
    let _ = stack.pop();
    Ok(ExpandedShaderSource {
        text: output,
        line_aliases,
    })
}

fn substitute_line(
    line: &str,
    substitutions: &BTreeMap<String, String>,
) -> Result<String, ShaderError> {
    let mut output = String::with_capacity(line.len());
    let mut remainder = line;
    while let Some(start) = remainder.find("${") {
        output.push_str(&remainder[..start]);
        let after_start = &remainder[start + 2..];
        let end = after_start
            .find('}')
            .ok_or_else(|| ShaderError::InvalidSubstitution(remainder[start..].to_owned()))?;
        let key = &after_start[..end];
        if key.is_empty()
            || !key
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            return Err(ShaderError::InvalidSubstitution(key.to_owned()));
        }
        let value = substitutions
            .get(key)
            .ok_or_else(|| ShaderError::UnknownSubstitution(key.to_owned()))?;
        if value.is_empty()
            || !value
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || ".+-_()".contains(character))
        {
            return Err(ShaderError::InvalidSubstitution(key.to_owned()));
        }
        output.push_str(value);
        remainder = &after_start[end + 1..];
    }
    output.push_str(remainder);
    Ok(output)
}

fn hex_digest(bytes: [u8; 32]) -> String {
    let mut output = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expansion_is_bounded_and_deterministic() {
        let includes = BTreeMap::from([(
            "shaders/includes/common.wgsl".to_owned(),
            "const WORKGROUP: u32 = ${WORKGROUP};\n".to_owned(),
        )]);
        let substitutions = BTreeMap::from([("WORKGROUP".to_owned(), "256u".to_owned())]);
        let source = "// rusttable:include shaders/includes/common.wgsl\n@compute @workgroup_size(${WORKGROUP}) fn main() {}";
        let first = expand_template(source, &includes, &substitutions).expect("expansion");
        let second = expand_template(source, &includes, &substitutions).expect("expansion");
        assert_eq!(first, second);
        assert!(first.contains("256u"));
    }

    #[test]
    fn traversal_cycle_and_unknown_substitution_are_rejected() {
        let includes = BTreeMap::from([
            (
                "shaders/includes/a.wgsl".to_owned(),
                "// rusttable:include shaders/includes/b.wgsl\n".to_owned(),
            ),
            (
                "shaders/includes/b.wgsl".to_owned(),
                "// rusttable:include shaders/includes/a.wgsl\n".to_owned(),
            ),
        ]);
        let cycle = expand_template(
            "// rusttable:include shaders/includes/a.wgsl\n",
            &includes,
            &BTreeMap::new(),
        );
        assert!(matches!(cycle, Err(ShaderError::IncludeCycle(_))));
        let unknown = expand_template("${MISSING}", &BTreeMap::new(), &BTreeMap::new());
        assert!(matches!(unknown, Err(ShaderError::UnknownSubstitution(_))));
        assert!(matches!(
            validate_source_alias("shaders/../secret.wgsl"),
            Err(ShaderError::InvalidSourceAlias(_))
        ));
    }
}
