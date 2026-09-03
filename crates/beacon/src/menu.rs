//! The menu's logic, kept free of the emulator.
//!
//! Following [`crate::config_modal`]: choosing, descending, going back and
//! reporting what is selected needs no emulator, audio or speech, so it lives
//! here as a pure unit and is testable without a running console. Every method
//! returns the sentence to speak rather than speaking it, so the caller decides
//! how it is voiced and a test can assert on the words.
//!
//! # Reading a menu by ear
//!
//! A sighted user sees the shape of a menu at a glance: how long it is, where
//! they are in it, and which entries lead somewhere else. None of that survives
//! being read one line at a time, so every announcement carries it:
//!
//! - the entry's label, first, because it is what the listener is waiting for;
//! - its place in the list, so the length is known and wrapping is audible —
//!   going from "4 of 4" back to "1 of 4" needs no separate wrap noise;
//! - "submenu" when it leads somewhere, so nobody presses right on a dead end
//!   or expects a list to open when the entry simply acts.
//!
//! Changing level says the level's name before the entry, which is the spoken
//! equivalent of seeing a new panel appear.

use std::path::PathBuf;

/// The information a level needs to list itself, gathered by the caller.
///
/// Passed in rather than read through a borrow of the session, which is what
/// keeps this module pure: the caller assembles it, so nothing here needs to know
/// where a save slot or a ROM directory lives.
#[derive(Debug, Default, Clone)]
pub struct Context {
    /// Whether each save slot holds a state, indexed by slot number.
    pub slots: Vec<bool>,
    /// The ROMs `File, Open` offers: what to call each, and where it is.
    pub roms: Vec<(String, PathBuf)>,

    // The settings the menu shows. Their CURRENT values, because a toggle whose label does not
    // say what it is set to has to be activated to find out — which for a toggle means changing
    // the thing you were only asking about.
    /// 0 critical only, 3 everything.
    pub verbosity: u8,
    pub speech: bool,
    /// Whether announcements go to the screen reader instead of Beacon's own voice.
    pub screen_reader: bool,
    pub beacons: bool,
    pub braille: bool,
    pub json_events: bool,
    pub muted: bool,
    pub map_shown: bool,

    /// The loaded plugin's commands, as `(id, label)`.
    ///
    /// Everything the plugin can do, reachable without knowing which key it is on — including
    /// the navigation toggle, which is what a player is most likely to want and least likely to
    /// remember. Empty with no game, so the level reads "Empty".
    pub commands: Vec<(String, String)>,
}

/// A level of the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Root,
    File,
    Open,
    Save,
    Load,
    Settings,
    Verbosity,
    Debug,
    Game,
    Input,
}

impl Level {
    /// The name spoken on arriving at this level.
    fn title(self) -> &'static str {
        match self {
            Level::Root => "Menu",
            Level::File => "File",
            Level::Open => "Open",
            Level::Save => "Save",
            Level::Load => "Load",
            Level::Settings => "Settings",
            Level::Verbosity => "Verbosity",
            Level::Debug => "Debug",
            Level::Game => "Game",
            Level::Input => "Input",
        }
    }
}

/// What the caller should carry out when an entry is chosen.
///
/// The menu decides nothing about how these happen — it does not know how to
/// quit, save, or load a ROM — it only reports which one the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    /// Quit Beacon.
    Exit,
    /// Load this ROM in place of the current one.
    OpenRom(PathBuf),
    /// Save the state to this slot.
    SaveSlot(u8),
    /// Load the state from this slot.
    LoadSlot(u8),
    /// Open the input configuration.
    MapKeys,
    /// Store a setting by its dotted key. One variant for every setting, because the menu knows
    /// which key and which value and the session knows how to store it — a variant per toggle
    /// would be the same code a dozen times.
    ///
    /// `said` is what to speak once it is stored, and it comes from here because here is where
    /// the wording lives. Deriving it from the key gave "enabled false", which is the setting's
    /// name in a file rather than anything a player would say.
    SetSetting {
        key: &'static str,
        value: String,
        said: String,
    },
    /// Toggle the global mute.
    ToggleMute,
    /// Show or hide the plugin's map.
    ToggleMap,
    /// Advance a single frame, for watching a plugin work.
    FrameAdvance,
    /// Rebuild the plugin from its source.
    ReloadPlugin,
    /// Run one of the plugin's commands by id.
    Command(String),
}

