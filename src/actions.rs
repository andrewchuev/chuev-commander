//! # Action System
//!
//! Decouples raw `KeyEvent` values from application logic.
//! Every module that reacts to user input receives an `Action`, never a
//! `KeyCode`.  This makes key-rebinding trivial: only `key_event_to_action`
//! needs to change.
//!
//! ## Key bindings (Step 2 additions)
//! | Key        | Action          |
//! |------------|-----------------|
//! | Ctrl+F3    | SortByName      |
//! | Ctrl+F5    | SortByDate      |
//! | Ctrl+F6    | SortBySize      |
//! | Ctrl+H     | ToggleHidden    |
//! | F2         | Refresh         |
//! | any letter | QuickSearchChar |
//! | Backspace  | QuickSearchPop (when search active) / NavigateUp |
//! | Esc        | QuickSearchClear (when search active) / PopupClose |

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::theme::ThemeKind;

/// All things a user can *intend* to do, independent of which key triggered it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    // ── Cursor navigation ──────────────────────────────────────────────────
    MoveUp,
    MoveDown,
    PageUp,
    PageDown,
    Home,
    End,

    // ── Directory navigation ───────────────────────────────────────────────
    NavigateInto, // Enter  — open selected dir / file
    NavigateUp,   // Bksp   — go to parent directory

    // ── Panel management ───────────────────────────────────────────────────
    TogglePanel, // Tab — switch focus between left and right panel

    // ── Sorting ───────────────────────────────────────────────────────────
    SortByName,
    SortBySize,
    SortByDate,

    // ── Filtering / display ───────────────────────────────────────────────
    ToggleHidden,          // Ctrl+H — show/hide dot-files
    QuickSearchChar(char), // printable char — extend search query
    Refresh,               // F2 — reload current directory

    // ── File operations ────────────────────────────────────────────────────
    View,    // F3
    Edit,    // F4  — suspend TUI, launch $EDITOR, restore TUI
    Copy,    // F5
    Move,    // F6
    MakeDir, // F7
    Delete,  // F8
    Rename,  // Shift+F6

    /// Toggle the selection mark on the current entry and advance the cursor.
    Select, // Insert

    /// Pack the selected/marked files into a new ZIP archive (Shift+F5).
    CreateArchive,

    // ── Quick search (Ctrl+S to activate; Esc to exit) ────────────────────
    /// Activate quick-search mode in the active panel.
    QuickSearchActivate, // Ctrl+S

    // ── Clipboard ─────────────────────────────────────────────────────────
    CopyToClipboard,    // Ctrl+C — copy selected/marked entry paths
    PasteFromClipboard, // Ctrl+V — paste clipboard text into cmdline

    // ── Command-line ───────────────────────────────────────────────────────
    /// A printable character to append to the command line.
    CmdlineChar(char),
    /// Insert the name of the file under the cursor into the command line.
    CmdlineInsertName, // Ctrl+Enter
    /// Clear the entire command-line input.
    CmdlineClear,      // Ctrl+U

    // ── Menu navigation ───────────────────────────────────────────────────
    MoveLeft,            // ← — move to previous top-level menu entry
    MoveRight,           // → — move to next top-level menu entry
    SetTheme(ThemeKind), // dispatched from Options submenu

    // ── App-level ──────────────────────────────────────────────────────────
    TogglePanelsVisible, // Ctrl+O   — hide/show both panels (see terminal output)
    PanelHeightGrow,     // Ctrl+Up  — grow panels area (shrink output strip)
    PanelHeightShrink,   // Ctrl+Down — shrink panels area (grow output strip)
    Menu, // F9  — open top menu bar
    Quit, // F10 / Ctrl+Q

    // ── Modal popup handling ───────────────────────────────────────────────
    PopupConfirm, // Enter inside a popup
    PopupClose,   // Esc   inside a popup

    /// Key was recognised but maps to no semantic action.
    None,
}

/// Translates a raw crossterm `KeyEvent` into an [`Action`].
///
/// This is the single place that knows about physical key codes.
/// Everything downstream works with `Action` values only.
pub fn key_event_to_action(key: &KeyEvent) -> Action {
    // ── Ctrl combos take priority ──────────────────────────────────────────
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => Action::Quit,
            KeyCode::Char('h') | KeyCode::Char('H') => Action::ToggleHidden,
            KeyCode::Char('s') | KeyCode::Char('S') => Action::QuickSearchActivate,
            KeyCode::Char('u') | KeyCode::Char('U') => Action::CmdlineClear,
            KeyCode::Char('o') | KeyCode::Char('O') => Action::TogglePanelsVisible,
            KeyCode::Char('c') | KeyCode::Char('C') => Action::CopyToClipboard,
            KeyCode::Char('v') | KeyCode::Char('V') => Action::PasteFromClipboard,
            // Ctrl+Enter — insert filename into command line
            KeyCode::Enter => Action::CmdlineInsertName,
            // Ctrl+Up/Down — resize panels vertically
            KeyCode::Up   => Action::PanelHeightGrow,
            KeyCode::Down => Action::PanelHeightShrink,
            // Ctrl+F3/F5/F6 sort shortcuts
            KeyCode::F(3) => Action::SortByName,
            KeyCode::F(5) => Action::SortByDate,
            KeyCode::F(6) => Action::SortBySize,
            _ => Action::None,
        };
    }

    if key.modifiers.contains(KeyModifiers::SHIFT) {
        return match key.code {
            KeyCode::F(5) => Action::CreateArchive,
            KeyCode::F(6) => Action::Rename,
            _ => Action::None,
        };
    }

    // Alt combos reserved for future use (bookmarks, history navigation)
    if key.modifiers.contains(KeyModifiers::ALT) {
        return Action::None;
    }

    match key.code {
        KeyCode::Up        => Action::MoveUp,
        KeyCode::Down      => Action::MoveDown,
        KeyCode::PageUp    => Action::PageUp,
        KeyCode::PageDown  => Action::PageDown,
        KeyCode::Home      => Action::Home,
        KeyCode::End       => Action::End,
        KeyCode::Enter     => Action::NavigateInto,
        KeyCode::Backspace => Action::NavigateUp, // app.rs decides if this pops search
        KeyCode::Left      => Action::MoveLeft,
        KeyCode::Right     => Action::MoveRight,
        KeyCode::Tab       => Action::TogglePanel,
        KeyCode::Esc       => Action::PopupClose,  // app.rs decides if this clears search
        KeyCode::F(2)      => Action::Refresh,
        KeyCode::F(3)      => Action::View,
        KeyCode::F(4)      => Action::Edit,
        KeyCode::F(5)      => Action::Copy,
        KeyCode::F(6)      => Action::Move,
        KeyCode::F(7)      => Action::MakeDir,
        KeyCode::F(8)      => Action::Delete,
        KeyCode::F(9)      => Action::Menu,
        KeyCode::F(10)     => Action::Quit,
        KeyCode::Insert    => Action::Select,
        KeyCode::Char(c)   => Action::CmdlineChar(c),
        _                  => Action::None,
    }
}
