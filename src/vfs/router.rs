//! `RoutingProvider` — top-level `VfsProvider` that delegates to the
//! appropriate backend based on the `VfsPath` variant and archive format.
//!
//! | VfsPath variant | Delegated to                    |
//! |-----------------|---------------------------------|
//! | Local(_)        | LocalFsProvider                 |
//! | Archive { .. }  | archive::list_archive_dir / … (format-detected) |

use anyhow::{Context, Result};

use super::{ArchiveFormat, VfsFileInfo, VfsPath, VfsProvider};
use super::{archive, local::LocalFsProvider};

pub struct RoutingProvider;

impl VfsProvider for RoutingProvider {
    fn read_dir(&self, path: &VfsPath) -> Result<Vec<VfsFileInfo>> {
        match path {
            VfsPath::Local(_) => LocalFsProvider.read_dir(path),
            VfsPath::Archive { archive_path, internal_path } => {
                let format = detect_format(archive_path)?;
                archive::list_archive_dir(archive_path, format, internal_path)
            }
        }
    }

    fn read_file(&self, path: &VfsPath) -> Result<Vec<u8>> {
        match path {
            VfsPath::Local(_) => LocalFsProvider.read_file(path),
            VfsPath::Archive { archive_path, internal_path } => {
                let format = detect_format(archive_path)?;
                archive::read_archive_file(archive_path, format, internal_path)
            }
        }
    }

    fn parent(&self, path: &VfsPath) -> Option<VfsPath> {
        path.parent()
    }
}

/// Detect archive format from the archive file's name.
fn detect_format(archive_path: &std::path::Path) -> Result<ArchiveFormat> {
    let fname = archive_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    ArchiveFormat::detect(fname)
        .with_context(|| format!("unrecognised archive format: {}", archive_path.display()))
}
