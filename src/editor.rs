//! External-editor integration (F4).
//!
//! The TUI must be fully suspended before handing control to the editor
//! and fully restored afterward, even if the editor crashes.
//! The function is async so it lives naturally in the tokio event loop,
//! but the actual `Command::wait()` is offloaded to `spawn_blocking` to
//! avoid starving other tokio tasks while the editor runs.

use std::io;
use std::path::Path;

use anyhow::{Context, Result};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use tracing::{info, warn};

/// Suspend the TUI, open `path` in `$VISUAL` / `$EDITOR` / `vi`, then
/// restore the TUI.  Errors from the editor itself (non-zero exit) are
/// logged but not propagated — that's the editor's business, not ours.
pub async fn launch(
    term: &mut ratatui::Terminal<CrosstermBackend<io::Stdout>>,
    path: &Path,
) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());

    info!(editor = %editor, path = %path.display(), "launching external editor");

    // ── 1. Tear down the TUI ─────────────────────────────────────────────────
    disable_raw_mode().context("disable_raw_mode before editor")?;
    execute!(
        term.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .context("leaving alternate screen before editor")?;

    // ── 2. Run the editor (blocking; offloaded so tokio stays responsive) ────
    let path_buf = path.to_path_buf();
    let status = tokio::task::spawn_blocking(move || {
        std::process::Command::new(&editor)
            .arg(&path_buf)
            .status()
    })
    .await
    .context("spawn_blocking for editor")?  // JoinError
    .context("spawning editor process")?;   // io::Error

    if !status.success() {
        warn!(code = ?status.code(), "editor exited with non-zero status");
    }

    // ── 3. Restore the TUI unconditionally ───────────────────────────────────
    enable_raw_mode().context("enable_raw_mode after editor")?;
    execute!(
        term.backend_mut(),
        EnterAlternateScreen,
        EnableMouseCapture
    )
    .context("entering alternate screen after editor")?;

    // Force a full redraw so stale editor output is wiped
    term.clear().context("clearing terminal after editor")?;

    info!("TUI restored after editor");
    Ok(())
}
