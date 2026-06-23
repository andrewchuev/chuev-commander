//! # Virtual File System (VFS) abstraction
//!
//! Panels never call `std::fs` directly.  Everything goes through a
//! `VfsProvider`, which makes adding archive support (zip, tar, …) or a
//! remote-file backend a matter of writing one new struct that implements the
//! trait — zero changes to panel or UI code.
//!
//! ## Extension path
//! 1. Add a new variant to `VfsPath` (e.g. `Sftp { host, path }`).
//! 2. Implement `VfsProvider` for the new backend.
//! 3. Construct the provider and store it in `PanelState`.

pub mod archive;
pub mod local;
pub mod router;

// ─────────────────────────────────────────────────────────────────────────────
// Archive format
// ─────────────────────────────────────────────────────────────────────────────

/// Supported archive formats, auto-detected from the file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    Tar,
    TarGz,
    TarBz2,
}

impl ArchiveFormat {
    /// Detect format from a filename.  Compound extensions (`.tar.gz`) are
    /// checked before simple ones (`.gz`) to avoid false matches.
    pub fn detect(filename: &str) -> Option<Self> {
        let low = filename.to_lowercase();
        if      low.ends_with(".tar.gz")  || low.ends_with(".tgz")   { Some(Self::TarGz)  }
        else if low.ends_with(".tar.bz2") || low.ends_with(".tbz2")  { Some(Self::TarBz2) }
        else if low.ends_with(".tar")                                  { Some(Self::Tar)    }
        else if low.ends_with(".zip")                                  { Some(Self::Zip)    }
        else                                                           { None               }
    }
}

#[cfg(test)]
mod archive_format_tests {
    use super::ArchiveFormat;

    #[test]
    fn detect_zip() {
        assert_eq!(ArchiveFormat::detect("archive.zip"), Some(ArchiveFormat::Zip));
    }

    #[test]
    fn detect_zip_uppercase() {
        assert_eq!(ArchiveFormat::detect("ARCHIVE.ZIP"), Some(ArchiveFormat::Zip));
    }

    #[test]
    fn detect_plain_tar() {
        assert_eq!(ArchiveFormat::detect("backup.tar"), Some(ArchiveFormat::Tar));
    }

    #[test]
    fn detect_tar_gz() {
        assert_eq!(ArchiveFormat::detect("project.tar.gz"), Some(ArchiveFormat::TarGz));
    }

    #[test]
    fn detect_tgz_alias() {
        assert_eq!(ArchiveFormat::detect("project.tgz"), Some(ArchiveFormat::TarGz));
    }

    #[test]
    fn detect_tar_bz2() {
        assert_eq!(ArchiveFormat::detect("dump.tar.bz2"), Some(ArchiveFormat::TarBz2));
    }

    #[test]
    fn detect_tbz2_alias() {
        assert_eq!(ArchiveFormat::detect("dump.tbz2"), Some(ArchiveFormat::TarBz2));
    }

    /// Compound extensions must win over simple ones: "data.tar.gz" is TarGz,
    /// not Tar (which would match if we checked ".tar" after stripping ".gz").
    #[test]
    fn compound_extension_wins() {
        assert_eq!(ArchiveFormat::detect("data.tar.gz"),  Some(ArchiveFormat::TarGz));
        assert_eq!(ArchiveFormat::detect("data.tar.bz2"), Some(ArchiveFormat::TarBz2));
    }

    #[test]
    fn detect_unknown_extension() {
        assert_eq!(ArchiveFormat::detect("report.pdf"), None);
    }

    #[test]
    fn detect_no_extension() {
        assert_eq!(ArchiveFormat::detect("Makefile"), None);
    }
}

use std::fmt;
use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::Result;

// ─────────────────────────────────────────────────────────────────────────────
// Path
// ─────────────────────────────────────────────────────────────────────────────

/// A location that can be displayed inside a panel.
///
/// `Local` covers ordinary directories.  `Archive` covers an entry *inside* an
/// archive file — the panel shows archive contents as if they were directories.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum VfsPath {
    Local(PathBuf),
    Archive {
        archive_path:  PathBuf,
        internal_path: String,
    },
}

