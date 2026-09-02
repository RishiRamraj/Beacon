//! The platform's own menu, on the platforms that have one.
//!
//! Windows and macOS both have a real menu widget, drawn by the OS and read by the OS's
//! screen reader — NVDA and JAWS on Windows, VoiceOver on macOS — with the roles, the
//! keyboard conventions and the review commands their users already have. That is better
//! than anything Beacon can invent, so where it exists it is what should be used.
//!
//! Linux has no equivalent reachable from here: [`muda`] needs a `gtk::ApplicationWindow`
//! there, and Beacon's window is winit over X11 or Wayland, which hands out no GTK handle.
//! Getting a native menu bar on Linux means replacing the window layer, so Linux keeps the
//! menu it navigates itself — the spoken one, plus the accessibility tree in
//! [`crate::access`].
//!
//! Built from [`crate::menu::full`], the same entries the spoken menu walks, so the two
//! cannot drift into describing different menus.
//!
//! UNTESTED. Written and cross-compiled on Linux, where this module is not even built.

use std::collections::HashMap;

use muda::{Menu, MenuId, MenuItem, Submenu};

use crate::menu::{Act, Node};

/// A built native menu, and what each of its items means.
///
/// The menu is kept alive because dropping it takes the menu bar with it; the map is how a
/// click comes back as something to do, since the platform reports only an item's id.
pub struct NativeMenu {
    _menu: Menu,
    acts: HashMap<MenuId, Act>,
}

impl NativeMenu {
    /// Builds the menu and attaches it to the window.
    ///
    /// # Safety
    ///
    /// `hwnd` must be a live window handle for this process.
    #[cfg(windows)]
    pub unsafe fn attach(hwnd: isize, tree: &[Node]) -> muda::Result<Self> {
        let (menu, acts) = build(tree)?;
        menu.init_for_hwnd(hwnd)?;
        Ok(NativeMenu { _menu: menu, acts })
    }

    /// Builds the menu and installs it as the application menu.
    #[cfg(target_os = "macos")]
    pub fn attach_app(tree: &[Node]) -> muda::Result<Self> {
        let (menu, acts) = build(tree)?;
        menu.init_for_nsapp();
        Ok(NativeMenu { _menu: menu, acts })
    }

    /// What a chosen item means, or `None` for an id from somewhere else.
    pub fn act_for(&self, id: &MenuId) -> Option<Act> {
        self.acts.get(id).cloned()
    }
}

/// Builds the menu bar, collecting every leaf's id against what it does.
fn build(tree: &[Node]) -> muda::Result<(Menu, HashMap<MenuId, Act>)> {
    let menu = Menu::new();
    let mut acts = HashMap::new();
    for node in tree {
        match node {
            // A top-level entry that acts rather than opening anything. None of Beacon's do
            // today, but the tree allows it and silently dropping one would be a trap.
            Node::Act { label, act } => {
                let item = MenuItem::new(label, true, None);
                acts.insert(item.id().clone(), act.clone());
                menu.append(&item)?;
            }
            Node::Menu { label, children } => {
                let sub = Submenu::new(label, true);
                fill(&sub, children, &mut acts)?;
                menu.append(&sub)?;
            }
        }
    }
    Ok((menu, acts))
}

fn fill(into: &Submenu, nodes: &[Node], acts: &mut HashMap<MenuId, Act>) -> muda::Result<()> {
    for node in nodes {
        match node {
            Node::Act { label, act } => {
                let item = MenuItem::new(label, true, None);
                acts.insert(item.id().clone(), act.clone());
                into.append(&item)?;
            }
            Node::Menu { label, children } => {
                let sub = Submenu::new(label, true);
                fill(&sub, children, acts)?;
                into.append(&sub)?;
            }
        }
    }
    Ok(())
}
