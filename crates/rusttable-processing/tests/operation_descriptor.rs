use rusttable_processing::descriptor::{
    DescriptorError, OperationDescriptor, ParameterDefault, SchemaDiff, diff_schema,
    exposure_descriptor, rgb_gain_descriptor,
};

#[test]
fn operation_descriptor_representative_descriptors_are_valid_and_have_stable_identity() {
    let exposure = exposure_descriptor();
    let gain = rgb_gain_descriptor();

    exposure.validate().expect("exposure descriptor");
    gain.validate().expect("RGB gain descriptor");
    assert_ne!(
        exposure.canonical_hash().expect("exposure hash"),
        gain.canonical_hash().expect("gain hash")
    );
}

#[test]
fn operation_descriptor_parameter_construction_order_does_not_change_canonical_encoding() {
    let mut first = rgb_gain_descriptor();
    let mut second = first.clone();
    second.parameters.reverse();

    assert_eq!(
        first.canonical_bytes().expect("first encoding"),
        second.canonical_bytes().expect("second encoding")
    );
    assert_eq!(diff_schema(&first, &second), SchemaDiff::CompatibleAddition);

    first.parameters[0].default = ParameterDefault::Scalar(2.0);
    assert_eq!(diff_schema(&second, &first), SchemaDiff::DefaultChanged);
}

#[test]
fn operation_descriptor_invalid_schema_values_are_rejected_before_encoding() {
    let mut descriptor = exposure_descriptor();
    descriptor.parameters[0].default = ParameterDefault::Bool(true);
    assert!(matches!(
        descriptor.canonical_bytes(),
        Err(DescriptorError::InvalidDefault(_))
    ));
}

#[test]
fn operation_descriptor_is_plain_canonical_data() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<OperationDescriptor>();
}
