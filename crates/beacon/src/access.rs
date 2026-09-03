//! Beacon's accessibility tree, for the platform's own screen reader.
//!
//! Beacon speaks for itself, straight to speech-dispatcher, and that is the right
//! default: it controls priority, interruption and rate precisely, and works with no
//! screen reader running at all. But a window drawn with softbuffer is opaque to
//! assistive technology — Orca, NVDA and VoiceOver see an unlabelled surface — so
//! nothing inside Beacon can be reached the way every other application is reached.
//!
//! This publishes the menu as a real menu: [`accesskit`] carries one tree to three
//! platform backends, AT-SPI on Linux, UI Automation on Windows, NSAccessibility on
//! macOS. The screen reader then announces it in its own words, with the conventions
//! and the keyboard habits its user already has, rather than the ones invented here.
//!
//! Beacon's own speech is kept alongside rather than replaced. On this platform the
//! two coexist as separate speech-dispatcher clients — every command Beacon sends is
//! scoped `self`, so its interruptions never cut off the screen reader's — and with no
//! screen reader running, Beacon's voice is the only one there is.
//!
//! Kept free of the window and the adapter, so the shape of the tree is testable
//! without a screen reader, a display, or an event loop.

use accesskit::{HasPopup, Live, Node, NodeId, Rect, Role, TreeId, TreeInfo, TreeUpdate};

use crate::config_modal::View;
use crate::menu::Menu;

/// The window itself, and the root of the tree.
pub const WINDOW: NodeId = NodeId(0);
/// The menu, when one is open.
pub const MENU: NodeId = NodeId(1);
/// Where an announcement goes when Beacon is speaking through the screen reader.
pub const ANNOUNCEMENT: NodeId = NodeId(2);
/// The input configuration, when it is open.
pub const DIALOG: NodeId = NodeId(3);
/// The dialog's list of actions.
pub const ACTIONS: NodeId = NodeId(4);
/// The dialog's instructions.
pub const DIALOG_HELP: NodeId = NodeId(5);
/// The first entry. Entry `i` is `ENTRY_BASE + i`.
const ENTRY_BASE: u64 = 16;
/// The first row of the dialog's list. Row `i` is `ROW_BASE + i`.
///
/// Far from the menu's range so the two can never be confused for one another: an id is all
/// a screen reader sends back, and acting on the wrong list would move the wrong cursor.
const ROW_BASE: u64 = 4096;

fn entry_id(i: usize) -> NodeId {
    NodeId(ENTRY_BASE + i as u64)
}

fn row_id(i: usize) -> NodeId {
    NodeId(ROW_BASE + i as u64)
}

/// Which row of the input configuration a node id refers to, or `None` for anything else.
pub fn row_index(id: NodeId) -> Option<usize> {
    if id.0 < ROW_BASE {
        return None;
    }
    Some((id.0 - ROW_BASE) as usize)
}

/// Which entry a node id refers to, or `None` for the window or the menu itself.
///
/// Needed because assistive technology drives the menu by node: a reader clicking an
/// item or moving its own review cursor names the node, not an index.
pub fn entry_index(id: NodeId) -> Option<usize> {
    if id.0 < ENTRY_BASE || id.0 >= ROW_BASE {
        return None;
    }
    Some((id.0 - ENTRY_BASE) as usize)
}

/// A node whose text a screen reader reads aloud when it changes.
///
/// A live region — the same idea as `aria-live` on the web — and the only way to have a screen
/// reader speak something that is not a focus change. It carries Beacon's announcements when
/// `speech.screen_reader` is on: the reader says them in its user's own voice, on any platform,
/// with no separate speech backend for each.
///
/// `Polite` rather than `Assertive` deliberately. Assertive interrupts whatever is being read,
/// including the menu item the player is trying to hear, and Beacon narrates often enough that
/// it would talk over itself. Losing the arbiter's fine-grained interruption is the price of
/// this route, and is why it is not the default.
fn announcement_node(text: &str) -> Node {
    let mut node = Node::new(Role::Label);
    // Both spellings: AT-SPI takes a Label's name from its value, UI Automation from its label.
    node.set_value(text.to_string());
    node.set_label(text.to_string());
    node.set_live(Live::Polite);
    node
}

