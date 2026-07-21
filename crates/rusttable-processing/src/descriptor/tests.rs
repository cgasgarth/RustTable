use super::*;

#[test]
fn representative_descriptors_are_valid_and_canonical() {
    let exposure = exposure_descriptor();
    let gain = rgb_gain_descriptor();
    exposure.validate().expect("exposure schema");
    gain.validate().expect("gain schema");
    assert_eq!(exposure.canonical_hash(), exposure.canonical_hash());
    assert_eq!(diff_schema(&exposure, &exposure), SchemaDiff::Identical);
}

#[test]
fn invalid_default_and_duplicate_parameter_are_rejected() {
    let mut descriptor = exposure_descriptor();
    descriptor.parameters.push(descriptor.parameters[0].clone());
    assert!(matches!(
        descriptor.validate(),
        Err(DescriptorError::DuplicateParameter(_))
    ));
    descriptor.parameters.truncate(1);
    descriptor.parameters[0].default = ParameterDefault::Bool(true);
    assert!(matches!(
        descriptor.validate(),
        Err(DescriptorError::InvalidDefault(_))
    ));
}
