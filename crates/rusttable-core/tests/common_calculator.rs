//! Source-derived contracts for Darktable's `src/common/calculator.c` and
//! `src/common/calculator.h`.

use rusttable_core::common::calculator::solve;

fn assert_close(actual: f64, expected: f64) {
    let tolerance = f64::EPSILON * expected.abs().max(1.0) * 8.0;
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn missing_empty_and_tokenless_formulas_return_nan() {
    assert!(solve(3.0, None).is_nan());
    assert!(solve(3.0, Some("")).is_nan());
    assert!(solve(3.0, Some(" \t\n")).is_nan());
    assert!(solve(3.0, Some("TRUE")).is_nan());
    assert!(solve(3.0, Some(".")).is_nan());
}

#[test]
fn scanner_accepts_c_locale_numbers_comma_decimals_and_the_x_variable() {
    assert_close(solve(7.5, Some("x")), 7.5);
    assert_close(solve(7.5, Some("X + 2")), 9.5);
    assert_close(solve(0.0, Some("1,5 + 2.25")), 3.75);
    assert_close(solve(0.0, Some("1.25e2 + 5E-1")), 125.5);
    assert_close(solve(0.0, Some("garbage 2 qwerty + 3")), 5.0);
    assert_close(solve(0.0, Some("0x1p2 + 0X1.8p+1")), 7.0);
    assert_close(solve(0.0, Some("0x1p2junk")), 4.0);
    assert_close(solve(0.0, Some("0x1pignored")), 1.0);
    assert_close(solve(0.0, Some("0x1.")), 1.0);
    assert_close(solve(0.0, Some("0x.8P-1")), 0.25);
    assert_eq!(solve(0.0, Some("0x1.2e3")).to_bits(), 0x3ff2_e300_0000_0000);
    assert!(solve(0.0, Some("0x.p2")).is_nan());
    assert_close(solve(0.0, Some("2\0+3")), 2.0);
}

#[test]
fn hexadecimal_numbers_round_once_across_the_full_binary64_range() {
    let cases = [
        ("0x1.00000000000008p0", 0x3ff0_0000_0000_0000),
        ("0x1.0000000000000800000000000001p0", 0x3ff0_0000_0000_0001),
        ("0x1.00000000000018p0", 0x3ff0_0000_0000_0002),
        ("0x1p-1074", 0x0000_0000_0000_0001),
        ("0x1p-1075", 0x0000_0000_0000_0000),
        (
            "0x1.0000000000000000000000000001p-1075",
            0x0000_0000_0000_0001,
        ),
        ("0x0.fffffffffffffp-1022", 0x000f_ffff_ffff_ffff),
        ("0x0.fffffffffffff8p-1022", 0x0010_0000_0000_0000),
        ("0x1.fffffffffffffp1023", 0x7fef_ffff_ffff_ffff),
        ("0x1.fffffffffffff8p1023", 0x7ff0_0000_0000_0000),
        ("0x1.fffffffffffff7ffffffffffffp1023", 0x7fef_ffff_ffff_ffff),
    ];
    for (formula, expected_bits) in cases {
        assert_eq!(
            solve(0.0, Some(formula)).to_bits(),
            expected_bits,
            "{formula}"
        );
    }

    let compensated_integer = format!("0x1{}p-4400", "0".repeat(1_100));
    assert_eq!(
        solve(0.0, Some(&compensated_integer)).to_bits(),
        1.0_f64.to_bits()
    );
    let compensated_fraction = format!("0x0.{}1p+4404", "0".repeat(1_100));
    assert_eq!(
        solve(0.0, Some(&compensated_fraction)).to_bits(),
        1.0_f64.to_bits()
    );
}

#[test]
fn parser_preserves_darktable_precedence_and_associativity() {
    assert_close(solve(0.0, Some("2 + 3 * 4")), 14.0);
    assert_close(solve(0.0, Some("(2 + 3) * 4")), 20.0);
    assert_close(solve(0.0, Some("2^3^2")), 64.0);
    assert_close(solve(0.0, Some("-2^2")), 4.0);
    assert_close(solve(0.0, Some("2^-2")), 0.25);
    assert_close(solve(0.0, Some("+ + 2")), 2.0);
}

#[test]
fn leading_increment_and_decrement_ignore_the_remainder() {
    assert_close(solve(4.0, Some("++")), 5.0);
    assert_close(solve(4.0, Some("++ ignored 999")), 5.0);
    assert_close(solve(4.0, Some("--")), 3.0);
    assert_close(solve(4.0, Some("-- ignored 999")), 3.0);
    assert!(solve(4.0, Some("1++")).is_nan());
}

#[test]
fn multiplicative_operators_match_c_floating_point_behavior() {
    assert_close(solve(0.0, Some("8%3")), 2.0);
    assert_close(solve(0.0, Some("2:8")), 4.0);
    assert_close(solve(0.0, Some("8:2")), 4.0);
    assert_close(solve(0.0, Some("2:-4")), -0.5);
    assert!(solve(0.0, Some("5/0")).is_infinite());
    assert!(solve(0.0, Some("0/0")).is_nan());
    assert_close(solve(0.0, Some("(0/0):2")), 1.0);
    assert!(solve(0.0, Some("2:(0/0)")).is_nan());
}

#[test]
fn malformed_or_adjacent_expressions_are_rejected() {
    for formula in [
        "2x", "2 3", "(2+3", "2+3)", "1..2", "x(", "()", ".:2", ".^0", "1^.", ",:2",
    ] {
        assert!(
            solve(2.0, Some(formula)).is_nan(),
            "{formula:?} must be rejected"
        );
    }
}

#[test]
fn overflow_underflow_and_signed_zero_remain_ieee_values() {
    assert!(solve(0.0, Some("1e9999")).is_infinite());
    assert_eq!(solve(0.0, Some("1e-9999")).to_bits(), 0.0_f64.to_bits());
    assert_eq!(solve(0.0, Some("-0")).to_bits(), (-0.0_f64).to_bits());
}
