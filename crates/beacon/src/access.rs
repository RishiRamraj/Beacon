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

use accesskit::{HasPopup, Node, NodeId, Rect, Role, TreeId, TreeInfo, TreeUpdate};

use crate::menu::Menu;

/// The window itself, and the root of the tree.
pub const WINDOW: NodeId = NodeId(0);
/// The menu, when one is open.
pub const MENU: NodeId = NodeId(1);
/// The first entry. Entry `i` is `ENTRY_BASE + i`.
const ENTRY_BASE: u64 = 16;

fn entry_id(i: usize) -> NodeId {
    NodeId(ENTRY_BASE + i as u64)
}

/// Which entry a node id refers to, or `None` for the window or the menu itself.
///
/// Needed because assistive technology drives the menu by node: a reader clicking an
/// item or moving its own review cursor names the node, not an index.
pub fn entry_index(id: NodeId) -> Option<usize> {
    id.0.checked_sub(ENTRY_BASE).map(|i| i as usize)
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
pub fn window_only(title: &str, size: (f64, f64)) -> TreeUpdate {
    let root = window_node(title, size);
    TreeUpdate {
        nodes: vec![(WINDOW, root)],
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
pub fn menu_tree(title: &str, size: (f64, f64), menu: &Menu) -> TreeUpdate {
    let entries = menu.shown();
    let ids: Vec<NodeId> = (0..entries.len()).map(entry_id).collect();

    let mut root = window_node(title, size);
    root.set_children(vec![MENU]);

    let mut list = Node::new(Role::Menu);
    list.set_label(menu.title().to_string());
    list.set_children(ids.clone());

    let mut nodes = vec![(WINDOW, root), (MENU, list)];
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::{Context, Menu};
    use std::path::PathBuf;

    fn ctx() -> Context {
        Context {
            slots: vec![true, false],
            roms: vec![("Zelda".to_string(), PathBuf::from("/roms/zelda.sfc"))],
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
        let update = window_only("Beacon", (768.0, 672.0));
        assert_eq!(update.nodes.len(), 1);
        assert_eq!(label_of(&update, WINDOW).as_deref(), Some("Beacon"));
        assert_eq!(update.focus, WINDOW);
    }

    #[test]
    fn the_focus_follows_the_selected_entry() {
        // Focus is the whole mechanism: it is what makes a screen reader speak at all.
        let ctx = ctx();
        let mut menu = Menu::open(&ctx);
        let first = menu_tree("Beacon", (768.0, 672.0), &menu);
        assert_eq!(first.focus, entry_id(0));
        assert_eq!(label_of(&first, entry_id(0)).as_deref(), Some("File"));

        menu.navigate(1);
        let second = menu_tree("Beacon", (768.0, 672.0), &menu);
        assert_eq!(second.focus, entry_id(1));
        assert_eq!(label_of(&second, entry_id(1)).as_deref(), Some("Save"));
    }

    #[test]
    fn an_entry_carries_its_place_in_the_list_as_a_property() {
        // Not as words. The reader says "3 of 4" in its user's own phrasing and language,
        // which is the point of publishing a tree rather than prose.
        let ctx = ctx();
        let menu = Menu::open(&ctx);
        let update = menu_tree("Beacon", (768.0, 672.0), &menu);
        for i in 0..4 {
            let node = node_of(&update, entry_id(i));
            assert_eq!(node.position_in_set(), Some(i + 1));
            assert_eq!(node.size_of_set(), Some(4));
        }
    }

    #[test]
    fn an_entry_that_leads_somewhere_is_marked_as_having_a_popup() {
        let ctx = ctx();
        let mut menu = Menu::open(&ctx);
        // Every root entry is a submenu.
        let root = menu_tree("Beacon", (768.0, 672.0), &menu);
        assert_eq!(
            node_of(&root, entry_id(0)).has_popup(),
            Some(HasPopup::Menu)
        );

        // A slot acts, so it has no popup and the reader will not offer to open one.
        menu.navigate(1);
        menu.choose(&ctx);
        let slots = menu_tree("Beacon", (768.0, 672.0), &menu);
        assert_eq!(node_of(&slots, entry_id(0)).has_popup(), None);
        assert_eq!(
            label_of(&slots, entry_id(0)).as_deref(),
            Some("Slot 0, occupied")
        );
    }

    #[test]
    fn the_menu_node_is_labelled_with_the_level() {
        let ctx = ctx();
        let mut menu = Menu::open(&ctx);
        assert_eq!(
            label_of(&menu_tree("Beacon", (768.0, 672.0), &menu), MENU).as_deref(),
            Some("Menu")
        );
        menu.navigate(2); // Load
        menu.choose(&ctx);
        assert_eq!(
            label_of(&menu_tree("Beacon", (768.0, 672.0), &menu), MENU).as_deref(),
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
        };
        let mut menu = Menu::open(&ctx);
        menu.choose(&ctx); // File
        menu.choose(&ctx); // Open, which is empty
        let update = menu_tree("Beacon", (768.0, 672.0), &menu);
        assert_eq!(update.focus, MENU);
        assert_eq!(label_of(&update, MENU).as_deref(), Some("Open"));
    }
}
