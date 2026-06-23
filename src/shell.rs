//! Interactive shell-command execution via a pseudo-terminal (PTY).
//!
//! ## Why a PTY?
//!
//! Piped stdio (`Stdio::piped()`) causes `isatty()` to return false on the
//! child's file descriptors, disabling interactive prompts in programs like
//! `docker system prune`.  Inheriting stdio allows interaction but loses
//! the output for the output buffer.  A PTY gives the child `isatty()=true`
//! on all three fds while letting the parent read and capture all output.
//!
//! ## Thread design
//!
//! Two concurrent operations are needed while the child runs:
//! * **PTY-read loop** (spawn_blocking main thread): reads PTY master →
//!   writes to real terminal stdout + capture buffer.
//! * **Stdin-forward thread**: reads raw bytes from stdin (fd 0) →
//!   writes to PTY master, so keystrokes reach the child.
//!
//! The forward thread uses `libc::poll()` with a short timeout instead of a
//! plain blocking `read()`.  This lets it check a stop-flag and exit cleanly
//! when the command finishes, before the keyboard producer is restarted.
//! Without this, the lingering thread would consume the first keystroke typed
//! after the command returns (e.g. Ctrl+O), making the UI appear frozen.

use std::io::{Read, Write};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::{io, path::Path};

use anyhow::{Context, Result};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen},
};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use ratatui::{backend::CrosstermBackend, Terminal};
use tracing::info;

/// Suspend the TUI, run `command` in a PTY, restore the TUI, and return the
/// captured output (ANSI codes stripped) for the output panel.
pub async fn run_interactive(
    term:    &mut Terminal<CrosstermBackend<io::Stdout>>,
    command: &str,
    cwd:     &Path,
) -> Result<String> {
    info!(command, cwd = %cwd.display(), "running interactive shell command");

    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let shell_name = Path::new(&shell)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("bash")
        .to_string();

    // Leave alternate screen so child output appears on the primary buffer.
    // Raw mode stays ON so keystrokes are forwarded byte-for-byte to the PTY.
    execute!(term.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)
        .context("leave alternate screen")?;

    // In raw mode \n alone does not CR — use explicit CRLF.
    {
        let mut out = io::stdout();
        out.write_all(format!("\r\n$ {command}\r\n").as_bytes())
            .context("write command header")?;
        out.flush()?;
    }

    let cmd_str = command.to_string();
    let cwd_    = cwd.to_path_buf();

    let captured = tokio::task::spawn_blocking(move || -> Result<String> {
        // ── Open PTY ──────────────────────────────────────────────────────────
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .context("openpty")?;

        // ── Spawn command in PTY slave ────────────────────────────────────────
        let mut builder = CommandBuilder::new(&shell);
        match shell_name.as_str() {
            "bash" | "zsh" => { builder.arg("-i"); }
            _ => {}
        }
        builder.arg("-c");
        builder.arg(&cmd_str);
        builder.cwd(&cwd_);

        let mut child = pair.slave.spawn_command(builder).context("spawn command")?;
        // Close slave fd in the parent so EOF propagates when the child exits.
        drop(pair.slave);

        let mut master_reader = pair.master.try_clone_reader().context("clone PTY reader")?;
        let mut master_writer = pair.master.take_writer().context("take PTY writer")?;

        // ── Stdin-forward thread ──────────────────────────────────────────────
        // Uses libc::poll() with a 50 ms timeout so it can react to the stop
        // flag without permanently blocking on a full-blocking read().  This
        // guarantees the thread exits before we restart the keyboard producer,
        // preventing it from consuming the next keystroke typed by the user.
        let stop = Arc::new(AtomicBool::new(false));
        let stop_fwd = Arc::clone(&stop);

        let forward_thread = std::thread::spawn(move || {
            let mut buf = [0u8; 256];

            loop {
                // poll() on stdin (fd 0) with a 50 ms timeout
                let mut pfd = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
                let ready = loop {
                    // SAFETY: valid pollfd, count = 1, timeout = 50 ms
                    let r = unsafe { libc::poll(&mut pfd, 1, 50) };
                    // Retry if interrupted by a signal (e.g. SIGWINCH)
                    if r < 0 && unsafe { *libc::__errno_location() } == libc::EINTR {
                        continue;
                    }
                    break r;
                };

                if stop_fwd.load(Ordering::Relaxed) { break; }
                if ready < 0 { break; }  // poll error
                if ready == 0 { continue; }  // timeout — check stop flag

                // SAFETY: buf is valid, fd 0 is readable per poll result above
                let n = unsafe {
                    libc::read(0, buf.as_mut_ptr() as *mut libc::c_void, buf.len())
                };
                if n <= 0 { break; }
                if master_writer.write_all(&buf[..n as usize]).is_err() { break; }
            }
        });

        // ── PTY master → real terminal + capture ─────────────────────────────
        let mut captured_bytes = Vec::<u8>::new();
        let stdout = io::stdout();
        let mut stdout = stdout.lock();
        let mut buf = [0u8; 4096];

        loop {
            match master_reader.read(&mut buf) {
                // EOF or EIO: slave side closed (child exited)
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    stdout.write_all(&buf[..n]).ok();
                    stdout.flush().ok();
                    captured_bytes.extend_from_slice(&buf[..n]);
                }
            }
        }

        child.wait().context("wait for child process")?;

        // Signal the forward thread and join it.
        // join() takes at most one poll cycle (~50 ms) after the flag is set.
        stop.store(true, Ordering::Relaxed);
        let _ = forward_thread.join();

        Ok(strip_ansi(String::from_utf8_lossy(&captured_bytes).into_owned()))
    })
    .await
    .context("join blocking PTY task")?
    .context("interactive shell execution")?;

    // Restore the alternate screen; raw mode was always on so no need to
    // re-enable it.
    execute!(term.backend_mut(), EnterAlternateScreen, EnableMouseCapture)
        .context("enter alternate screen")?;
    term.clear().context("clear terminal after interactive command")?;

    Ok(captured)
}

/// Strip ANSI / VT100 escape sequences and normalise carriage returns.
///
/// PTY output includes colour codes and `\r` from the terminal line discipline.
/// This function produces plain text for the output panel.
fn strip_ansi(s: String) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.peek().copied() {
                Some('[') => {
                    // CSI: ESC [ <params…> <final-byte>
                    chars.next();
                    while let Some(&nc) = chars.peek() {
                        chars.next();
                        if nc.is_ascii_alphabetic() { break; }
                    }
                }
                Some(']') => {
                    // OSC: ESC ] … BEL | ST
                    chars.next();
                    while let Some(&nc) = chars.peek() {
                        chars.next();
                        if nc == '\x07' || nc == '\x1b' { break; }
                    }
                }
                _ => { chars.next(); }
            },
            '\r' => {
                // \r\n → handled naturally (\n comes in next iteration)
                // bare \r (cursor-return overwrite) → newline in the buffer
                if chars.peek().copied() != Some('\n') {
                    out.push('\n');
                }
            }
            _ => out.push(c),
        }
    }
    out
}
