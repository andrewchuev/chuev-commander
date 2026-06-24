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
use tracing::{debug, info, trace, warn};

// ─────────────────────────────────────────────────────────────────────────────
// Command line
// ─────────────────────────────────────────────────────────────────────────────

/// Persistent command-line state (Variant A: always-active, typing goes here).
#[derive(Debug)]
pub struct CmdLine {
    /// Text currently in the input field.
    pub input: String,
    /// Byte offset of the cursor within `input` (always on a char boundary).
    pub cursor_pos: usize,
    /// Byte offset of the selection anchor; `None` = no selection.
    /// The selection spans `min(anchor, cursor_pos)..max(anchor, cursor_pos)`.
    pub selection_anchor: Option<usize>,
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
            input:            String::new(),
            cursor_pos:       0,
            selection_anchor: None,
            history:          Self::load_history(),
            history_idx:      None,
            saved_input:      String::new(),
        }
    }

    /// Insert a character at the current cursor position; exits history-browsing mode.
    pub fn push_char(&mut self, c: char) {
        self.history_idx    = None;
        self.selection_anchor = None;
        self.input.insert(self.cursor_pos, c);
        self.cursor_pos += c.len_utf8();
    }

    /// Insert a string at the cursor position.
    pub fn insert_str(&mut self, s: &str) {
        self.selection_anchor = None;
        self.input.insert_str(self.cursor_pos, s);
        self.cursor_pos += s.len();
    }

    /// Delete the character at the cursor position (forward delete).
    pub fn delete_forward(&mut self) {
        self.history_idx = None;
        self.selection_anchor = None;
        if self.cursor_pos >= self.input.len() { return; }
        self.input.remove(self.cursor_pos);
    }

    /// Delete the character immediately before the cursor.
    /// Returns `false` if the cursor was already at the start.
    pub fn backspace(&mut self) -> bool {
        self.history_idx    = None;
        self.selection_anchor = None;
        if self.cursor_pos == 0 {
            return false;
        }
        let before = &self.input[..self.cursor_pos];
        let (byte_idx, _) = before.char_indices().next_back().unwrap();
        self.input.remove(byte_idx);
        self.cursor_pos = byte_idx;
        true
    }

    /// Clear input and exit history-browsing mode.
    pub fn clear(&mut self) {
        self.input.clear();
        self.cursor_pos   = 0;
        self.selection_anchor = None;
        self.history_idx  = None;
        self.saved_input.clear();
    }

    /// Move the cursor one character to the left, clearing any selection.
    pub fn move_cursor_left(&mut self) {
        self.selection_anchor = None;
        if self.cursor_pos == 0 { return; }
        let before = &self.input[..self.cursor_pos];
        self.cursor_pos = before.char_indices().next_back().map(|(i, _)| i).unwrap_or(0);
    }

    /// Move the cursor one character to the right, clearing any selection.
    pub fn move_cursor_right(&mut self) {
        self.selection_anchor = None;
        if self.cursor_pos >= self.input.len() { return; }
        let ch = self.input[self.cursor_pos..].chars().next().unwrap();
        self.cursor_pos += ch.len_utf8();
    }

    /// Extend (or start) the selection one character to the left.
    pub fn select_left(&mut self) {
        if self.selection_anchor.is_none() {
            self.selection_anchor = Some(self.cursor_pos);
        }
        if self.cursor_pos == 0 { return; }
        let before = &self.input[..self.cursor_pos];
        self.cursor_pos = before.char_indices().next_back().map(|(i, _)| i).unwrap_or(0);
    }

    /// Extend (or start) the selection one character to the right.
    pub fn select_right(&mut self) {
        if self.selection_anchor.is_none() {
            self.selection_anchor = Some(self.cursor_pos);
        }
        if self.cursor_pos >= self.input.len() { return; }
        let ch = self.input[self.cursor_pos..].chars().next().unwrap();
        self.cursor_pos += ch.len_utf8();
    }

    /// Returns the currently selected text, or `None` when there is no selection.
    pub fn selected_text(&self) -> Option<&str> {
        let anchor = self.selection_anchor?;
        let (start, end) = if anchor <= self.cursor_pos {
            (anchor, self.cursor_pos)
        } else {
            (self.cursor_pos, anchor)
        };
        if start == end { None } else { Some(&self.input[start..end]) }
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
        self.history_idx     = Some(idx);
        self.input           = self.history[idx].clone();
        self.cursor_pos      = self.input.len();
        self.selection_anchor = None;
    }

    /// Scroll to the next (newer) history entry, or back to the live input.
    pub fn history_next(&mut self) {
        let Some(idx) = self.history_idx else { return };
        if idx + 1 < self.history.len() {
            let next = idx + 1;
            self.history_idx = Some(next);
            self.input       = self.history[next].clone();
        } else {
            // Past the newest entry → restore live input
            self.history_idx = None;
            self.input       = self.saved_input.clone();
        }
        self.cursor_pos      = self.input.len();
        self.selection_anchor = None;
    }

    /// Return all history entries that contain `query` as a substring (most-recent first).
    /// When `query` is empty, returns the `HISTORY_MAX_MATCHES` most-recent entries.
    pub fn history_matches(&self, query: &str) -> Vec<String> {
        let q = query.to_lowercase();
        self.history.iter().rev()
            .filter(|h| q.is_empty() || h.to_lowercase().contains(&q))
            .take(HISTORY_MAX_MATCHES)
            .cloned()
            .collect()
    }

    /// Remove an entry from history and save the file.
    pub fn delete_entry(&mut self, entry: &str) {
        self.history.retain(|h| h != entry);
        self.save_history();
    }

    /// Take the current input, add it to history, clear the field.
    /// Returns the command string (trimmed).  Returns `""` if input was blank.
    pub fn take_input(&mut self) -> String {
        let cmd = self.input.trim().to_string();
        self.input.clear();
        self.cursor_pos      = 0;
        self.selection_anchor = None;
        self.history_idx     = None;
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
        cl.push_char('a');
        cl.push_char('b');
        cl.push_char('c');
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
        for c in "something".chars() { cl.push_char(c); }
        cl.clear();
        assert!(cl.input.is_empty());
        assert_eq!(cl.cursor_pos, 0);
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
        for c in "draft".chars() { cl.push_char(c); }
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

/// Rows scrolled per Page-Up / Page-Down keypress.
const PAGE_SIZE: usize = 20;
/// Maximum history entries returned by `CmdLine::history_matches`.
const HISTORY_MAX_MATCHES: usize = 20;

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
        // apply_filter_sort preserves the cursor on the same entry name when
        // reloading the same directory; it falls back to 0 on a new directory.
        self.apply_filter_sort();
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

        // Name sorts use sort_by_cached_key to compute the lowercase key once per
        // entry (O(N)) instead of once per comparison (O(N log N) allocations).
        // Size/Modified avoid String allocation entirely so sort_by is sufficient.
        match (col, order) {
            (SortColumn::Name, SortOrder::Asc) => {
                self.entries.sort_by_cached_key(|e| {
                    (!e.is_dir, e.name.to_ascii_lowercase())
                });
            }
            (SortColumn::Name, SortOrder::Desc) => {
                self.entries.sort_by_cached_key(|e| {
                    (!e.is_dir, std::cmp::Reverse(e.name.to_ascii_lowercase()))
                });
            }
            _ => {
                self.entries.sort_by(|a, b| {
                    if a.is_dir != b.is_dir {
                        return if a.is_dir {
                            std::cmp::Ordering::Less
                        } else {
                            std::cmp::Ordering::Greater
                        };
                    }
                    let cmp = match col {
                        SortColumn::Name     => unreachable!(),
                        SortColumn::Size     => a.size.cmp(&b.size),
                        SortColumn::Modified => a.modified.cmp(&b.modified),
                    };
                    if order == SortOrder::Desc { cmp.reverse() } else { cmp }
                });
            }
        }

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
        title:           String,
        content:         Vec<u8>,
        mode:            ViewerMode,
        scroll_y:        usize,
        /// Cached line count (text mode). Computed once at popup creation to
        /// avoid re-scanning the entire content on every scroll keypress.
        text_line_count: usize,
    },
    /// Top menu bar (F9) with optional open dropdown.
    Menu {
        top_idx: usize,
        sub_idx: usize,
        open:    bool,
    },
    /// Folder bookmarks manager (Ctrl+B).
    BookmarkManager {
        entries:  Vec<(u8, PathBuf)>,
        selected: usize,
    },
}

