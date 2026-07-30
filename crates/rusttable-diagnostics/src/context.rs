use crate::privacy::{Alias, Redactor};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CorrelationContext {
    pub(crate) request: Option<Alias>,
    pub(crate) photo: Option<Alias>,
    pub(crate) edit: Option<Alias>,
    pub(crate) task: Option<Alias>,
    pub(crate) device: Option<Alias>,
}

impl CorrelationContext {
    #[must_use]
    pub fn request(self, redactor: &Redactor, value: &str) -> Self {
        Self {
            request: Some(redactor.alias(value)),
            ..self
        }
    }

    #[must_use]
    pub fn photo(self, redactor: &Redactor, value: &str) -> Self {
        Self {
            photo: Some(redactor.alias(value)),
            ..self
        }
    }

    #[must_use]
    pub fn edit(self, redactor: &Redactor, value: &str) -> Self {
        Self {
            edit: Some(redactor.alias(value)),
            ..self
        }
    }

    #[must_use]
    pub fn task(self, redactor: &Redactor, value: &str) -> Self {
        Self {
            task: Some(redactor.alias(value)),
            ..self
        }
    }

    #[must_use]
    pub fn device(self, redactor: &Redactor, value: &str) -> Self {
        Self {
            device: Some(redactor.alias(value)),
            ..self
        }
    }
}
