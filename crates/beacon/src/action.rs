//! What a key can do.
//!
//! Every binding maps a key to an [`Action`]. An action is either a built-in
//! host function (save a state, advance a frame, open the input configuration)
//! or a plugin command dispatched by name. The keymap in [`beacon_config`] stores
//! these as strings; this module is the single place that translates between the
//! string form and the typed form, and the catalogue of what can be bound.

use beacon_config::Keymap;
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
    /// Toggle a global mute of all audio and speech.
    Mute,
    /// Advance exactly one frame, pausing if not already paused. A debugging aid
    /// for watching a plugin frame by frame.
    FrameAdvance,
    /// Show or hide the plugin's map view.
    ToggleMap,
    /// Fill the screen, or go back to a window: for playing, and for streaming with OBS.
    ToggleFullscreen,
    /// Open the input configuration modal.
    OpenInputConfig,
    /// Open the menu.
    OpenMenu,
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
            "mute" => Action::Mute,
            "frame_advance" => Action::FrameAdvance,
            "toggle_map" => Action::ToggleMap,
            "toggle_fullscreen" => Action::ToggleFullscreen,
            "bind" => Action::OpenInputConfig,
            "menu" => Action::OpenMenu,
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
const BUILTIN: [(&str, &str); 15] = [
    ("save_state", "Save state"),
    ("load_state", "Load state"),
    ("next_slot", "Next save slot"),
    ("prev_slot", "Previous save slot"),
    ("pause", "Pause or resume"),
    ("mute", "Mute or unmute"),
    ("frame_advance", "Advance one frame"),
    ("toggle_map", "Show or hide the map"),
    ("toggle_fullscreen", "Full screen on or off"),
    ("cycle_verbosity", "Cycle verbosity"),
    ("repeat_last", "Repeat last announcement"),
    ("menu", "Open menu"),
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

/// Gives any action the player has never bound its default keys, where those keys are
/// free.
///
/// A saved keymap REPLACES the defaults rather than merging with them — `Keymap` is
/// transparent over the map it holds — so without this, an action added after a player's
/// settings were first written is unreachable. They keep every key they chose and have no
/// way to get at the new thing, which is how the menu first arrived: unreachable for the
/// only player who had settings.
///
/// Two rules, matching what the plugin's suggested keys already do: never override a
/// binding the player made, and never take a key already in use.
pub fn apply_defaults(keymap: &mut Keymap) {
    let defaults: Vec<(String, String)> = Keymap::default()
        .iter()
        .map(|(k, a)| (k.to_string(), a.to_string()))
        .collect();

    // Which actions count as unbound is decided BEFORE anything is bound, so an action
    // with both a key and a pad button among its defaults gets both. Re-checking as it
    // went made the first default reached the only one applied — and the map is ordered by
    // key name, so "Pad:Mode" comes before "Tab" and the menu arrived as a pad button with
    // no key at all.
    //
    // And per device: a keyboard key and a pad button are separate gaps, so an action the
    // player has a key for still gets its controller default, which is what a controller-only
    // player needs.
    let unbound: Vec<(String, bool)> = defaults
        .iter()
        .map(|(key, action)| (action.clone(), key.starts_with("Pad:")))
        .filter(|(action, pad)| {
            !keymap
                .keys_for(action)
                .iter()
                .any(|k| k.starts_with("Pad:") == *pad)
        })
        .collect();

    for (key, action) in defaults {
        let pad = key.starts_with("Pad:");
        if unbound.contains(&(action.clone(), pad))
            && keymap.action_for(&key).is_none()
            && !crate::input::is_game_input_name(&key)
        {
            keymap.bind(&key, &action);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_builtins_and_commands() {
        assert_eq!(Action::from_id("save_state"), Some(Action::SaveState));
        assert_eq!(Action::from_id("mute"), Some(Action::Mute));
        assert_eq!(Action::from_id("frame_advance"), Some(Action::FrameAdvance));
        assert_eq!(
            Action::from_id("command:coordinates"),
            Some(Action::Command("coordinates".to_string()))
        );
        assert_eq!(Action::from_id("menu"), Some(Action::OpenMenu));
        assert_eq!(Action::from_id("no_such_action"), None);
    }

    /// The gap-filling rule, asserted against the keymap rather than a Session,
    /// which would need an emulator to build.
    ///
    /// A saved keymap replaces the defaults, so every built-in default has to be
    /// reachable through this or an action added later is unreachable for anyone who
    /// already had settings — which is exactly what happened when the menu landed.
    #[test]
    fn every_builtin_action_has_a_default_key_to_fall_back_on() {
        use beacon_config::Settings;
        let defaults = Settings::default();
        for (id, label) in BUILTIN {
            // Commands are the plugin's business; host actions are ours.
            if id.starts_with("command:") {
                continue;
            }
            assert!(
                !defaults.keymap.keys_for(id).is_empty(),
                "{label} ({id}) has no default key, so a player with saved settings \
                 could never reach it"
            );
        }
    }

    #[test]
    fn an_action_never_bound_gains_every_default_it_has() {
        // The menu has two defaults, a key and a pad button. Deciding "unbound" as it went
        // gave it only whichever came first by key name, which is the pad button.
        let mut keymap = Keymap::default();
        for key in keymap.keys_for("menu") {
            keymap.unbind(&key);
        }
        apply_defaults(&mut keymap);
        let mut keys = keymap.keys_for("menu");
        keys.sort();
        assert_eq!(
            keys,
            vec![
                "Escape".to_string(),
                "Pad:Mode".to_string(),
                "Pad:Select+Start".to_string(),
                "Tab".to_string()
            ]
        );
    }

    #[test]
    fn a_keymap_saved_by_the_build_where_f11_quit_gets_full_screen_back() {
        // The tester's case: the Oct 30 build bound F11 to Quit and saved it, and alpha.1 had
        // Escape as Quit. Without migrating, F11 and Escape kept quitting in every later build.
        let path = std::env::temp_dir().join(format!("beacon-f11-{}.toml", std::process::id()));
        std::fs::write(
            &path,
            "[keys]\nF11 = \"quit\"\nEscape = \"quit\"\nTab = \"menu\"\nKeyC = \"command:scan\"\n",
        )
        .unwrap();
        let mut s = beacon_config::Settings::load(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        s.migrate_keymap();
        apply_defaults(&mut s.keymap);
        assert_eq!(s.keymap.action_for("F11"), Some("toggle_fullscreen"));
        assert_eq!(s.keymap.action_for("Escape"), Some("menu"));
        assert_eq!(s.keymap.action_for("Pause"), Some("quit"));
        assert_eq!(s.keymap.action_for("KeyV"), Some("command:scan"));
    }

    #[test]
    fn a_players_own_bindings_are_never_touched() {
        // Modelled on a real saved keymap: heavily rebound, with the menu's default letter
        // taken by a plugin command and the menu itself unheard of.
        let mut keymap = Keymap::default();
        for key in keymap.keys_for("menu") {
            keymap.unbind(&key);
        }
        keymap.bind("KeyO", "command:explore");
        keymap.bind("KeyK", "save_state");

        apply_defaults(&mut keymap);

        // Their choices stand, including the one on a default key.
        assert_eq!(keymap.action_for("KeyO"), Some("command:explore"));
        assert_eq!(keymap.action_for("KeyK"), Some("save_state"));
        // And the action they never had is now reachable.
        assert!(!keymap.keys_for("menu").is_empty());
    }

    #[test]
    fn a_keyboard_binding_does_not_keep_an_action_from_its_controller_default() {
        // A player with saved settings has scan on a key. A controller-only player still needs
        // its Select chord, so a key and a pad button are filled as separate gaps.
        let mut keymap = Keymap::default();
        for key in keymap.keys_for("command:scan") {
            keymap.unbind(&key);
        }
        keymap.bind("KeyC", "command:scan");
        apply_defaults(&mut keymap);
        let keys = keymap.keys_for("command:scan");
        assert!(keys.contains(&"KeyC".to_string()), "the player's key stays: {keys:?}");
        assert!(keys.contains(&"Pad:Select+Y".to_string()), "and the chord arrives: {keys:?}");
        assert!(!keys.contains(&"KeyV".to_string()), "but the default key does not: {keys:?}");
    }

    #[test]
    fn an_action_the_player_moved_is_not_given_its_default_back() {
        // Rebinding is not the same as never having bound: putting the default back would
        // undo a deliberate choice on every launch.
        let mut keymap = Keymap::default();
        for key in keymap.keys_for("pause") {
            keymap.unbind(&key);
        }
        keymap.bind("KeyU", "pause");

        apply_defaults(&mut keymap);

        assert_eq!(keymap.keys_for("pause"), vec!["KeyU".to_string()]);
        assert_eq!(keymap.action_for("KeyP"), None);
    }

    /// A plugin may SUGGEST a default key, and the host refuses one the game owns — rightly,
    /// since a rebind must never steal a SNES button mid-fight. The failure mode is quiet: the
    /// command simply ships with no key. That is what happened to the ALttP room sweep, whose
    /// manifest asked for "s" whilst s is the X button, so the most useful command in the
    /// plugin was unbound out of the box and nothing said so.
    #[test]
    fn a_plugins_suggested_keys_are_all_ones_the_host_can_accept() {
        let registry = beacon_plugin::Registry::builtin();
        let mut suggested = 0;
        for spec in registry.specs() {
            for cmd in &spec.manifest.commands {
                for key in [&cmd.key, &cmd.pad].into_iter().flatten() {
                    suggested += 1;
                    assert!(
                        !crate::input::is_game_input_name(key),
                        "{} suggests {key}, which drives the game: the suggestion can never be \
                         applied, so the command would ship unbound",
                        cmd.id
                    );
                }
            }
        }
        assert!(suggested > 0, "the built-in plugin suggests at least one key");
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
        // The menu is bindable like anything else, so a controller-only player can
        // reach it without a keyboard.
        assert!(ids.contains(&"menu"));
    }
}
