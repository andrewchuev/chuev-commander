//! # chuev-commander — entry point
//!
//! Responsibilities of `main.rs`:
//!   1. Initialise file-based logging (nothing ever goes to stdout/stderr).
//!   2. Put the terminal into raw + alternate-screen mode.
//!   3. Run the async event loop.
//!   4. Unconditionally restore the terminal on exit (even on panic / error).

// Skeleton stubs: many public items are defined for future use and will gain
// callers as features are implemented.  Remove this attribute then.
#![allow(dead_code)]

mod actions;
mod app;
mod editor;
mod events;
mod menu;
mod ops;
mod platform;
mod shell;
mod theme;
mod ui;
mod vfs;

use std::{io, path::{Path, PathBuf}, sync::Arc, time::{Duration, Instant}};

use anyhow::{Context, Result};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, EventStream},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use actions::key_event_to_action;
use app::{App, PendingAction};
use events::{AppEvent, EventSender};
use vfs::router::RoutingProvider;

// ─────────────────────────────────────────────────────────────────────────────
// Entry point
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    // Logging must be set up before anything else so early errors are captured.
    // The returned guard must live for the entire process lifetime so the
    // background writer thread is not dropped prematurely.
    let _log_guard = setup_logging().context("initialising logging")?;
    info!(version = env!("CARGO_PKG_VERSION"), "chuev-commander starting");

    // ── Terminal initialisation ───────────────────────────────────────────────
    enable_raw_mode().context("enabling raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .context("entering alternate screen")?;

    let backend  = CrosstermBackend::new(stdout);
    let mut term = Terminal::new(backend).context("creating terminal")?;

    // ── Run; restore terminal even if `run` returns an error ─────────────────
    let result = run(&mut term).await;

    // Restoration order matters: leave raw mode first, then alternate screen.
    disable_raw_mode().ok();
    execute!(
        term.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
    )
    .ok();
    term.show_cursor().ok();

    if let Err(ref e) = result {
        error!(error = %e, "fatal error — check debug.log for details");
        eprintln!("Error: {e:#}");
    }

    info!(version = env!("CARGO_PKG_VERSION"), "chuev-commander exiting");
    result
}

// ─────────────────────────────────────────────────────────────────────────────
// Main event loop
// ─────────────────────────────────────────────────────────────────────────────

