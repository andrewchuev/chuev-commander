//! # Background file operations (Copy / Move)
//!
//! Each public function is designed to run inside `tokio::spawn`.
//! Progress is reported through the `AppEvent` channel.
//! Cancellation is checked at every chunk boundary via `CancellationToken`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::events::{AppEvent, EventSender, ProgressData};
use crate::vfs::ArchiveFormat;

const CHUNK: usize = 256 * 1024; // 256 KB read buffer

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Copy a single file or directory tree to `dst` (full destination path).
pub async fn copy(
    src:    PathBuf,
    dst:    PathBuf,
    tx:     EventSender,
    cancel: CancellationToken,
) -> Result<()> {
    if src.is_dir() {
        copy_dir_recursive(src, dst, tx, cancel, true).await
    } else {
        copy_file(src, dst, tx, cancel, "Copying", true).await
    }
}

/// Move a single file or directory to `dst` (full destination path).
pub async fn move_entry(
    src:    PathBuf,
    dst:    PathBuf,
    tx:     EventSender,
    cancel: CancellationToken,
) -> Result<()> {
    move_entry_inner(src, dst, tx, cancel, true).await
}

/// Copy multiple entries to `dst_dir`, sending a single `done: true` at the end.
pub async fn copy_batch(
    srcs:    Vec<PathBuf>,
    dst_dir: PathBuf,
    tx:      EventSender,
    cancel:  CancellationToken,
) -> Result<()> {
    let count = srcs.len();
    for (i, src) in srcs.into_iter().enumerate() {
        if cancel.is_cancelled() {
            return Err(anyhow::anyhow!("cancelled by user"));
        }
        let file_name = src.file_name()
            .ok_or_else(|| anyhow::anyhow!("invalid source path (no filename): {}", src.display()))?;
        let dst     = dst_dir.join(file_name);
        let is_last = i + 1 == count;
        if src.is_dir() {
            copy_dir_recursive(src, dst, tx.clone(), cancel.clone(), is_last).await?;
        } else {
            copy_file(src, dst, tx.clone(), cancel.clone(), "Copying", is_last).await?;
        }
    }
    Ok(())
}

/// Extract an entire archive to `dst_dir`, running the blocking I/O on a thread pool.
/// Sends per-entry `AppEvent::Progress` events while running.
pub async fn extract_archive(
    archive_path: PathBuf,
    format: ArchiveFormat,
    dst_dir: PathBuf,
    tx: EventSender,
    cancel: CancellationToken,
) -> Result<()> {
    let src_name = file_name(&archive_path);

    let tx_prog      = tx.clone();
    let src_name_prog = src_name.clone();

    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            return Err(anyhow::anyhow!("cancelled by user"));
        }
        r = tokio::task::spawn_blocking(move || {
            let progress = |done: u64, total: u64| {
                let _ = tx_prog.try_send(AppEvent::Progress(ProgressData {
                    operation:   "Extracting".into(),
                    source_name: src_name_prog.clone(),
                    bytes_done:  done,
                    bytes_total: total,
                    done:        false,
                }));
            };
            crate::vfs::archive::extract_archive_to(&archive_path, format, &dst_dir, &progress)
        }) => r.context("extraction task panicked")?
    };

    let count = result?;

    let _ = tx
        .send(AppEvent::Progress(ProgressData {
            operation:   "Extracting".into(),
            source_name: src_name,
            bytes_done:  count,
            bytes_total: count,
            done:        true,
        }))
        .await;

    Ok(())
}

