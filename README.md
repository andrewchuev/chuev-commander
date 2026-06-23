# chuev-commander

A terminal-based dual-panel file manager written in Rust, inspired by the classic Far Manager / Midnight Commander workflow.

```
╔═ /home/user  ↑Name  45.2 G free ═══════════════════╗╔═ /tmp ═════════════════════════════════════════════╗
║  ↑Name                           Size    Date      ║║  Name                            Size    Date      ║
║────────────────────────────────────────────────────║║────────────────────────────────────────────────────║
║▸ Documents/                     <DIR>    2024-01-15║║  cache/                         <DIR>    2024-01-14║
║  .config/                       <DIR>    2024-01-12║║  logs/                          <DIR>    2024-01-13║
║  README.md                        12.1 K 2024-01-10║║  output.log                        4.1 K 2024-01-15║
║  Cargo.toml                        1.2 K 2024-01-09║║                                                    ║
╚════════════════════════════════════════════════════╝╚════════════════════════════════════════════════════╝
[/home/user]$ ls -la█
F2 Refresh  F3 View  F4 Edit  F5 Copy/Extract  F6 Move  F7 MkDir  F8 Del  F9 Menu  F10 Quit
```

## Features

- **Dual-panel layout** with configurable split ratio; panels framed with classic double-line borders (`╔═╗║╚╝`)
- **Virtual filesystem (VFS)** — transparent navigation into ZIP, TAR, TAR.GZ, and TAR.BZ2 archives alongside local directories; auto-detected by extension
- **Archive browsing** — enter any supported archive with Enter; navigate internal subdirectories; view files inside archives with F3; modification dates and Unix permissions are shown for each archive entry
- **Archive extraction** (F5 inside archive) — context-sensitive:
  - Cursor on a file/directory → extracts that entry only to the other panel
  - Multiple entries marked with Insert → extracts all marked entries
  - Cursor on `..` (or nothing marked) → extracts the entire archive
- **ZIP archive creation** (Shift+F5) — packs selected/marked files and directories into a new compressed ZIP; an input popup lets you name the archive; created in the other panel's directory
- **Always-active command line** — every printable key types into the command bar; Enter executes
- **`cd` navigation** — typing `cd <path>` navigates the active panel directly; supports `~`, `~/path`, relative, and absolute paths; errors shown as a popup
- **Shell output buffer** — captured stdout/stderr of commands displayed in-panel (Ctrl+O toggles)
- **Command history** — Up/Down browses previous commands; persisted to disk across sessions
- **Quick search** (Ctrl+S) — incremental prefix search jumps the cursor as you type
- **File viewer** (F3) — text and hex modes with scrolling; works on files inside archives
- **External editor** (F4) — opens `$VISUAL` / `$EDITOR` / `vi`; keyboard input is fully handed over to the editor, TUI restores cleanly on exit
- **Copy / Move** (F5/F6) — async background transfer with live per-chunk progress bar and cancellation
- **Delete** (F8) — with confirmation; supports bulk operations on marked files
- **MkDir** (F7) and **Rename** (Shift+F6) — inline input dialog; cursor moves to the newly created directory after F7
- **File marking** (Insert) — mark multiple entries for batch operations
- **Sort** by name, size, or date (Ctrl+F3/F5/F6); toggles Asc/Desc
- **Hidden files** toggle (Ctrl+H)
- **Disk free space** shown in panel header
- **Adjustable panel height** — Ctrl+Down shrinks both panels to reveal a shell output strip below; Ctrl+Up grows them back; height (10–100 %) is saved and restored across sessions
- **Panel state persistence** — both panels' paths, cursor positions, active theme, and panel height are saved on exit and restored on next launch; stored in `$XDG_CACHE_HOME/chuev-commander/panel_state`
- **Navigate-up cursor placement** — when going to the parent directory (Backspace or Enter on `..`), the cursor lands on the directory just left
- **System clipboard** — Ctrl+C copies the selected/marked entry paths to the clipboard; Ctrl+V pastes clipboard text into the command line (requires a running display server)
- **Mouse support** — left-click focuses a panel and moves the cursor to the clicked entry; scroll wheel scrolls entries in the active panel (or the output buffer in Ctrl+O mode)
- **Main menu** (F9) — horizontal menu bar with keyboard-navigable dropdowns: Left, Files, Commands, Options, Right
- **Color schemes** — two built-in themes selectable from the Options menu: *Blue Classic* (default) and *Dos Navigator*; theme persists between launches
- **F-key bar** — proportionally stretched across the full terminal width
- **File-based logging** — all output goes to `$XDG_CACHE_HOME/chuev-commander/debug.log`; terminal is never polluted

