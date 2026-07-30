use rusttable_ui::bauhaus::numeric_input::{
    MAX_EXPRESSION_BYTES, MAX_TEXT_BYTES, NumericInputBuffer, resolve_raw_value,
};

#[test]
fn source_slider_character_set_is_exact() {
    let mut input = NumericInputBuffer::new();

    for character in "0123456789.,%+-*Xx/:^~ ()".chars() {
        assert!(input.push(character), "{character:?} must be accepted");
    }
    assert_eq!(input.as_str(), "0123456789.,%+-*Xx/:^~ ()");

    for character in ['a', '=', '_', '\t', '\n', 'é', '∞'] {
        assert!(!input.push(character), "{character:?} must not be accepted");
    }
}

#[test]
fn source_buffer_reserves_terminator_and_guard_byte() {
    assert_eq!(MAX_TEXT_BYTES, 180);
    assert_eq!(MAX_EXPRESSION_BYTES, 178);

    let mut input = NumericInputBuffer::new();
    for _ in 0..MAX_EXPRESSION_BYTES {
        assert!(input.push('1'));
    }
    assert_eq!(input.as_str().len(), MAX_EXPRESSION_BYTES);
    assert!(!input.push('2'));
}

#[test]
fn backspace_and_delete_remove_the_previous_character() {
    let mut input = NumericInputBuffer::new();
    assert!(input.push('1'));
    assert!(input.push('+'));
    assert!(input.push('2'));

    assert!(input.erase_last());
    assert_eq!(input.as_str(), "1+");
    assert!(input.erase_last());
    assert_eq!(input.as_str(), "1");
    assert!(input.erase_last());
    assert!(!input.erase_last());
}

#[test]
fn commit_evaluates_the_display_value_and_converts_back_to_raw() {
    assert_eq!(resolve_raw_value(2.0, 100.0, 5.0, "x + 5"), Some(2.05));
    assert_eq!(resolve_raw_value(2.0, -10.0, 5.0, "++"), Some(1.9));
}

#[test]
fn empty_invalid_and_non_finite_input_do_not_replace_the_value() {
    for expression in ["", " ", "garbage", "1/0", "0/0"] {
        assert_eq!(
            resolve_raw_value(3.0, 1.0, 0.0, expression),
            None,
            "{expression:?}"
        );
    }
}
