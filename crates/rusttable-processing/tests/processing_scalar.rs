use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use rusttable_core::FiniteF64;
use rusttable_processing::{FiniteF32, FiniteF32Error, ScalarNarrowingError};

#[test]
fn rejects_non_finite_values() {
    assert_eq!(FiniteF32::new(f32::NAN), Err(FiniteF32Error));
    assert_eq!(FiniteF32::new(f32::INFINITY), Err(FiniteF32Error));
    assert_eq!(FiniteF32::new(f32::NEG_INFINITY), Err(FiniteF32Error));
}

#[test]
fn normalizes_negative_zero() {
    let value = FiniteF32::new(-0.0).expect("negative zero is finite");

    assert_eq!(value.get().to_bits(), 0.0f32.to_bits());
    assert_eq!(value.get().to_bits(), 0.0f32.to_bits());
}

#[test]
fn orders_and_hashes_equal_values_consistently() {
    let zero = FiniteF32::new(0.0).expect("zero is finite");
    let negative_zero = FiniteF32::new(-0.0).expect("negative zero is finite");
    let low = FiniteF32::new(-1.0).expect("finite");
    let high = FiniteF32::new(1.0).expect("finite");

    assert_eq!(zero, negative_zero);
    assert!(low < zero);
    assert!(zero < high);
    assert_eq!(hash(zero), hash(negative_zero));
}

#[test]
fn narrows_exact_values() {
    let source = FiniteF64::new(1.5).expect("finite");

    assert_eq!(
        FiniteF32::try_from(source)
            .expect("representable")
            .get()
            .to_bits(),
        1.5f32.to_bits()
    );
}

#[test]
fn normalizes_narrowed_zero() {
    let source = FiniteF64::new(-0.0).expect("finite");

    assert_eq!(
        FiniteF32::try_from(source)
            .expect("exact zero is representable")
            .get()
            .to_bits(),
        0.0f32.to_bits()
    );
}

#[test]
fn rejects_narrowing_overflow() {
    let source = FiniteF64::new(f64::MAX).expect("finite");

    assert_eq!(
        FiniteF32::try_from(source),
        Err(ScalarNarrowingError::Overflow)
    );
}

#[test]
fn rejects_narrowing_underflow() {
    let source = FiniteF64::new(f64::MIN_POSITIVE * 0.5).expect("finite");

    assert_eq!(
        FiniteF32::try_from(source),
        Err(ScalarNarrowingError::Underflow)
    );
}

fn hash(value: FiniteF32) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}