## Key Bindings

| Key | Action |
|-----|--------|
| `↑` / `↓` | Move cursor (or browse command history when cmdline is non-empty) |
| `PgUp` / `PgDn` | Page up / page down |
| `Home` / `End` | Jump to first / last entry |
| `Enter` | Navigate into directory, or execute command line |
| `Backspace` | Delete last character in the command line |
| `Tab` | Switch active panel |
| `Insert` | Toggle mark on current entry |
| `Ctrl+S` | Activate quick-search mode |
| `Ctrl+H` | Toggle hidden files |
| `Ctrl+O` | Toggle panels / shell output buffer view |
| `Ctrl+↑` / `Ctrl+↓` | Grow / shrink panels height |
| `Ctrl+C` | Copy selected/marked entry paths to system clipboard |
| `Ctrl+V` | Paste clipboard text into command line |
| `Ctrl+U` | Clear command line |
| `Ctrl+Enter` | Insert current filename into command line |
| `←` / `→` | Navigate between top-level menu items (when menu is open) |
| `Ctrl+Q` | Quit |
| `F2` | Refresh current directory |
| `F3` | View file (text / hex) |
| `F4` | Edit file in `$EDITOR` |
| `F5` | Copy to other panel; inside an archive — extract selected entry (or whole archive if on `..`) |
| `F6` | Move to other panel |
| `F7` | Create directory |
| `F8` | Delete (with confirmation) |
| `F9` | Open main menu |
| `F10` | Quit |
| `Shift+F5` | Create ZIP archive from selected/marked files |
| `Shift+F6` | Rename |
| `Ctrl+F3` | Sort by name |
| `Ctrl+F5` | Sort by date |
| `Ctrl+F6` | Sort by size |
| `Esc` | Close popup / clear command line |

## Architecture

The application follows the **Model-View-Update (MVU)** pattern:

```
KeyEvent
   │
   ▼
key_event_to_action()   ← actions.rs: translates raw key codes to semantic Actions
   │
   ▼
App::update(Action)     ← app.rs: pure state transition, no I/O
   │
   ▼
ui::render(Frame, App)  ← ui/: reads App, draws widgets — never mutates state
```

`App` is the single source of truth. UI widgets read it; the event loop writes it through `update`. No widget ever mutates state directly.

### Module Overview

| Module | Responsibility |
|--------|---------------|
| `main.rs` | Entry point — terminal init/restore, async event loop, producer tasks; `cd` built-in handling |
| `events.rs` | `AppEvent` enum and typed `mpsc` channel shared by all producers |
| `actions.rs` | `Action` enum + `key_event_to_action` — single point of key-binding logic |
| `app.rs` | `App` struct (full application state), `PanelState`, popup stack, `CmdLine` |
| `theme.rs` | `Theme` struct + `ThemeKind` enum; two built-in color schemes |
| `menu.rs` | Menu bar titles and per-dropdown item definitions |
| `vfs/mod.rs` | `VfsProvider` trait, `VfsPath`, `VfsFileInfo`, `ArchiveFormat::detect` |
| `vfs/local.rs` | `LocalFsProvider` — reads the local filesystem |
| `vfs/archive.rs` | ZIP/TAR listing, reading, extraction, selective entry extraction, ZIP creation |
| `vfs/router.rs` | `RoutingProvider` — dispatches by `VfsPath` variant and `ArchiveFormat` |
| `platform.rs` | Unix `statvfs` wrapper for free-space query |
| `editor.rs` | TUI suspend/restore around `$EDITOR` launch |
| `shell.rs` | Interactive shell execution with stdout/stderr capture |
| `ops.rs` | Async copy/move/extract/create-archive with `CancellationToken` and per-entry progress |
| `ui/mod.rs` | Top-level `render` function, layout split |
| `ui/panels.rs` | Dual-panel file listing with column layout |
| `ui/cmdline.rs` | Command-line bar renderer |
| `ui/output.rs` | Shell output buffer view (Ctrl+O mode) |
| `ui/menu.rs` | F9 menu bar and dropdown overlay rendering |
| `ui/popups.rs` | Modal stack: Error, Confirm, Input, Progress, Viewer, Menu |
| `ui/status.rs` | F-key hint bar |