/// The window node.
///
/// Given bounds, because assistive technology that locates things spatially — hit testing
/// under the mouse, a magnifier following focus — has nothing to go on otherwise, and a
/// window reported as zero-sized is a window some readers will not look inside.
fn window_node(title: &str, size: (f64, f64)) -> Node {
    let mut root = Node::new(Role::Window);
    root.set_label(title.to_string());
    root.set_bounds(Rect {
        x0: 0.0,
        y0: 0.0,
        x1: size.0,
        y1: size.1,
    });
    root
}

/// The tree for a window with no menu open.
///
/// Deliberately bare. Beacon narrates the game through its own speech, at a rate and
/// with interruption rules a live game needs, and mirroring that into the
/// accessibility tree would have the screen reader talk over it. So the tree describes
/// the parts that are genuinely user interface, and the menu is the first of them.
pub fn window_only(title: &str, size: (f64, f64), announcement: Option<&str>) -> TreeUpdate {
    let mut root = window_node(title, size);
    let mut nodes = Vec::new();
    if let Some(text) = announcement {
        root.set_children(vec![ANNOUNCEMENT]);
        nodes.push((ANNOUNCEMENT, announcement_node(text)));
    }
    nodes.insert(0, (WINDOW, root));
    TreeUpdate {
        nodes,
        tree: Some(TreeInfo::new(WINDOW)),
        tree_id: TreeId::ROOT,
        focus: WINDOW,
    }
}

/// The tree for an open menu, focused on the selected entry.
///
/// The focus is what makes a screen reader speak: moving it to the entry the cursor is
/// on is the whole mechanism, and the reader then says the label, that it is a menu
/// item, its place in the list and whether it opens a submenu — all from the node's
/// properties rather than from prose.
///
/// So `position_in_set` and `size_of_set` are set explicitly even though the list is
/// right there in the children: it is what lets the reader say "3 of 4" in the phrasing
/// and language its user has configured, instead of Beacon's.
pub fn menu_tree(
    title: &str,
    size: (f64, f64),
    menu: &Menu,
    announcement: Option<&str>,
) -> TreeUpdate {
    let entries = menu.shown();
    let ids: Vec<NodeId> = (0..entries.len()).map(entry_id).collect();

    let mut root = window_node(title, size);
    let mut top = vec![MENU];
    if announcement.is_some() {
        top.push(ANNOUNCEMENT);
    }
    root.set_children(top);

    let mut list = Node::new(Role::Menu);
    list.set_label(menu.title().to_string());
    list.set_children(ids.clone());

    let mut nodes = vec![(WINDOW, root), (MENU, list)];
    if let Some(text) = announcement {
        nodes.push((ANNOUNCEMENT, announcement_node(text)));
    }
    for (i, entry) in entries.iter().enumerate() {
        let mut node = Node::new(Role::MenuItem);
        node.set_label(entry.label.clone());
        node.set_position_in_set(i + 1);
        node.set_size_of_set(entries.len());
        if entry.submenu {
            // The reader announces this its own way, and collapsed is the truth: choosing
            // it replaces this level rather than expanding underneath it.
            node.set_has_popup(HasPopup::Menu);
            node.set_expanded(false);
        }
        nodes.push((ids[i], node));
    }

    // An empty level has no entry to focus, so the menu itself takes it and the reader
    // says the level's name. Focusing nothing at all would leave the reader silent,
    // which is the one thing an empty list must not be.
    let focus = ids.get(menu.selected()).copied().unwrap_or(MENU);

    TreeUpdate {
        nodes,
        tree: Some(TreeInfo::new(WINDOW)),
        tree_id: TreeId::ROOT,
        focus,
    }
}

