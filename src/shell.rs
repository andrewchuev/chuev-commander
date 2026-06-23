//! Shell-command execution with output capture.
//!
//! Commands run without suspending the TUI. stdout and stderr are captured
//! and returned so the caller can store them in the output buffer, which is
//! rendered in the panels area when Ctrl+O hides the panels.
//!
//! ## Why `-i` (interactive mode)?
//!
//! `.bashrc` and `.zshrc` start with an interactive guard:
//!
//! ```bash
//! case $- in
//!     *i*) ;;   # interactive — continue
//!       *) return;;  # non-interactive — bail out immediately
//! esac
//! ```
//!
//! A plain `bash -c "cmd"` is non-interactive, so `$-` never contains `i`,
//! the guard fires, `.bashrc` returns before defining any aliases, and user
//! aliases like `ll` are unavailable.
//!
//! Running with `bash -i -c "cmd"` sets the `i` flag, `.bashrc` executes in
//! full, and all aliases/functions become available.
//!
//! The trade-off: interactive bash without a TTY prints two harmless startup
//! warnings to stderr.  We filter these before returning output to the caller.

use std::{path::Path, process::Stdio};

use anyhow::{Context, Result};
use tracing::{info, warn};

/// Lines emitted to stderr when bash/zsh runs interactively without a tty.
/// These are cosmetic noise — filter them so they don't appear in the output
/// buffer.
fn is_shell_startup_noise(line: &str) -> bool {
    // bash -i without a tty always prints these two lines:
    line.contains("cannot set terminal process group")
        || line.contains("no job control in this shell")
}

/// Run `command` in the user's login shell with `cwd` as the working directory.
///
/// Uses `$SHELL` (falling back to `/bin/bash`).  For bash and zsh the shell is
/// started with the `-i` (interactive) flag so that `.bashrc` / `.zshrc` is
/// sourced in full — enabling user-defined aliases and functions.
///
/// Returns the combined stdout + filtered-stderr output.
pub async fn capture(command: &str, cwd: &Path) -> Result<String> {
    info!(command, cwd = %cwd.display(), "executing shell command");

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let shell_name = Path::new(&shell)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("bash")
        .to_string();

    let cmd  = command.to_string();
    let cwd_ = cwd.to_path_buf();

    let output = tokio::task::spawn_blocking(move || {
        let mut proc = std::process::Command::new(&shell);

        // bash and zsh both respect `-i` to force interactive startup files.
        // fish always sources config.fish; other shells fall through unchanged.
        match shell_name.as_str() {
            "bash" | "zsh" => { proc.arg("-i"); }
            _ => {}
        }

        proc.arg("-c")
            .arg(&cmd)
            .current_dir(&cwd_)
            .stdin(Stdio::null())   // no tty — avoid blocking reads
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
    })
    .await
    .context("joining shell task")?
    .context("spawning shell command")?;

    if !output.status.success() {
        warn!(exit_code = ?output.status.code(), "command exited with non-zero status");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Filter interactive-mode startup noise before surfacing stderr.
    let stderr_raw = String::from_utf8_lossy(&output.stderr);
    let stderr: String = stderr_raw
        .lines()
        .filter(|l| !is_shell_startup_noise(l))
        .collect::<Vec<_>>()
        .join("\n");

    let mut combined = stdout.into_owned();
    if !stderr.is_empty() {
        if !combined.is_empty() && !combined.ends_with('\n') {
            combined.push('\n');
        }
        combined.push_str(&stderr);
    }

    Ok(combined)
}
