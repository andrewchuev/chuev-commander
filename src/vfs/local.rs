//! Local-filesystem `VfsProvider` implementation.
//!
//! Wraps `std::fs` and translates `DirEntry` values into `VfsFileInfo`.
//! All errors are returned (not panicked) so callers can show a popup.

use anyhow::{Context, Result};
use tracing::warn;

use super::{VfsFileInfo, VfsPath, VfsProvider};

pub struct LocalFsProvider;

impl VfsProvider for LocalFsProvider {
    fn read_dir(&self, path: &VfsPath) -> Result<Vec<VfsFileInfo>> {
        let dir_path = match path {
            VfsPath::Local(p) => p.clone(),
            VfsPath::Archive { .. } => {
                anyhow::bail!("LocalFsProvider cannot browse inside archives")
            }
        };

        let read_dir = std::fs::read_dir(&dir_path)
            .with_context(|| format!("reading directory {}", dir_path.display()))?;

        let mut entries = Vec::new();

        for result in read_dir {
            let entry = match result {
                Ok(e) => e,
                Err(e) => {
                    warn!("skipping unreadable entry: {e}");
                    continue;
                }
            };

            // Use symlink_metadata so we can detect symlinks without following them
            let meta = match entry.path().symlink_metadata() {
                Ok(m) => m,
                Err(e) => {
                    warn!(
                        "skipping '{}' (no metadata): {e}",
                        entry.file_name().to_string_lossy()
                    );
                    continue;
                }
            };

            let name       = entry.file_name().to_string_lossy().into_owned();
            let is_symlink = meta.file_type().is_symlink();
            // For the is_dir flag follow the symlink so we can enter symlinked dirs
            let is_dir       = entry.path().is_dir();
            let size         = if is_dir { None } else { Some(meta.len()) };
            let modified     = meta.modified().ok();
            let permissions  = format_permissions(&meta);
            let is_executable = !is_dir && is_exec(&meta);

            entries.push(VfsFileInfo {
                name,
                size,
                is_dir,
                is_symlink,
                is_executable,
                modified,
                permissions,
                path: VfsPath::Local(entry.path()),
            });
        }

        // Directories first, then files; each group sorted case-insensitively.
        // Sorting here is the provider's default; PanelState re-sorts per user preference.
        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _             => a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()),
        });

        Ok(entries)
    }

    fn read_file(&self, path: &VfsPath) -> Result<Vec<u8>> {
        let file_path = match path {
            VfsPath::Local(p) => p.clone(),
            VfsPath::Archive { .. } => {
                anyhow::bail!("LocalFsProvider cannot read archive members")
            }
        };

        std::fs::read(&file_path)
            .with_context(|| format!("reading file {}", file_path.display()))
    }

    fn parent(&self, path: &VfsPath) -> Option<VfsPath> {
        match path {
            VfsPath::Local(p) => p.parent().map(|p| VfsPath::Local(p.to_path_buf())),
            // Navigating "up" from inside an archive lands on the archive file itself
            VfsPath::Archive { archive_path, .. } => {
                Some(VfsPath::Local(archive_path.clone()))
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(unix)]
fn is_exec(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_exec(_meta: &std::fs::Metadata) -> bool { false }

#[cfg(unix)]
fn format_permissions(meta: &std::fs::Metadata) -> String {
    use std::os::unix::fs::PermissionsExt;
    let m = meta.permissions().mode();
    let chars = [
        if m & 0o400 != 0 { 'r' } else { '-' },
        if m & 0o200 != 0 { 'w' } else { '-' },
        if m & 0o100 != 0 { 'x' } else { '-' },
        if m & 0o040 != 0 { 'r' } else { '-' },
        if m & 0o020 != 0 { 'w' } else { '-' },
        if m & 0o010 != 0 { 'x' } else { '-' },
        if m & 0o004 != 0 { 'r' } else { '-' },
        if m & 0o002 != 0 { 'w' } else { '-' },
        if m & 0o001 != 0 { 'x' } else { '-' },
    ];
    chars.iter().collect()
}

#[cfg(not(unix))]
fn format_permissions(meta: &std::fs::Metadata) -> String {
    if meta.permissions().readonly() { "R".into() } else { String::new() }
}
