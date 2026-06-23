//! # Application State (MVU model)
//!
//! `App` is the single source of truth for everything the UI renders.
//! The UI *reads* `App`; the event loop *writes* it via `App::update(Action)`.
//! No UI widget mutates state directly.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

// ─────────────────────────────────────────────────────────────────────────────
// Command line
// ─────────────────────────────────────────────────────────────────────────────

/// Persistent command-line state (Variant A: always-active, typing goes here).
#[derive(Debug)]
pub struct CmdLine {
    /// Text currently in the input field.
    pub input: String,
    /// Command history, oldest first.  Loaded from disk on startup.
    pub history: Vec<String>,
    /// `None` = showing live input.  `Some(i)` = browsing history entry `i`.
    history_idx: Option<usize>,
    /// Snapshot of `input` taken when history browsing started.
    saved_input: String,
}

impl Default for CmdLine {
    fn default() -> Self {
        Self::new()
    }
}

impl CmdLine {
    pub fn new() -> Self {
        Self {
            input:       String::new(),
            history:     Self::load_history(),
            history_idx: None,
            saved_input: String::new(),
        }
    }

    /// Append a character; exits history-browsing mode.
    pub fn push_char(&mut self, c: char) {
        self.history_idx = None;
        self.input.push(c);
    }

    /// Delete the last character.  Returns `false` if input was already empty.
    pub fn backspace(&mut self) -> bool {
        self.history_idx = None;
        if self.input.is_empty() {
            return false;
        }
        self.input.pop();
        true
    }

    /// Clear input and exit history-browsing mode.
    pub fn clear(&mut self) {
        self.input.clear();
        self.history_idx = None;
        self.saved_input.clear();
    }

    /// Scroll to the previous (older) history entry.
    pub fn history_prev(&mut self) {
        if self.history.is_empty() { return; }
        let idx = match self.history_idx {
            None => {
                // Save live input before entering history mode
                self.saved_input = self.input.clone();
                self.history.len() - 1
            }
            Some(0) => return, // already at oldest entry
            Some(i) => i - 1,
        };
        self.history_idx = Some(idx);
        self.input = self.history[idx].clone();
    }

    /// Scroll to the next (newer) history entry, or back to the live input.
    pub fn history_next(&mut self) {
        let Some(idx) = self.history_idx else { return };
        if idx + 1 < self.history.len() {
            let next = idx + 1;
            self.history_idx = Some(next);
            self.input = self.history[next].clone();
        } else {
            // Past the newest entry → restore live input
            self.history_idx = None;
            self.input = self.saved_input.clone();
        }
    }

    /// Return the suffix of the most-recent history entry that starts with the
    /// current `input`, or `None` if there is no such entry.
    /// Used to render a "ghost" completion hint after the cursor.
    pub fn history_hint(&self) -> Option<&str> {
        if self.input.is_empty() || self.history_idx.is_some() {
            return None;
        }
        let input = self.input.as_str();
        self.history.iter().rev()
            .find(|h| h.starts_with(input) && h.len() > input.len())
            .map(|h| &h[input.len()..])
    }

    /// Complete the current input to the full hint entry (→ key).
    pub fn accept_hint(&mut self) {
        if let Some(hint_suffix) = self.history_hint() {
            let full = format!("{}{}", self.input, hint_suffix);
            self.input = full;
            self.history_idx = None;
        }
    }

    /// Take the current input, add it to history, clear the field.
    /// Returns the command string (trimmed).  Returns `""` if input was blank.
    pub fn take_input(&mut self) -> String {
        let cmd = self.input.trim().to_string();
        self.input.clear();
        self.history_idx = None;
        self.saved_input.clear();

        if !cmd.is_empty() {
            if self.history.last().map(|s| s.as_str()) != Some(&cmd) {
                self.history.push(cmd.clone());
                const MAX: usize = 1_000;
                if self.history.len() > MAX {
                    self.history.drain(0..self.history.len() - MAX);
                }
            }
            self.save_history();
        }
        cmd
    }

    // ── History persistence ───────────────────────────────────────────────

    fn history_path() -> Option<PathBuf> {
        dirs::cache_dir().map(|d| d.join(APP_DIR).join("history"))
    }

    fn load_history() -> Vec<String> {
        let path = match Self::history_path() { Some(p) => p, None => return Vec::new() };
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect()
    }

