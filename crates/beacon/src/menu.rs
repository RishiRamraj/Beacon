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
}

/// A level of the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Root,
    File,
    Open,
    Save,
    Load,
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
        }
    }

    #[test]
    fn the_structure_is_readable_from_outside_for_the_platform_to_publish() {
        // Same content as the spoken form, without the words: a screen reader says "submenu"
        // and the position in its own way, so handing it prose would have it read twice.
        let ctx = ctx();
        let mut menu = Menu::open(&ctx);
        assert_eq!(menu.title(), "Menu");
        assert_eq!(menu.selected(), 0);
        assert_eq!(
            menu.shown(),
            vec![
                Shown {
                    label: "File".into(),
                    submenu: true
                },
                Shown {
                    label: "Save".into(),
                    submenu: true
                },
                Shown {
                    label: "Load".into(),
                    submenu: true
                },
                Shown {
                    label: "Input".into(),
                    submenu: true
                },
            ]
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
        assert_eq!(menu.announce(), "File, 1 of 4, submenu.");
        assert_eq!(menu.navigate(1), "Save, 2 of 4, submenu.");
        assert_eq!(menu.navigate(1), "Load, 3 of 4, submenu.");
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
        assert_eq!(menu.select(2), "Load, 3 of 4, submenu.");
        assert_eq!(menu.select(99), "Load, 3 of 4, submenu.");
    }

    #[test]
    fn wrapping_is_audible_from_the_position_alone() {
        let mut menu = Menu::open(&ctx());
        assert_eq!(menu.navigate(-1), "Input, 4 of 4, submenu.");
        assert_eq!(menu.navigate(1), "File, 1 of 4, submenu.");
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
            Backed::Moved("Menu. Save, 2 of 4, submenu.".to_string())
        );
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
        menu.navigate(3); // Input
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
        };
        let mut menu = Menu::open(&ctx);
        menu.navigate(1); // Save
        ctx.slots[0] = true;
        menu.choose(&ctx);
        assert_eq!(menu.announce(), "Slot 0, occupied, 1 of 2.");
    }
}
