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

/// What the dialog is called, in the tree and in speech.
pub const HEADING: &str = "Input configuration";

/// How the dialog works, spoken on opening and published for a reader to find again.
pub const HELP: &str = "Up and down to choose an action, then press a key to bind it. \
                        Delete to clear a binding, escape to finish.";

/// An open configuration: the list of bindable actions and the cursor into it.
pub struct ConfigModal {
    actions: Vec<Bindable>,
    index: usize,
}

/// What the dialog looks like to a screen reader: every action with its binding, and
/// which one the cursor is on.
///
/// The same content the spoken announcements carry, in the form the accessibility tree
/// wants, so a reader's own review keys can walk the whole list — which spoken narration
/// alone cannot offer, since it only ever says the one row the cursor is on.
pub struct View {
    /// What the dialog is called.
    pub heading: &'static str,
    /// How to work it, for a reader that goes looking.
    pub help: &'static str,
    /// One line per action: its label and what it is bound to.
    pub rows: Vec<String>,
    /// Which row the cursor is on.
    pub selected: usize,
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
        self.described(self.index, keymap)
    }

    /// The dialog as a whole, for the accessibility tree.
    pub fn view(&self, keymap: &Keymap) -> View {
        View {
            heading: HEADING,
            help: HELP,
            rows: (0..self.actions.len())
                .map(|i| self.described(i, keymap))
                .collect(),
            selected: self.index,
        }
    }

    /// Moves the cursor to a row a screen reader named, and says what it landed on.
    ///
    /// Out of range does nothing: the reader's idea of the list can lag a rebuild of it.
    pub fn select(&mut self, index: usize, keymap: &Keymap) -> Option<String> {
        if index >= self.actions.len() {
            return None;
        }
        self.index = index;
        Some(self.announce(keymap))
    }

    /// Whether `text` is only what the tree is already showing.
    ///
    /// The mirror of the menu's rule: with narration routed through a screen reader, the
    /// focused row is already spoken from the tree, so repeating Beacon's own wording for it
    /// would read every row twice. A binding confirmed or refused is not in the tree, and is
    /// exactly what still needs saying.
    pub fn duplicates(&self, text: &str, keymap: &Keymap) -> bool {
        text == self.announce(keymap)
    }

    /// One action and its binding.
    fn described(&self, index: usize, keymap: &Keymap) -> String {
        let item = &self.actions[index];
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
    fn the_view_carries_every_action_so_a_reader_can_review_the_whole_list() {
        // The point of publishing the dialog rather than only speaking it. Narration says the
        // one row the cursor is on; a reader with the list can read any of it, in any order.
        let keymap = Keymap::default();
        let mut modal = ConfigModal::new(actions());
        let view = modal.view(&keymap);
        assert_eq!(view.rows.len(), 2);
        assert!(view.rows[0].starts_with("Save state."), "{:?}", view.rows);
        assert_eq!(view.rows[1], "Scan. C, right stick button.");
        assert_eq!(view.selected, 0);
        // The row the cursor is on is the row that is spoken: one source, not two wordings.
        assert_eq!(view.rows[view.selected], modal.announce(&keymap));

        // And moving the cursor moves the selection the tree reports.
        modal.navigate(1, &keymap);
        assert_eq!(modal.view(&keymap).selected, 1);
    }

    #[test]
    fn a_reader_can_move_the_cursor_by_row() {
        // A screen reader drives the list by node, so a row it names has to land somewhere —
        // and a row it names that is no longer there must not.
        let keymap = Keymap::default();
        let mut modal = ConfigModal::new(actions());
        assert_eq!(
            modal.select(1, &keymap).as_deref(),
            Some("Scan. C, right stick button.")
        );
        assert_eq!(modal.view(&keymap).selected, 1);
        assert_eq!(modal.select(9, &keymap), None, "out of range does nothing");
        assert_eq!(modal.view(&keymap).selected, 1);
    }

    #[test]
    fn the_rows_wording_is_recognised_as_already_in_the_tree() {
        // With narration routed through a screen reader, the focused row is spoken from the
        // tree. Beacon saying it too would read every row twice — but a binding confirmed is
        // not in the tree and does still need saying.
        let mut keymap = Keymap::default();
        let modal = ConfigModal::new(actions());
        assert!(modal.duplicates(&modal.announce(&keymap), &keymap));
        let Bound::Ok(said) = modal.bind("KeyD", &mut keymap) else {
            panic!("D is free");
        };
        assert!(!modal.duplicates(&said, &keymap));
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