/// Extract specific entries (files or directories) from an archive to `dst_dir`.
pub async fn extract_archive_entries(
    archive_path:   PathBuf,
    format:         ArchiveFormat,
    internal_paths: Vec<String>,
    dst_dir:        PathBuf,
    tx:             EventSender,
    cancel:         CancellationToken,
) -> Result<()> {
    let src_name = file_name(&archive_path);

    let tx_prog       = tx.clone();
    let src_name_prog = src_name.clone();

    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            return Err(anyhow::anyhow!("cancelled by user"));
        }
        r = tokio::task::spawn_blocking(move || {
            let progress = |done: u64, total: u64| {
                let _ = tx_prog.try_send(AppEvent::Progress(ProgressData {
                    operation:   "Extracting".into(),
                    source_name: src_name_prog.clone(),
                    bytes_done:  done,
                    bytes_total: total,
                    done:        false,
                }));
            };
            crate::vfs::archive::extract_archive_entries(
                &archive_path, format, &internal_paths, &dst_dir, &progress,
            )
        }) => r.context("extraction task panicked")?
    };

    let count = result?;

    let _ = tx
        .send(AppEvent::Progress(ProgressData {
            operation:   "Extracting".into(),
            source_name: src_name,
            bytes_done:  count,
            bytes_total: count,
            done:        true,
        }))
        .await;

    Ok(())
}

/// Create a ZIP archive at `dst_path` from the given local `sources`.
pub async fn create_archive(
    sources:  Vec<PathBuf>,
    dst_path: PathBuf,
    tx:       EventSender,
    cancel:   CancellationToken,
) -> Result<()> {
    let src_name = if sources.len() == 1 {
        file_name(&sources[0])
    } else {
        format!("{} items", sources.len())
    };

    let tx_prog       = tx.clone();
    let src_name_prog = src_name.clone();

    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            return Err(anyhow::anyhow!("cancelled by user"));
        }
        r = tokio::task::spawn_blocking(move || {
            let progress = |done: u64, total: u64| {
                let _ = tx_prog.try_send(AppEvent::Progress(ProgressData {
                    operation:   "Creating".into(),
                    source_name: src_name_prog.clone(),
                    bytes_done:  done,
                    bytes_total: total,
                    done:        false,
                }));
            };
            crate::vfs::archive::create_zip_archive(&sources, &dst_path, &progress)
        }) => r.context("create archive task panicked")?
    };

    let count = result?;

    let _ = tx
        .send(AppEvent::Progress(ProgressData {
            operation:   "Creating".into(),
            source_name: src_name,
            bytes_done:  count,
            bytes_total: count,
            done:        true,
        }))
        .await;

    Ok(())
}

