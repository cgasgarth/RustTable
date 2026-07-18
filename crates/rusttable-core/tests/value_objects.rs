use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use rusttable_core::{FiniteF64, FiniteF64Error, Revision, RevisionOverflow};

fn hash<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn revision_starts_at_zero_and_increments_checked() {
    assert_eq!(Revision::ZERO.get(), 0);
    assert_eq!(
        Revision::ZERO
            .checked_increment()
            .expect("no overflow")
            .get(),
        1
    );
    assert_eq!(
        Revision::from_u64(u64::MAX).checked_increment(),
        Err(RevisionOverflow)
    );
}

#[test]
fn finite_float_rejects_non_finite_values() {
    assert_eq!(FiniteF64::new(f64::NAN), Err(FiniteF64Error));
    assert_eq!(FiniteF64::new(f64::INFINITY), Err(FiniteF64Error));
    assert_eq!(FiniteF64::new(f64::NEG_INFINITY), Err(FiniteF64Error));
}

#[test]
fn finite_float_normalizes_zero_and_hashes_equal_values_equally() {
    let positive = FiniteF64::new(0.0).expect("zero is finite");
    let negative = FiniteF64::new(-0.0).expect("negative zero is finite");

    assert_eq!(positive, negative);
    assert_eq!(positive.get().to_bits(), 0.0_f64.to_bits());
    assert_eq!(hash(&positive), hash(&negative));
}

#[test]
fn finite_float_has_total_ordering() {
    let low = FiniteF64::new(-1.0).expect("finite");
    let middle = FiniteF64::new(0.0).expect("finite");
    let high = FiniteF64::new(1.0).expect("finite");

    assert!(low < middle);
    assert!(middle < high);
    assert_eq!(low.cmp(&low), std::cmp::Ordering::Equal);
}
