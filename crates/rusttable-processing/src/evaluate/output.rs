use crate::{TerminalOutputFrame, WorkingRgbImage};

/// Result of evaluating a graph. Terminal colorout values are not working
/// pixels and therefore cannot be represented by [`WorkingRgbImage`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvaluationOutput {
    Working(WorkingRgbImage),
    Terminal(TerminalOutputFrame),
}

impl EvaluationOutput {
    #[must_use]
    pub const fn terminal(&self) -> Option<&TerminalOutputFrame> {
        match self {
            Self::Working(_) => None,
            Self::Terminal(output) => Some(output),
        }
    }
}
