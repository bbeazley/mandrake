use super::*;

#[test]
fn tfls_key_sets_give_escape_to_the_quit_dialog() {
    use crate::controls::MythKey;
    for preset in crate::controls::TFL_PRESETS {
        let controls = Controls::preset(preset).expect("the preset");
        let actions = controls.actions_for_press(MythKey(0), &HeldKeys::default());
        assert!(
            actions.contains(&ControlAction::PauseGame),
            "{preset}: {actions:?}"
        );
    }
}