/// What an entry does when chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Does {
    /// Descends into a level.
    Enter(Level),
    /// Carries out an act.
    Act(Act),
}

/// One line of a menu.
#[derive(Debug, Clone)]
struct Entry {
    label: String,
    does: Does,
}

impl Entry {
    fn enter(label: &str, level: Level) -> Entry {
        Entry {
            label: label.to_string(),
            does: Does::Enter(level),
        }
    }

    fn act(label: String, act: Act) -> Entry {
        Entry {
            label,
            does: Does::Act(act),
        }
    }
}

/// The entries of a level, as they stand right now.
///
/// Built on entering a level rather than once up front, because most of it moves:
/// a slot fills, a ROM appears in the directory. A menu that listed a slot as
/// empty because it was empty when Beacon started would be worse than no menu.
fn entries(level: Level, ctx: &Context) -> Vec<Entry> {
    match level {
        Level::Root => vec![
            Entry::enter("File", Level::File),
            Entry::enter("Save", Level::Save),
            Entry::enter("Load", Level::Load),
            Entry::enter("Game", Level::Game),
            Entry::enter("Settings", Level::Settings),
            Entry::enter("Debug", Level::Debug),
            Entry::enter("Input", Level::Input),
        ],
        Level::File => vec![
            Entry::enter("Open", Level::Open),
            Entry::act("Exit".to_string(), Act::Exit),
        ],
        Level::Open => ctx
            .roms
            .iter()
            .map(|(label, path)| Entry::act(label.clone(), Act::OpenRom(path.clone())))
            .collect(),
        // Save says whether a slot is taken because saving over one is the
        // mistake worth hearing about before it happens, not after.
        Level::Save => slot_entries(ctx, Act::SaveSlot),
        Level::Load => slot_entries(ctx, Act::LoadSlot),
        Level::Input => vec![Entry::act("Map keys".to_string(), Act::MapKeys)],

        // Each toggle says what it is set to, and choosing it flips it. So the label is the
        // state and the act is its opposite, which is why they are built together.
        Level::Settings => vec![
            Entry::enter(
                &format!("Verbosity: {}", verbosity_name(ctx.verbosity)),
                Level::Verbosity,
            ),
            toggle("Speech", ctx.speech, "speech.enabled"),
            toggle(
                "Announce through screen reader",
                ctx.screen_reader,
                "speech.screen_reader",
            ),
            toggle("Spatial audio", ctx.beacons, "beacons.enabled"),
            toggle("Braille", ctx.braille, "braille.enabled"),
            Entry::act(state("Mute", ctx.muted), Act::ToggleMute),
        ],

        Level::Verbosity => (0..=3)
            .map(|level| {
                // The one in force says so, since a list of four with no mark tells a listener
                // nothing about where they already are.
                let mark = if level == ctx.verbosity {
                    ", current"
                } else {
                    ""
                };
                Entry::act(
                    format!("{}{mark}", verbosity_name(level)),
                    Act::SetSetting {
                        key: "arbiter.verbosity",
                        value: level.to_string(),
                        said: format!("Verbosity {}.", verbosity_name(level)),
                    },
                )
            })
            .collect(),

        Level::Debug => vec![
            Entry::act(
                format!("Map: {}", if ctx.map_shown { "shown" } else { "hidden" }),
                Act::ToggleMap,
            ),
            Entry::act("Advance one frame".to_string(), Act::FrameAdvance),
            Entry::act("Reload plugin".to_string(), Act::ReloadPlugin),
            toggle("JSON events", ctx.json_events, "speech.json_events"),
        ],

        Level::Game => ctx
            .commands
            .iter()
            .map(|(id, label)| Entry::act(label.clone(), Act::Command(id.clone())))
            .collect(),
    }
}

/// "Speech: on", and the act that turns it off.
fn toggle(label: &str, on: bool, key: &'static str) -> Entry {
    Entry::act(
        state(label, on),
        Act::SetSetting {
            key,
            value: (!on).to_string(),
            // The state it is being moved TO, since that is what the player wants confirmed.
            said: format!("{}.", state(label, !on)),
        },
    )
}

fn state(label: &str, on: bool) -> String {
    format!("{label}: {}", if on { "on" } else { "off" })
}

/// The verbosity levels by name, matching what cycling through them already says.
fn verbosity_name(level: u8) -> &'static str {
    match level {
        0 => "critical only",
        1 => "navigation",
        2 => "interaction",
        _ => "everything",
    }
}