/// The tree for the open input configuration: a real dialog, with every action in it.
///
/// This is the part spoken narration cannot do. Beacon says the row the cursor is on, which
/// is enough to *use* the dialog and no use at all for reviewing it — a player cannot hear
/// what else is in the list, or how far down it they are, without walking the whole thing.
/// Published as a listbox, a reader can read it in any order with the keys it already has.
///
/// The rows carry their binding in the label rather than in a second column, because a
/// two-column grid is more structure for a reader to navigate and the pair is one fact.
pub fn dialog_tree(
    title: &str,
    size: (f64, f64),
    view: &View,
    announcement: Option<&str>,
) -> TreeUpdate {
    let ids: Vec<NodeId> = (0..view.rows.len()).map(row_id).collect();

    let mut root = window_node(title, size);
    root.set_children(vec![DIALOG]);

    let mut dialog = Node::new(Role::Dialog);
    dialog.set_label(view.heading.to_string());
    // Modal because it is: the game is suspended and every key goes to the dialog, so a
    // reader should not offer the window behind it as somewhere to move.
    dialog.set_modal();
    // And because it is modal, the announcement goes INSIDE it. A reader that honours modality
    // may ignore everything outside the dialog, which would lose exactly the lines that matter
    // most here — a key bound, a key refused.
    let mut children = vec![DIALOG_HELP, ACTIONS];
    if announcement.is_some() {
        children.push(ANNOUNCEMENT);
    }
    dialog.set_children(children);

    let mut help = Node::new(Role::Label);
    help.set_label(view.help.to_string());
    // Both, because the two backends read a label differently: AT-SPI takes a Label node's
    // name from its value, and leaves the name empty without one.
    help.set_value(view.help.to_string());

    let mut list = Node::new(Role::ListBox);
    list.set_label("Actions".to_string());
    list.set_children(ids.clone());

    let mut nodes = vec![
        (WINDOW, root),
        (DIALOG, dialog),
        (DIALOG_HELP, help),
        (ACTIONS, list),
    ];
    if let Some(text) = announcement {
        nodes.push((ANNOUNCEMENT, announcement_node(text)));
    }
    for (i, row) in view.rows.iter().enumerate() {
        let mut node = Node::new(Role::ListBoxOption);
        node.set_label(row.clone());
        node.set_position_in_set(i + 1);
        node.set_size_of_set(view.rows.len());
        if i == view.selected {
            // So a reader reviewing the list from elsewhere can still tell which row the
            // next key press will bind to, which focus alone does not say.
            node.set_selected(true);
        }
        nodes.push((ids[i], node));
    }

    let focus = ids.get(view.selected).copied().unwrap_or(DIALOG);

    TreeUpdate {
        nodes,
        tree: Some(TreeInfo::new(WINDOW)),
        tree_id: TreeId::ROOT,
        focus,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::{Context, Menu};
    use std::path::PathBuf;

    fn ctx() -> Context {
        Context {
            slots: vec![true, false],
            roms: vec![("Zelda".to_string(), PathBuf::from("/roms/zelda.sfc"))],
            // The rest is only read by levels these tests do not enter.
            ..Context::default()
        }
    }

    /// The label of a node in an update, by id.
    fn label_of(update: &TreeUpdate, id: NodeId) -> Option<String> {
        update
            .nodes
            .iter()
            .find(|(node_id, _)| *node_id == id)
            .and_then(|(_, node)| node.label().map(|l| l.to_string()))
    }

    fn node_of(update: &TreeUpdate, id: NodeId) -> &Node {
        &update
            .nodes
            .iter()
            .find(|(node_id, _)| *node_id == id)
            .expect("node is in the update")
            .1
    }

    #[test]
    fn a_node_id_maps_back_to_the_entry_it_came_from() {
        // A screen reader drives the menu by node, so the mapping has to work both ways.
        for i in [0usize, 1, 9] {
            assert_eq!(entry_index(entry_id(i)), Some(i));
        }
        assert_eq!(entry_index(WINDOW), None);
        assert_eq!(entry_index(MENU), None);
    }

    #[test]
    fn a_closed_menu_publishes_only_the_window() {
        // Nothing of the game is mirrored into the tree: Beacon narrates that itself, and
        // a screen reader reading it too would talk over it.
        let update = window_only("Beacon", (768.0, 672.0), None);
        assert_eq!(update.nodes.len(), 1);
        assert_eq!(label_of(&update, WINDOW).as_deref(), Some("Beacon"));
        assert_eq!(update.focus, WINDOW);
    }

    #[test]
    fn an_announcement_is_published_as_a_live_region_the_reader_will_speak() {
        // The mechanism for having a screen reader say something that is not a focus change,
        // and the whole of how Beacon narrates through a reader rather than its own voice.
        let update = window_only("Beacon", (768.0, 672.0), Some("Two hearts."));
        let node = node_of(&update, ANNOUNCEMENT);
        assert_eq!(node.value(), Some("Two hearts."));
        assert_eq!(node.live(), Some(Live::Polite));
        // Reachable from the root, or a reader will never look at it.
        assert!(node_of(&update, WINDOW).children().contains(&ANNOUNCEMENT));
    }

    #[test]
    fn an_announcement_reaches_the_reader_while_a_menu_is_open_too() {
        // A toggle confirming its new state is said while the menu is up, and the focused
        // entry keeps the focus: the announcement rides alongside rather than stealing it.
        let ctx = ctx();
        let menu = Menu::open(&ctx);
        let update = menu_tree("Beacon", (768.0, 672.0), &menu, Some("Speech: off."));
        assert_eq!(node_of(&update, ANNOUNCEMENT).value(), Some("Speech: off."));
        assert_eq!(update.focus, entry_id(menu.selected()));
        assert!(node_of(&update, WINDOW).children().contains(&ANNOUNCEMENT));
    }

    #[test]
    fn nothing_is_published_when_beacon_is_speaking_for_itself() {
        // The default. A live region carrying the narration as well would have the reader
        // talking over Beacon's own voice.
        let update = window_only("Beacon", (768.0, 672.0), None);
        assert!(update.nodes.iter().all(|(id, _)| *id != ANNOUNCEMENT));
        assert!(node_of(&update, WINDOW).children().is_empty());
    }

    #[test]
    fn the_focus_follows_the_selected_entry() {
        // Focus is the whole mechanism: it is what makes a screen reader speak at all.
        let ctx = ctx();
        let mut menu = Menu::open(&ctx);
        let first = menu_tree("Beacon", (768.0, 672.0), &menu, None);
        assert_eq!(first.focus, entry_id(0));
        assert_eq!(label_of(&first, entry_id(0)).as_deref(), Some("File"));

        menu.navigate(1);
        let second = menu_tree("Beacon", (768.0, 672.0), &menu, None);
        assert_eq!(second.focus, entry_id(1));
        assert_eq!(label_of(&second, entry_id(1)).as_deref(), Some("Save"));
    }

    #[test]
    fn an_entry_carries_its_place_in_the_list_as_a_property() {
        // Not as words. The reader says "3 of 4" in its user's own phrasing and language,
        // which is the point of publishing a tree rather than prose.
        let ctx = ctx();
        let menu = Menu::open(&ctx);
        let update = menu_tree("Beacon", (768.0, 672.0), &menu, None);
        // However many the root holds; the point is that each carries its own place.
        let n = menu.shown().len();
        for i in 0..n {
            let node = node_of(&update, entry_id(i));
            assert_eq!(node.position_in_set(), Some(i + 1));
            assert_eq!(node.size_of_set(), Some(n));
        }
    }

    #[test]
    fn an_entry_that_leads_somewhere_is_marked_as_having_a_popup() {
        let ctx = ctx();
        let mut menu = Menu::open(&ctx);
        // Every root entry is a submenu.
        let root = menu_tree("Beacon", (768.0, 672.0), &menu, None);
        assert_eq!(
            node_of(&root, entry_id(0)).has_popup(),
            Some(HasPopup::Menu)
        );

        // A slot acts, so it has no popup and the reader will not offer to open one.
        menu.navigate(1);
        menu.choose(&ctx);
        let slots = menu_tree("Beacon", (768.0, 672.0), &menu, None);
        assert_eq!(node_of(&slots, entry_id(0)).has_popup(), None);
        assert_eq!(
            label_of(&slots, entry_id(0)).as_deref(),
            Some("Slot 0, occupied")
        );
    }

    #[test]
    fn the_input_configuration_is_published_as_a_real_dialog() {
        // Spoken narration can say the row the cursor is on and nothing else. This is what
        // lets a reader review the list: a modal dialog, a listbox, and every action in it.
        let view = View {
            heading: "Input configuration",
            help: "Up and down to choose an action.",
            rows: vec![
                "Save state. T.".to_string(),
                "Scan. C.".to_string(),
                "Quit. Escape.".to_string(),
            ],
            selected: 1,
        };
        let update = dialog_tree("Beacon", (768.0, 672.0), &view, None);

        let dialog = node_of(&update, DIALOG);
        assert_eq!(dialog.role(), Role::Dialog);
        assert_eq!(dialog.label(), Some("Input configuration"));
        assert!(dialog.is_modal(), "the game is suspended behind it");
        // The instructions are in the tree, not only in the sentence said on opening, so a
        // reader can go back and find out how the dialog works.
        assert_eq!(
            label_of(&update, DIALOG_HELP).as_deref(),
            Some("Up and down to choose an action.")
        );

        // The cursor's row has the focus, which is what makes a reader speak it, and is also
        // marked selected, so a reader reviewing elsewhere can still tell which row a key
        // press would bind to.
        assert_eq!(update.focus, NodeId(4096 + 1));
        let row = node_of(&update, NodeId(4096 + 1));
        assert_eq!(row.label(), Some("Scan. C."));
        assert_eq!(row.is_selected(), Some(true));
        assert_eq!(row.position_in_set(), Some(2));
        assert_eq!(row.size_of_set(), Some(3));
        assert_eq!(node_of(&update, NodeId(4096)).is_selected(), None);

        // Rows and menu entries can never be mistaken for each other: an id is all a reader
        // sends back, and acting on the wrong list would move the wrong cursor.
        assert_eq!(row_index(NodeId(4096 + 2)), Some(2));
        assert_eq!(row_index(entry_id(2)), None);
        assert_eq!(entry_index(NodeId(4096 + 2)), None);
    }

    #[test]
    fn an_announcement_reaches_the_reader_from_inside_the_dialog() {
        // A binding confirmed or refused is not in the tree, so it goes to the live region —
        // and must not take the focus off the row the player is on.
        let view = View {
            heading: "Input configuration",
            help: "help",
            rows: vec!["Save state. T.".to_string()],
            selected: 0,
        };
        let update = dialog_tree("Beacon", (768.0, 672.0), &view, Some("D bound to Scan."));
        assert_eq!(
            node_of(&update, ANNOUNCEMENT).value(),
            Some("D bound to Scan.")
        );
        assert_eq!(update.focus, NodeId(4096));
        // Inside the dialog, not beside it: a reader honouring modality would never look
        // outside, and these are the lines it most needs to say.
        assert!(node_of(&update, DIALOG).children().contains(&ANNOUNCEMENT));
    }

    #[test]
    fn the_menu_node_is_labelled_with_the_level() {
        let ctx = ctx();
        let mut menu = Menu::open(&ctx);
        assert_eq!(
            label_of(&menu_tree("Beacon", (768.0, 672.0), &menu, None), MENU).as_deref(),
            Some("Menu")
        );
        menu.navigate(2); // Load
        menu.choose(&ctx);
        assert_eq!(
            label_of(&menu_tree("Beacon", (768.0, 672.0), &menu, None), MENU).as_deref(),
            Some("Load")
        );
    }

    #[test]
    fn an_empty_level_focuses_the_menu_so_the_reader_still_says_something() {
        // A level with no entries has nothing to focus. Focusing nothing would leave the
        // reader silent, which is the one thing an empty list must not be.
        let ctx = Context {
            slots: vec![false],
            roms: Vec::new(),
            // The rest is only read by levels these tests do not enter.
            ..Context::default()
        };
        let mut menu = Menu::open(&ctx);
        menu.choose(&ctx); // File
        menu.choose(&ctx); // Open, which is empty
        let update = menu_tree("Beacon", (768.0, 672.0), &menu, None);
        assert_eq!(update.focus, MENU);
        assert_eq!(label_of(&update, MENU).as_deref(), Some("Open"));
    }
}