### Virtual Filesystem

```
VfsPath::Local(PathBuf)
VfsPath::Archive { archive_path, internal_path }
         │
         ▼
   VfsProvider (trait)
         │
    ┌────┴──────────────────────────────────────┐
LocalProvider  ArchiveProvider (zip/tar/tgz/tbz2)
                     │
              RoutingProvider   ← dispatches by path type and ArchiveFormat
```

Panels never call `std::fs` directly. Adding a new backend (SFTP, S3, rar) only requires implementing `VfsProvider` and registering the format in `ArchiveFormat::detect`.

`VfsPath` exposes a `parent()` method used uniformly by both `PanelState` (for synthesising `..`) and `RoutingProvider` — no duplicate logic.

### Archive Operations

| Operation | Function | Notes |
|-----------|----------|-------|
| List directory | `archive::list_archive_dir` | Synthesises dir entries from file paths; populates dates and permissions |
| Read file | `archive::read_archive_file` | Returns raw bytes; works inside archives from F3 viewer |
| Extract all | `archive::extract_archive_to` | Calls `on_progress(done, total)` per entry; ZIP total is known, TAR is streaming |
| Extract entries | `archive::extract_archive_entries` | Matches exact paths and directory prefixes; safe against path traversal |
| Create ZIP | `archive::create_zip_archive` | Deflate compression; recurses into directories; calls `on_progress` per file |

### Event Flow

```
keyboard_producer  ──┐
tick_producer      ──┤──► mpsc::channel<AppEvent> ──► event loop ──► App::update
background tasks   ──┘   (Progress / Key / Tick)                 └──► ui::render
```

Long-running operations (copy, move, extract, create archive) run as independent `tokio::spawn` tasks and communicate back through `AppEvent::Progress` — the UI thread never blocks.

### Popup Stack

Modals are stored as `Vec<Popup>` on `App`. The topmost entry is rendered over the panels. Closing one modal reveals the one beneath, enabling nested confirmations without boolean flags.

```rust
pub enum Popup {
    Error(String),
    Confirm  { title, message, action_on_confirm },
    Input    { title, prompt, value, on_confirm },
    Progress { title, source_name, bytes_done, bytes_total, is_move },
    Viewer   { title, content: Vec<u8>, mode: ViewerMode, scroll_y },
    Menu     { top_idx, sub_idx, open },
}
```

## Building

```sh
cargo build --release
```

Requires Rust 1.75+ (edition 2021). No system dependencies beyond a Unix terminal.

The binary lands at `target/release/chuev-commander`.

## Testing

```sh
cargo test
```

46 tests across three levels:

| Level | Location | What is verified |
|-------|----------|-----------------|
| Unit | `src/vfs/mod.rs` | `ArchiveFormat::detect` — all extensions, compound priority |
| Unit | `src/app.rs` | `CmdLine` — history navigation, backspace, isolation from disk |
| Unit | `src/main.rs` | `parse_cd_arg`, `resolve_cd_path` — edge cases and false-positives |
| Integration | `src/vfs/archive.rs` | Real zip/tar archives in tempdir — listing, synthesised dirs, `./` stripping, file reads, selective extraction, ZIP creation, path traversal sanitisation, permissions |
| Integration | `src/ops.rs` | Real file copy/move in tempdir — content correctness, rename on same-fs, cancellation cleanup |

UI rendering (ratatui widgets) is intentionally not tested — it is never the source of bugs in this architecture.

## Logging

All log output is written to `$XDG_CACHE_HOME/chuev-commander/debug.log` (Linux) or `~/Library/Caches/chuev-commander/debug.log` (macOS). The terminal is never touched by log messages. Set `RUST_LOG=trace` to increase verbosity.
