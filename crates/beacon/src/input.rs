//! Keyboard and gamepad.
//!
//! Two responsibilities, kept apart on purpose:
//!
//! - **Game input** — the SNES buttons. A fixed keyboard layout plus the
//!   gamepad, both always live, so a player can drive the game with a pad and
//!   Beacon's own actions from either device.
//! - **Naming** — translating a physical key or gamepad button to and from the
//!   stable string the keymap stores, and to a form worth speaking. Action
//!   *resolution* lives in the app, which owns the keymap; this module only names
//!   inputs and reports which are pressed.
//!
//! A blind player may use a controller and nothing else, so actions must be
//! reachable from the pad too — not only the keyboard. The pad's **extra** buttons
//! (the triggers, stick clicks, and so on that the SNES pad does not use) are
//! free to bind to actions; the SNES buttons are not, so a rebind can never steal
//! a control the game needs.

use beacon_emu::button;
use gilrs::{Axis, Button, Gilrs};
use winit::keyboard::KeyCode;

/// Analogue stick deflection past which a direction counts as held.
const STICK_DEADZONE: f32 = 0.5;

// --- Keyboard: SNES buttons ------------------------------------------------

/// Maps a physical key to a SNES button.
///
/// The layout is the retro-emulator convention, which players arriving from
/// other emulators will already know. This mapping is fixed: game controls are
/// not rebindable in this version, and keeping them fixed is what lets action
/// keys be rebindable safely.
fn key_to_button(key: KeyCode) -> Option<u16> {
    Some(match key {
        KeyCode::ArrowUp => button::UP,
        KeyCode::ArrowDown => button::DOWN,
        KeyCode::ArrowLeft => button::LEFT,
        KeyCode::ArrowRight => button::RIGHT,
        KeyCode::KeyX => button::A,
        KeyCode::KeyZ => button::B,
        KeyCode::KeyS => button::X,
        KeyCode::KeyA => button::Y,
        KeyCode::KeyQ => button::L,
        KeyCode::KeyW => button::R,
        KeyCode::Enter => button::START,
        KeyCode::ShiftRight => button::SELECT,
        _ => return None,
    })
}

/// Whether a key drives the game, and so must not be bound to an action.
pub fn is_game_button(key: KeyCode) -> bool {
    key_to_button(key).is_some()
}

// --- Keyboard: naming ------------------------------------------------------

/// The keys the keymap can name, paired with their stable string form.
///
/// The string is what is written to the settings file, so it must not change
/// once shipped. An explicit table rather than deriving from `Debug` keeps that
/// guarantee in one visible place, and gives a clean round trip both ways.
macro_rules! key_table {
    ($($code:ident => $name:literal),* $(,)?) => {
        const KEY_TABLE: &[(KeyCode, &str)] = &[ $((KeyCode::$code, $name)),* ];
    };
}

key_table! {
    KeyA => "KeyA", KeyB => "KeyB", KeyC => "KeyC", KeyD => "KeyD",
    KeyE => "KeyE", KeyF => "KeyF", KeyG => "KeyG", KeyH => "KeyH",
    KeyI => "KeyI", KeyJ => "KeyJ", KeyK => "KeyK", KeyL => "KeyL",
    KeyM => "KeyM", KeyN => "KeyN", KeyO => "KeyO", KeyP => "KeyP",
    KeyQ => "KeyQ", KeyR => "KeyR", KeyS => "KeyS", KeyT => "KeyT",
    KeyU => "KeyU", KeyV => "KeyV", KeyW => "KeyW", KeyX => "KeyX",
    KeyY => "KeyY", KeyZ => "KeyZ",
    Digit0 => "Digit0", Digit1 => "Digit1", Digit2 => "Digit2",
    Digit3 => "Digit3", Digit4 => "Digit4", Digit5 => "Digit5",
    Digit6 => "Digit6", Digit7 => "Digit7", Digit8 => "Digit8",
    Digit9 => "Digit9",
    F1 => "F1", F2 => "F2", F3 => "F3", F4 => "F4", F5 => "F5", F6 => "F6",
    F7 => "F7", F8 => "F8", F9 => "F9", F10 => "F10", F11 => "F11", F12 => "F12",
    ArrowUp => "ArrowUp", ArrowDown => "ArrowDown",
    ArrowLeft => "ArrowLeft", ArrowRight => "ArrowRight",
    Escape => "Escape", Enter => "Enter", Space => "Space", Tab => "Tab",
    Backspace => "Backspace", Delete => "Delete", Insert => "Insert",
    Home => "Home", End => "End", PageUp => "PageUp", PageDown => "PageDown",
    ShiftLeft => "ShiftLeft", ShiftRight => "ShiftRight",
    Minus => "Minus", Equal => "Equal",
    BracketLeft => "BracketLeft", BracketRight => "BracketRight",
    Backslash => "Backslash", Semicolon => "Semicolon", Quote => "Quote",
    Backquote => "Backquote", Comma => "Comma", Period => "Period",
    Slash => "Slash",
}