// ─────────────────────────────────────────────────────────────────────────────
// App
// ─────────────────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────────────────
// Deferred I/O
// ─────────────────────────────────────────────────────────────────────────────

/// An I/O operation that requires resources only available in `main.rs`
/// (the `Terminal` handle for TUI suspension, or `arboard::Clipboard`).
///
/// `App::update` sets `App::pending_action`; the event loop checks it on each
/// iteration and performs the actual work before the next frame.  At most one
/// action can be pending — the last `update()` call in an iteration wins.
#[derive(Debug)]
pub enum PendingAction {
    /// Suspend the TUI and open the file in `$VISUAL` / `$EDITOR`.
    Edit(PathBuf),
    /// Suspend the TUI and run this shell command through a PTY.
    Shell { cmd: String, cwd: PathBuf },
    /// Write this text to the system clipboard.
    ClipboardCopy(String),
    /// Read the system clipboard and insert its text into the command line.
    ClipboardPaste,
}

// ─────────────────────────────────────────────────────────────────────────────
// History-suggestion overlay (non-modal)
// ─────────────────────────────────────────────────────────────────────────────

/// State for the command-history suggestion overlay.
///
/// Deliberately **not** on `popup_stack` — see the doc-comment on
/// [`App::history_popup`] for the architectural reasoning.
#[derive(Debug, Default)]
pub struct HistoryPopupState {
    pub selected_idx: usize,
}

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

    /// I/O operation deferred to the event loop in `main.rs`.
    /// See [`PendingAction`] for the full set of variants and rationale.
    pub pending_action: Option<PendingAction>,

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


    /// Screen rectangles of the main areas, updated by the render pass.
    /// Used by `handle_mouse` to map click coordinates to panel entries.
    pub layout: LayoutCache,

    /// Folder bookmarks 0–9: `Some(path)` when set, `None` when empty.
    /// Persisted to `config_dir/chuev-commander/bookmarks`.
    pub bookmarks: [Option<PathBuf>; 10],

    // ── Two distinct overlay / popup mechanisms ───────────────────────────
    //
    // 1. `popup_stack` — modal layers that capture *all* keyboard input.
    //    Only the topmost entry is rendered and receives events.  Used for
    //    error dialogs, confirm dialogs, file viewer, menu, progress bars,
    //    bookmark manager.
    //
    // 2. `history_popup` — a non-modal overlay that *coexists* with the
    //    command line.  The cmdline remains editable while it is visible; it
    //    auto-appears/disappears based on the current input and is dismissed
    //    with Esc without losing the typed text.
    //
    // The split is intentional: putting the history overlay on the modal
    // stack would require pausing cmdline input while it is open, breaking
    // the "type and see suggestions" interaction model.

    /// Live history-suggestion overlay (`None` = hidden).
    pub history_popup: Option<HistoryPopupState>,
    /// Cmdline text at the moment the user last pressed Esc to dismiss the
    /// history overlay.  The overlay will not auto-reopen while
    /// `cmdline.input == history_popup_closed_for`.
    history_popup_closed_for: String,

    /// Sender half of the AppEvent channel — cloned into spawned I/O tasks.
    tx:       EventSender,
    provider: Arc<dyn VfsProvider>,
}

