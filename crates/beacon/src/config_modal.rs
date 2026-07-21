//! The input configuration modal's logic, kept free of the emulator.
//!
//! Choosing an action, reading its current binding, assigning a key, clearing
//! it: none of that needs the emulator, audio, or speech, so it lives here as a
//! pure unit over the [`Keymap`]. That keeps it testable without a running
//! console — which is the whole reason the modal was hard to exercise before —
//! and lets both the interactive session and the MCP server drive the same code.
//!
//! Every method returns the sentence to speak rather than speaking it, so the
//! caller decides how it is voiced and a test can simply assert on the words.

use beacon_config::Keymap;

use crate::action::Bindable;
use crate::input;

/// An open configuration: the list of bindable actions and the cursor into it.
pub struct ConfigModal {
    actions: Vec<Bindable>,
    index: usize,
}

/// The result of trying to bind an input to the selected action.
#[derive(Debug, PartialEq, Eq)]
pub enum Bound {
    /// The input was assigned; carries the sentence to speak.
    Ok(String),
    /// The input drives the game and was refused; carries the explanation.
    Refused(String),
}

impl ConfigModal {
    pub fn new(actions: Vec<Bindable>) -> Self {
        ConfigModal { actions, index: 0 }
    }

    /// The currently selected action.
    pub fn current(&self) -> &Bindable {
        &self.actions[self.index]
    }

    /// Moves the cursor, wrapping, and returns the new selection's announcement.
    pub fn navigate(&mut self, delta: i32, keymap: &Keymap) -> String {
        let n = self.actions.len() as i32;
        self.index = (((self.index as i32 + delta) % n + n) % n) as usize;
        self.announce(keymap)
    }

    /// The selected action and its current binding, as spoken when landing on it.
    pub fn announce(&self, keymap: &Keymap) -> String {
        let item = self.current();
        let keys = keymap.keys_for(&item.id);
        let bound = if keys.is_empty() {
            "unbound".to_string()
        } else {
            keys.iter()
                .map(|k| input::key_label(k))
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!("{}. {}.", item.label, bound)
    }

    /// Binds an input name to the selected action, unless it is a game control.
    ///
    /// A game key or button is refused, preserving the invariant that action
    /// inputs and game inputs never overlap.
    pub fn bind(&self, name: &str, keymap: &mut Keymap) -> Bound {
        if input::is_game_input_name(name) {
            return Bound::Refused(
                "That input controls the game and can't be reassigned.".to_string(),
            );
        }
        let item = self.current();
        keymap.bind(name, &item.id);
        Bound::Ok(format!(
            "{} bound to {}.",
            input::key_label(name),
            item.label
        ))
    }

    /// Clears every key bound to the selected action, and returns the sentence
    /// to speak.
    pub fn clear(&self, keymap: &mut Keymap) -> String {
        let item = self.current();
        for key in keymap.keys_for(&item.id) {
            keymap.unbind(&key);
        }
        format!("{} unbound.", item.label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actions() -> Vec<Bindable> {
        vec![
            Bindable {
                id: "save_state".into(),
                label: "Save state".into(),
            },
            Bindable {
                id: "command:scan".into(),
                label: "Scan".into(),
            },
        ]
    }

    #[test]
    fn navigation_wraps_and_announces_current_binding() {
        let keymap = Keymap::default(); // scan is bound to KeyC by default.
        let mut modal = ConfigModal::new(actions());

        // Starts on the first action.
        assert!(modal.announce(&keymap).starts_with("Save state."));

        // Down to scan, which the default keymap binds to C and, on a pad, the
        // right stick button. Both are announced, in key-name order.
        let said = modal.navigate(1, &keymap);
        assert_eq!(said, "Scan. C, right stick button.");

        // Wraps back to the top.
        let said = modal.navigate(1, &keymap);
        assert!(said.starts_with("Save state."));
    }

    #[test]
    fn binding_a_free_key_updates_the_keymap() {
        let mut keymap = Keymap::default();
        let modal = ConfigModal::new(actions()); // selected: save_state

        let result = modal.bind("KeyD", &mut keymap);
        assert_eq!(result, Bound::Ok("D bound to Save state.".to_string()));
        assert_eq!(keymap.action_for("KeyD"), Some("save_state"));
    }

    #[test]
    fn binding_a_game_key_is_refused_and_changes_nothing() {
        let mut keymap = Keymap::default();
        let modal = ConfigModal::new(actions());

        // KeyX is the SNES A button; the arrow keys and enter are game keys too.
        let result = modal.bind("KeyX", &mut keymap);
        assert!(matches!(result, Bound::Refused(_)));
        assert_eq!(keymap.action_for("KeyX"), None);
    }

    #[test]
    fn game_pad_buttons_are_refused_too() {
        let mut keymap = Keymap::default();
        let modal = ConfigModal::new(actions());
        assert!(matches!(
            modal.bind("Pad:South", &mut keymap),
            Bound::Refused(_)
        ));
        // A free pad button binds fine.
        assert!(matches!(modal.bind("Pad:C", &mut keymap), Bound::Ok(_)));
        assert_eq!(keymap.action_for("Pad:C"), Some("save_state"));
    }

    #[test]
    fn clearing_removes_all_keys_for_the_action() {
        let mut keymap = Keymap::default();
        keymap.bind("KeyD", "save_state");
        keymap.bind("KeyT", "save_state"); // default already binds T to save_state
        let modal = ConfigModal::new(actions());

        let said = modal.clear(&mut keymap);
        assert_eq!(said, "Save state unbound.");
        assert!(keymap.keys_for("save_state").is_empty());
    }
}