    fn save_history(&self) {
        let Some(path) = Self::history_path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let content = self.history.join("\n") + "\n";
        let _ = std::fs::write(path, content);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Unit tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod cmdline_tests {
    use super::CmdLine;

    fn with_history(entries: &[&str]) -> CmdLine {
        let mut cl = CmdLine::new();
        // Clear any entries loaded from disk so tests are isolated from user state.
        cl.history.clear();
        for &e in entries {
            cl.history.push(e.to_owned());
        }
        cl
    }

    #[test]
    fn push_char_appends() {
        let mut cl = CmdLine::new();
        cl.push_char('h');
        cl.push_char('i');
        assert_eq!(cl.input, "hi");
    }

    #[test]
    fn backspace_removes_last_char() {
        let mut cl = CmdLine::new();
        cl.input = "abc".into();
        let changed = cl.backspace();
        assert!(changed);
        assert_eq!(cl.input, "ab");
    }

    #[test]
    fn backspace_on_empty_returns_false() {
        let mut cl = CmdLine::new();
        assert!(!cl.backspace());
        assert!(cl.input.is_empty());
    }

    #[test]
    fn clear_resets_state() {
        let mut cl = CmdLine::new();
        cl.input = "something".into();
        cl.clear();
        assert!(cl.input.is_empty());
    }

    /// history_prev should load the most-recent entry first.
    #[test]
    fn history_prev_loads_last_entry() {
        let mut cl = with_history(&["first", "second", "third"]);
        cl.history_prev();
        assert_eq!(cl.input, "third");
    }

    /// Calling history_prev twice should go one step further back.
    #[test]
    fn history_prev_walks_backwards() {
        let mut cl = with_history(&["alpha", "beta"]);
        cl.history_prev();
        cl.history_prev();
        assert_eq!(cl.input, "alpha");
    }

    /// history_next after going back one step should restore the live input.
    #[test]
    fn history_next_restores_live_input() {
        let mut cl = with_history(&["cmd1", "cmd2"]);
        cl.input = "draft".into();
        cl.history_prev();
        cl.history_next();
        assert_eq!(cl.input, "draft");
    }

    /// history_prev on an empty history must be a no-op (no panic).
    #[test]
    fn history_prev_empty_is_noop() {
        let mut cl = with_history(&[]); // explicitly empty, isolated from disk
        cl.history_prev(); // must not panic
        assert!(cl.input.is_empty());
    }
}

const APP_DIR: &str = "chuev-commander";

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::actions::Action;
use crate::events::{EventSender, ProgressData};
use crate::menu::{first_selectable, menu_entries, MenuItem, MENU_TITLES};
use crate::platform;
use crate::theme::{Theme, ThemeKind};
use crate::vfs::{ArchiveFormat, VfsFileInfo, VfsPath, VfsProvider};

// ─────────────────────────────────────────────────────────────────────────────
// Layout cache (populated by the render pass; consumed by mouse handling)
// ─────────────────────────────────────────────────────────────────────────────

/// Screen rects of the main interactive areas, updated every render frame.
#[derive(Debug, Default, Clone)]
pub struct LayoutCache {
    pub left_panel:  Option<Rect>,
    pub right_panel: Option<Rect>,
    pub output:      Option<Rect>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Sort
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortColumn {
    #[default]
    Name,
    Size,
    Modified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}

// ─────────────────────────────────────────────────────────────────────────────
// Panel
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelSide {
    Left,
    Right,
}

/// All state needed to display and navigate one panel.
#[derive(Debug)]
pub struct PanelState {
    pub current_path: VfsPath,

    /// Raw list from the provider (may include hidden files).
    entries_all: Vec<VfsFileInfo>,
    /// Filtered + sorted slice displayed in the panel.
    pub entries: Vec<VfsFileInfo>,

    pub selected_index: usize,
    /// Index of the first *visible* entry (virtual scroll offset).
    pub scroll_offset: usize,

    // ── Display / filter options ──────────────────────────────────────────
    pub sort_column:  SortColumn,
    pub sort_order:   SortOrder,
    pub show_hidden:  bool,
    /// Running quick-search query typed by the user (empty = inactive).
    pub quick_search: String,
    /// Cached free-space value for the current filesystem; refreshed on `load`.
    pub disk_free:    Option<u64>,

    /// Names of entries the user has marked with Space / Insert.
    /// Cleared on directory change.
    pub selected_names: HashSet<String>,

    /// `true` while Ctrl+S quick-search mode is active.
    /// In Variant-A routing, typed characters normally go to the command line;
    /// this flag redirects them to the panel's quick-search filter instead.
    pub search_mode: bool,
}

impl PanelState {
    pub fn new(path: VfsPath) -> Self {
        Self {
            current_path:   path,
            entries_all:    Vec::new(),
            entries:        Vec::new(),
            selected_index: 0,
            scroll_offset:  0,
            sort_column:    SortColumn::default(),
            sort_order:     SortOrder::default(),
            show_hidden:    false,
            quick_search:   String::new(),
            disk_free:      None,
            selected_names: HashSet::new(),
            search_mode:    false,
        }
    }

    // ── Loading ───────────────────────────────────────────────────────────

    /// Reload the entry list from the provider, then re-apply filters and sort.
    /// Resets cursor and clears the quick-search query.
    pub fn load(&mut self, provider: &dyn VfsProvider) {
        match provider.read_dir(&self.current_path) {
            Ok(raw) => {
                info!(
                    path  = %self.current_path.display_string(),
                    count = raw.len(),
                    "panel loaded"
                );
                self.entries_all = raw;
            }
            Err(e) => {
                warn!(
                    path  = %self.current_path.display_string(),
                    error = %e,
                    "failed to read directory"
                );
                self.entries_all = Vec::new();
            }
        }

        // Fetch disk free space once per directory change
        if let VfsPath::Local(ref p) = self.current_path {
            self.disk_free = platform::free_space_bytes(p);
        }

        self.quick_search.clear();
        self.search_mode = false;
        self.selected_names.clear();
        self.apply_filter_sort();
        self.selected_index = 0;
        self.scroll_offset  = 0;
    }

    // ── Filtering / sorting ───────────────────────────────────────────────

    /// Rebuild `self.entries` from `self.entries_all` using the current
    /// `show_hidden` and sort settings.  Tries to preserve cursor position.
    fn apply_filter_sort(&mut self) {
        let selected_name = self.entries.get(self.selected_index).map(|e| e.name.clone());

        self.entries = self
            .entries_all
            .iter()
            .filter(|e| self.show_hidden || !e.name.starts_with('.'))
            .cloned()
            .collect();

        let col   = self.sort_column;
        let order = self.sort_order;

        self.entries.sort_by(|a, b| {
            // Directories always sort before files regardless of column
            if a.is_dir != b.is_dir {
                return if a.is_dir {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                };
            }

            let cmp = match col {
                SortColumn::Name     => a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()),
                SortColumn::Size     => a.size.cmp(&b.size),
                SortColumn::Modified => a.modified.cmp(&b.modified),
            };

            if order == SortOrder::Desc { cmp.reverse() } else { cmp }
        });

        // Prepend ".." if a parent directory exists — always at index 0,
        // above any sorting.  navigate_into() treats it as a normal dir entry.
        if let Some(parent_path) = self.parent_vfs_path() {
            self.entries.insert(0, VfsFileInfo {
                name:          "..".into(),
                size:          None,
                is_dir:        true,
                is_symlink:    false,
                is_executable: false,
                modified:      None,
                permissions:   String::new(),
                path:          parent_path,
            });
        }

        // Restore cursor to the same entry name if it's still visible
        if let Some(name) = selected_name {
            if let Some(idx) = self.entries.iter().position(|e| e.name == name) {
                self.selected_index = idx.min(self.entries.len().saturating_sub(1));
                return;
            }
        }
        self.selected_index = 0;
    }

    /// Toggle sort column (same column → flip direction; new column → Asc).
    pub fn toggle_sort(&mut self, col: SortColumn) {
        if self.sort_column == col {
            self.sort_order = match self.sort_order {
                SortOrder::Asc  => SortOrder::Desc,
                SortOrder::Desc => SortOrder::Asc,
            };
        } else {
            self.sort_column = col;
            self.sort_order  = SortOrder::Asc;
        }
        self.apply_filter_sort();
    }

    /// Toggle visibility of dot-files (`.hidden`, `.config`, …).
    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.apply_filter_sort();
        self.selected_index = self.selected_index.min(self.entries.len().saturating_sub(1));
        info!(show_hidden = self.show_hidden, "hidden files toggled");
    }

    // ── Parent path ───────────────────────────────────────────────────────

    fn parent_vfs_path(&self) -> Option<VfsPath> {
        self.current_path.parent()
    }

    // ── Quick search ─────────────────────────────────────────────────────

    pub fn push_quick_search(&mut self, c: char) {
        self.quick_search.push(c);
        self.jump_to_search_match();
    }

    /// Remove the last character from the search query.
    pub fn pop_quick_search(&mut self) {
        self.quick_search.pop();
        self.jump_to_search_match();
    }

    pub fn clear_quick_search(&mut self) {
        self.quick_search.clear();
    }

    pub fn enter_search_mode(&mut self) {
        self.search_mode = true;
        self.quick_search.clear();
    }

    /// Exit search mode, clear the filter, and re-apply sort.
    pub fn exit_search_mode(&mut self) {
        self.search_mode = false;
        self.quick_search.clear();
        self.apply_filter_sort();
    }

    /// Move `selected_index` to the first entry whose name starts with the
    /// current query (case-insensitive).
    fn jump_to_search_match(&mut self) {
        if self.quick_search.is_empty() {
            return;
        }
        let q = self.quick_search.to_lowercase();
        if let Some(idx) = self
            .entries
            .iter()
            .position(|e| e.name.to_lowercase().starts_with(&q))
        {
            self.selected_index = idx;
        }
    }

    // ── Cursor movement ───────────────────────────────────────────────────

    pub fn move_up(&mut self) {
        self.selected_index = self.selected_index.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        if self.selected_index + 1 < self.entries.len() {
            self.selected_index += 1;
        }
    }

    pub fn page_up(&mut self, page_size: usize) {
        self.selected_index = self.selected_index.saturating_sub(page_size);
    }

    pub fn page_down(&mut self, page_size: usize) {
        let max = self.entries.len().saturating_sub(1);
        self.selected_index = (self.selected_index + page_size).min(max);
    }

    pub fn home(&mut self) { self.selected_index = 0; }

    pub fn end(&mut self) {
        if !self.entries.is_empty() {
            self.selected_index = self.entries.len() - 1;
        }
    }

    pub fn selected_entry(&self) -> Option<&VfsFileInfo> {
        self.entries.get(self.selected_index)
    }

    /// Toggle the selection mark on the current entry (skipping ".."), then
    /// advance the cursor one row.
    pub fn toggle_select(&mut self) {
        if let Some(entry) = self.entries.get(self.selected_index) {
            if entry.name != ".." {
                let name = entry.name.clone();
                if !self.selected_names.remove(&name) {
                    self.selected_names.insert(name);
                }
            }
        }
        self.move_down();
    }

    /// All currently marked entries, in display order.
    pub fn marked_entries(&self) -> Vec<&VfsFileInfo> {
        self.entries
            .iter()
            .filter(|e| self.selected_names.contains(&e.name))
            .collect()
    }

    pub fn clear_selection(&mut self) {
        self.selected_names.clear();
    }

    /// Adjust `scroll_offset` so that `selected_index` falls within the visible
    /// `visible_height` rows.  Called by the render pass.
    pub fn ensure_visible(&mut self, visible_height: usize) {
        if visible_height == 0 {
            return;
        }
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + visible_height {
            self.scroll_offset = self.selected_index + 1 - visible_height;
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Popup stack
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmAction {
    DeleteSelected,
    Quit,
    CopySelected { srcs: Vec<PathBuf>, dst_dir: PathBuf },
    MoveSelected { srcs: Vec<PathBuf>, dst_dir: PathBuf },
    /// Extract the whole archive to the other panel's directory.
    ExtractArchive { archive_path: PathBuf, format: ArchiveFormat, dst_dir: PathBuf },
    /// Extract specific entries from an archive to the other panel's directory.
    ExtractArchiveEntries {
        archive_path:   PathBuf,
        format:         ArchiveFormat,
        internal_paths: Vec<String>,
        dst_dir:        PathBuf,
    },
}

/// What to do when an `Input` popup is confirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputAction {
    MkDir,
    /// Rename `old_path` to a new name typed by the user (same directory).
    Rename { old_path: PathBuf },
    /// Create a ZIP archive named `value` in `dst_dir` from `srcs`.
    CreateArchive { srcs: Vec<PathBuf>, dst_dir: PathBuf },
}

/// Text / hex mode for the built-in viewer popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewerMode {
    Text,
    Hex,
}

/// A modal layer rendered on top of the panels.
///
/// `popup_stack: Vec<Popup>` replaces a sea of boolean flags like
/// `show_error`, `show_confirm_delete`, …  The top of the stack is shown;
/// closing it reveals the one underneath (useful for nested confirmations).
///
/// Note: `PartialEq + Eq` are intentionally NOT derived because
/// `Progress` contains a `CancellationToken` that doesn't implement them.
#[derive(Debug, Clone)]
pub enum Popup {
    Error(String),
    Confirm {
        title:             String,
        message:           String,
        action_on_confirm: ConfirmAction,
    },
    /// Single-line text-input dialog (F7 MkDir, future rename, etc.).
    Input {
        title:      String,
        prompt:     String,
        /// Live value typed by the user.
        value:      String,
        on_confirm: InputAction,
    },
    /// Shown while a background copy / move task is running.
    Progress {
        title:       String,
        source_name: String,
        bytes_done:  u64,
        bytes_total: u64,
        /// Whether the source should also be refreshed on completion (Move).
        is_move:     bool,
    },
    /// Built-in file viewer (F3) — text or hex mode.
    Viewer {
        title:    String,
        content:  Vec<u8>,
        mode:     ViewerMode,
        scroll_y: usize,
    },
    /// Top menu bar (F9) with optional open dropdown.
    Menu {
        top_idx: usize,
        sub_idx: usize,
        open:    bool,
    },
}

// ─────────────────────────────────────────────────────────────────────────────
// App
// ─────────────────────────────────────────────────────────────────────────────

pub struct App {
    pub left_panel:  PanelState,
    pub right_panel: PanelState,
    pub active_panel: PanelSide,

    // ── Layout flags — the ratatui Layout is built dynamically from these ─
    pub left_panel_visible:       bool,
    pub right_panel_visible:      bool,
    pub left_panel_width_percent: u16,

    pub popup_stack: Vec<Popup>,
    pub should_quit: bool,

    /// Set by the F4 handler; cleared and acted on by the event loop in
    /// `main.rs` which has access to the `Terminal` needed for suspend/restore.
    pub pending_edit: Option<PathBuf>,

    /// Set when the user presses Enter with a non-empty command line.
    /// Cleared and acted on by the event loop in `main.rs`.
    pub pending_shell: Option<(String, PathBuf)>,

    /// Accumulated output of all executed shell commands.
    /// Rendered in the panels area when panels are hidden (Ctrl+O).
    pub output_buffer: Vec<String>,
    /// Index of the first visible line in the output view.
    pub output_scroll: usize,

    /// Always-active command line (Variant A — default typing destination).
    pub cmdline: CmdLine,

    /// CancellationToken for the currently running copy/move task.
    /// `None` when no background I/O operation is active.
    pub cancel_token: Option<CancellationToken>,

    /// Active color theme — read by the render pass each frame.
    pub theme: Theme,

    /// Height of the panels area as a percentage of the available vertical
    /// space (excluding cmdline and status bar).  Range 10–100; default 100
    /// (panels fill everything).  Ctrl+Down shrinks, Ctrl+Up grows.
    pub panels_height_percent: u16,

    /// Text to copy to the system clipboard on the next event-loop iteration.
    /// Clipboard I/O happens in `main.rs` which owns the `arboard::Clipboard`.
    pub pending_clipboard_copy: Option<String>,
    /// When `true`, the event loop should read from the clipboard and append
    /// its text to the command line.
    pub pending_clipboard_paste: bool,

    /// Screen rectangles of the main areas, updated by the render pass.
    /// Used by `handle_mouse` to map click coordinates to panel entries.
    pub layout: LayoutCache,

    /// Sender half of the AppEvent channel — cloned into spawned I/O tasks.
    tx:       EventSender,
    provider: Arc<dyn VfsProvider>,
}

impl App {
    pub fn new(provider: Arc<dyn VfsProvider>, tx: EventSender) -> Result<Self> {
        let cwd = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("/"));

        let mut left_panel  = PanelState::new(VfsPath::Local(cwd.clone()));
        let mut right_panel = PanelState::new(VfsPath::Local(cwd));

        left_panel.load(provider.as_ref());
        right_panel.load(provider.as_ref());

        let mut app = Self {
            left_panel,
            right_panel,
            active_panel:             PanelSide::Left,
            left_panel_visible:       true,
            right_panel_visible:      true,
            left_panel_width_percent: 50,
            popup_stack:              Vec::new(),
            should_quit:              false,
            pending_edit:             None,
            pending_shell:            None,
            output_buffer:            Vec::new(),
            output_scroll:            0,
            cmdline:                  CmdLine::new(),
            cancel_token:             None,
            theme:                    Theme::from_kind(ThemeKind::Blue),
            panels_height_percent:    100,
            pending_clipboard_copy:   None,
            pending_clipboard_paste:  false,
            layout:                   LayoutCache::default(),
            tx,
            provider,
        };
        app.load_panel_state();
        Ok(app)
    }

    // ── Update entry point ────────────────────────────────────────────────

    pub fn update(&mut self, action: Action) {
        // Global shortcuts work regardless of popup / search state
        if action == Action::TogglePanelsVisible {
            let any_visible = self.left_panel_visible || self.right_panel_visible;
            self.left_panel_visible  = !any_visible;
            self.right_panel_visible = !any_visible;
            return;
        }
        if action == Action::PanelHeightGrow {
            self.panels_height_percent = (self.panels_height_percent + 10).min(100);
            return;
        }
        if action == Action::PanelHeightShrink {
            self.panels_height_percent = self.panels_height_percent.saturating_sub(10).max(10);
            return;
        }

        // When panels are hidden, scroll keys work on the output buffer;
        // cmdline / quit actions fall through to handle_panel_action as usual.
        if !self.left_panel_visible && !self.right_panel_visible {
            if self.handle_output_scroll(&action) {
                return;
            }
        }

        if !self.popup_stack.is_empty() {
            self.handle_popup_action(action);
            return;
        }
        // Ctrl+S quick-search mode intercepts typed characters
        if self.active_panel().search_mode {
            self.handle_search_action(action);
            return;
        }
        self.handle_panel_action(action);
    }

    /// Handle actions specific to the output view (panels hidden).
    /// Returns `true` if the action was consumed here.
    fn handle_output_scroll(&mut self, action: &Action) -> bool {
        match action {
            // Always browse history regardless of whether cmdline is empty
            Action::MoveUp   => { self.cmdline.history_prev(); true }
            Action::MoveDown => { self.cmdline.history_next(); true }
            Action::PageUp   => { self.output_scroll = self.output_scroll.saturating_sub(20); true }
            Action::PageDown => { self.output_scroll = self.output_scroll.saturating_add(20); true }
            Action::Home     => { self.output_scroll = 0; true }
            Action::End      => { self.output_scroll = self.output_buffer.len().saturating_sub(1); true }
            _ => false,
        }
    }

    // ── Quick-search mode (Ctrl+S) ────────────────────────────────────────

    /// Handles input while the active panel's quick-search filter is active.
    /// Navigation keys still work so the user can see results while filtering.
    fn handle_search_action(&mut self, action: Action) {
        match action {
            // Typed characters extend the filter
            Action::CmdlineChar(c) => self.active_panel_mut().push_quick_search(c),
            // Backspace: shrink filter; empty → exit search mode
            Action::NavigateUp => {
                let panel = self.active_panel_mut();
                if panel.quick_search.is_empty() {
                    panel.exit_search_mode();
                } else {
                    panel.pop_quick_search();
                }
            }
            // Esc or Enter: commit and exit search mode
            Action::PopupClose => self.active_panel_mut().exit_search_mode(),
            Action::NavigateInto => {
                self.active_panel_mut().exit_search_mode();
                self.navigate_into();
            }
            // Cursor movement still works during search
            Action::MoveUp   => self.active_panel_mut().move_up(),
            Action::MoveDown => self.active_panel_mut().move_down(),
            Action::PageUp   => self.active_panel_mut().page_up(20),
            Action::PageDown => self.active_panel_mut().page_down(20),
            Action::Home     => self.active_panel_mut().home(),
            Action::End      => self.active_panel_mut().end(),
            _ => {}
        }
    }

    // ── Panel-level actions ───────────────────────────────────────────────

    fn handle_panel_action(&mut self, action: Action) {
        match action {
            Action::Quit => {
                info!("quit requested");
                self.should_quit = true;
            }

            Action::TogglePanel => {
                self.active_panel = match self.active_panel {
                    PanelSide::Left  => PanelSide::Right,
                    PanelSide::Right => PanelSide::Left,
                };
                info!(panel = ?self.active_panel, "focus switched");
            }

            // ── Cursor movement ───────────────────────────────────────────
            // Up / Down: navigate history if cmdline has content, otherwise
            // move panel cursor.
            Action::MoveUp => {
                if self.cmdline.input.is_empty() {
                    self.active_panel_mut().move_up();
                } else {
                    self.cmdline.history_prev();
                }
            }
            Action::MoveDown => {
                if self.cmdline.input.is_empty() {
                    self.active_panel_mut().move_down();
                } else {
                    self.cmdline.history_next();
                }
            }
            Action::PageUp   => { self.active_panel_mut().page_up(20); }
            Action::PageDown => { self.active_panel_mut().page_down(20); }
            Action::Home     => { self.active_panel_mut().home(); }
            Action::End      => { self.active_panel_mut().end(); }

            // ── Navigation ────────────────────────────────────────────────
            // Enter: execute cmdline command if non-empty, else navigate into.
            Action::NavigateInto => {
                if self.cmdline.input.is_empty() {
                    self.navigate_into();
                } else {
                    let cmd = self.cmdline.take_input();
                    let cwd = match &self.active_panel().current_path {
                        VfsPath::Local(p) => Some(p.clone()),
                        _ => None,
                    };
                    if let Some(cwd) = cwd {
                        self.pending_shell = Some((cmd, cwd));
                    }
                }
            }
            // Backspace: always deletes the last cmdline character (command line first).
            // Navigate up via Enter on the ".." entry.
            Action::NavigateUp => {
                self.cmdline.backspace();
            }

            // ── Sorting ───────────────────────────────────────────────────
            Action::SortByName => self.active_panel_mut().toggle_sort(SortColumn::Name),
            Action::SortBySize => self.active_panel_mut().toggle_sort(SortColumn::Size),
            Action::SortByDate => self.active_panel_mut().toggle_sort(SortColumn::Modified),

            // ── Filtering / search ────────────────────────────────────────
            Action::ToggleHidden => self.active_panel_mut().toggle_hidden(),

            // Ctrl+S — activate quick-search mode (Variant A)
            Action::QuickSearchActivate => {
                self.cmdline.clear();
                self.active_panel_mut().enter_search_mode();
            }

            // Typed characters always go to the command line (Variant A)
            Action::CmdlineChar(c) => self.cmdline.push_char(c),

            // Esc: clear cmdline if non-empty
            Action::PopupClose => { self.cmdline.clear(); }

            // ── Command-line helpers ──────────────────────────────────────
            Action::CmdlineInsertName => {
                if let Some(entry) = self.active_panel().selected_entry() {
                    if entry.name != ".." {
                        let name = entry.name.clone();
                        if !self.cmdline.input.is_empty()
                            && !self.cmdline.input.ends_with(' ')
                        {
                            self.cmdline.input.push(' ');
                        }
                        self.cmdline.input.push_str(&name);
                    }
                }
            }
            Action::CmdlineClear => self.cmdline.clear(),

            // ── Refresh ───────────────────────────────────────────────────
            Action::Refresh => {
                let provider = Arc::clone(&self.provider);
                self.active_panel_mut().load(provider.as_ref());
            }

            // ── File operations ───────────────────────────────────────────
            Action::Edit => {
                // F4: request the event loop to open the selected file in
                // $VISUAL / $EDITOR.  We only set a flag here because
                // terminal suspend/restore requires the Terminal handle,
                // which lives in main.rs, not in App.
                let entry = self.active_panel().selected_entry().cloned();
                if let Some(entry) = entry {
                    // Don't try to edit ".." or directories
                    if !entry.is_dir {
                        if let VfsPath::Local(ref p) = entry.path {
                            self.pending_edit = Some(p.clone());
                        }
                    }
                }
            }

            Action::Select => self.active_panel_mut().toggle_select(),

            Action::Copy => self.start_copy_move(false),
            Action::Move => self.start_copy_move(true),
            Action::CreateArchive => self.start_create_archive(),

            Action::MakeDir => {
                self.popup_stack.push(Popup::Input {
                    title:      "Make Directory".into(),
                    prompt:     "Directory name:".into(),
                    value:      String::new(),
                    on_confirm: InputAction::MkDir,
                });
            }

            Action::Delete => {
                let panel = self.active_panel();
                let marked = panel.marked_entries();
                let message = if marked.is_empty() {
                    match panel.selected_entry() {
                        Some(e) if e.name != ".." => format!("Delete «{}»?", e.name),
                        _ => return,
                    }
                } else if marked.len() == 1 {
                    format!("Delete «{}»?", marked[0].name)
                } else {
                    format!("Delete {} marked items?", marked.len())
                };
                self.popup_stack.push(Popup::Confirm {
                    title:             "Delete".into(),
                    message,
                    action_on_confirm: ConfirmAction::DeleteSelected,
                });
            }

            Action::Rename => {
                let entry = self.active_panel().selected_entry().cloned();
                if let Some(entry) = entry {
                    if entry.name == ".." { return; }
                    match &entry.path {
                        VfsPath::Local(p) => {
                            self.popup_stack.push(Popup::Input {
                                title:      "Rename".into(),
                                prompt:     "New name:".into(),
                                value:      entry.name.clone(),
                                on_confirm: InputAction::Rename { old_path: p.clone() },
                            });
                        }
                        VfsPath::Archive { .. } => {
                            self.push_error("Rename inside archives is not supported.");
                        }
                    }
                }
            }

            Action::View => {
                let entry = self.active_panel().selected_entry().cloned();
                if let Some(entry) = entry {
                    if !entry.is_dir && entry.name != ".." {
                        const MAX: usize = 1 << 20; // 1 MiB
                        match self.provider.read_file(&entry.path) {
                            Ok(mut raw) => {
                                let truncated = raw.len() > MAX;
                                raw.truncate(MAX);
                                let title = if truncated {
                                    format!("{} [truncated at 1 MiB]", entry.name)
                                } else {
                                    entry.name.clone()
                                };
                                self.popup_stack.push(Popup::Viewer {
                                    title,
                                    content: raw,
                                    mode:     ViewerMode::Text,
                                    scroll_y: 0,
                                });
                            }
                            Err(e) => {
                                self.push_error(format!("Cannot read «{}»: {e}", entry.name));
                            }
                        }
                    }
                }
            }

            Action::Menu => {
                self.popup_stack.push(Popup::Menu {
                    top_idx: 0,
                    sub_idx: first_selectable(0),
                    open:    false,
                });
            }

            Action::SetTheme(kind) => {
                self.theme = Theme::from_kind(kind);
            }

            Action::CopyToClipboard => {
                let panel  = self.active_panel();
                let marked = panel.marked_entries();
                let text   = if marked.is_empty() {
                    match panel.selected_entry() {
                        Some(e) if e.name != ".." => path_to_string(&e.path),
                        _ => return,
                    }
                } else {
                    marked
                        .iter()
                        .map(|e| path_to_string(&e.path))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                self.pending_clipboard_copy = Some(text);
            }

            Action::PasteFromClipboard => {
                self.pending_clipboard_paste = true;
            }

            Action::MoveLeft => {}

            // Right arrow: accept history hint if one is visible, otherwise no-op.
            Action::MoveRight => {
                self.cmdline.accept_hint();
            }

            Action::None => {}
            other => { info!(action = ?other, "unhandled action"); }
        }
    }

    // ── Popup-level actions ───────────────────────────────────────────────

    fn handle_popup_action(&mut self, action: Action) {
        let top = self.popup_stack.last();
        let is_input    = matches!(top, Some(Popup::Input    { .. }));
        let is_progress = matches!(top, Some(Popup::Progress { .. }));
        let is_viewer   = matches!(top, Some(Popup::Viewer   { .. }));
        let is_menu     = matches!(top, Some(Popup::Menu     { .. }));

        if is_progress {
            // Only Esc is meaningful during a running operation
            if matches!(action, Action::PopupClose | Action::Quit) {
                if let Some(token) = self.cancel_token.take() {
                    token.cancel();
                }
                self.popup_stack.pop();
            }
        } else if is_input {
            self.handle_input_popup_action(action);
        } else if is_viewer {
            self.handle_viewer_action(action);
        } else if is_menu {
            self.handle_menu_action(action);
        } else {
            self.handle_passive_popup_action(action);
        }
    }

    /// Actions for `Popup::Input` — forwards typed characters into the value.
    fn handle_input_popup_action(&mut self, action: Action) {
        match action {
            Action::CmdlineChar(c) => {
                if let Some(Popup::Input { value, .. }) = self.popup_stack.last_mut() {
                    value.push(c);
                }
            }
            // Backspace removes the last char; does NOT navigate up
            Action::NavigateUp => {
                if let Some(Popup::Input { value, .. }) = self.popup_stack.last_mut() {
                    value.pop();
                }
            }
            Action::NavigateInto | Action::PopupConfirm => {
                if let Some(Popup::Input { value, on_confirm, .. }) = self.popup_stack.pop() {
                    let trimmed = value.trim().to_string();
                    if !trimmed.is_empty() {
                        self.execute_input_action(on_confirm, trimmed);
                    }
                }
            }
            Action::PopupClose | Action::Quit => { self.popup_stack.pop(); }
            _ => {}
        }
    }

    /// Scroll / mode-toggle / close actions while the viewer popup is on top.
    fn handle_viewer_action(&mut self, action: Action) {
        match action {
            Action::MoveUp => {
                if let Some(Popup::Viewer { scroll_y, .. }) = self.popup_stack.last_mut() {
                    *scroll_y = scroll_y.saturating_sub(1);
                }
            }
            Action::MoveDown => {
                let max = self.viewer_max_scroll();
                if let Some(Popup::Viewer { scroll_y, .. }) = self.popup_stack.last_mut() {
                    if *scroll_y < max { *scroll_y += 1; }
                }
            }
            Action::PageUp => {
                if let Some(Popup::Viewer { scroll_y, .. }) = self.popup_stack.last_mut() {
                    *scroll_y = scroll_y.saturating_sub(20);
                }
            }
            Action::PageDown => {
                let max = self.viewer_max_scroll();
                if let Some(Popup::Viewer { scroll_y, .. }) = self.popup_stack.last_mut() {
                    *scroll_y = (*scroll_y + 20).min(max);
                }
            }
            Action::Home => {
                if let Some(Popup::Viewer { scroll_y, .. }) = self.popup_stack.last_mut() {
                    *scroll_y = 0;
                }
            }
            Action::End => {
                let max = self.viewer_max_scroll();
                if let Some(Popup::Viewer { scroll_y, .. }) = self.popup_stack.last_mut() {
                    *scroll_y = max;
                }
            }
            // H / h — toggle between text and hex display
            Action::CmdlineChar('h' | 'H') => {
                if let Some(Popup::Viewer { mode, scroll_y, .. }) = self.popup_stack.last_mut() {
                    *mode = match *mode {
                        ViewerMode::Text => ViewerMode::Hex,
                        ViewerMode::Hex  => ViewerMode::Text,
                    };
                    *scroll_y = 0;
                }
            }
            // Q / q, Esc, F3 — close viewer
            Action::CmdlineChar('q' | 'Q')
            | Action::PopupClose
            | Action::View => {
                self.popup_stack.pop();
            }
            _ => {}
        }
    }

    /// Total number of scroll-lines for the top viewer popup in its current mode.
    fn viewer_max_scroll(&self) -> usize {
        if let Some(Popup::Viewer { content, mode, .. }) = self.popup_stack.last() {
            match mode {
                ViewerMode::Text => {
                    String::from_utf8_lossy(content)
                        .lines()
                        .count()
                        .saturating_sub(1)
                }
                ViewerMode::Hex => {
                    (content.len() + 15) / 16
                }
            }
        } else {
            0
        }
    }

    /// Actions for `Popup::Error` and `Popup::Confirm`.
    fn handle_passive_popup_action(&mut self, action: Action) {
        match action {
            Action::PopupClose | Action::Quit => { self.popup_stack.pop(); }
            Action::PopupConfirm | Action::NavigateInto => {
                if let Some(popup) = self.popup_stack.pop() {
                    if let Popup::Confirm { action_on_confirm, .. } = popup {
                        self.execute_confirm_action(action_on_confirm);
                    }
                }
            }
            _ => {}
        }
    }

    // ── Menu popup actions ────────────────────────────────────────────────

    fn handle_menu_action(&mut self, action: Action) {
        let (top_idx, sub_idx, open) = match self.popup_stack.last() {
            Some(Popup::Menu { top_idx, sub_idx, open }) => (*top_idx, *sub_idx, *open),
            _ => return,
        };
        let n_tops = MENU_TITLES.len();

        match action {
            Action::PopupClose | Action::Quit => {
                if open {
                    if let Some(Popup::Menu { open: o, .. }) = self.popup_stack.last_mut() {
                        *o = false;
                    }
                } else {
                    self.popup_stack.pop();
                }
            }
            Action::MoveLeft => {
                let new_top = if top_idx == 0 { n_tops - 1 } else { top_idx - 1 };
                if let Some(Popup::Menu { top_idx: t, sub_idx: s, .. }) = self.popup_stack.last_mut() {
                    *t = new_top;
                    *s = first_selectable(new_top);
                }
            }
            Action::MoveRight => {
                let new_top = (top_idx + 1) % n_tops;
                if let Some(Popup::Menu { top_idx: t, sub_idx: s, .. }) = self.popup_stack.last_mut() {
                    *t = new_top;
                    *s = first_selectable(new_top);
                }
            }
            Action::MoveUp => {
                if open {
                    self.menu_move_sub(-1, top_idx, sub_idx);
                }
            }
            Action::MoveDown => {
                if open {
                    self.menu_move_sub(1, top_idx, sub_idx);
                } else {
                    if let Some(Popup::Menu { open: o, sub_idx: s, .. }) = self.popup_stack.last_mut() {
                        *o = true;
                        *s = first_selectable(top_idx);
                    }
                }
            }
            Action::NavigateInto | Action::PopupConfirm => {
                if open {
                    self.menu_execute(top_idx, sub_idx);
                } else {
                    if let Some(Popup::Menu { open: o, sub_idx: s, .. }) = self.popup_stack.last_mut() {
                        *o = true;
                        *s = first_selectable(top_idx);
                    }
                }
            }
            _ => {}
        }
    }

    fn menu_move_sub(&mut self, delta: i32, top_idx: usize, sub_idx: usize) {
        let items = menu_entries(top_idx);
        let n = items.len();
        if n == 0 { return; }

        let mut next = sub_idx as i32 + delta;
        let mut tries = n;
        while tries > 0 {
            next = next.rem_euclid(n as i32);
            if items[next as usize].is_selectable() { break; }
            next += delta;
            tries -= 1;
        }

        if let Some(Popup::Menu { sub_idx: s, .. }) = self.popup_stack.last_mut() {
            *s = next as usize;
        }
    }

    fn menu_execute(&mut self, top_idx: usize, sub_idx: usize) {
        let items = menu_entries(top_idx);
        let action = match items.get(sub_idx) {
            Some(MenuItem::Entry { action, .. }) => action.clone(),
            _ => return,
        };

        self.popup_stack.pop();

        // For Left/Right panel menus, focus the corresponding panel first.
        match top_idx {
            0 => self.active_panel = PanelSide::Left,
            4 => self.active_panel = PanelSide::Right,
            _ => {}
        }

        self.handle_panel_action(action);
    }

    fn execute_confirm_action(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::Quit => { self.should_quit = true; }

            ConfirmAction::CopySelected { srcs, dst_dir } => {
                self.launch_operation(srcs, dst_dir, false);
            }
            ConfirmAction::MoveSelected { srcs, dst_dir } => {
                self.launch_operation(srcs, dst_dir, true);
            }

            ConfirmAction::ExtractArchive { archive_path, format, dst_dir } => {
                let cancel   = CancellationToken::new();
                let tx       = self.tx.clone();
                let src_name = archive_path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();

                self.popup_stack.push(Popup::Progress {
                    title:       "Extracting".into(),
                    source_name: src_name,
                    bytes_done:  0,
                    bytes_total: 0,
                    is_move:     false,
                });
                self.cancel_token = Some(cancel.clone());
                tokio::spawn(crate::ops::extract_archive(
                    archive_path, format, dst_dir, tx, cancel,
                ));
            }

            ConfirmAction::ExtractArchiveEntries { archive_path, format, internal_paths, dst_dir } => {
                let cancel   = CancellationToken::new();
                let tx       = self.tx.clone();
                let src_name = archive_path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();

                self.popup_stack.push(Popup::Progress {
                    title:       "Extracting".into(),
                    source_name: src_name,
                    bytes_done:  0,
                    bytes_total: 0,
                    is_move:     false,
                });
                self.cancel_token = Some(cancel.clone());
                tokio::spawn(crate::ops::extract_archive_entries(
                    archive_path, format, internal_paths, dst_dir, tx, cancel,
                ));
            }

            ConfirmAction::DeleteSelected => {
                // Collect: marked entries first, fall back to cursor entry
                let entries: Vec<(String, VfsPath)> = {
                    let panel = match self.active_panel {
                        PanelSide::Left  => &self.left_panel,
                        PanelSide::Right => &self.right_panel,
                    };
                    let marked = panel.marked_entries();
                    if marked.is_empty() {
                        panel.selected_entry()
                            .map(|e| vec![(e.name.clone(), e.path.clone())])
                            .unwrap_or_default()
                    } else {
                        marked.into_iter().map(|e| (e.name.clone(), e.path.clone())).collect()
                    }
                };

                let mut first_error: Option<String> = None;
                for (name, path) in &entries {
                    if let VfsPath::Local(ref p) = path {
                        let result = if p.is_dir() {
                            std::fs::remove_dir_all(p)
                        } else {
                            std::fs::remove_file(p)
                        };
                        match result {
                            Ok(()) => info!(path = %p.display(), "deleted"),
                            Err(e) => {
                                first_error = Some(format!("Cannot delete «{name}»: {e}"));
                                break;
                            }
                        }
                    }
                }

                if let Some(err) = first_error {
                    self.push_error(err);
                }

                let provider = Arc::clone(&self.provider);
                let panel = match self.active_panel {
                    PanelSide::Left  => &mut self.left_panel,
                    PanelSide::Right => &mut self.right_panel,
                };
                panel.clear_selection();
                panel.load(provider.as_ref());
            }
        }
    }

    fn execute_input_action(&mut self, action: InputAction, value: String) {
        match action {
            InputAction::Rename { old_path } => {
                let new_name = value.trim().to_string();
                if new_name.is_empty() || new_name == ".." {
                    return;
                }
                if let Some(parent) = old_path.parent() {
                    let new_path = parent.join(&new_name);
                    match std::fs::rename(&old_path, &new_path) {
                        Ok(()) => {
                            info!(
                                from = %old_path.display(),
                                to   = %new_path.display(),
                                "renamed"
                            );
                            let provider = Arc::clone(&self.provider);
                            self.active_panel_mut().load(provider.as_ref());
                        }
                        Err(e) => {
                            let old_name = old_path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            self.push_error(format!("Cannot rename «{old_name}»: {e}"));
                        }
                    }
                }
            }

            InputAction::MkDir => {
                let current = match self.active_panel {
                    PanelSide::Left  => self.left_panel.current_path.clone(),
                    PanelSide::Right => self.right_panel.current_path.clone(),
                };
                if let VfsPath::Local(ref dir) = current {
                    let new_dir = dir.join(&value);
                    match std::fs::create_dir(&new_dir) {
                        Ok(()) => {
                            info!(path = %new_dir.display(), "directory created");
                            let provider = Arc::clone(&self.provider);
                            let panel = self.active_panel_mut();
                            panel.load(provider.as_ref());
                            // Place cursor on the newly created directory
                            if let Some(idx) = panel.entries.iter().position(|e| e.name == value) {
                                panel.selected_index = idx;
                            }
                        }
                        Err(e) => {
                            self.popup_stack.push(Popup::Error(
                                format!("Cannot create «{value}»: {e}"),
                            ));
                        }
                    }
                }
            }

            InputAction::CreateArchive { srcs, dst_dir } => {
                // Ensure the name ends with .zip
                let name = if value.to_lowercase().ends_with(".zip") {
                    value
                } else {
                    format!("{value}.zip")
                };
                let dst_path = dst_dir.join(&name);
                let cancel   = CancellationToken::new();
                let tx       = self.tx.clone();

                self.popup_stack.push(Popup::Progress {
                    title:       "Creating Archive".into(),
                    source_name: name,
                    bytes_done:  0,
                    bytes_total: 0,
                    is_move:     false,
                });
                self.cancel_token = Some(cancel.clone());
                tokio::spawn(crate::ops::create_archive(srcs, dst_path, tx, cancel));
            }
        }
    }

    // ── Copy / Move ───────────────────────────────────────────────────────

    fn start_copy_move(&mut self, is_move: bool) {
        let other_path = match self.active_panel {
            PanelSide::Left  => self.right_panel.current_path.clone(),
            PanelSide::Right => self.left_panel.current_path.clone(),
        };
        let VfsPath::Local(dst_dir) = other_path else { return; };

        // F5 while browsing inside an archive → offer extraction
        if !is_move {
            if let VfsPath::Archive { archive_path, .. } =
                self.active_panel().current_path.clone()
            {
                let fname = archive_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                if let Some(format) = ArchiveFormat::detect(fname) {
                    // Collect marked entries; fall back to cursor entry.
                    let panel = self.active_panel();
                    let marked = panel.marked_entries();
                    let internal_paths: Vec<String> = if !marked.is_empty() {
                        marked.iter()
                            .filter(|e| e.name != "..")
                            .filter_map(|e| {
                                if let VfsPath::Archive { internal_path, .. } = &e.path {
                                    Some(internal_path.clone())
                                } else { None }
                            })
                            .collect()
                    } else {
                        panel.selected_entry()
                            .filter(|e| e.name != "..")
                            .and_then(|e| {
                                if let VfsPath::Archive { internal_path, .. } = &e.path {
                                    Some(vec![internal_path.clone()])
                                } else { None }
                            })
                            .unwrap_or_default()
                    };

                    if internal_paths.is_empty() {
                        // Cursor on ".." or nothing usable → extract entire archive
                        let msg = format!(
                            "Extract «{}» to\n{}?",
                            archive_path.file_name().unwrap_or_default().to_string_lossy(),
                            dst_dir.display()
                        );
                        self.popup_stack.push(Popup::Confirm {
                            title:             "Extract Archive".into(),
                            message:           msg,
                            action_on_confirm: ConfirmAction::ExtractArchive {
                                archive_path,
                                format,
                                dst_dir,
                            },
                        });
                    } else {
                        // Extract only the selected/marked entries
                        let display_names: Vec<String> = internal_paths.iter()
                            .map(|p| p.rsplit('/').next().unwrap_or(p.as_str()).to_owned())
                            .collect();
                        let msg = if display_names.len() == 1 {
                            format!("Extract «{}» to\n{}?", display_names[0], dst_dir.display())
                        } else {
                            format!(
                                "Extract {} items to\n{}?",
                                display_names.len(), dst_dir.display()
                            )
                        };
                        self.popup_stack.push(Popup::Confirm {
                            title:             "Extract".into(),
                            message:           msg,
                            action_on_confirm: ConfirmAction::ExtractArchiveEntries {
                                archive_path,
                                format,
                                internal_paths,
                                dst_dir,
                            },
                        });
                    }
                    return;
                }
            }
        }

        // Collect marked entries or fall back to the current cursor entry
        let panel = self.active_panel();
        let marked = panel.marked_entries();
        let srcs: Vec<PathBuf> = if marked.is_empty() {
            let entry = panel.selected_entry();
            match entry {
                Some(e) if e.name != ".." => {
                    if let VfsPath::Local(ref p) = e.path { vec![p.clone()] } else { return; }
                }
                _ => return,
            }
        } else {
            marked
                .into_iter()
                .filter_map(|e| if let VfsPath::Local(ref p) = e.path { Some(p.clone()) } else { None })
                .collect()
        };

        if srcs.is_empty() { return; }

        let verb  = if is_move { "Move" } else { "Copy" };
        let arrow = if is_move { "→ (move)" } else { "→" };
        let msg = if srcs.len() == 1 {
            format!(
                "«{}» {arrow}\n{}",
                srcs[0].file_name().unwrap_or_default().to_string_lossy(),
                dst_dir.display()
            )
        } else {
            format!("{} items {arrow}\n{}", srcs.len(), dst_dir.display())
        };

        let action = if is_move {
            ConfirmAction::MoveSelected { srcs, dst_dir }
        } else {
            ConfirmAction::CopySelected { srcs, dst_dir }
        };
        self.popup_stack.push(Popup::Confirm {
            title:             verb.into(),
            message:           msg,
            action_on_confirm: action,
        });
    }

    fn launch_operation(&mut self, srcs: Vec<PathBuf>, dst_dir: PathBuf, is_move: bool) {
        let cancel = CancellationToken::new();
        let tx     = self.tx.clone();

        // Rough total size (dirs show 0; accurate count comes from copy task).
        // block_in_place tells tokio the current thread may block briefly for
        // metadata syscalls, avoiding a stall on the async executor.
        let total: u64 = tokio::task::block_in_place(|| {
            srcs.iter()
                .map(|p| if p.is_dir() { 0 } else { p.metadata().map(|m| m.len()).unwrap_or(0) })
                .sum()
        });

        let src_name = if srcs.len() == 1 {
            srcs[0].file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            format!("{} items", srcs.len())
        };

        self.popup_stack.push(Popup::Progress {
            title:       if is_move { "Moving" } else { "Copying" }.into(),
            source_name: src_name,
            bytes_done:  0,
            bytes_total: total,
            is_move,
        });
        self.cancel_token = Some(cancel.clone());

        if is_move {
            tokio::spawn(crate::ops::move_batch(srcs, dst_dir, tx, cancel));
        } else {
            tokio::spawn(crate::ops::copy_batch(srcs, dst_dir, tx, cancel));
        }
    }

    /// Open an Input popup that asks for an archive name, then creates a ZIP.
    fn start_create_archive(&mut self) {
        if let VfsPath::Archive { .. } = self.active_panel().current_path.clone() {
            self.push_error("Cannot create archive inside another archive.");
            return;
        }

        let other_path = match self.active_panel {
            PanelSide::Left  => self.right_panel.current_path.clone(),
            PanelSide::Right => self.left_panel.current_path.clone(),
        };
        let VfsPath::Local(dst_dir) = other_path else {
            self.push_error("Destination must be a local directory.");
            return;
        };

        let panel  = self.active_panel();
        let marked = panel.marked_entries();
        let srcs: Vec<PathBuf> = if marked.is_empty() {
            match panel.selected_entry() {
                Some(e) if e.name != ".." => {
                    if let VfsPath::Local(ref p) = e.path { vec![p.clone()] } else { return; }
                }
                _ => return,
            }
        } else {
            marked.into_iter()
                .filter_map(|e| if let VfsPath::Local(ref p) = e.path { Some(p.clone()) } else { None })
                .collect()
        };

        if srcs.is_empty() { return; }

        let default_name = if srcs.len() == 1 {
            format!(
                "{}.zip",
                srcs[0].file_name().unwrap_or_default().to_string_lossy()
            )
        } else {
            "archive.zip".to_owned()
        };

        self.popup_stack.push(Popup::Input {
            title:      "Create ZIP Archive".into(),
            prompt:     format!("Archive name (in {}):", dst_dir.display()),
            value:      default_name,
            on_confirm: InputAction::CreateArchive { srcs, dst_dir },
        });
    }

    /// Called by the event loop when an `AppEvent::Progress` arrives.
    pub fn handle_progress(&mut self, data: ProgressData) {
        // Update the progress popup's counters
        if let Some(Popup::Progress { bytes_done, bytes_total, source_name, .. }) =
            self.popup_stack.last_mut()
        {
            *bytes_done  = data.bytes_done;
            *bytes_total = data.bytes_total;
            *source_name = data.source_name.clone();
        }

        if data.done {
            self.popup_stack.pop();
            self.cancel_token = None;

            // Refresh both panels so changes are visible immediately
            let provider = Arc::clone(&self.provider);
            self.left_panel.load(provider.as_ref());
            self.right_panel.load(provider.as_ref());
        }
    }

    // ── Directory traversal ───────────────────────────────────────────────

    fn navigate_into(&mut self) {
        let entry = match self.active_panel {
            PanelSide::Left  => self.left_panel.selected_entry().cloned(),
            PanelSide::Right => self.right_panel.selected_entry().cloned(),
        };
        if let Some(entry) = entry {
            // ".." is handled by navigate_up so cursor lands on the child we left.
            if entry.name == ".." {
                self.navigate_up();
                return;
            }
            if entry.is_dir {
                // Normal directory (or archive sub-dir) navigation
                let provider = Arc::clone(&self.provider);
                let panel = match self.active_panel {
                    PanelSide::Left  => &mut self.left_panel,
                    PanelSide::Right => &mut self.right_panel,
                };
                panel.current_path = entry.path;
                panel.load(provider.as_ref());
            } else if ArchiveFormat::detect(&entry.name).is_some() {
                // Enter any supported archive as a virtual directory
                if let VfsPath::Local(ref p) = entry.path {
                    let archive_path = p.clone();
                    let provider = Arc::clone(&self.provider);
                    let panel = match self.active_panel {
                        PanelSide::Left  => &mut self.left_panel,
                        PanelSide::Right => &mut self.right_panel,
                    };
                    panel.current_path = VfsPath::Archive {
                        archive_path,
                        internal_path: String::new(),
                    };
                    panel.load(provider.as_ref());
                }
            }
            // For other regular files Enter is a no-op (use F3/F4)
        }
    }

    fn navigate_up(&mut self) {
        let current = match self.active_panel {
            PanelSide::Left  => self.left_panel.current_path.clone(),
            PanelSide::Right => self.right_panel.current_path.clone(),
        };
        // Remember which child we're leaving so we can place the cursor on it.
        let child_name = current.last_component();
        if let Some(parent) = self.provider.parent(&current) {
            let provider = Arc::clone(&self.provider);
            let panel = match self.active_panel {
                PanelSide::Left  => &mut self.left_panel,
                PanelSide::Right => &mut self.right_panel,
            };
            panel.current_path = parent;
            panel.load(provider.as_ref());
            if let Some(name) = child_name {
                if let Some(idx) = panel.entries.iter().position(|e| e.name == name) {
                    panel.selected_index = idx;
                }
            }
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────

    pub fn active_panel(&self) -> &PanelState {
        match self.active_panel {
            PanelSide::Left  => &self.left_panel,
            PanelSide::Right => &self.right_panel,
        }
    }

    pub fn active_panel_mut(&mut self) -> &mut PanelState {
        match self.active_panel {
            PanelSide::Left  => &mut self.left_panel,
            PanelSide::Right => &mut self.right_panel,
        }
    }

    fn active_entry_name(&self) -> Option<String> {
        self.active_panel().selected_entry().map(|e| e.name.clone())
    }

    pub fn push_error(&mut self, message: impl Into<String>) {
        self.popup_stack.push(Popup::Error(message.into()));
    }

    pub fn reload_active_panel(&mut self) {
        let provider = Arc::clone(&self.provider);
        self.active_panel_mut().load(provider.as_ref());
    }

    /// Navigate the active panel to `path` and reload its contents.
    pub fn navigate_active_to(&mut self, path: PathBuf) {
        let provider = Arc::clone(&self.provider);
        let panel = self.active_panel_mut();
        panel.current_path = VfsPath::Local(path);
        panel.load(provider.as_ref());
    }

    // ── Panel state persistence ───────────────────────────────────────────

    fn panel_state_path() -> Option<PathBuf> {
        dirs::cache_dir().map(|d| d.join(APP_DIR).join("panel_state"))
    }

    /// Persist both panels' paths, cursor indices, theme, and panel height to disk.
    /// Format: six lines — left_path, left_idx, right_path, right_idx, theme_id, panels_height_pct.
    pub fn save_panel_state(&self) {
        let Some(path) = Self::panel_state_path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let left_dir  = self.left_panel.current_path.local_root().to_path_buf();
        let right_dir = self.right_panel.current_path.local_root().to_path_buf();
        let content = format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n",
            left_dir.display(),  self.left_panel.selected_index,
            right_dir.display(), self.right_panel.selected_index,
            self.theme.kind.id(),
            self.panels_height_percent,
        );
        let _ = std::fs::write(path, content);
    }

    /// Restore both panels' paths, cursor indices, and theme from disk.
    /// Silently ignores missing or malformed files and non-existent directories.
    fn load_panel_state(&mut self) {
        let Some(path) = Self::panel_state_path() else { return };
        let Ok(content) = std::fs::read_to_string(path) else { return };
        let lines: Vec<&str> = content.lines().collect();
        if lines.len() < 4 { return; }

        // Left panel
        let left_dir = PathBuf::from(lines[0]);
        if left_dir.is_dir() {
            let idx = lines[1].parse::<usize>().unwrap_or(0);
            self.left_panel.current_path = VfsPath::Local(left_dir);
            let provider = Arc::clone(&self.provider);
            self.left_panel.load(provider.as_ref());
            self.left_panel.selected_index =
                idx.min(self.left_panel.entries.len().saturating_sub(1));
        }

        // Right panel
        let right_dir = PathBuf::from(lines[2]);
        if right_dir.is_dir() {
            let idx = lines[3].parse::<usize>().unwrap_or(0);
            self.right_panel.current_path = VfsPath::Local(right_dir);
            let provider = Arc::clone(&self.provider);
            self.right_panel.load(provider.as_ref());
            self.right_panel.selected_index =
                idx.min(self.right_panel.entries.len().saturating_sub(1));
        }

        // Theme (optional 5th line — default to Blue if absent)
        if let Some(&theme_id) = lines.get(4) {
            self.theme = Theme::from_kind(ThemeKind::from_id(theme_id));
        }

        // Panel height percent (optional 6th line — default 100 if absent)
        if let Some(&pct_str) = lines.get(5) {
            if let Ok(pct) = pct_str.parse::<u16>() {
                self.panels_height_percent = pct.clamp(10, 100);
            }
        }
    }

    /// Handle a mouse event.  Called by the event loop in `main.rs`.
    pub fn handle_mouse(&mut self, event: MouseEvent) {
        let col = event.column;
        let row = event.row;

        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Click in left panel
                if let Some(rect) = self.layout.left_panel {
                    if in_rect(col, row, rect) {
                        self.active_panel = PanelSide::Left;
                        click_panel(&mut self.left_panel, col, row, rect);
                        return;
                    }
                }
                // Click in right panel
                if let Some(rect) = self.layout.right_panel {
                    if in_rect(col, row, rect) {
                        self.active_panel = PanelSide::Right;
                        click_panel(&mut self.right_panel, col, row, rect);
                        return;
                    }
                }
            }

            MouseEventKind::ScrollUp => {
                let panels_hidden = !self.left_panel_visible || !self.right_panel_visible;
                if panels_hidden {
                    self.output_scroll = self.output_scroll.saturating_sub(3);
                } else {
                    self.active_panel_mut().move_up();
                }
            }

            MouseEventKind::ScrollDown => {
                let panels_hidden = !self.left_panel_visible || !self.right_panel_visible;
                if panels_hidden {
                    self.output_scroll = self.output_scroll.saturating_add(3);
                } else {
                    self.active_panel_mut().move_down();
                }
            }

            _ => {}
        }
    }

