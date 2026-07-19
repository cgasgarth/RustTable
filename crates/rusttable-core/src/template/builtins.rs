use std::fmt;

use super::{Template, TemplateLimits};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinTemplate {
    SourceStem,
    SourceStemVirtualCopy,
    DatedFolders,
    RecipeSuffix,
    SequenceName,
}

impl BuiltinTemplate {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::SourceStem => "source-stem",
            Self::SourceStemVirtualCopy => "source-stem-virtual-copy",
            Self::DatedFolders => "dated-folders",
            Self::RecipeSuffix => "recipe-suffix",
            Self::SequenceName => "sequence-name",
        }
    }

    /// Returns the immutable AST for this built-in template.
    ///
    /// # Errors
    ///
    /// Returns an error only if a built-in source string violates the parser contract.
    pub fn template(self) -> Result<Template, BuiltinTemplateError> {
        let source = match self {
            Self::SourceStem => "${source_stem}",
            Self::SourceStemVirtualCopy => "${source_stem|image}-${virtual_copy:04}",
            Self::DatedFolders => {
                "${capture_year:04}/${capture_month:02}/${capture_day:02}/${source_stem}"
            }
            Self::RecipeSuffix => "${source_stem}-${recipe|default}",
            Self::SequenceName => "${sequence:04}-${source_stem}",
        };
        Template::parse_with_limits(
            source,
            &super::VariableRegistry::standard(),
            TemplateLimits::default(),
        )
        .map_err(BuiltinTemplateError::Parse)
    }

    #[must_use]
    pub fn all() -> [Self; 5] {
        [
            Self::SourceStem,
            Self::SourceStemVirtualCopy,
            Self::DatedFolders,
            Self::RecipeSuffix,
            Self::SequenceName,
        ]
    }

    #[must_use]
    pub fn content_hash(self) -> String {
        self.template()
            .map_or_else(|_| String::new(), |template| template.ast_hash())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuiltinTemplateError {
    Parse(super::ParseError),
}

impl fmt::Display for BuiltinTemplateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "builtin template error: {self:?}")
    }
}

impl std::error::Error for BuiltinTemplateError {}