/// The save slots, numbered as the rest of Beacon numbers them, each saying
/// whether it holds anything.
fn slot_entries(ctx: &Context, act: impl Fn(u8) -> Act) -> Vec<Entry> {
    ctx.slots
        .iter()
        .enumerate()
        .map(|(i, &taken)| {
            let state = if taken { "occupied" } else { "empty" };
            Entry::act(format!("Slot {i}, {state}"), act(i as u8))
        })
        .collect()
}

/// The whole menu as a tree.
///
/// The stack above is for a menu Beacon navigates itself, one level at a time, because that
/// is what a menu read aloud has to be. A native menu bar is the opposite: the platform owns
/// the navigation and wants the whole thing up front, so it can open, traverse and close it
/// with the conventions its users already have. Same entries either way, from the same
/// [`entries`], so the two can never drift into describing different menus.
//
// Only the platforms with a native menu bar build a caller for these, so on Linux the binary
// has none — hence the allow. Not dead: the tests exercise them everywhere, which is what
// keeps the native menu's entries honest on a machine that cannot build it.
#[cfg_attr(
    not(any(windows, target_os = "macos")),
    allow(dead_code, reason = "only the native menu bar calls these")
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// A line that does something.
    Act { label: String, act: Act },
    /// A line that opens a menu of its own.
    Menu { label: String, children: Vec<Node> },
}

/// The menu, expanded in full.
#[cfg_attr(
    not(any(windows, target_os = "macos")),
    allow(dead_code, reason = "only the native menu bar calls this")
)]
pub fn full(ctx: &Context) -> Vec<Node> {
    expand(Level::Root, ctx)
}

#[cfg_attr(
    not(any(windows, target_os = "macos")),
    allow(dead_code, reason = "only the native menu bar calls this")
)]
fn expand(level: Level, ctx: &Context) -> Vec<Node> {
    entries(level, ctx)
        .into_iter()
        .map(|entry| match entry.does {
            Does::Enter(inner) => Node::Menu {
                label: entry.label,
                children: expand(inner, ctx),
            },
            Does::Act(act) => Node::Act {
                label: entry.label,
                act,
            },
        })
        .collect()
}

/// One level being shown, and where the cursor is in it.
struct Frame {
    level: Level,
    index: usize,
    entries: Vec<Entry>,
}

/// An open menu: the levels descended into, innermost last.
pub struct Menu {
    stack: Vec<Frame>,
}

/// One entry as something outside the menu can see it.
///
/// The speech this module returns is Beacon's own voice. This is the same content in a
/// form the platform's accessibility layer can be handed instead, so a screen reader
/// announces the menu itself, in its own words and conventions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub label: String,
    /// Whether it leads to another level, which a screen reader renders its own way —
    /// as a submenu, rather than by the word being read out.
    pub submenu: bool,
}

/// What choosing an entry did.
#[derive(Debug, PartialEq, Eq)]
pub enum Chosen {
    /// A level was entered; carries what to speak.
    Moved(String),
    /// An act was asked for. The caller performs it and closes the menu.
    Act(Act),
    /// The level was empty, so there was nothing to choose; carries what to say.
    Nothing(String),
}

/// What going back did.
#[derive(Debug, PartialEq, Eq)]
pub enum Backed {
    /// Returned to the level above; carries what to speak.
    Moved(String),
    /// Already at the top, so the menu should close.
    Close,
}

/// Spoken on opening, once, before the first entry.
pub const HELP: &str = "Menu. Up and down to move, right or enter to choose, \
                        left to go back, escape to close.";

impl Menu {
    /// Opens at the root.
    pub fn open(ctx: &Context) -> Menu {
        Menu {
            stack: vec![Frame {
                level: Level::Root,
                index: 0,
                entries: entries(Level::Root, ctx),
            }],
        }
    }

    fn top(&self) -> &Frame {
        self.stack.last().expect("the root frame is never popped")
    }

    /// The selected entry, said as landing on it: what it is, where it is in the
    /// list, and whether it leads anywhere.
    pub fn announce(&self) -> String {
        let frame = self.top();
        let n = frame.entries.len();
        if n == 0 {
            return "Empty.".to_string();
        }
        let entry = &frame.entries[frame.index];
        let place = format!("{} of {}", frame.index + 1, n);
        match entry.does {
            Does::Enter(_) => format!("{}, {}, submenu.", entry.label, place),
            Does::Act(_) => format!("{}, {}.", entry.label, place),
        }
    }