/// Move multiple entries to `dst_dir`, sending a single `done: true` at the end.
pub async fn move_batch(
    srcs:    Vec<PathBuf>,
    dst_dir: PathBuf,
    tx:      EventSender,
    cancel:  CancellationToken,
) -> Result<()> {
    let count = srcs.len();
    for (i, src) in srcs.into_iter().enumerate() {
        if cancel.is_cancelled() {
            return Err(anyhow::anyhow!("cancelled by user"));
        }
        let file_name = src.file_name()
            .ok_or_else(|| anyhow::anyhow!("invalid source path (no filename): {}", src.display()))?;
        let dst     = dst_dir.join(file_name);
        let is_last = i + 1 == count;
        move_entry_inner(src, dst, tx.clone(), cancel.clone(), is_last).await?;
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Internals
// ─────────────────────────────────────────────────────────────────────────────

/// Core move logic shared by `move_entry` and `move_batch`.
/// `emit_done` controls whether `done: true` is sent after this operation.
async fn move_entry_inner(
    src:       PathBuf,
    dst:       PathBuf,
    tx:        EventSender,
    cancel:    CancellationToken,
    emit_done: bool,
) -> Result<()> {
    // Try atomic rename first (same filesystem → instant)
    if let Ok(()) = tokio::fs::rename(&src, &dst).await {
        info!(src = %src.display(), dst = %dst.display(), "renamed (atomic)");
        let name = file_name(&src);
        let size = tokio::fs::metadata(&dst).await.map(|m| m.len()).unwrap_or(0);
        let _ = tx.send(AppEvent::Progress(ProgressData {
            operation:   "Moving".into(),
            source_name: name,
            bytes_done:  size,
            bytes_total: size,
            done:        emit_done,
        })).await;
        return Ok(());
    }

    // Cross-device: copy then remove source
    if src.is_dir() {
        copy_dir_recursive(src.clone(), dst, tx, cancel, emit_done).await?;
        tokio::fs::remove_dir_all(&src).await
            .with_context(|| format!("removing source dir {}", src.display()))?;
    } else {
        copy_file(src.clone(), dst, tx, cancel, "Moving", emit_done).await?;
        tokio::fs::remove_file(&src).await
            .with_context(|| format!("removing source {}", src.display()))?;
    }
    Ok(())
}

/// Stream-copy a single file, reporting byte progress.
/// `emit_done` — whether to set `done: true` in the final progress event.
async fn copy_file(
    src:       PathBuf,
    dst:       PathBuf,
    tx:        EventSender,
    cancel:    CancellationToken,
    operation: &str,
    emit_done: bool,
) -> Result<()> {
    let source_name = file_name(&src);
    let total = tokio::fs::metadata(&src).await
        .map(|m| m.len())
        .unwrap_or(0);

    let mut src_f = tokio::fs::File::open(&src).await
        .with_context(|| format!("opening {}", src.display()))?;
    let mut dst_f = tokio::fs::File::create(&dst).await
        .with_context(|| format!("creating {}", dst.display()))?;

    let mut buf  = vec![0u8; CHUNK];
    let mut done = 0u64;
    let op       = operation.to_owned();

    loop {
        tokio::select! {
            biased;

            _ = cancel.cancelled() => {
                drop(dst_f);
                let _ = tokio::fs::remove_file(&dst).await;
                return Err(anyhow::anyhow!("cancelled by user"));
            }

            result = src_f.read(&mut buf) => {
                let n = result.with_context(|| format!("reading {}", src.display()))?;
                if n == 0 { break; }

                dst_f.write_all(&buf[..n]).await
                    .with_context(|| format!("writing {}", dst.display()))?;

                done += n as u64;

                if tx.try_send(AppEvent::Progress(ProgressData {
                    operation:   op.clone(),
                    source_name: source_name.clone(),
                    bytes_done:  done,
                    bytes_total: total,
                    done:        false,
                })).is_err() {
                    tracing::trace!("progress channel full — update dropped");
                }
            }
        }
    }

    dst_f.flush().await.context("flushing destination")?;

    let _ = tx.send(AppEvent::Progress(ProgressData {
        operation:   op,
        source_name,
        bytes_done:  done,
        bytes_total: total,
        done:        emit_done,
    })).await;

    Ok(())
}

/// Recursively copy a directory tree.
/// `emit_done` controls the final progress event.
async fn copy_dir_recursive(
    src:       PathBuf,
    dst:       PathBuf,
    tx:        EventSender,
    cancel:    CancellationToken,
    emit_done: bool,
) -> Result<()> {
    let entries = tokio::task::spawn_blocking({
        let src = src.clone();
        let dst = dst.clone();
        move || collect_files(&src, &dst)
    })
    .await
    .context("spawn_blocking for directory scan")??;

    let total_bytes: u64 = entries.iter().map(|(_, _, s)| s).sum();
    let mut done_bytes = 0u64;
    let entry_count = entries.len();

    for (idx, (src_file, dst_file, size)) in entries.into_iter().enumerate() {
        if cancel.is_cancelled() {
            return Err(anyhow::anyhow!("cancelled by user"));
        }

        if let Some(parent) = dst_file.parent() {
            tokio::fs::create_dir_all(parent).await
                .with_context(|| format!("creating {}", parent.display()))?;
        }

        tokio::fs::copy(&src_file, &dst_file).await
            .with_context(|| format!("copying {}", src_file.display()))?;

        done_bytes += size;
        let is_last_file = idx + 1 == entry_count;

        let _ = tx.try_send(AppEvent::Progress(ProgressData {
            operation:   "Copying".into(),
            source_name: file_name(&src_file),
            bytes_done:  done_bytes,
            bytes_total: total_bytes,
            done:        false,
        }));

        // Only send the closing event on the very last file and only if requested
        if is_last_file {
            let _ = tx.send(AppEvent::Progress(ProgressData {
                operation:   "Copying".into(),
                source_name: file_name(&src),
                bytes_done:  done_bytes,
                bytes_total: total_bytes,
                done:        emit_done,
            })).await;
        }
    }

    // Edge case: empty directory
    if entry_count == 0 {
        tokio::fs::create_dir_all(&dst).await
            .with_context(|| format!("creating {}", dst.display()))?;
        if emit_done {
            let _ = tx.send(AppEvent::Progress(ProgressData {
                operation:   "Copying".into(),
                source_name: file_name(&src),
                bytes_done:  0,
                bytes_total: 0,
                done:        true,
            })).await;
        }
    }

    Ok(())
}

fn collect_files(src: &Path, dst: &Path) -> Result<Vec<(PathBuf, PathBuf, u64)>> {
    let mut out = Vec::new();
    collect_files_inner(src, dst, src, &mut out)?;
    Ok(out)
}

fn collect_files_inner(
    root:     &Path,
    dst_root: &Path,
    dir:      &Path,
    out:      &mut Vec<(PathBuf, PathBuf, u64)>,
) -> Result<()> {
    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
    {
        let entry    = entry?;
        let src_path = entry.path();
        let rel      = src_path.strip_prefix(root).unwrap_or(&src_path);
        let dst_path = dst_root.join(rel);

        if src_path.is_dir() {
            collect_files_inner(root, dst_root, &src_path, out)?;
        } else {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            out.push((src_path, dst_path, size));
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Integration tests — real file I/O in tempdir
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio::sync::mpsc;

    fn make_channel() -> EventSender {
        // Receiver is intentionally dropped; senders use `let _ = tx.send(...)` everywhere
        mpsc::channel::<crate::events::AppEvent>(32).0
    }

    #[tokio::test]
    async fn copy_single_file_creates_exact_copy() {
        let tmp   = TempDir::new().unwrap();
        let src   = tmp.path().join("source.txt");
        let dst   = tmp.path().join("copy.txt");
        let content = b"the quick brown fox";
        std::fs::write(&src, content).unwrap();

        copy(src, dst.clone(), make_channel(), CancellationToken::new())
            .await
            .unwrap();

        assert!(dst.exists(), "destination not created");
        assert_eq!(std::fs::read(&dst).unwrap(), content);
    }

    #[tokio::test]
    async fn copy_directory_tree_preserves_structure() {
        let tmp    = TempDir::new().unwrap();
        let src_dir = tmp.path().join("src");
        std::fs::create_dir_all(src_dir.join("nested")).unwrap();
        std::fs::write(src_dir.join("a.txt"),        b"a").unwrap();
        std::fs::write(src_dir.join("nested/b.txt"), b"b").unwrap();

        let dst_dir = tmp.path().join("dst");

        copy(src_dir, dst_dir.clone(), make_channel(), CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(std::fs::read(dst_dir.join("a.txt")).unwrap(),        b"a");
        assert_eq!(std::fs::read(dst_dir.join("nested/b.txt")).unwrap(), b"b");
    }

    /// On the same filesystem `move_entry` should use atomic rename — instant,
    /// no data copied.  Verify the source disappears and the content is intact.
    #[tokio::test]
    async fn move_renames_on_same_filesystem() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("original.txt");
        let dst = tmp.path().join("moved.txt");
        std::fs::write(&src, b"move me").unwrap();

        move_entry(src.clone(), dst.clone(), make_channel(), CancellationToken::new())
            .await
            .unwrap();

        assert!(!src.exists(), "source still exists after move");
        assert_eq!(std::fs::read(&dst).unwrap(), b"move me");
    }

    /// A pre-cancelled token must abort immediately AND remove any partial
    /// destination file — the copy loop is biased toward the cancel branch.
    #[tokio::test]
    async fn cancelled_copy_removes_partial_destination() {
        let tmp   = TempDir::new().unwrap();
        let src   = tmp.path().join("large.bin");
        let dst   = tmp.path().join("partial.bin");
        // 512 KB — large enough that cancellation happens before all data is read
        std::fs::write(&src, vec![0u8; 512 * 1024]).unwrap();

        let cancel = CancellationToken::new();
        cancel.cancel(); // trigger before the copy even starts

        let result = copy(src, dst.clone(), make_channel(), cancel).await;

        assert!(result.is_err(), "expected cancellation error");
        // The partial destination (created before the select loop) must be cleaned up
        assert!(!dst.exists(), "partial destination was not removed");
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}