/// The stable string form of a key, or `None` if Beacon does not name it.
pub fn key_name(key: KeyCode) -> Option<&'static str> {
    KEY_TABLE
        .iter()
        .find(|(code, _)| *code == key)
        .map(|(_, name)| *name)
}

/// The key for a stable string, the inverse of [`key_name`].
pub fn key_from_name(name: &str) -> Option<KeyCode> {
    KEY_TABLE
        .iter()
        .find(|(_, n)| *n == name)
        .map(|(code, _)| *code)
}

// --- Gamepad: SNES buttons and extras --------------------------------------

/// The gamepad buttons that drive the game, and the SNES button each is.
///
/// These are off-limits to action binding, exactly like the fixed keyboard game
/// keys, so an action can never shadow a game control.
const GAME_PAD: &[(Button, &str, u16)] = &[
    (Button::DPadUp, "Pad:DPadUp", button::UP),
    (Button::DPadDown, "Pad:DPadDown", button::DOWN),
    (Button::DPadLeft, "Pad:DPadLeft", button::LEFT),
    (Button::DPadRight, "Pad:DPadRight", button::RIGHT),
    (Button::South, "Pad:South", button::B),
    (Button::East, "Pad:East", button::A),
    (Button::West, "Pad:West", button::Y),
    (Button::North, "Pad:North", button::X),
    (Button::LeftTrigger, "Pad:LeftTrigger", button::L),
    (Button::RightTrigger, "Pad:RightTrigger", button::R),
    (Button::Start, "Pad:Start", button::START),
    (Button::Select, "Pad:Select", button::SELECT),
];

/// The gamepad buttons the SNES pad does not use, free to bind to actions.
const ACTION_PAD: &[(Button, &str)] = &[
    (Button::LeftTrigger2, "Pad:LeftTrigger2"),
    (Button::RightTrigger2, "Pad:RightTrigger2"),
    (Button::LeftThumb, "Pad:LeftThumb"),
    (Button::RightThumb, "Pad:RightThumb"),
    (Button::Mode, "Pad:Mode"),
    (Button::C, "Pad:C"),
    (Button::Z, "Pad:Z"),
];

/// The stable name for a gamepad button, or `None` for one Beacon ignores.
pub fn pad_button_name(b: Button) -> Option<&'static str> {
    GAME_PAD
        .iter()
        .find(|(x, _, _)| *x == b)
        .map(|(_, n, _)| *n)
        .or_else(|| ACTION_PAD.iter().find(|(x, _)| *x == b).map(|(_, n)| *n))
}

/// Whether a gamepad button name drives the game, and so cannot be bound.
pub fn is_game_pad_name(name: &str) -> bool {
    GAME_PAD.iter().any(|(_, n, _)| *n == name)
}

/// Whether an input name — key or gamepad button — drives the game.
///
/// The device-independent form of the binding-safety check: it lets the modal
/// and any programmatic binder (the MCP server) refuse a game control by name,
/// without knowing whether it came from a keyboard or a pad.
pub fn is_game_input_name(name: &str) -> bool {
    if name.starts_with("Pad:") {
        is_game_pad_name(name)
    } else {
        key_from_name(name).is_some_and(is_game_button)
    }
}

/// A key or gamepad name in a form worth speaking.
///
/// `"KeyC"` becomes `"C"`, `"ArrowUp"` becomes `"Up arrow"`, `"Pad:LeftThumb"`
/// becomes `"left stick button"`. Used when telling the player what is bound.
pub fn key_label(name: &str) -> String {
    if let Some(letter) = name.strip_prefix("Key") {
        return letter.to_string();
    }
    if let Some(digit) = name.strip_prefix("Digit") {
        return digit.to_string();
    }
    match name {
        "ArrowUp" => "Up arrow".into(),
        "ArrowDown" => "Down arrow".into(),
        "ArrowLeft" => "Left arrow".into(),
        "ArrowRight" => "Right arrow".into(),
        "ShiftLeft" => "Left shift".into(),
        "ShiftRight" => "Right shift".into(),
        "BracketLeft" => "Left bracket".into(),
        "BracketRight" => "Right bracket".into(),
        "Backquote" => "Backtick".into(),
        "Pad:LeftTrigger2" => "L2".into(),
        "Pad:RightTrigger2" => "R2".into(),
        "Pad:LeftThumb" => "left stick button".into(),
        "Pad:RightThumb" => "right stick button".into(),
        "Pad:Mode" => "mode button".into(),
        "Pad:C" => "C button".into(),
        "Pad:Z" => "Z button".into(),
        other => other.to_string(),
    }
}

pub struct Input {
    keyboard_buttons: u16,
    gamepad_buttons: u16,
    gilrs: Option<Gilrs>,
}

impl Input {
    pub fn new() -> Self {
        // A missing gamepad subsystem is not fatal: the keyboard still works,
        // and refusing to start over it would be absurd.
        let gilrs = match Gilrs::new() {
            Ok(g) => Some(g),
            Err(e) => {
                eprintln!("gamepad support unavailable: {e}");
                None
            }
        };

        Input {
            keyboard_buttons: 0,
            gamepad_buttons: 0,
            gilrs,
        }
    }