    /// Whether `text` is nothing more than what the menu itself is showing.
    ///
    /// For when a screen reader is reading the menu out of the accessibility tree: it already
    /// says the focused entry, so Beacon repeating the same words would have every item read
    /// twice. Anything else said while the menu is open — a toggle confirming its new state, a
    /// state saved — is not in the tree and does still need saying.
    pub fn duplicates(&self, text: &str) -> bool {
        text == self.announce() || text == self.arrival()
    }

    /// The level's name followed by the selected entry, as said on changing level.
    fn arrival(&self) -> String {
        format!("{}. {}", self.top().level.title(), self.announce())
    }

    /// The name of the level on show, for labelling the menu.
    pub fn title(&self) -> &'static str {
        self.top().level.title()
    }

    /// The entries on show, in order.
    pub fn shown(&self) -> Vec<Shown> {
        self.top()
            .entries
            .iter()
            .map(|entry| Shown {
                label: entry.label.clone(),
                submenu: matches!(entry.does, Does::Enter(_)),
            })
            .collect()
    }

    /// Which entry is selected, as an index into [`Menu::shown`].
    pub fn selected(&self) -> usize {
        self.top().index
    }

    /// Moves the cursor, wrapping, and returns the new selection's announcement.
    pub fn navigate(&mut self, delta: i32) -> String {
        let frame = self
            .stack
            .last_mut()
            .expect("the root frame is never popped");
        let n = frame.entries.len() as i32;
        if n > 0 {
            frame.index = (((frame.index as i32 + delta) % n + n) % n) as usize;
        }
        self.announce()
    }

    /// Moves the cursor to an absolute position, for assistive technology driving the
    /// menu by node rather than by direction. Out of range is ignored, since a stale node
    /// id from a level that has since changed must not put the cursor nowhere.
    pub fn select(&mut self, index: usize) -> String {
        let frame = self
            .stack
            .last_mut()
            .expect("the root frame is never popped");
        if index < frame.entries.len() {
            frame.index = index;
        }
        self.announce()
    }

    /// Chooses the selected entry: descends into a level, or reports the act.
    pub fn choose(&mut self, ctx: &Context) -> Chosen {
        let frame = self.top();
        if frame.entries.is_empty() {
            return Chosen::Nothing(format!("{} is empty.", frame.level.title()));
        }
        match frame.entries[frame.index].does.clone() {
            Does::Enter(level) => {
                self.stack.push(Frame {
                    level,
                    index: 0,
                    entries: entries(level, ctx),
                });
                Chosen::Moved(self.arrival())
            }
            Does::Act(act) => Chosen::Act(act),
        }
    }

    /// Goes back a level, or reports that the menu should close.
    pub fn back(&mut self) -> Backed {
        if self.stack.len() == 1 {
            return Backed::Close;
        }
        self.stack.pop();
        Backed::Moved(self.arrival())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Context {
        Context {
            slots: vec![true, false, false],
            roms: vec![
                ("Zelda".to_string(), PathBuf::from("/roms/zelda.sfc")),
                ("Metroid".to_string(), PathBuf::from("/roms/metroid.sfc")),
            ],
            // A plugin with two commands, and settings at their defaults, so the levels that
            // read them have something to show.
            commands: vec![
                ("scan".to_string(), "Scan".to_string()),
                (
                    "advance".to_string(),
                    "Guide to the next quest step".to_string(),
                ),
            ],
            verbosity: 2,
            speech: true,
            beacons: true,
            ..Context::default()
        }
    }

    /// Which announcements the accessibility tree already carries.
    ///
    /// When a screen reader is reading the menu from the tree, Beacon must not also announce
    /// the entry: the reader would say it twice. But everything else said while the menu is
    /// open — a toggle's new state, a slot saved — is not in the tree and still has to be said.
    #[test]
    fn the_menus_own_wording_is_recognised_as_already_in_the_tree() {
        let ctx = ctx();
        let mut menu = Menu::open(&ctx);
        assert!(menu.duplicates(&menu.announce()));
        // Entering a level says the level's name first; that phrasing counts too.
        let arrival = menu.choose(&ctx);
        let Chosen::Moved(arrival) = arrival else {
            panic!("File opens a level: {arrival:?}");
        };
        assert!(menu.duplicates(&arrival));
        // A confirmation is not the menu talking about itself.
        assert!(!menu.duplicates("Speech: off."));
        assert!(!menu.duplicates("Saved to slot 1."));
    }

    #[test]
    fn the_full_tree_holds_the_same_entries_as_walking_it() {
        // A native menu bar gets the whole thing at once. It has to be the SAME menu — built
        // from the same entries — or the platform's menu and the spoken one would drift.
        let ctx = ctx();
        let tree = full(&ctx);
        assert_eq!(tree.len(), 7);

        let Node::Menu { label, children } = &tree[0] else {
            panic!("File is a menu: {:?}", tree[0]);
        };
        assert_eq!(label, "File");
        // Open is a menu of ROMs; Exit acts.
        assert!(matches!(&children[0], Node::Menu { label, children }
            if label == "Open" && children.len() == ctx.roms.len()));
        assert_eq!(
            children[1],
            Node::Act {
                label: "Exit".into(),
                act: Act::Exit
            }
        );

        // Save and Load carry a slot each, labelled as the spoken menu labels them.
        let Node::Menu {
            children: saves, ..
        } = &tree[1]
        else {
            panic!("Save is a menu");
        };
        assert_eq!(
            saves[0],
            Node::Act {
                label: "Slot 0, occupied".into(),
                act: Act::SaveSlot(0)
            }
        );
        let Node::Menu {
            children: loads, ..
        } = &tree[2]
        else {
            panic!("Load is a menu");
        };
        assert_eq!(
            loads[1],
            Node::Act {
                label: "Slot 1, empty".into(),
                act: Act::LoadSlot(1)
            }
        );

        // And Input reaches the key mapping.
        let Node::Menu {
            children: input, ..
        } = &tree[6]
        else {
            panic!("Input is a menu");
        };
        assert_eq!(
            input[0],
            Node::Act {
                label: "Map keys".into(),
                act: Act::MapKeys
            }
        );
    }

    #[test]
    fn the_structure_is_readable_from_outside_for_the_platform_to_publish() {
        // Same content as the spoken form, without the words: a screen reader says "submenu"
        // and the position in its own way, so handing it prose would have it read twice.
        let ctx = ctx();
        let mut menu = Menu::open(&ctx);
        assert_eq!(menu.title(), "Menu");
        assert_eq!(menu.selected(), 0);
        let labels: Vec<String> = menu.shown().into_iter().map(|e| e.label).collect();
        assert_eq!(
            labels,
            vec!["File", "Save", "Load", "Game", "Settings", "Debug", "Input"]
        );
        assert!(
            menu.shown().iter().all(|e| e.submenu),
            "every root entry leads somewhere"
        );

        menu.navigate(1);
        assert_eq!(menu.selected(), 1);

        menu.choose(&ctx); // into Save
        assert_eq!(menu.title(), "Save");
        assert_eq!(menu.selected(), 0);
        let slots = menu.shown();
        assert_eq!(slots[0].label, "Slot 0, occupied");
        assert!(
            !slots[0].submenu,
            "a slot acts rather than leading anywhere"
        );

        menu.choose(&ctx); // File, Open: a level whose entries are files
        menu.back();
        assert_eq!(menu.title(), "Menu");
    }

    #[test]
    fn every_announcement_says_where_it_is_in_the_list() {
        // Length and position are what a sighted user gets for free and a listener
        // does not, so they are in every line rather than in a separate one.
        let mut menu = Menu::open(&ctx());
        assert_eq!(menu.announce(), "File, 1 of 7, submenu.");
        assert_eq!(menu.navigate(1), "Save, 2 of 7, submenu.");
        assert_eq!(menu.navigate(1), "Load, 3 of 7, submenu.");
    }

    #[test]
    fn an_entry_that_leads_somewhere_says_so() {
        // Otherwise the only way to find out is to press right and hear nothing.
        let mut menu = Menu::open(&ctx());
        assert!(menu.announce().ends_with("submenu."));
        menu.choose(&ctx()); // into File
        assert_eq!(menu.announce(), "Open, 1 of 2, submenu.");
        assert_eq!(menu.navigate(1), "Exit, 2 of 2.");
    }

    #[test]
    fn selecting_by_position_ignores_one_out_of_range() {
        // A screen reader can name a node from a level that has since changed under it.
        // Landing on nothing would be worse than staying put.
        let mut menu = Menu::open(&ctx());
        assert_eq!(menu.select(2), "Load, 3 of 7, submenu.");
        assert_eq!(menu.select(99), "Load, 3 of 7, submenu.");
    }

    #[test]
    fn wrapping_is_audible_from_the_position_alone() {
        let mut menu = Menu::open(&ctx());
        assert_eq!(menu.navigate(-1), "Input, 7 of 7, submenu.");
        assert_eq!(menu.navigate(1), "File, 1 of 7, submenu.");
    }

    #[test]
    fn changing_level_names_the_level_first() {
        // The spoken equivalent of a new panel appearing.
        let mut menu = Menu::open(&ctx());
        menu.navigate(1); // Save
        assert_eq!(
            menu.choose(&ctx()),
            Chosen::Moved("Save. Slot 0, occupied, 1 of 3.".to_string())
        );
        assert_eq!(
            menu.back(),
            Backed::Moved("Menu. Save, 2 of 7, submenu.".to_string())
        );
    }

    #[test]
    fn a_toggle_says_what_it_is_set_to_and_chooses_the_opposite() {
        // A toggle whose label does not say its state has to be activated to find out — which
        // for a toggle means changing the thing you were only asking about.
        let mut ctx = ctx();
        ctx.speech = true;
        ctx.beacons = false;

        let mut menu = Menu::open(&ctx);
        menu.navigate(4); // Settings
        menu.choose(&ctx);

        assert_eq!(menu.navigate(1), "Speech: on, 2 of 6.");
        assert_eq!(
            menu.choose(&ctx),
            Chosen::Act(Act::SetSetting {
                key: "speech.enabled",
                value: "false".to_string(),
                said: "Speech: off.".to_string()
            }),
            "choosing an `on` toggle asks for off"
        );

        assert_eq!(
            menu.navigate(1),
            "Announce through screen reader: off, 3 of 6."
        );
        assert_eq!(menu.navigate(1), "Spatial audio: off, 4 of 6.");
        assert_eq!(
            menu.choose(&ctx),
            Chosen::Act(Act::SetSetting {
                key: "beacons.enabled",
                value: "true".to_string(),
                said: "Spatial audio: on.".to_string()
            }),
            "and an `off` one asks for on"
        );
    }

    #[test]
    fn the_verbosity_in_force_is_marked() {
        // Four names with nothing marked tells a listener nothing about where they already are.
        let mut ctx = ctx();
        ctx.verbosity = 1;

        let mut menu = Menu::open(&ctx);
        menu.navigate(4); // Settings
        menu.choose(&ctx);
        // The Settings entry carries the level in force, so it can be read without entering.
        assert_eq!(menu.announce(), "Verbosity: navigation, 1 of 6, submenu.");
        menu.choose(&ctx); // into Verbosity

        assert_eq!(menu.announce(), "critical only, 1 of 4.");
        assert_eq!(menu.navigate(1), "navigation, current, 2 of 4.");
        assert_eq!(menu.navigate(1), "interaction, 3 of 4.");
        assert_eq!(
            menu.choose(&ctx),
            Chosen::Act(Act::SetSetting {
                key: "arbiter.verbosity",
                value: "2".to_string(),
                said: "Verbosity interaction.".to_string()
            })
        );
    }

    #[test]
    fn debug_offers_the_map_the_frame_step_and_a_plugin_reload() {
        let mut ctx = ctx();
        ctx.map_shown = true;

        let mut menu = Menu::open(&ctx);
        menu.navigate(5); // Debug
        menu.choose(&ctx);

        assert_eq!(menu.announce(), "Map: shown, 1 of 4.");
        assert_eq!(menu.choose(&ctx), Chosen::Act(Act::ToggleMap));
        assert_eq!(menu.navigate(1), "Advance one frame, 2 of 4.");
        assert_eq!(menu.choose(&ctx), Chosen::Act(Act::FrameAdvance));
        assert_eq!(menu.navigate(1), "Reload plugin, 3 of 4.");
        assert_eq!(menu.choose(&ctx), Chosen::Act(Act::ReloadPlugin));
    }

    #[test]
    fn game_lists_the_plugins_own_commands() {
        // Which is how navigation is reached without knowing its key — the thing a player is
        // most likely to want and least likely to remember.
        let ctx = ctx();
        let mut menu = Menu::open(&ctx);
        menu.navigate(3); // Game
        menu.choose(&ctx);

        assert_eq!(menu.announce(), "Scan, 1 of 2.");
        assert_eq!(
            menu.navigate(1),
            "Guide to the next quest step, 2 of 2.",
            "the plugin's own label, not one invented here"
        );
        assert_eq!(
            menu.choose(&ctx),
            Chosen::Act(Act::Command("advance".to_string()))
        );
    }

    #[test]
    fn game_is_empty_with_no_plugin() {
        // No game loaded, so nothing to run. The level says so rather than going quiet.
        let ctx = Context {
            slots: Vec::new(),
            ..Context::default()
        };
        let mut menu = Menu::open(&ctx);
        menu.navigate(3); // Game
        assert_eq!(menu.choose(&ctx), Chosen::Moved("Game. Empty.".to_string()));
    }

    #[test]
    fn back_at_the_top_closes_rather_than_going_nowhere() {
        let mut menu = Menu::open(&ctx());
        assert_eq!(menu.back(), Backed::Close);
    }

    #[test]
    fn a_slot_says_whether_it_is_taken() {
        // Saving over a state is the mistake worth hearing about beforehand.
        let mut menu = Menu::open(&ctx());
        menu.navigate(1); // Save
        menu.choose(&ctx());
        assert_eq!(menu.announce(), "Slot 0, occupied, 1 of 3.");
        assert_eq!(menu.navigate(1), "Slot 1, empty, 2 of 3.");
    }

    #[test]
    fn choosing_a_slot_reports_which_one_and_which_menu_asked() {
        let ctx = ctx();
        let mut save = Menu::open(&ctx);
        save.navigate(1);
        save.choose(&ctx);
        save.navigate(2); // Slot 2
        assert_eq!(save.choose(&ctx), Chosen::Act(Act::SaveSlot(2)));

        let mut load = Menu::open(&ctx);
        load.navigate(2); // Load
        load.choose(&ctx);
        assert_eq!(load.choose(&ctx), Chosen::Act(Act::LoadSlot(0)));
    }

    #[test]
    fn the_other_leaves_report_their_acts() {
        let ctx = ctx();

        let mut menu = Menu::open(&ctx);
        menu.choose(&ctx); // File
        menu.navigate(1); // Exit
        assert_eq!(menu.choose(&ctx), Chosen::Act(Act::Exit));

        let mut menu = Menu::open(&ctx);
        menu.navigate(6); // Input
        menu.choose(&ctx);
        assert_eq!(menu.announce(), "Map keys, 1 of 1.");
        assert_eq!(menu.choose(&ctx), Chosen::Act(Act::MapKeys));

        let mut menu = Menu::open(&ctx);
        menu.choose(&ctx); // File
        menu.choose(&ctx); // Open
        assert_eq!(menu.announce(), "Zelda, 1 of 2.");
        assert_eq!(
            menu.choose(&ctx),
            Chosen::Act(Act::OpenRom(PathBuf::from("/roms/zelda.sfc")))
        );
    }

    #[test]
    fn an_empty_level_says_it_is_empty_rather_than_nothing() {
        // A ROM directory with nothing in it. Silence here reads as the menu being
        // broken, and pressing right on it must not leave the cursor nowhere.
        let ctx = Context {
            slots: vec![false],
            roms: Vec::new(),
            // The rest is only read by levels these tests do not enter.
            ..Context::default()
        };
        let mut menu = Menu::open(&ctx);
        menu.choose(&ctx); // File
        assert_eq!(menu.choose(&ctx), Chosen::Moved("Open. Empty.".to_string()));
        // Navigating an empty level is still answered, and cannot move off the end.
        assert_eq!(menu.navigate(1), "Empty.");
        assert_eq!(
            menu.choose(&ctx),
            Chosen::Nothing("Open is empty.".to_string())
        );
        // And it is still possible to get out.
        assert_eq!(
            menu.back(),
            Backed::Moved("File. Open, 1 of 2, submenu.".to_string())
        );
    }

    #[test]
    fn the_levels_reflect_the_context_they_were_entered_with() {
        // Entries are built on entry, not once at open, so a slot that filled
        // since the menu opened reads as occupied.
        let mut ctx = Context {
            slots: vec![false, false],
            roms: Vec::new(),
            // The rest is only read by levels these tests do not enter.
            ..Context::default()
        };
        let mut menu = Menu::open(&ctx);
        menu.navigate(1); // Save
        ctx.slots[0] = true;
        menu.choose(&ctx);
        assert_eq!(menu.announce(), "Slot 0, occupied, 1 of 2.");
    }
}
