//! Keyboard and gamepad, mapped to SNES buttons.
//!
//! Both are always live. A blind player may well use a gamepad for the game and
//! the keyboard for Beacon's own commands, so neither is a "mode".

use beacon_emu::button;
use gilrs::{Axis, Button, Gilrs};
use winit::keyboard::KeyCode;

/// Analogue stick deflection past which a direction counts as held.
const STICK_DEADZONE: f32 = 0.5;

/// Beacon's own commands, as opposed to SNES input.
///
/// Kept separate from the button mask so that game input and accessibility
/// controls never contend for the same key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Describe surroundings on demand.
    Scan,
    /// Report position and area.
    Where,
    /// Report health and resources.
    Status,
    /// Cycle verbosity, announcing the new level.
    CycleVerbosity,
    /// Repeat the last thing said.
    RepeatLast,
    Quit,
}

/// Maps a physical key to a SNES button.
///
/// The layout is the retro-emulator convention, which players arriving from
/// other emulators will already know.
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

/// Maps a physical key to a Beacon command.
///
/// Function keys and the numeric row are deliberately avoided: screen readers
/// claim many of those, and fighting a screen reader for a keystroke is a fight
/// Beacon should not pick.
fn key_to_command(key: KeyCode) -> Option<Command> {
    Some(match key {
        KeyCode::KeyC => Command::Scan,
        KeyCode::KeyE => Command::Where,
        KeyCode::KeyH => Command::Status,
        KeyCode::KeyV => Command::CycleVerbosity,
        KeyCode::KeyR => Command::RepeatLast,
        KeyCode::Escape => Command::Quit,
        _ => return None,
    })
}

pub struct Input {
    keyboard_buttons: u16,
    gamepad_buttons: u16,
    pending: Vec<Command>,
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
            pending: Vec::new(),
            gilrs,
        }
    }

    /// Records a key press or release from the window system.
    pub fn on_key(&mut self, key: KeyCode, pressed: bool) {
        if let Some(bit) = key_to_button(key) {
            if pressed {
                self.keyboard_buttons |= bit;
            } else {
                self.keyboard_buttons &= !bit;
            }
        }

        // Commands fire on press only, so holding a key does not repeat.
        if pressed {
            if let Some(cmd) = key_to_command(key) {
                self.pending.push(cmd);
            }
        }
    }

    /// Polls the gamepad. Call once per frame.
    pub fn poll_gamepad(&mut self) {
        let Some(gilrs) = self.gilrs.as_mut() else {
            return;
        };

        // Drain events so gilrs keeps its internal state current.
        while gilrs.next_event().is_some() {}

        let mut mask = 0u16;
        if let Some((_id, pad)) = gilrs.gamepads().next() {
            let held = |b: Button| pad.is_pressed(b);

            if held(Button::DPadUp) {
                mask |= button::UP;
            }
            if held(Button::DPadDown) {
                mask |= button::DOWN;
            }
            if held(Button::DPadLeft) {
                mask |= button::LEFT;
            }
            if held(Button::DPadRight) {
                mask |= button::RIGHT;
            }
            if held(Button::South) {
                mask |= button::B;
            }
            if held(Button::East) {
                mask |= button::A;
            }
            if held(Button::West) {
                mask |= button::Y;
            }
            if held(Button::North) {
                mask |= button::X;
            }
            if held(Button::LeftTrigger) {
                mask |= button::L;
            }
            if held(Button::RightTrigger) {
                mask |= button::R;
            }
            if held(Button::Start) {
                mask |= button::START;
            }
            if held(Button::Select) {
                mask |= button::SELECT;
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
    }

    /// The SNES button mask, keyboard and gamepad combined.
    pub fn buttons(&self) -> u16 {
        self.keyboard_buttons | self.gamepad_buttons
    }

    /// Takes any commands issued since the last call.
    pub fn take_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.pending)
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
            pending: Vec::new(),
            gilrs: None,
        };

        input.on_key(KeyCode::ArrowLeft, true);
        assert_eq!(input.buttons(), button::LEFT);

        input.on_key(KeyCode::KeyX, true);
        assert_eq!(input.buttons(), button::LEFT | button::A);

        input.on_key(KeyCode::ArrowLeft, false);
        assert_eq!(input.buttons(), button::A);
    }

    #[test]
    fn commands_fire_once_per_press() {
        let mut input = Input {
            keyboard_buttons: 0,
            gamepad_buttons: 0,
            pending: Vec::new(),
            gilrs: None,
        };

        input.on_key(KeyCode::KeyC, true);
        input.on_key(KeyCode::KeyC, false);
        assert_eq!(input.take_commands(), vec![Command::Scan]);
        assert!(input.take_commands().is_empty(), "not repeated");
    }

    #[test]
    fn command_keys_are_not_also_snes_buttons() {
        // Overlap would make a scan request also press a button, which during
        // combat would be actively dangerous.
        for key in [
            KeyCode::KeyC,
            KeyCode::KeyE,
            KeyCode::KeyH,
            KeyCode::KeyV,
            KeyCode::KeyR,
            KeyCode::Escape,
        ] {
            assert!(
                key_to_button(key).is_none(),
                "{key:?} is bound to both a command and a SNES button"
            );
        }
    }
}
