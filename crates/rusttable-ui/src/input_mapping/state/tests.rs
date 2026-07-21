use super::*;
use crate::input_mapping::{KeyModifier, default_snapshot};

#[test]
fn search_uses_labels_categories_and_stable_ids() {
    let mut state = EditorState::new(default_snapshot());
    state.search = "darkroom.exposure".to_owned();
    assert_eq!(state.visible_actions().len(), 1);
    state.search = "file".to_owned();
    assert_eq!(state.visible_actions().len(), 2);
    state.view = EditorView::Devices;
    state.search = "midi".to_owned();
    assert_eq!(state.visible_devices().len(), 1);
}

#[test]
fn exact_conflicts_block_but_context_shadowing_is_explained() {
    let mut state = EditorState::new(default_snapshot());
    let action_id = ActionId::from("image.import");
    state
        .add_keyboard_sequence(&action_id, vec![KeyChord::new("I", [])])
        .unwrap();
    state
        .add_keyboard_sequence(
            &ActionId::from("image.export"),
            vec![KeyChord::new("I", [])],
        )
        .unwrap();
    assert!(state.conflicts().iter().any(MappingConflict::blocks_apply));
    state.draft.bindings.last_mut().unwrap().context = ActionContext::Darkroom;
    assert!(
        state
            .conflicts()
            .iter()
            .any(|conflict| matches!(conflict.kind, ConflictKind::Shadowed { .. }))
    );
}

#[test]
fn builtins_disable_and_user_bindings_remove_on_delete() {
    let mut state = EditorState::new(default_snapshot());
    state
        .update(EditorMessage::RemoveBinding("default-undo".to_owned()))
        .unwrap();
    assert!(
        !state
            .draft
            .bindings
            .iter()
            .find(|binding| binding.id == "default-undo")
            .unwrap()
            .enabled
    );
    state
        .add_keyboard_sequence(&ActionId::from("edit.undo"), vec![KeyChord::new("U", [])])
        .unwrap();
    let user_id = state.draft.bindings.last().unwrap().id.clone();
    state.update(EditorMessage::RemoveBinding(user_id)).unwrap();
    assert!(
        !state
            .draft
            .bindings
            .iter()
            .any(|binding| binding.id.starts_with("user-edit.undo"))
    );
}

#[test]
fn reset_restores_immutable_defaults_after_an_apply() {
    let mut state = EditorState::new(default_snapshot());
    state
        .add_keyboard_sequence(&ActionId::from("edit.undo"), vec![KeyChord::new("U", [])])
        .unwrap();
    state
        .update(EditorMessage::Apply { live_generation: 1 })
        .unwrap();
    state.update(EditorMessage::Reset(ResetScope::All)).unwrap();
    assert!(
        !state
            .draft
            .bindings
            .iter()
            .any(|binding| binding.id.starts_with("user-edit.undo"))
    );
    assert!(state.draft.bindings.iter().all(|binding| binding.built_in));
}

#[test]
fn nonremovable_actions_keep_their_last_fallback() {
    let mut snapshot = default_snapshot();
    snapshot.actions[0].nonremovable = true;
    let mut state = EditorState::new(snapshot);
    assert_eq!(
        state.update(EditorMessage::RemoveBinding(
            "default-view-toggle".to_owned(),
        )),
        Err(EditorError::NonRemovableFallback)
    );
}

#[test]
fn learn_timeout_and_capture_never_execute_an_action() {
    let mut state = EditorState::new(default_snapshot());
    state
        .update(EditorMessage::BeginLearn(LearnTarget::Keyboard))
        .unwrap();
    state
        .update(EditorMessage::CaptureKeyboard(KeyChord::new(
            "K",
            [KeyModifier::Control],
        )))
        .unwrap();
    assert_eq!(state.status, EditorStatus::LearnCaptured);
    assert!(state.learn.is_none());
    state
        .update(EditorMessage::BeginLearn(LearnTarget::Keyboard))
        .unwrap();
    for _ in 0..LEARN_TIMEOUT_TICKS {
        state.update(EditorMessage::LearnTick).unwrap();
    }
    assert_eq!(state.status, EditorStatus::LearnTimedOut);
    assert!(state.test_preview.is_none());
}

#[test]
fn profile_round_trip_is_canonical_and_preserves_unknown_records() {
    let state = EditorState::new(default_snapshot());
    let profile = state.export_profile("Studio");
    let json = profile.canonical_json().unwrap();
    assert_eq!(json, profile.canonical_json().unwrap());
    let parsed = MappingProfile::parse_json(&json).unwrap();
    assert_eq!(parsed, profile);
    let unknown = Binding::user(
        "unknown",
        ActionId::from("missing.action"),
        "missing-device",
        ActionContext::Global,
        BindingSource::Keyboard {
            sequence: vec![KeyChord::new("M", [])],
        },
    );
    let mut imported = parsed;
    imported.mappings.push(unknown);
    let mut state = EditorState::new(default_snapshot());
    state.import_profile(imported);
    assert_eq!(state.inactive_imports.len(), 1);
    assert!(matches!(
        state.status,
        EditorStatus::Imported { unknown: 1, .. }
    ));
}

#[test]
fn apply_is_generation_safe_and_invalid_ranges_do_not_commit() {
    let mut state = EditorState::new(default_snapshot());
    state
        .add_keyboard_sequence(&ActionId::from("edit.undo"), vec![KeyChord::new("U", [])])
        .unwrap();
    let result = state
        .update(EditorMessage::Apply { live_generation: 1 })
        .unwrap()
        .unwrap();
    assert_eq!(result.generation, 2);
    assert!(
        state
            .update(EditorMessage::Apply { live_generation: 1 })
            .is_err()
    );
    let mut state = EditorState::new(default_snapshot());
    state.draft.bindings[2]
        .continuous
        .as_mut()
        .unwrap()
        .input_min = 2.0;
    assert!(matches!(
        state.update(EditorMessage::Apply { live_generation: 1 }),
        Err(EditorError::InvalidContinuous(_))
    ));
    assert_eq!(state.baseline.generation, 1);

    let mut state = EditorState::new(default_snapshot());
    state.draft.bindings[2].continuous.as_mut().unwrap().curve = Curve::Exponential {
        exponent_hundredths: 50,
    };
    assert!(matches!(
        state.update(EditorMessage::Apply { live_generation: 1 }),
        Err(EditorError::InvalidContinuous(_))
    ));
}

#[test]
fn key_chords_are_canonical_and_sequences_are_bounded() {
    let chord = KeyChord::new(
        "Z",
        [KeyModifier::Alt, KeyModifier::Control, KeyModifier::Alt],
    );
    assert_eq!(chord.display(), "Ctrl+Alt+Z");
    let mut state = EditorState::new(default_snapshot());
    let sequence = vec![chord; MAX_SEQUENCE_LENGTH + 1];
    assert_eq!(
        state.add_keyboard_sequence(&ActionId::from("edit.undo"), sequence),
        Err(EditorError::SequenceTooLong)
    );
}
