//! Menu structure — top-level titles and per-menu item definitions.

use crate::actions::Action;
use crate::theme::ThemeKind;

/// Top-level menu bar titles, left to right.
pub const MENU_TITLES: &[&str] = &["Left", "Files", "Commands", "Options", "Right"];

/// A single entry in a dropdown menu.
pub enum MenuItem {
    Entry {
        label:    &'static str,
        shortcut: &'static str,
        action:   Action,
    },
    Separator,
}

impl MenuItem {
    pub fn entry(label: &'static str, shortcut: &'static str, action: Action) -> Self {
        Self::Entry { label, shortcut, action }
    }

    pub fn sep() -> Self {
        Self::Separator
    }

    pub fn is_selectable(&self) -> bool {
        matches!(self, Self::Entry { .. })
    }
}

/// Returns the dropdown items for the top-level menu at `top_idx`.
pub fn menu_entries(top_idx: usize) -> Vec<MenuItem> {
    match top_idx {
        // Left / Right panels — identical layout; panel focus is set by App on execute.
        0 | 4 => vec![
            MenuItem::entry("Sort by Name", "Ctrl+F3", Action::SortByName),
            MenuItem::entry("Sort by Size", "Ctrl+F6", Action::SortBySize),
            MenuItem::entry("Sort by Date", "Ctrl+F5", Action::SortByDate),
            MenuItem::sep(),
            MenuItem::entry("Show Hidden",  "Ctrl+H",  Action::ToggleHidden),
        ],
        // Files
        1 => vec![
            MenuItem::entry("View",   "F3",       Action::View),
            MenuItem::entry("Edit",   "F4",       Action::Edit),
            MenuItem::entry("Copy",   "F5",       Action::Copy),
            MenuItem::entry("Move",   "F6",       Action::Move),
            MenuItem::sep(),
            MenuItem::entry("MkDir",  "F7",       Action::MakeDir),
            MenuItem::entry("Delete", "F8",       Action::Delete),
            MenuItem::entry("Rename", "Shift+F6", Action::Rename),
        ],
        // Commands (stubs)
        2 => vec![
            MenuItem::entry("Find File", "", Action::None),
            MenuItem::entry("History",   "", Action::None),
            MenuItem::sep(),
            MenuItem::entry("Refresh",   "F2", Action::Refresh),
        ],
        // Options — theme selection
        3 => vec![
            MenuItem::entry(ThemeKind::Blue.name(),         "", Action::SetTheme(ThemeKind::Blue)),
            MenuItem::entry(ThemeKind::DosNavigator.name(), "", Action::SetTheme(ThemeKind::DosNavigator)),
        ],
        _ => vec![],
    }
}

/// Returns the index of the first selectable (non-separator) item.
pub fn first_selectable(top_idx: usize) -> usize {
    menu_entries(top_idx)
        .iter()
        .position(MenuItem::is_selectable)
        .unwrap_or(0)
}
