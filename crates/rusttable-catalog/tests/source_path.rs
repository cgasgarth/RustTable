use rusttable_catalog::{SourcePath, SourcePathError};

#[test]
fn source_path_is_a_relative_utf8_logical_key() {
    let source = SourcePath::new("Camera/été.raw").expect("valid logical key");

    assert_eq!(source.as_str(), "Camera/été.raw");
    assert_eq!(
        source.components().collect::<Vec<_>>(),
        ["Camera", "été.raw"]
    );
}

#[test]
fn source_path_rejects_ambiguous_or_overlong_keys() {
    for (value, error) in [
        ("", SourcePathError::Empty),
        ("/leading", SourcePathError::LeadingSeparator),
        ("trailing/", SourcePathError::TrailingSeparator),
        ("double//separator", SourcePathError::EmptyComponent),
        (".", SourcePathError::DotComponent),
        ("parent/..", SourcePathError::DotComponent),
        ("has\\backslash", SourcePathError::Backslash),
        ("has\0nul", SourcePathError::Nul),
    ] {
        assert_eq!(SourcePath::new(value), Err(error), "value: {value:?}");
    }
    assert_eq!(
        SourcePath::new(&"x".repeat(256)),
        Err(SourcePathError::ComponentTooLong)
    );
    assert_eq!(
        SourcePath::new(&format!("{}/x", "x".repeat(4_096))),
        Err(SourcePathError::PathTooLong)
    );
}