impl VfsPath {
    /// Human-readable representation shown in the panel header.
    pub fn display_string(&self) -> String {
        match self {
            VfsPath::Local(p) => p.display().to_string(),
            VfsPath::Archive { archive_path, internal_path } => {
                format!("{}:/{internal_path}", archive_path.display())
            }
        }
    }

    /// The name of the last path component — used to restore cursor position
    /// after navigating up to the parent directory.
    pub fn last_component(&self) -> Option<String> {
        match self {
            VfsPath::Local(p) => {
                p.file_name().map(|n| n.to_string_lossy().into_owned())
            }
            VfsPath::Archive { archive_path, internal_path } => {
                if internal_path.is_empty() {
                    // At archive root: the entry visible in the parent is the archive file itself.
                    archive_path.file_name().map(|n| n.to_string_lossy().into_owned())
                } else {
                    internal_path.rsplit('/').next().map(str::to_owned)
                }
            }
        }
    }

    /// Return the parent of this path, or `None` if already at the filesystem root.
    ///
    /// For `Archive` paths, going above the archive root transitions back to the
    /// local directory that contains the archive file.
    pub fn parent(&self) -> Option<VfsPath> {
        match self {
            VfsPath::Local(p) => p.parent().map(|pp| VfsPath::Local(pp.to_path_buf())),
            VfsPath::Archive { archive_path, internal_path } => {
                if internal_path.is_empty() {
                    // Exit archive → local directory containing the archive file
                    archive_path.parent().map(|p| VfsPath::Local(p.to_path_buf()))
                } else if let Some(slash) = internal_path.rfind('/') {
                    Some(VfsPath::Archive {
                        archive_path:  archive_path.clone(),
                        internal_path: internal_path[..slash].to_owned(),
                    })
                } else {
                    // Top-level entry inside archive → archive root
                    Some(VfsPath::Archive {
                        archive_path:  archive_path.clone(),
                        internal_path: String::new(),
                    })
                }
            }
        }
    }

    /// The local filesystem path that is the closest ancestor of this location.
    /// For `Local` paths this is the path itself; for `Archive` paths it is the
    /// directory that contains the archive file.  Used when persisting panel state.
    pub fn local_root(&self) -> &std::path::Path {
        match self {
            VfsPath::Local(p) => p.as_path(),
            VfsPath::Archive { archive_path, .. } => {
                archive_path.parent().unwrap_or(archive_path.as_path())
            }
        }
    }
}

impl fmt::Display for VfsPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display_string())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// File info
// ─────────────────────────────────────────────────────────────────────────────

/// Normalised metadata for a single file-system entry.
///
/// Panels work exclusively with this struct — they never inspect a raw
/// `DirEntry`, `ZipFile`, or network response.
#[derive(Debug, Clone)]
pub struct VfsFileInfo {
    /// Base name (no parent path component).
    pub name:          String,
    /// `None` for directories or when stat fails.
    pub size:          Option<u64>,
    pub is_dir:        bool,
    pub is_symlink:    bool,
    /// True when any execute bit (owner/group/other) is set on Unix.
    pub is_executable: bool,
    pub modified:      Option<SystemTime>,
    /// Unix permission string "rwxr-xr-x"; empty string on Windows or on error.
    pub permissions:   String,
    /// Full VFS location, used when opening / copying / deleting this entry.
    pub path:          VfsPath,
}

// ─────────────────────────────────────────────────────────────────────────────
// Provider trait
// ─────────────────────────────────────────────────────────────────────────────

/// Capability required of any storage backend a panel can browse.
///
/// `Send + Sync` are required because providers will be wrapped in `Arc<dyn
/// VfsProvider>` and shared between the UI task and background I/O tasks.
pub trait VfsProvider: Send + Sync {
    /// List the contents of `path`.
    fn read_dir(&self, path: &VfsPath) -> Result<Vec<VfsFileInfo>>;

    /// Read the raw bytes of a file at `path`.
    ///
    /// For large files callers should consider a streaming variant (future work).
    fn read_file(&self, path: &VfsPath) -> Result<Vec<u8>>;

    /// Return the parent of `path`, or `None` if already at the root.
    fn parent(&self, path: &VfsPath) -> Option<VfsPath>;
}