    /// Records a key press or release, updating the SNES button mask.
    ///
    /// Only game keys affect state here; action keys are the app's concern. Does
    /// nothing for a key that is not a SNES button.
    pub fn on_key(&mut self, key: KeyCode, pressed: bool) {
        if let Some(bit) = key_to_button(key) {
            if pressed {
                self.keyboard_buttons |= bit;
            } else {
                self.keyboard_buttons &= !bit;
            }
        }
    }

    /// Releases all held keyboard buttons.
    ///
    /// Called when leaving play — opening the configuration modal — so a key held
    /// at that moment does not stay stuck down while the game is not listening.
    pub fn clear_keyboard(&mut self) {
        self.keyboard_buttons = 0;
    }

    /// Polls the gamepad, updating held SNES state and returning the names of any
    /// buttons pressed this tick.
    ///
    /// Call once per event-loop wake, not per emulated frame: the presses are
    /// edge events (a button going down), which the app routes to actions or to
    /// the configuration modal. The held state drives the SNES buttons.
    pub fn poll_gamepad(&mut self) -> Vec<&'static str> {
        let Some(gilrs) = self.gilrs.as_mut() else {
            return Vec::new();
        };

        // Drain events, capturing button-down edges for the action layer.
        let mut pressed = Vec::new();
        while let Some(ev) = gilrs.next_event() {
            if let gilrs::EventType::ButtonPressed(b, _) = ev.event {
                if let Some(name) = pad_button_name(b) {
                    pressed.push(name);
                }
            }
        }

        // Rebuild the held SNES mask from the first connected pad.
        let mut mask = 0u16;
        if let Some((_id, pad)) = gilrs.gamepads().next() {
            for (b, _, bit) in GAME_PAD {
                if pad.is_pressed(*b) {
                    mask |= *bit;
                }
            }

            // The left stick doubles as a d-pad. Players with limited dexterity
            // often find a stick easier than a d-pad, and it costs nothing.
            let x = pad.value(Axis::LeftStickX);
            let y = pad.value(Axis::LeftStickY);
            if x < -STICK_DEADZONE {
                mask |= button::LEFT;
            }
            if x > STICK_DEADZONE {
                mask |= button::RIGHT;
            }
            if y > STICK_DEADZONE {
                mask |= button::UP;
            }
            if y < -STICK_DEADZONE {
                mask |= button::DOWN;
            }
        }
        self.gamepad_buttons = mask;
        pressed
    }

    /// The SNES button mask, keyboard and gamepad combined.
    pub fn buttons(&self) -> u16 {
        self.keyboard_buttons | self.gamepad_buttons
    }
}

impl Default for Input {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_set_and_clear_their_bit() {
        let mut input = Input {
            keyboard_buttons: 0,
            gamepad_buttons: 0,
            gilrs: None,
        };

        input.on_key(KeyCode::ArrowLeft, true);
        assert_eq!(input.buttons(), button::LEFT);

        input.on_key(KeyCode::KeyX, true);
        assert_eq!(input.buttons(), button::LEFT | button::A);

        input.on_key(KeyCode::ArrowLeft, false);
        assert_eq!(input.buttons(), button::A);

        input.clear_keyboard();
        assert_eq!(input.buttons(), 0);
    }

    #[test]
    fn key_names_round_trip() {
        for (code, name) in KEY_TABLE {
            assert_eq!(key_name(*code), Some(*name));
            assert_eq!(key_from_name(name), Some(*code));
        }
    }

    #[test]
    fn key_labels_are_speakable() {
        assert_eq!(key_label("KeyC"), "C");
        assert_eq!(key_label("Digit5"), "5");
        assert_eq!(key_label("F5"), "F5");
        assert_eq!(key_label("ArrowUp"), "Up arrow");
        assert_eq!(key_label("Pad:LeftThumb"), "left stick button");
    }

    #[test]
    fn game_and_action_pad_buttons_are_disjoint() {
        for (_, action_name) in ACTION_PAD {
            assert!(
                !is_game_pad_name(action_name),
                "{action_name} is both a game and an action button"
            );
        }
    }

    #[test]
    fn default_bindings_never_collide_with_game_inputs() {
        // The safety invariant across both devices: no default binding steals a
        // game control. A collision would make one press both act and move.
        let keymap = beacon_config::Keymap::default();
        for (name, action) in keymap.iter() {
            if let Some(rest) = name.strip_prefix("Pad:") {
                let _ = rest;
                assert!(
                    !is_game_pad_name(name),
                    "default binding {name} -> {action} is a game pad button"
                );
            } else {
                let code = key_from_name(name).unwrap_or_else(|| panic!("unknown key {name}"));
                assert!(
                    !is_game_button(code),
                    "default binding {name} -> {action} collides with a SNES button"
                );
            }
        }
    }
}
