//! # Action System
//!
//! Decouples raw `KeyEvent` values from application logic.
//! Every module that reacts to user input receives an `Action`, never a
//! `KeyCode`.  This makes key-rebinding trivial: only `key_event_to_action`
//! needs to change.
//!
//! ## Key bindings
//! | Key              | Action                  |
//! |------------------|-------------------------|
//! | Ctrl+F3          | SortByName              |
//! | Ctrl+F5          | SortByDate              |
//! | Ctrl+F6          | SortBySize              |
//! | Ctrl+H           | ToggleHidden            |
//! | Ctrl+F           | CmdlineInsertPath       |
//! | Tab              | TabComplete (panels hidden) / switch panel (panels visible) |
//! | Ctrl+Shift+C     | CopyAbsPathToClipboard  |
//! | Ctrl+Alt+Shift+C | CopyOutputToClipboard   |
//! | Ctrl+B           | OpenBookmarkManager     |
//! | Ctrl+0..9        | BookmarkGoto(n)         |
//! | Delete           | CmdlineDeleteForward    |
//! | Shift+Delete     | HistoryDeleteEntry      |
//! | F2               | Refresh                 |
//! | any letter       | CmdlineChar             |
//! | Backspace        | NavigateUp (Backspace)  |
//! | Esc              | PopupClose              |

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
    /// Tab key: switch panel focus when panels are visible; trigger path
    /// completion when both panels are hidden (Ctrl+O mode).
    TabComplete,

    // ── Sorting ───────────────────────────────────────────────────────────
    SortByName,
    SortBySize,
    SortByDate,

    // ── Filtering / display ───────────────────────────────────────────────
    ToggleHidden, // Ctrl+H — show/hide dot-files
    Refresh,      // F2 — reload current directory

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

    // ── History popup ─────────────────────────────────────────────────────
    /// Remove the selected entry from command history (Shift+Delete).
    HistoryDeleteEntry,

    // ── Quick search (Ctrl+S to activate; Esc to exit) ────────────────────
    /// Activate quick-search mode in the active panel.
    QuickSearchActivate, // Ctrl+S

    // ── Clipboard ─────────────────────────────────────────────────────────
    CopyToClipboard,        // Ctrl+C  — copy selected/marked entry paths
    CopyAbsPathToClipboard, // Ctrl+Shift+C — always copy absolute path of current entry
    CopyOutputToClipboard,  // Ctrl+Alt+Shift+C — copy full output buffer text
    PasteFromClipboard,     // Ctrl+V — paste clipboard text into cmdline

    // ── Command-line ───────────────────────────────────────────────────────
    /// A printable character to append to the command line.
    CmdlineChar(char),
    /// Insert the name of the file under the cursor into the command line.
    CmdlineInsertName, // Ctrl+Enter
    /// Insert the absolute path of the current entry into the command line.
    CmdlineInsertPath, // Ctrl+F
    /// Clear the entire command-line input.
    CmdlineClear,      // Ctrl+U
    /// Extend the cmdline selection one character to the left (Shift+←).
    CmdlineSelectLeft,
    /// Extend the cmdline selection one character to the right (Shift+→).
    CmdlineSelectRight,

    // ── Menu navigation ───────────────────────────────────────────────────
    MoveLeft,            // ← — move to previous top-level menu entry
    MoveRight,           // → — move to next top-level menu entry
    SetTheme(ThemeKind), // dispatched from Options submenu

    // ── Bookmarks ─────────────────────────────────────────────────────────
    /// Navigate the active panel to bookmark number `n` (Ctrl+0..9).
    BookmarkGoto(u8),
    /// Open the bookmark manager popup (Ctrl+B).
    OpenBookmarkManager,

    // ── Delete key ────────────────────────────────────────────────────────
    /// The Delete key was pressed.
    ///
    /// Context determines the effect:
    /// * Panel / cmdline mode  — forward-delete the character at the cursor.
    /// * Bookmark manager popup — remove the selected bookmark entry.
    CmdlineDeleteForward,

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
    let ctrl  = key.modifiers.contains(KeyModifiers::CONTROL);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let alt   = key.modifiers.contains(KeyModifiers::ALT);

    // ── Ctrl+Alt+Shift combos (must come before Ctrl+Shift and plain Ctrl) ──
    if ctrl && alt && shift {
        return match key.code {
            KeyCode::Char('c') | KeyCode::Char('C') => Action::CopyOutputToClipboard,
            _ => Action::None,
        };
    }

    // ── Ctrl+Shift combos (must come before plain Ctrl check) ─────────────
    if ctrl && shift {
        return match key.code {
            KeyCode::Char('c') | KeyCode::Char('C') => Action::CopyAbsPathToClipboard,
            _ => Action::None,
        };
    }

    // ── Ctrl combos ────────────────────────────────────────────────────────
    if ctrl {
        return match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => Action::Quit,
            KeyCode::Char('h') | KeyCode::Char('H') => Action::ToggleHidden,
            KeyCode::Char('s') | KeyCode::Char('S') => Action::QuickSearchActivate,
            KeyCode::Char('u') | KeyCode::Char('U') => Action::CmdlineClear,
            KeyCode::Char('o') | KeyCode::Char('O') => Action::TogglePanelsVisible,
            KeyCode::Char('c') | KeyCode::Char('C') => Action::CopyToClipboard,
            KeyCode::Char('v') | KeyCode::Char('V') => Action::PasteFromClipboard,
            KeyCode::Char('f') | KeyCode::Char('F') => Action::CmdlineInsertPath,
            KeyCode::Char('b') | KeyCode::Char('B') => Action::OpenBookmarkManager,
            // Ctrl+0..9 — go to bookmark
            KeyCode::Char('0') => Action::BookmarkGoto(0),
            KeyCode::Char('1') => Action::BookmarkGoto(1),
            KeyCode::Char('2') => Action::BookmarkGoto(2),
            KeyCode::Char('3') => Action::BookmarkGoto(3),
            KeyCode::Char('4') => Action::BookmarkGoto(4),
            KeyCode::Char('5') => Action::BookmarkGoto(5),
            KeyCode::Char('6') => Action::BookmarkGoto(6),
            KeyCode::Char('7') => Action::BookmarkGoto(7),
            KeyCode::Char('8') => Action::BookmarkGoto(8),
            KeyCode::Char('9') => Action::BookmarkGoto(9),
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

    if shift {
        return match key.code {
            KeyCode::F(5)    => Action::CreateArchive,
            KeyCode::F(6)    => Action::Rename,
            KeyCode::Left    => Action::CmdlineSelectLeft,
            KeyCode::Right   => Action::CmdlineSelectRight,
            KeyCode::Delete  => Action::HistoryDeleteEntry,
            _                => Action::None,
        };
    }

    // Alt combos reserved for future use
    if alt {
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
        KeyCode::Delete    => Action::CmdlineDeleteForward,
        KeyCode::Left      => Action::MoveLeft,
        KeyCode::Right     => Action::MoveRight,
        KeyCode::Tab       => Action::TabComplete,
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