    /// Append a command header and its captured output to the output buffer.
    /// Scrolls to the bottom so the latest output is visible.
    pub fn append_output(&mut self, command: &str, cwd: &Path, output: &str) {
        if !self.output_buffer.is_empty() {
            self.output_buffer.push(String::new());
        }
        self.output_buffer.push(format!("[{}]$ {}", cwd.display(), command));
        for line in output.lines() {
            self.output_buffer.push(line.to_owned());
        }
        if output.is_empty() {
            self.output_buffer.push(String::new());
        }
        // Scroll to bottom so the newest output is visible
        self.output_scroll = self.output_buffer.len().saturating_sub(1);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Free helpers
// ─────────────────────────────────────────────────────────────────────────────

fn in_rect(col: u16, row: u16, rect: Rect) -> bool {
    col >= rect.x
        && col < rect.x + rect.width
        && row >= rect.y
        && row < rect.y + rect.height
}

/// Move the cursor to the entry that was clicked.
/// Panels have 2 header rows (border line + column-name line).
fn click_panel(panel: &mut PanelState, _col: u16, row: u16, rect: Rect) {
    const HEADER_ROWS: u16 = 2;
    if row < rect.y + HEADER_ROWS { return; }
    let entry_row = (row - rect.y - HEADER_ROWS) as usize;
    let target    = panel.scroll_offset + entry_row;
    if target < panel.entries.len() {
        panel.selected_index = target;
    }
}

fn path_to_string(path: &VfsPath) -> String {
    match path {
        VfsPath::Local(p) => p.display().to_string(),
        VfsPath::Archive { archive_path, internal_path } => {
            format!("{}:{}", archive_path.display(), internal_path)
        }
    }
}
