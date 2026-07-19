use std::fmt::Write;

use sha2::{Digest, Sha256};

use crate::event::{DiagnosticField, FieldValue, PrivacyClass};

pub(crate) struct Redactor {
    key: [u8; 32],
}

impl Redactor {
    pub(crate) fn new() -> Self {
        let mut key = [0_u8; 32];
        if getrandom::fill(&mut key).is_err() {
            let seed = format!(
                "{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |value| value.as_nanos()),
                std::thread::current().name().unwrap_or("main")
            );
            key.copy_from_slice(&Sha256::digest(seed.as_bytes()));
        }
        Self { key }
    }

    pub(crate) fn render(&self, field: &DiagnosticField) -> Option<RenderedField> {
        match field.privacy() {
            PrivacyClass::Secret | PrivacyClass::Payload => None,
            PrivacyClass::Public | PrivacyClass::Operational => Some(RenderedField {
                name: field.name().to_owned(),
                value: value_string(field.value()),
                private: false,
            }),
            PrivacyClass::Private => Some(RenderedField {
                name: field.name().to_owned(),
                value: self.alias(field.value()),
                private: true,
            }),
        }
    }

    fn alias(&self, value: &FieldValue) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable-private-alias-v1\0");
        hasher.update(self.key);
        hasher.update(value_string(value).as_bytes());
        let digest = hasher.finalize();
        let mut alias = String::with_capacity(17);
        alias.push('@');
        for byte in digest.iter().take(8) {
            write!(alias, "{byte:02x}").expect("String cannot fail");
        }
        alias
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenderedField {
    pub(crate) name: String,
    pub(crate) value: String,
    pub(crate) private: bool,
}

pub(crate) fn value_string(value: &FieldValue) -> String {
    match value {
        FieldValue::Text(value) => value.clone(),
        FieldValue::Unsigned(value) => value.to_string(),
        FieldValue::Signed(value) => value.to_string(),
        FieldValue::Float(value) => value.to_string(),
        FieldValue::Boolean(value) => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::Redactor;
    use crate::{DiagnosticField, PrivacyClass};

    #[test]
    fn private_values_correlate_but_secret_and_payload_never_render() {
        let redactor = Redactor::new();
        let first =
            DiagnosticField::text("path", "/private/a", PrivacyClass::Private).expect("field");
        let second =
            DiagnosticField::text("path", "/private/a", PrivacyClass::Private).expect("field");
        assert_eq!(redactor.render(&first), redactor.render(&second));
        let secret =
            DiagnosticField::text("token", "sensitive", PrivacyClass::Secret).expect("field");
        let payload =
            DiagnosticField::text("pixels", "sensitive", PrivacyClass::Payload).expect("field");
        assert!(redactor.render(&secret).is_none());
        assert!(redactor.render(&payload).is_none());
    }
}
