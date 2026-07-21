//! What a key can do.
//!
//! Every binding maps a key to an [`Action`]. An action is either a built-in
//! host function (save a state, advance a frame, open the input configuration)
//! or a plugin command dispatched by name. The keymap in [`beacon_config`] stores
//! these as strings; this module is the single place that translates between the
//! string form and the typed form, and the catalogue of what can be bound.

use beacon_plugin::{Plugin, STANDARD_COMMANDS};

/// A bound host function or plugin command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Quit Beacon.
    Quit,
    /// Cycle the verbosity level.
    CycleVerbosity,
    /// Repeat the last thing said.
    RepeatLast,
    /// Save the emulator state to the active slot.
    SaveState,
    /// Load the emulator state from the active slot.
    LoadState,
    /// Move to the next save slot.
    NextSlot,
    /// Move to the previous save slot.
    PrevSlot,
    /// Toggle pause.
    Pause,
    /// Advance exactly one frame, pausing if not already paused. A debugging aid
    /// for watching a plugin frame by frame.
    FrameAdvance,
    /// Show or hide the plugin's map view.
    ToggleMap,
    /// Open the input configuration modal.
    OpenInputConfig,
    /// Run a plugin command by id (scan, where, status, or a custom one).
    Command(String),
}

impl Action {
    /// Parses an action id as stored in the keymap.
    ///
    /// A `command:<id>` string is a plugin command; anything else is a built-in,
    /// matched by name. An unrecognised id is `None`, so a stale binding to an
    /// action that no longer exists is ignored rather than fatal.
    pub fn from_id(id: &str) -> Option<Action> {
        if let Some(cmd) = id.strip_prefix("command:") {
            return Some(Action::Command(cmd.to_string()));
        }
        Some(match id {
            "quit" => Action::Quit,
            "cycle_verbosity" => Action::CycleVerbosity,
            "repeat_last" => Action::RepeatLast,
            "save_state" => Action::SaveState,
            "load_state" => Action::LoadState,
            "next_slot" => Action::NextSlot,
            "prev_slot" => Action::PrevSlot,
            "pause" => Action::Pause,
            "frame_advance" => Action::FrameAdvance,
            "toggle_map" => Action::ToggleMap,
            "bind" => Action::OpenInputConfig,
            _ => return None,
        })
    }
}

/// A thing the user can bind a key to, with a label to speak while choosing.
#[derive(Debug, Clone)]
pub struct Bindable {
    /// The action id as stored in the keymap.
    pub id: String,
    /// Human label, spoken in the input configuration.
    pub label: String,
}

/// The built-in host actions, in the order the configuration presents them.
///
/// Ordered by how often they are reached for, not alphabetically: a player
/// scrolling the list hears the common ones first.
const BUILTIN: [(&str, &str); 12] = [
    ("save_state", "Save state"),
    ("load_state", "Load state"),
    ("next_slot", "Next save slot"),
    ("prev_slot", "Previous save slot"),
    ("pause", "Pause or resume"),
    ("frame_advance", "Advance one frame"),
    ("toggle_map", "Show or hide the map"),
    ("cycle_verbosity", "Cycle verbosity"),
    ("repeat_last", "Repeat last announcement"),
    ("bind", "Open input configuration"),
    ("quit", "Quit"),
    ("command:scan", "Scan, describe surroundings"),
];

/// Labels for the standard commands the host always offers.
fn standard_command_label(id: &str) -> &'static str {
    match id {
        "where" => "Where am I",
        "status" => "Status, health and resources",
        _ => "Command",
    }
}

/// Everything bindable right now: built-in actions, the standard commands, and
/// whatever custom commands the loaded plugin declares.
///
/// The plugin's commands come last and carry the plugin's own labels, so a
/// game-specific action reads as the plugin author wrote it.
pub fn bindable_actions(plugin: &dyn Plugin) -> Vec<Bindable> {
    let mut out: Vec<Bindable> = BUILTIN
        .iter()
        .map(|(id, label)| Bindable {
            id: id.to_string(),
            label: label.to_string(),
        })
        .collect();

    // scan is already in BUILTIN (it is the most-used command); add the other
    // standard commands here.
    for id in STANDARD_COMMANDS {
        if id == "scan" {
            continue;
        }
        out.push(Bindable {
            id: format!("command:{id}"),
            label: standard_command_label(id).to_string(),
        });
    }

    for cmd in plugin.commands() {
        out.push(Bindable {
            id: format!("command:{}", cmd.id),
            label: cmd.label.clone(),
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_builtins_and_commands() {
        assert_eq!(Action::from_id("save_state"), Some(Action::SaveState));
        assert_eq!(Action::from_id("frame_advance"), Some(Action::FrameAdvance));
        assert_eq!(
            Action::from_id("command:coordinates"),
            Some(Action::Command("coordinates".to_string()))
        );
        assert_eq!(Action::from_id("no_such_action"), None);
    }

    #[test]
    fn bindables_include_standard_commands_without_a_plugin() {
        use beacon_plugin::NullPlugin;
        let list = bindable_actions(&NullPlugin);
        let ids: Vec<&str> = list.iter().map(|b| b.id.as_str()).collect();
        assert!(ids.contains(&"command:scan"));
        assert!(ids.contains(&"command:where"));
        assert!(ids.contains(&"command:status"));
        assert!(ids.contains(&"frame_advance"));
    }
}