impl App {
    pub fn new(provider: Arc<dyn VfsProvider>, tx: EventSender) -> Result<Self> {
        let cwd = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("/"));

        // Panels are NOT loaded here — load_panel_state() restores saved paths and
        // calls load() itself.  We only fall back to cwd if no saved state exists,
        // avoiding a redundant read_dir on every startup.
        let left_panel  = PanelState::new(VfsPath::Local(cwd.clone()));
        let right_panel = PanelState::new(VfsPath::Local(cwd));

        let mut app = Self {
            left_panel,
            right_panel,
            active_panel:             PanelSide::Left,
            left_panel_visible:       true,
            right_panel_visible:      true,
            left_panel_width_percent: 50,
            popup_stack:              Vec::new(),
            should_quit:              false,
            pending_action:           None,
            output_buffer:            Vec::new(),
            output_scroll:            0,
            cmdline:                  CmdLine::new(),
            cancel_token:             None,
            theme:                    Theme::from_kind(ThemeKind::Blue),
            panels_height_percent:    100,
            layout:                   LayoutCache::default(),
            bookmarks:                 std::array::from_fn(|_| None),
            history_popup:             None,
            history_popup_closed_for:  String::new(),
            tx,
            provider,
        };
        app.load_panel_state();
        // Fallback: if load_panel_state found no saved file, panels are still
        // unloaded (entries empty).  Populate them from the cwd now.
        if app.left_panel.entries.is_empty() {
            let p = Arc::clone(&app.provider);
            app.left_panel.load(p.as_ref());
        }
        if app.right_panel.entries.is_empty() {
            let p = Arc::clone(&app.provider);
            app.right_panel.load(p.as_ref());
        }
        app.load_bookmarks();
        Ok(app)
    }

    // ── Update entry point ────────────────────────────────────────────────

    pub fn update(&mut self, action: Action) {
        // Global shortcuts work regardless of popup / search state
        if action == Action::TogglePanelsVisible {
            let any_visible = self.left_panel_visible || self.right_panel_visible;
            let new_vis = !any_visible;
            self.left_panel_visible  = new_vis;
            self.right_panel_visible = new_vis;
            debug!(panels_visible = new_vis, "panels: toggled");
            return;
        }
        if action == Action::PanelHeightGrow {
            self.panels_height_percent = (self.panels_height_percent + 10).min(100);
            debug!(pct = self.panels_height_percent, "panels: height grew");
            return;
        }
        if action == Action::PanelHeightShrink {
            self.panels_height_percent = self.panels_height_percent.saturating_sub(10).max(10);
            debug!(pct = self.panels_height_percent, "panels: height shrunk");
            return;
        }

        // When panels are hidden, scroll keys work on the output buffer;
        // cmdline / quit actions fall through to handle_panel_action as usual.
        if !self.left_panel_visible && !self.right_panel_visible && self.handle_output_scroll(&action) {
            return;
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
            Action::PageUp   => { self.output_scroll = self.output_scroll.saturating_sub(PAGE_SIZE); true }
            Action::PageDown => { self.output_scroll = self.output_scroll.saturating_add(PAGE_SIZE); true }
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
            Action::PageUp   => self.active_panel_mut().page_up(PAGE_SIZE),
            Action::PageDown => self.active_panel_mut().page_down(PAGE_SIZE),
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
                debug!(panel = ?self.active_panel, "panel: focus switched");
            }

            // ── Cursor movement ───────────────────────────────────────────
            // Up/Down: navigate history popup when it is open, otherwise move panel.
            Action::MoveUp => {
                if let Some(ref mut p) = self.history_popup {
                    p.selected_idx = p.selected_idx.saturating_sub(1);
                } else {
                    self.active_panel_mut().move_up();
                }
            }
            Action::MoveDown => {
                if self.history_popup.is_some() {
                    // total items = 1 empty + n matches; max valid idx = n
                    let n = self.cmdline.history_matches(&self.cmdline.input.clone()).len();
                    if let Some(ref mut p) = self.history_popup {
                        if p.selected_idx < n { p.selected_idx += 1; }
                    }
                } else {
                    self.active_panel_mut().move_down();
                }
            }
            Action::PageUp   => { self.active_panel_mut().page_up(PAGE_SIZE); }
            Action::PageDown => { self.active_panel_mut().page_down(PAGE_SIZE); }
            Action::Home     => { self.active_panel_mut().home(); }
            Action::End      => { self.active_panel_mut().end(); }

            // ── Navigation ────────────────────────────────────────────────
            // Enter: execute from history popup when open; else execute cmdline or navigate.
            // idx=0 is the blank "execute-typed" sentinel; idx>0 selects matches[idx-1].
            Action::NavigateInto => {
                if self.history_popup.is_some() {
                    let input   = self.cmdline.input.clone();
                    let matches = self.cmdline.history_matches(&input);
                    let idx     = self.history_popup.as_ref().map(|p| p.selected_idx).unwrap_or(0);

                    self.history_popup = None;
                    self.history_popup_closed_for.clear();

                    // Load history entry into cmdline when idx > 0
                    if idx > 0 {
                        if let Some(cmd) = matches.get(idx - 1) {
                            let cmd = cmd.clone();
                            debug!(cmd = %cmd, "history popup: command selected");
                            self.cmdline.clear();
                            self.cmdline.insert_str(&cmd);
                        }
                    }

                    // Execute whatever is now in cmdline (either typed or loaded from history)
                    if !self.cmdline.input.trim().is_empty() {
                        let cmd = tokio::task::block_in_place(|| self.cmdline.take_input());
                        if let VfsPath::Local(cwd) = self.active_panel().current_path.clone() {
                            info!(cmd = %cmd, cwd = %cwd.display(), "shell: command enqueued");
                            self.pending_action = Some(PendingAction::Shell { cmd, cwd });
                        }
                    }
                } else if self.cmdline.input.is_empty() {
                    self.navigate_into();
                } else {
                    let cmd = tokio::task::block_in_place(|| self.cmdline.take_input());
                    if let VfsPath::Local(cwd) = self.active_panel().current_path.clone() {
                        info!(cmd = %cmd, cwd = %cwd.display(), "shell: command enqueued");
                        self.pending_action = Some(PendingAction::Shell { cmd, cwd });
                    }
                }
            }
            // Backspace: delete last cmdline character and refresh history popup.
            Action::NavigateUp => {
                self.history_popup_closed_for.clear();
                self.cmdline.backspace();
                self.update_history_popup();
            }

            // ── Sorting ───────────────────────────────────────────────────
            Action::SortByName => {
                debug!(panel = ?self.active_panel, "sort: by name");
                self.active_panel_mut().toggle_sort(SortColumn::Name);
            }
            Action::SortBySize => {
                debug!(panel = ?self.active_panel, "sort: by size");
                self.active_panel_mut().toggle_sort(SortColumn::Size);
            }
            Action::SortByDate => {
                debug!(panel = ?self.active_panel, "sort: by date");
                self.active_panel_mut().toggle_sort(SortColumn::Modified);
            }

            // ── Filtering / search ────────────────────────────────────────
            Action::ToggleHidden => self.active_panel_mut().toggle_hidden(),

            // Ctrl+S — activate quick-search mode (Variant A)
            Action::QuickSearchActivate => {
                self.cmdline.clear();
                self.active_panel_mut().enter_search_mode();
            }

            // Typed characters go to cmdline; refresh history popup afterwards.
            Action::CmdlineChar(c) => {
                self.history_popup_closed_for.clear();
                self.cmdline.push_char(c);
                self.update_history_popup();
            }

            // Esc: close history popup if open; else clear cmdline.
            Action::PopupClose => {
                if self.history_popup.is_some() {
                    self.history_popup_closed_for = self.cmdline.input.clone();
                    self.history_popup = None;
                } else {
                    self.cmdline.clear();
                }
            }

            // ── Command-line helpers ──────────────────────────────────────
            Action::CmdlineInsertName => {
                if let Some(entry) = self.active_panel().selected_entry() {
                    if entry.name != ".." {
                        let name = entry.name.clone();
                        // Insert a space separator if needed
                        let need_space = !self.cmdline.input.is_empty()
                            && self.cmdline.cursor_pos > 0
                            && !self.cmdline.input[..self.cmdline.cursor_pos].ends_with(' ');
                        if need_space {
                            self.cmdline.insert_str(" ");
                        }
                        self.cmdline.insert_str(&name);
                    }
                }
            }
            Action::CmdlineClear => {
                self.history_popup = None;
                self.history_popup_closed_for.clear();
                self.cmdline.clear();
            }

            // Delete key: forward-delete the character at the cursor.
            Action::CmdlineDeleteForward => {
                self.history_popup_closed_for.clear();
                self.cmdline.delete_forward();
                self.update_history_popup();
            }

            // Shift+Delete: remove selected history entry from history.
            Action::HistoryDeleteEntry => {
                if self.history_popup.is_some() {
                    let input   = self.cmdline.input.clone();
                    let matches = self.cmdline.history_matches(&input);
                    let idx     = self.history_popup.as_ref().map(|p| p.selected_idx).unwrap_or(0);
                    // idx 0 = blank sentinel item — nothing to delete
                    if idx > 0 {
                        if let Some(entry) = matches.get(idx - 1) {
                            let entry = entry.clone();
                            info!(entry = %entry, "history: entry deleted");
                            tokio::task::block_in_place(|| self.cmdline.delete_entry(&entry));
                        } else {
                            debug!("history: Shift+Delete pressed but no entry at selected index");
                        }
                    }
                    self.update_history_popup();
                } else {
                    debug!("history: Shift+Delete pressed but history popup is not open");
                }
            }

            // ── Insert absolute path of current entry into cmdline ────────────
            Action::CmdlineInsertPath => {
                let panel = self.active_panel();
                let path_str = match panel.selected_entry() {
                    Some(e) if e.name != ".." => path_to_string(&e.path),
                    _ => path_to_string(&panel.current_path),
                };
                let need_space = !self.cmdline.input.is_empty()
                    && self.cmdline.cursor_pos > 0
                    && !self.cmdline.input[..self.cmdline.cursor_pos].ends_with(' ');
                if need_space { self.cmdline.insert_str(" "); }
                self.cmdline.insert_str(&path_str);
            }

            // ── Copy absolute path to clipboard (no selection/mark logic) ────
            Action::CopyAbsPathToClipboard => {
                let panel = self.active_panel();
                let path_str = match panel.selected_entry() {
                    Some(e) if e.name != ".." => path_to_string(&e.path),
                    _ => path_to_string(&panel.current_path),
                };
                self.pending_action = Some(PendingAction::ClipboardCopy(path_str));
            }

            Action::CopyOutputToClipboard => {
                if self.output_buffer.is_empty() {
                    debug!("copy output: buffer is empty, nothing to copy");
                } else {
                    let text = self.output_buffer.join("\n");
                    info!(lines = self.output_buffer.len(), "copy output: copied to clipboard");
                    self.pending_action = Some(PendingAction::ClipboardCopy(text));
                }
            }

            // ── Bookmarks ─────────────────────────────────────────────────
            Action::BookmarkGoto(n) => {
                let path = self.bookmarks[n as usize].clone();
                if let Some(p) = path {
                    info!(slot = n, path = %p.display(), "bookmark: goto");
                    self.navigate_active_to(p);
                } else {
                    debug!(slot = n, "bookmark: slot empty — showing error");
                    self.push_error(format!(
                        "Bookmark {} is not set.  Open the bookmark manager (Ctrl+B) and press Ins to add the current folder.",
                        n
                    ));
                }
            }

            Action::OpenBookmarkManager => {
                let entries: Vec<(u8, PathBuf)> = self.bookmarks.iter()
                    .enumerate()
                    .filter_map(|(i, opt)| opt.as_ref().map(|p| (i as u8, p.clone())))
                    .collect();
                debug!(count = entries.len(), "bookmark manager: opened");
                self.popup_stack.push(Popup::BookmarkManager { entries, selected: 0 });
            }

            // ── Refresh ───────────────────────────────────────────────────
            Action::Refresh => {
                debug!(panel = ?self.active_panel, "panel: manual refresh");
                let provider = Arc::clone(&self.provider);
                self.active_panel_mut().load(provider.as_ref());
            }

            // ── File operations ───────────────────────────────────────────
            Action::Edit => {
                let entry = self.active_panel().selected_entry().cloned();
                if let Some(entry) = entry {
                    if !entry.is_dir {
                        if let VfsPath::Local(ref p) = entry.path {
                            info!(path = %p.display(), "editor: enqueuing");
                            self.pending_action = Some(PendingAction::Edit(p.clone()));
                        }
                    }
                }
            }

            Action::Select => {
                let name = self.active_panel().selected_entry().map(|e| e.name.clone());
                trace!(entry = ?name, "select: toggled");
                self.active_panel_mut().toggle_select();
            }

            Action::Copy => {
                debug!(panel = ?self.active_panel, "copy: F5 pressed");
                self.start_copy_move(false);
            }
            Action::Move => {
                debug!(panel = ?self.active_panel, "move: F6 pressed");
                self.start_copy_move(true);
            }
            Action::CreateArchive => {
                debug!(panel = ?self.active_panel, "archive: create triggered");
                self.start_create_archive();
            }

            Action::MakeDir => {
                debug!(panel = ?self.active_panel, "mkdir: popup opened");
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
                let count = if marked.is_empty() { 1 } else { marked.len() };
                debug!(count, "delete: confirm popup opened");
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
                        debug!(file = %entry.name, "viewer: opening");
                        match self.provider.read_file(&entry.path) {
                            Ok(mut raw) => {
                                let truncated = raw.len() > MAX;
                                raw.truncate(MAX);
                                info!(
                                    file    = %entry.name,
                                    bytes   = raw.len(),
                                    truncated,
                                    "viewer: loaded"
                                );
                                let title = if truncated {
                                    format!("{} [truncated at 1 MiB]", entry.name)
                                } else {
                                    entry.name.clone()
                                };
                                let text_line_count = raw.iter()
                                    .filter(|&&b| b == b'\n')
                                    .count()
                                    .max(1);
                                self.popup_stack.push(Popup::Viewer {
                                    title,
                                    content: raw,
                                    mode:     ViewerMode::Text,
                                    scroll_y: 0,
                                    text_line_count,
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
                // Active cmdline selection takes priority over file paths.
                if let Some(sel) = self.cmdline.selected_text() {
                    debug!(bytes = sel.len(), "clipboard: copying cmdline selection");
                    self.pending_action = Some(PendingAction::ClipboardCopy(sel.to_owned()));
                    return;
                }
                let panel  = self.active_panel();
                let marked = panel.marked_entries();
                let text   = if marked.is_empty() {
                    match panel.selected_entry() {
                        Some(e) if e.name != ".." => path_to_string(&e.path),
                        _ => return,
                    }
                } else {
                    marked.iter().map(|e| path_to_string(&e.path)).collect::<Vec<_>>().join("\n")
                };
                self.pending_action = Some(PendingAction::ClipboardCopy(text));
            }

            Action::PasteFromClipboard => {
                self.pending_action = Some(PendingAction::ClipboardPaste);
            }

            // Left arrow: move the cmdline cursor one char left.
            Action::MoveLeft => {
                self.cmdline.move_cursor_left();
            }

            // Right arrow: move the cursor one character to the right.
            Action::MoveRight => {
                self.cmdline.move_cursor_right();
            }

            // Shift+arrows: extend the cmdline selection.
            Action::CmdlineSelectLeft  => { self.cmdline.select_left(); }
            Action::CmdlineSelectRight => { self.cmdline.select_right(); }

            Action::None => {}
            other => { info!(action = ?other, "unhandled action"); }
        }
    }

    // ── Popup-level actions ───────────────────────────────────────────────

    fn handle_popup_action(&mut self, action: Action) {
        let top = self.popup_stack.last();
        let is_input     = matches!(top, Some(Popup::Input           { .. }));
        let is_progress  = matches!(top, Some(Popup::Progress        { .. }));
        let is_viewer    = matches!(top, Some(Popup::Viewer          { .. }));
        let is_menu      = matches!(top, Some(Popup::Menu            { .. }));
        let is_bookmarks = matches!(top, Some(Popup::BookmarkManager { .. }));

        if is_progress {
            if matches!(action, Action::PopupClose | Action::Quit) {
                debug!("progress popup: cancelled by user");
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
        } else if is_bookmarks {
            self.handle_bookmark_manager_action(action);
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
                        debug!(value = %trimmed, action = ?on_confirm, "input popup: confirmed");
                        self.execute_input_action(on_confirm, trimmed);
                    } else {
                        debug!("input popup: confirmed with empty value — ignored");
                    }
                }
            }
            Action::PopupClose | Action::Quit => {
                debug!("input popup: cancelled");
                self.popup_stack.pop();
            }
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
                    *scroll_y = scroll_y.saturating_sub(PAGE_SIZE);
                }
            }
            Action::PageDown => {
                let max = self.viewer_max_scroll();
                if let Some(Popup::Viewer { scroll_y, .. }) = self.popup_stack.last_mut() {
                    *scroll_y = (*scroll_y + PAGE_SIZE).min(max);
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
        if let Some(Popup::Viewer { content, mode, text_line_count, .. }) = self.popup_stack.last() {
            match mode {
                ViewerMode::Text => text_line_count.saturating_sub(1),
                ViewerMode::Hex  => content.len().div_ceil(16),
            }
        } else {
            0
        }
    }

    // ── Bookmark-manager popup ────────────────────────────────────────────

    fn handle_bookmark_manager_action(&mut self, action: Action) {
        match action {
            Action::MoveUp => {
                if let Some(Popup::BookmarkManager { selected, .. }) = self.popup_stack.last_mut() {
                    *selected = selected.saturating_sub(1);
                }
            }
            Action::MoveDown => {
                if let Some(Popup::BookmarkManager { entries, selected }) = self.popup_stack.last_mut() {
                    let max = entries.len().saturating_sub(1);
                    if *selected < max { *selected += 1; }
                }
            }
            Action::NavigateInto | Action::PopupConfirm => {
                if let Some(Popup::BookmarkManager { entries, selected }) = self.popup_stack.last() {
                    let path = entries.get(*selected).map(|(_, p)| p.clone());
                    self.popup_stack.pop();
                    if let Some(p) = path {
                        self.navigate_active_to(p);
                    }
                }
            }
            // Insert — add current panel folder to the first free bookmark slot
            Action::Select => {
                if let VfsPath::Local(ref p) = self.active_panel().current_path.clone() {
                    if let Some(slot) = self.bookmarks.iter().position(|b| b.is_none()) {
                        info!(slot, path = %p.display(), "bookmark: added via popup Insert");
                        self.bookmarks[slot] = Some(p.clone());
                        tokio::task::block_in_place(|| self.save_bookmarks());
                        let new_entries: Vec<(u8, PathBuf)> = self.bookmarks.iter()
                            .enumerate()
                            .filter_map(|(i, opt)| opt.as_ref().map(|p| (i as u8, p.clone())))
                            .collect();
                        let new_selected = new_entries.len().saturating_sub(1);
                        if let Some(Popup::BookmarkManager { entries, selected }) = self.popup_stack.last_mut() {
                            *selected = new_selected;
                            *entries  = new_entries;
                        }
                    } else {
                        debug!("bookmark: all 10 slots full");
                        self.push_error(
                            "All 10 bookmark slots are occupied. Delete one first (Del key)."
                        );
                    }
                }
            }
            Action::CmdlineDeleteForward => {
                // Extract which bookmark slot to clear
                let slot: Option<usize> = match self.popup_stack.last() {
                    Some(Popup::BookmarkManager { entries, selected }) => {
                        entries.get(*selected).map(|(n, _)| *n as usize)
                    }
                    _ => None,
                };
                if let Some(idx) = slot {
                    self.bookmarks[idx] = None;
                    tokio::task::block_in_place(|| self.save_bookmarks());
                    // Rebuild entries in the popup
                    let new_entries: Vec<(u8, PathBuf)> = self.bookmarks.iter()
                        .enumerate()
                        .filter_map(|(i, opt)| opt.as_ref().map(|p| (i as u8, p.clone())))
                        .collect();
                    if let Some(Popup::BookmarkManager { entries, selected }) = self.popup_stack.last_mut() {
                        *selected = (*selected).min(new_entries.len().saturating_sub(1));
                        *entries  = new_entries;
                    }
                }
            }
            Action::PopupClose | Action::Quit => {
                self.popup_stack.pop();
            }
            _ => {}
        }
    }

    /// Actions for `Popup::Error` and `Popup::Confirm`.
    fn handle_passive_popup_action(&mut self, action: Action) {
        match action {
            Action::PopupClose | Action::Quit => {
                if let Some(Popup::Error(msg)) = self.popup_stack.last() {
                    debug!(message = %msg, "error popup: dismissed");
                } else {
                    debug!("confirm popup: cancelled");
                }
                self.popup_stack.pop();
            }
            Action::PopupConfirm | Action::NavigateInto => {
                if let Some(Popup::Confirm { action_on_confirm, .. }) = self.popup_stack.pop() {
                    info!(action = ?action_on_confirm, "confirm popup: accepted");
                    self.execute_confirm_action(action_on_confirm);
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

                info!(count = entries.len(), "delete: starting");
                let mut first_error: Option<String> = None;
                for (name, path) in &entries {
                    if let VfsPath::Local(ref p) = path {
                        let result = if p.is_dir() {
                            std::fs::remove_dir_all(p)
                        } else {
                            std::fs::remove_file(p)
                        };
                        match result {
                            Ok(()) => info!(path = %p.display(), "delete: entry removed"),
                            Err(e) => {
                                warn!(path = %p.display(), error = %e, "delete: failed");
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

        info!(
            is_move,
            count       = srcs.len(),
            total_bytes = total,
            dst         = %dst_dir.display(),
            "file-op: launched"
        );

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
            info!(source = %data.source_name, "file-op: completed");
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
                debug!(
                    panel  = ?self.active_panel,
                    target = %entry.path.display_string(),
                    "navigate: into directory"
                );
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
                    debug!(
                        panel   = ?self.active_panel,
                        archive = %archive_path.display(),
                        "navigate: opening archive"
                    );
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
            } else {
                trace!(file = %entry.name, "navigate: Enter on regular file (no-op)");
            }
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
            debug!(
                panel = ?self.active_panel,
                from  = %current.display_string(),
                to    = %parent.display_string(),
                "navigate: up"
            );
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
        } else {
            trace!(panel = ?self.active_panel, "navigate: already at root");
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

    /// Recompute whether the history suggestion popup should be visible.
    /// Call after every cmdline text modification.
    pub fn update_history_popup(&mut self) {
        // Never show while a modal popup is blocking input
        if !self.popup_stack.is_empty() {
            self.history_popup = None;
            return;
        }
        let input = self.cmdline.input.clone();
        if input.is_empty() {
            if self.history_popup.is_some() {
                trace!("history popup: hidden (input cleared)");
            }
            self.history_popup = None;
            self.history_popup_closed_for.clear();
            return;
        }
        // Respect the user's explicit Esc dismissal for this exact input
        if self.history_popup.is_none() && input == self.history_popup_closed_for {
            return;
        }
        let matches = self.cmdline.history_matches(&input);
        if matches.is_empty() {
            if self.history_popup.is_some() {
                trace!("history popup: hidden (no matches)");
            }
            self.history_popup = None;
        } else {
            let n = matches.len();
            if self.history_popup.is_none() {
                trace!(matches = n, input = %input, "history popup: shown");
                self.history_popup = Some(HistoryPopupState { selected_idx: 0 });
            }
            if let Some(ref mut p) = self.history_popup {
                // total items = 1 blank sentinel + n matches; max valid idx = n
                p.selected_idx = p.selected_idx.min(n);
            }
        }
    }

    /// Clear the output buffer and reset the scroll position.
    /// Invoked when the user types `clear` or `cls` in the command line.
    pub fn clear_output_buffer(&mut self) {
        info!(lines = self.output_buffer.len(), "output buffer: cleared");
        self.output_buffer.clear();
        self.output_scroll = 0;
    }

    pub fn push_error(&mut self, message: impl Into<String>) {
        let msg: String = message.into();
        warn!(message = %msg, popup_depth = self.popup_stack.len(), "error popup");
        self.popup_stack.push(Popup::Error(msg));
    }

    pub fn reload_active_panel(&mut self) {
        debug!(panel = ?self.active_panel, "panel: reload after shell command");
        let provider = Arc::clone(&self.provider);
        self.active_panel_mut().load(provider.as_ref());
    }

    /// Navigate the active panel to `path` and reload its contents.
    pub fn navigate_active_to(&mut self, path: PathBuf) {
        debug!(panel = ?self.active_panel, path = %path.display(), "navigate: active panel to");
        let provider = Arc::clone(&self.provider);
        let panel = self.active_panel_mut();
        panel.current_path = VfsPath::Local(path);
        panel.load(provider.as_ref());
    }

    // ── Bookmark persistence ──────────────────────────────────────────────

    fn bookmarks_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join(APP_DIR).join("bookmarks"))
    }

    /// Persist the bookmark array to disk (one path per line; empty line = unset).
    pub fn save_bookmarks(&self) {
        let Some(path) = Self::bookmarks_path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let content: String = self.bookmarks.iter()
            .map(|b| match b {
                Some(p) => p.display().to_string(),
                None    => String::new(),
            })
            .collect::<Vec<_>>()
            .join("\n") + "\n";
        let _ = std::fs::write(path, content);
    }

    /// Load bookmarks from disk; silently ignores missing/malformed files.
    fn load_bookmarks(&mut self) {
        let Some(path) = Self::bookmarks_path() else { return };
        let Ok(content) = std::fs::read_to_string(path) else { return };
        let mut count = 0usize;
        for (i, line) in content.lines().enumerate().take(10) {
            if !line.is_empty() {
                let p = PathBuf::from(line);
                if p.is_dir() {
                    self.bookmarks[i] = Some(p);
                    count += 1;
                }
            }
        }
        debug!(loaded = count, "bookmarks: restored from disk");
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
        debug!(
            left  = %left_dir.display(),
            right = %right_dir.display(),
            "panel state: saved"
        );
        let _ = std::fs::write(path, content);
    }

    /// Restore both panels' paths, cursor indices, and theme from disk.
    /// Silently ignores missing or malformed files and non-existent directories.
    fn load_panel_state(&mut self) {
        let Some(path) = Self::panel_state_path() else {
            debug!("panel state: no cache dir, skipping restore");
            return;
        };
        let Ok(content) = std::fs::read_to_string(&path) else {
            debug!(file = %path.display(), "panel state: file not found, using defaults");
            return;
        };
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

        debug!(
            left  = %self.left_panel.current_path.display_string(),
            right = %self.right_panel.current_path.display_string(),
            theme = %self.theme.kind.id(),
            "panel state: restored"
        );
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
        info!(
            command,
            cwd        = %cwd.display(),
            lines      = output.lines().count(),
            bytes      = output.len(),
            buf_before = self.output_buffer.len(),
            "output buffer: appended"
        );
        // Blank separator between successive commands (visual breathing room).
        if !self.output_buffer.is_empty() {
            self.output_buffer.push(String::new());
        }
        self.output_buffer.push(format!("[{}]$ {}", cwd.display(), command));
        for line in output.lines() {
            self.output_buffer.push(line.to_owned());
        }
        // No trailing empty line: the next command's separator already provides it.
        // (Previously an extra empty line was added when output was empty, causing
        // double-spacing for no-output commands like `clear`.)

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