async fn run(term: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let provider = Arc::new(RoutingProvider);

    // Channel created before App so we can pass the sender in
    let (tx, mut rx) = mpsc::channel::<AppEvent>(256);

    let mut app = App::new(provider, tx.clone()).context("initialising app state")?;

    // System clipboard — None if arboard cannot connect (e.g. no display server)
    let mut clipboard = arboard::Clipboard::new().ok();

    // Producer: keyboard / mouse events from crossterm (stoppable via token)
    let mut kb_cancel = CancellationToken::new();
    let mut kb_handle = tokio::spawn(keyboard_producer(tx.clone(), kb_cancel.clone()));

    // Producer: 100 ms heartbeat for animations and periodic UI updates
    tokio::spawn(tick_producer(tx.clone(), Duration::from_millis(100)));

    loop {
        // Draw first so the initial state is visible immediately
        term.draw(|frame| ui::render(frame, &mut app))
            .context("drawing frame")?;

        // Block until the next event arrives
        match rx.recv().await {
            Some(AppEvent::Mouse(mouse)) => {
                app.handle_mouse(mouse);
            }

            Some(AppEvent::Key(key)) => {
                let action = key_event_to_action(&key);
                // Cursor movement and typing are very high frequency — trace level only.
                // All other actions log at debug so they appear in normal debug sessions.
                match &action {
                    actions::Action::MoveUp
                    | actions::Action::MoveDown
                    | actions::Action::MoveLeft
                    | actions::Action::MoveRight
                    | actions::Action::PageUp
                    | actions::Action::PageDown
                    | actions::Action::Home
                    | actions::Action::End
                    | actions::Action::CmdlineChar(_)
                    | actions::Action::None => {
                        trace!(key = ?key.code, mods = ?key.modifiers, action = ?action, "key");
                    }
                    _ => {
                        debug!(key = ?key.code, mods = ?key.modifiers, action = ?action, "key");
                    }
                }
                app.update(action);

                // Dispatch any I/O operation the update produced.
                if let Some(action) = app.pending_action.take() {
                    match action {
                        // ── Clipboard (instant; no TUI suspension needed) ──────
                        PendingAction::ClipboardCopy(text) => {
                            debug!(bytes = text.len(), "clipboard: writing");
                            if let Some(cb) = clipboard.as_mut() {
                                if let Err(e) = cb.set_text(text) {
                                    tracing::warn!(error = %e, "clipboard: write failed");
                                }
                            }
                        }
                        PendingAction::ClipboardPaste => {
                            debug!("clipboard: reading");
                            if let Some(cb) = clipboard.as_mut() {
                                match cb.get_text() {
                                    Ok(text) => {
                                        debug!(bytes = text.len(), "clipboard: pasted into cmdline");
                                        for c in text.chars().filter(|c| !c.is_control()) {
                                            app.cmdline.push_char(c);
                                        }
                                    }
                                    Err(e) => tracing::warn!(error = %e, "clipboard: read failed"),
                                }
                            }
                        }

                        // ── TUI-suspending actions (need exclusive stdin) ──────
                        PendingAction::Edit(path) => {
                            info!(path = %path.display(), "editor: launching");
                            suspend_keyboard(&mut kb_cancel, &mut kb_handle, &mut rx).await;

                            let t0 = Instant::now();
                            if let Err(e) = editor::launch(term, &path).await {
                                error!(error = %e, "editor: launch failed");
                                app.push_error(format!("Editor error: {e:#}"));
                            } else {
                                info!(
                                    path = %path.display(),
                                    elapsed_ms = t0.elapsed().as_millis(),
                                    "editor: returned"
                                );
                            }

                            resume_keyboard(&tx, &mut kb_cancel, &mut kb_handle, &mut rx).await;
                        }

                        PendingAction::Shell { cmd, cwd } => {
                            if let Some(arg) = parse_cd_arg(&cmd) {
                                // `cd` is a shell built-in; handle as panel navigation.
                                debug!(arg, "cd: resolving built-in");
                                let new_path = resolve_cd_path(arg, &cwd);
                                match std::fs::canonicalize(&new_path) {
                                    Ok(canonical) if canonical.is_dir() => {
                                        info!(path = %canonical.display(), "cd: navigated");
                                        app.navigate_active_to(canonical);
                                    }
                                    _ => {
                                        let msg = if arg.is_empty() {
                                            "cd: home directory not found".into()
                                        } else {
                                            format!("cd: {arg}: No such file or directory")
                                        };
                                        tracing::warn!(msg, "cd: failed");
                                        app.push_error(msg);
                                    }
                                }
                            } else if is_clear_cmd(&cmd) {
                                // `clear` / `cls` clears the output buffer; no PTY needed.
                                info!(cmd = %cmd, "clear: built-in, clearing output buffer");
                                app.clear_output_buffer();
                            } else {
                                info!(cmd = %cmd, cwd = %cwd.display(), "shell: executing");
                                suspend_keyboard(&mut kb_cancel, &mut kb_handle, &mut rx).await;

                                let t0 = Instant::now();
                                match shell::run_interactive(term, &cmd, &cwd).await {
                                    Ok(output) => {
                                        info!(
                                            cmd = %cmd,
                                            elapsed_ms = t0.elapsed().as_millis(),
                                            output_lines = output.lines().count(),
                                            output_bytes = output.len(),
                                            "shell: completed"
                                        );
                                        app.append_output(&cmd, &cwd, &output);
                                    }
                                    Err(e) => {
                                        error!(
                                            cmd = %cmd,
                                            elapsed_ms = t0.elapsed().as_millis(),
                                            error = %e,
                                            "shell: failed"
                                        );
                                        app.push_error(format!("Shell error: {e:#}"));
                                    }
                                }

                                resume_keyboard(&tx, &mut kb_cancel, &mut kb_handle, &mut rx).await;
                                app.reload_active_panel();
                            }
                        }
                    }
                }
            }

            Some(AppEvent::Tick) => {
                // Reserved for: cursor blink, progress-bar animation, etc.
                // Currently a no-op; the draw call above already re-renders.
            }

            Some(AppEvent::Progress(data)) => {
                app.handle_progress(data);
            }

            // All senders were dropped — shouldn't happen in normal operation
            None => {
                info!("event channel closed; exiting");
                break;
            }
        }

        if app.should_quit {
            break;
        }
    }

    app.save_panel_state();
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Keyboard producer lifecycle helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Cancel the keyboard producer and drain any buffered events so the next
/// operation gets exclusive access to stdin.
async fn suspend_keyboard(
    kb_cancel: &mut CancellationToken,
    kb_handle: &mut tokio::task::JoinHandle<()>,
    rx:        &mut mpsc::Receiver<AppEvent>,
) {
    debug!("keyboard: suspending producer");
    kb_cancel.cancel();
    // Swap in a dummy handle so we can await the old one.
    let old = std::mem::replace(kb_handle, tokio::spawn(async {}));
    let _ = old.await;
    let mut drained = 0usize;
    while rx.try_recv().is_ok() { drained += 1; }
    debug!(drained_events = drained, "keyboard: producer suspended");
}

/// Restart the keyboard producer with a fresh cancellation token and drain
/// any stray events accumulated during the suspended period.
async fn resume_keyboard(
    tx:        &EventSender,
    kb_cancel: &mut CancellationToken,
    kb_handle: &mut tokio::task::JoinHandle<()>,
    rx:        &mut mpsc::Receiver<AppEvent>,
) {
    let mut drained = 0usize;
    while rx.try_recv().is_ok() { drained += 1; }
    *kb_cancel = CancellationToken::new();
    *kb_handle = tokio::spawn(keyboard_producer(tx.clone(), kb_cancel.clone()));
    debug!(drained_events = drained, "keyboard: producer resumed");
}

// ─────────────────────────────────────────────────────────────────────────────
// cd helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Returns `true` when `cmd` is a screen-clear command (`clear` or `cls`).
/// These are intercepted as a file-manager built-in that clears the output
/// buffer — running them in a PTY would produce no visible effect.
fn is_clear_cmd(cmd: &str) -> bool {
    matches!(cmd.trim(), "clear" | "cls")
}

/// If `cmd` is a `cd [arg]` command, returns the path argument (may be empty
/// for bare `cd`). Returns `None` for any other command.
fn parse_cd_arg(cmd: &str) -> Option<&str> {
    let t = cmd.trim();
    if t == "cd" {
        Some("")
    } else if let Some(rest) = t.strip_prefix("cd ").or_else(|| t.strip_prefix("cd\t")) {
        Some(rest.trim())
    } else {
        None
    }
}

/// Resolve the cd argument to an absolute `PathBuf`.
/// `~` / `~/…` is expanded to the home directory; relative paths are joined
/// onto `cwd`; absolute paths are used as-is.
fn resolve_cd_path(arg: &str, cwd: &PathBuf) -> PathBuf {
    if arg.is_empty() || arg == "~" {
        return dirs::home_dir().unwrap_or_else(|| cwd.clone());
    }
    if arg == "-" {
        // "cd -" would need OLDPWD tracking; not implemented — treat as home
        return dirs::home_dir().unwrap_or_else(|| cwd.clone());
    }
    let expanded = if arg.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            home.join(&arg[2..])
        } else {
            PathBuf::from(arg)
        }
    } else {
        PathBuf::from(arg)
    };
    if expanded.is_absolute() {
        expanded
    } else {
        cwd.join(expanded)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Event producers (run as independent tokio tasks)
// ─────────────────────────────────────────────────────────────────────────────

/// Forwards crossterm `KeyEvent`s to the shared event channel.
///
/// Stops cleanly when `cancel` is triggered — used to pause input during
/// an external editor session so the editor gets exclusive stdin access.
async fn keyboard_producer(tx: EventSender, cancel: CancellationToken) {
    let mut stream = EventStream::new();

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            maybe = stream.next() => {
                match maybe {
                    Some(Ok(crossterm::event::Event::Key(key))) => {
                        if tx.send(AppEvent::Key(key)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(crossterm::event::Event::Mouse(mouse))) => {
                        if tx.send(AppEvent::Mouse(mouse)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(_)) => {} // resize, focus — ignored
                    Some(Err(e)) => {
                        error!(error = %e, "crossterm event stream error");
                    }
                    None => break,
                }
            }
        }
    }
}

/// Sends `AppEvent::Tick` at a fixed `interval`.
async fn tick_producer(tx: EventSender, interval: Duration) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;
        if tx.send(AppEvent::Tick).await.is_err() {
            break;
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Logging setup
// ─────────────────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────────────────
// Unit tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod cd_tests {
    use super::{parse_cd_arg, resolve_cd_path};
    use std::path::PathBuf;

    // ── parse_cd_arg ─────────────────────────────────────────────────────────

    #[test]
    fn parse_bare_cd() {
        assert_eq!(parse_cd_arg("cd"), Some(""));
    }

    #[test]
    fn parse_cd_with_path() {
        assert_eq!(parse_cd_arg("cd /tmp"), Some("/tmp"));
    }

    #[test]
    fn parse_cd_leading_spaces_trimmed() {
        assert_eq!(parse_cd_arg("  cd  /home/user  "), Some("/home/user"));
    }

    #[test]
    fn parse_cd_tab_separator() {
        assert_eq!(parse_cd_arg("cd\t/foo"), Some("/foo"));
    }

    #[test]
    fn parse_non_cd_command() {
        assert_eq!(parse_cd_arg("ls -la"), None);
    }

    /// "cdf" must NOT be detected as a `cd` command (prefix false-positive).
    #[test]
    fn parse_cd_prefix_false_positive() {
        assert_eq!(parse_cd_arg("cdf /mnt"), None);
    }

    // ── resolve_cd_path ──────────────────────────────────────────────────────

    #[test]
    fn resolve_absolute_path_unchanged() {
        let cwd = PathBuf::from("/home/user");
        let result = resolve_cd_path("/etc", &cwd);
        assert_eq!(result, PathBuf::from("/etc"));
    }

    #[test]
    fn resolve_relative_path_joined_to_cwd() {
        let cwd = PathBuf::from("/home/user");
        let result = resolve_cd_path("projects/foo", &cwd);
        assert_eq!(result, PathBuf::from("/home/user/projects/foo"));
    }

    #[test]
    fn resolve_tilde_slash_expands_home() {
        let cwd = PathBuf::from("/tmp");
        let result = resolve_cd_path("~/docs", &cwd);
        // Only verify it's not the cwd and starts from home — home differs per system.
        assert_ne!(result, cwd);
        assert!(result.ends_with("docs"));
    }

    #[test]
    fn resolve_empty_arg_goes_to_home() {
        let cwd = PathBuf::from("/tmp");
        let home_result = resolve_cd_path("", &cwd);
        // Should equal home or cwd (fallback); either way, not literally empty.
        assert!(home_result.is_absolute());
    }
}

/// Initialise `tracing` with a per-session non-blocking file appender.
///
/// Each launch creates a new file:
///   `$XDG_CACHE_HOME/chuev-commander/session_YYYY-MM-DDTHH-MM-SS.log`
///
/// Up to `SESSION_LOG_KEEP` files are retained; older ones are deleted on
/// startup so the directory does not grow without bound.
///
/// Returns the `WorkerGuard` — **must be kept alive in `main`** or the
/// background writer thread exits and the final log lines are lost.
fn setup_logging() -> Result<tracing_appender::non_blocking::WorkerGuard> {
    const SESSION_LOG_KEEP: usize = 20;

    let log_dir = dirs::cache_dir()
        .map(|d| d.join("chuev-commander"))
        .unwrap_or_else(|| PathBuf::from("."));

    std::fs::create_dir_all(&log_dir)
        .with_context(|| format!("creating log dir {}", log_dir.display()))?;

    // Purge excess old session logs before creating the new one.
    trim_session_logs(&log_dir, SESSION_LOG_KEEP);

    let ts       = chrono::Local::now().format("%Y-%m-%dT%H-%M-%S");
    let log_path = log_dir.join(format!("session_{ts}.log"));
    let log_file = std::fs::File::create(&log_path)
        .with_context(|| format!("creating session log {}", log_path.display()))?;

    let (non_blocking, guard) = tracing_appender::non_blocking(log_file);

    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("debug")),
        )
        .with(
            fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false)          // no escape codes in the log file
                .with_target(true)
                .with_thread_ids(true),
        )
        .init();

    // The first log line is written after init() so it appears in the file.
    tracing::info!(log = %log_path.display(), "session log opened");
    Ok(guard)
}

/// Delete the oldest session log files, keeping the `keep` most recent.
/// Files are identified by the `session_*.log` naming pattern; ISO-8601
/// timestamps in the names sort lexicographically in chronological order.
fn trim_session_logs(log_dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(log_dir) else { return };
    let mut logs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("session_") && n.ends_with(".log"))
                .unwrap_or(false)
        })
        .collect();
    logs.sort_unstable();          // lexicographic = chronological for ISO names
    if logs.len() > keep {
        for old in &logs[..logs.len() - keep] {
            let _ = std::fs::remove_file(old);
        }
    }
}
