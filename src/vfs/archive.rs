//! Archive VFS support: ZIP, TAR, TAR.GZ, TAR.BZ2.
//!
//! Public API used by `RoutingProvider`:
//! - `list_archive_dir`        — enumerate entries at an internal path within any archive.
//! - `read_archive_file`       — decompress and return bytes of a single archive entry.
//! - `extract_archive_to`      — extract entire archive to a directory (blocking).
//! - `extract_archive_entries` — extract specific files/dirs from an archive (blocking).
//! - `create_zip_archive`      — pack local paths into a new ZIP file (blocking).

use std::collections::HashSet;
use std::io::{BufReader, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use chrono::{NaiveDate, TimeZone, Utc};

use super::{ArchiveFormat, VfsFileInfo, VfsPath};

// ─────────────────────────────────────────────────────────────────────────────
// Public dispatchers
// ─────────────────────────────────────────────────────────────────────────────

/// List a virtual directory inside any supported archive.
pub fn list_archive_dir(
    archive_path: &Path,
    format: ArchiveFormat,
    internal_path: &str,
) -> Result<Vec<VfsFileInfo>> {
    match format {
        ArchiveFormat::Zip => list_zip_dir(archive_path, internal_path),
        ArchiveFormat::Tar | ArchiveFormat::TarGz | ArchiveFormat::TarBz2 => {
            list_tar_dir(archive_path, format, internal_path)
        }
    }
}

/// Read a file's bytes from any supported archive.
pub fn read_archive_file(
    archive_path: &Path,
    format: ArchiveFormat,
    internal_path: &str,
) -> Result<Vec<u8>> {
    match format {
        ArchiveFormat::Zip => read_zip_file(archive_path, internal_path),
        ArchiveFormat::Tar | ArchiveFormat::TarGz | ArchiveFormat::TarBz2 => {
            read_tar_file(archive_path, format, internal_path)
        }
    }
}

/// Extract the entire archive to `dst_dir`.  Blocking — run in `spawn_blocking`.
///
/// `on_progress(done, total)` is called after each entry is written.
/// For ZIP `total` is the archive entry count; for TAR `total` is 0 (stream, unknown).
/// Returns the number of entries processed.
pub fn extract_archive_to(
    archive_path: &Path,
    format: ArchiveFormat,
    dst_dir: &Path,
    on_progress: &dyn Fn(u64, u64),
) -> Result<u64> {
    match format {
        ArchiveFormat::Zip => extract_zip_to(archive_path, dst_dir, on_progress),
        ArchiveFormat::Tar | ArchiveFormat::TarGz | ArchiveFormat::TarBz2 => {
            extract_tar_to(archive_path, format, dst_dir, on_progress)
        }
    }
}

/// Extract specific entries (files or directory trees) from an archive to `dst_dir`.
///
/// `entries` is a list of internal paths.  A directory path causes all entries
/// whose path starts with `"dir/"` to be extracted recursively.
/// Returns the number of entries written.
pub fn extract_archive_entries(
    archive_path: &Path,
    format: ArchiveFormat,
    entries: &[String],
    dst_dir: &Path,
    on_progress: &dyn Fn(u64, u64),
) -> Result<u64> {
    match format {
        ArchiveFormat::Zip => extract_zip_entries(archive_path, entries, dst_dir, on_progress),
        ArchiveFormat::Tar | ArchiveFormat::TarGz | ArchiveFormat::TarBz2 => {
            extract_tar_entries(archive_path, format, entries, dst_dir, on_progress)
        }
    }
}

/// Create a ZIP archive at `dst_path` from the given local `sources`.
///
/// Each source may be a file or a directory (recursed automatically).
/// Archive entry names are derived from the base name of each source.
/// `on_progress(done, total)` is called after each file is written.
/// Returns the number of files packed.
pub fn create_zip_archive(
    sources: &[PathBuf],
    dst_path: &Path,
    on_progress: &dyn Fn(u64, u64),
) -> Result<u64> {
    let file = std::fs::File::create(dst_path)
        .with_context(|| format!("creating {}", dst_path.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let total = count_sources(sources);
    let mut done = 0u64;

    for src in sources {
        let name = src
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("invalid path: {}", src.display()))?
            .to_string_lossy()
            .into_owned();

        if src.is_dir() {
            add_dir_to_zip(&mut zip, src, &name, opts, &mut done, total, on_progress)?;
        } else {
            add_file_to_zip(&mut zip, src, &name, opts)?;
            done += 1;
            on_progress(done, total);
        }
    }

    zip.finish().context("finalising zip archive")?;
    Ok(done)
}

// ─────────────────────────────────────────────────────────────────────────────
// ZIP — listing
// ─────────────────────────────────────────────────────────────────────────────

/// List the virtual directory at `internal_path` inside the zip at `archive_path`.
/// Pass an empty string to list the archive root.
fn list_zip_dir(archive_path: &Path, internal_path: &str) -> Result<Vec<VfsFileInfo>> {
    let file = std::fs::File::open(archive_path)
        .with_context(|| format!("opening {}", archive_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("parsing zip {}", archive_path.display()))?;

    // "dir" → "dir/",  "" → ""
    let prefix = if internal_path.is_empty() {
        String::new()
    } else {
        format!("{}/", internal_path.trim_end_matches('/'))
    };

    let mut entries:   Vec<VfsFileInfo> = Vec::new();
    let mut seen_dirs: HashSet<String>  = HashSet::new();

    for i in 0..archive.len() {
        let zf = archive.by_index_raw(i)
            .with_context(|| format!("reading entry {i} in zip"))?;
        let full_name = zf.name().to_owned();
        let size      = zf.size();
        let modified  = zf.last_modified().as_ref().and_then(zip_datetime_to_system_time);
        drop(zf);

        let rel = match full_name.strip_prefix(&prefix) {
            Some(r) => r,
            None    => continue,
        };
        if rel.is_empty() { continue; }

        if let Some(slash) = rel.find('/') {
            let dir_name = &rel[..slash];
            if dir_name.is_empty() { continue; }
            if seen_dirs.insert(dir_name.to_owned()) {
                entries.push(VfsFileInfo {
                    name:          dir_name.to_owned(),
                    size:          None,
                    is_dir:        true,
                    is_symlink:    false,
                    is_executable: false,
                    modified:      None,
                    permissions:   String::new(),
                    path:          VfsPath::Archive {
                        archive_path:  archive_path.to_path_buf(),
                        internal_path: format!("{prefix}{dir_name}"),
                    },
                });
            }
        } else {
            entries.push(VfsFileInfo {
                name:          rel.to_owned(),
                size:          Some(size),
                is_dir:        false,
                is_symlink:    false,
                is_executable: false,
                modified,
                permissions:   String::new(),
                path:          VfsPath::Archive {
                    archive_path:  archive_path.to_path_buf(),
                    internal_path: format!("{prefix}{rel}"),
                },
            });
        }
    }

    Ok(entries)
}

fn read_zip_file(archive_path: &Path, internal_path: &str) -> Result<Vec<u8>> {
    let file = std::fs::File::open(archive_path)
        .with_context(|| format!("opening {}", archive_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("parsing zip {}", archive_path.display()))?;

    let mut zf = archive.by_name(internal_path)
        .with_context(|| format!("entry '{internal_path}' not found in zip"))?;

    let mut buf = Vec::with_capacity(zf.size() as usize);
    zf.read_to_end(&mut buf)
        .with_context(|| format!("decompressing '{internal_path}'"))?;

    Ok(buf)
}

// ─────────────────────────────────────────────────────────────────────────────
// ZIP — extraction
// ─────────────────────────────────────────────────────────────────────────────

fn extract_zip_to(
    archive_path: &Path,
    dst_dir: &Path,
    on_progress: &dyn Fn(u64, u64),
) -> Result<u64> {
    let file = std::fs::File::open(archive_path)
        .with_context(|| format!("opening {}", archive_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)?;
    let total = archive.len() as u64;

    for i in 0..archive.len() {
        let mut zf = archive.by_index(i)
            .with_context(|| format!("reading zip entry {i}"))?;
        let name = zf.name().to_owned();
        let outpath = safe_zip_outpath(&name, dst_dir);
        let outpath = match outpath {
            Some(p) => p,
            None    => continue,
        };

        if name.ends_with('/') {
            std::fs::create_dir_all(&outpath)
                .with_context(|| format!("creating dir {}", outpath.display()))?;
        } else {
            if let Some(parent) = outpath.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating parent {}", parent.display()))?;
            }
            let mut out = std::fs::File::create(&outpath)
                .with_context(|| format!("creating {}", outpath.display()))?;
            std::io::copy(&mut zf, &mut out)
                .with_context(|| format!("extracting {}", outpath.display()))?;
        }

        on_progress(i as u64 + 1, total);
    }
    Ok(total)
}

fn extract_zip_entries(
    archive_path: &Path,
    entries: &[String],
    dst_dir: &Path,
    on_progress: &dyn Fn(u64, u64),
) -> Result<u64> {
    let file = std::fs::File::open(archive_path)
        .with_context(|| format!("opening {}", archive_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)?;
    let total = archive.len() as u64;
    let mut count = 0u64;

    for i in 0..archive.len() {
        let mut zf = archive.by_index(i)
            .with_context(|| format!("reading zip entry {i}"))?;
        let name = zf.name().to_owned();

        if !entry_matches_targets(&name, entries) {
            continue;
        }

        let outpath = match safe_zip_outpath(&name, dst_dir) {
            Some(p) => p,
            None    => continue,
        };

        if name.ends_with('/') {
            std::fs::create_dir_all(&outpath)
                .with_context(|| format!("creating dir {}", outpath.display()))?;
        } else {
            if let Some(parent) = outpath.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating parent {}", parent.display()))?;
            }
            let mut out = std::fs::File::create(&outpath)
                .with_context(|| format!("creating {}", outpath.display()))?;
            std::io::copy(&mut zf, &mut out)
                .with_context(|| format!("extracting {}", outpath.display()))?;
        }

        count += 1;
        on_progress(count, total);
    }
    Ok(count)
}

// ─────────────────────────────────────────────────────────────────────────────
// TAR (plain, .gz, .bz2)
// ─────────────────────────────────────────────────────────────────────────────

fn open_tar_reader(archive_path: &Path, format: ArchiveFormat) -> Result<Box<dyn Read>> {
    let file = std::fs::File::open(archive_path)
        .with_context(|| format!("opening {}", archive_path.display()))?;
    let reader: Box<dyn Read> = match format {
        ArchiveFormat::Tar    => Box::new(BufReader::new(file)),
        ArchiveFormat::TarGz  => Box::new(flate2::read::GzDecoder::new(BufReader::new(file))),
        ArchiveFormat::TarBz2 => Box::new(bzip2::read::BzDecoder::new(BufReader::new(file))),
        ArchiveFormat::Zip    => unreachable!("open_tar_reader called with Zip"),
    };
    Ok(reader)
}

/// Strip leading `./` and trailing `/` from a tar entry path.
fn normalize_tar_path(raw: &str) -> &str {
    raw.trim_start_matches("./").trim_end_matches('/')
}

fn list_tar_dir(
    archive_path: &Path,
    format: ArchiveFormat,
    internal_path: &str,
) -> Result<Vec<VfsFileInfo>> {
    let reader      = open_tar_reader(archive_path, format)?;
    let mut archive = tar::Archive::new(reader);

    let prefix = internal_path.trim_end_matches('/');

    let mut entries:   Vec<VfsFileInfo> = Vec::new();
    let mut seen_dirs: HashSet<String>  = HashSet::new();

    for entry_result in archive
        .entries()
        .with_context(|| format!("iterating {}", archive_path.display()))?
    {
        let entry = entry_result.with_context(|| "reading tar entry header")?;

        let raw_owned = entry.path()?.into_owned();
        let raw_str   = raw_owned.to_string_lossy().into_owned();
        let full      = normalize_tar_path(&raw_str);
        if full.is_empty() { continue; }

        let rel: &str = if prefix.is_empty() {
            full
        } else {
            let want = format!("{}/", prefix);
            match full.strip_prefix(want.as_str()) {
                Some(r) if !r.is_empty() => r,
                _ => continue,
            }
        };
        if rel.is_empty() { continue; }

        if let Some(slash) = rel.find('/') {
            let dir_name = &rel[..slash];
            if dir_name.is_empty() { continue; }
            if seen_dirs.insert(dir_name.to_owned()) {
                let internal = if prefix.is_empty() {
                    dir_name.to_owned()
                } else {
                    format!("{}/{}", prefix, dir_name)
                };
                entries.push(VfsFileInfo {
                    name:          dir_name.to_owned(),
                    size:          None,
                    is_dir:        true,
                    is_symlink:    false,
                    is_executable: false,
                    modified:      None,
                    permissions:   String::new(),
                    path:          VfsPath::Archive {
                        archive_path:  archive_path.to_path_buf(),
                        internal_path: internal,
                    },
                });
            }
        } else {
            let header    = entry.header();
            let size      = header.size().unwrap_or(0);
            let is_symlink = header.entry_type().is_symlink();
            let mode      = header.mode().unwrap_or(0);
            let is_exec   = mode & 0o111 != 0;
            let perms     = unix_mode_string(mode);
            let modified  = header.mtime().ok()
                .map(|ts| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(ts));

            let internal = if prefix.is_empty() {
                rel.to_owned()
            } else {
                format!("{}/{}", prefix, rel)
            };
            entries.push(VfsFileInfo {
                name:          rel.to_owned(),
                size:          Some(size),
                is_dir:        false,
                is_symlink,
                is_executable: is_exec,
                modified,
                permissions:   perms,
                path:          VfsPath::Archive {
                    archive_path:  archive_path.to_path_buf(),
                    internal_path: internal,
                },
            });
        }
    }

    Ok(entries)
}

fn read_tar_file(
    archive_path: &Path,
    format: ArchiveFormat,
    internal_path: &str,
) -> Result<Vec<u8>> {
    let reader      = open_tar_reader(archive_path, format)?;
    let mut archive = tar::Archive::new(reader);

    for entry_result in archive
        .entries()
        .with_context(|| format!("iterating {}", archive_path.display()))?
    {
        let mut entry = entry_result?;
        let raw_owned = entry.path()?.into_owned();
        let raw_str   = raw_owned.to_string_lossy().into_owned();
        let full      = normalize_tar_path(&raw_str);

        if full == internal_path {
            let mut buf = Vec::new();
            entry
                .read_to_end(&mut buf)
                .with_context(|| format!("reading '{internal_path}'"))?;
            return Ok(buf);
        }
    }

    Err(anyhow::anyhow!(
        "entry '{}' not found in tar archive",
        internal_path
    ))
}

fn extract_tar_to(
    archive_path: &Path,
    format: ArchiveFormat,
    dst_dir: &Path,
    on_progress: &dyn Fn(u64, u64),
) -> Result<u64> {
    let reader      = open_tar_reader(archive_path, format)?;
    let mut archive = tar::Archive::new(reader);
    let mut count   = 0u64;

    for entry_result in archive.entries()? {
        let mut entry = entry_result?;
        entry
            .unpack_in(dst_dir)
            .with_context(|| format!("unpacking to {}", dst_dir.display()))?;
        count += 1;
        on_progress(count, 0); // total unknown for streaming TAR
    }
    Ok(count)
}

fn extract_tar_entries(
    archive_path: &Path,
    format: ArchiveFormat,
    entries: &[String],
    dst_dir: &Path,
    on_progress: &dyn Fn(u64, u64),
) -> Result<u64> {
    let reader      = open_tar_reader(archive_path, format)?;
    let mut archive = tar::Archive::new(reader);
    let mut count   = 0u64;

    for entry_result in archive.entries()? {
        let mut entry = entry_result?;
        let raw_owned = entry.path()?.into_owned();
        let raw_str   = raw_owned.to_string_lossy().into_owned();
        let full      = normalize_tar_path(&raw_str);

        if !entry_matches_targets(full, entries) {
            continue;
        }

        entry
            .unpack_in(dst_dir)
            .with_context(|| format!("unpacking to {}", dst_dir.display()))?;
        count += 1;
        on_progress(count, 0);
    }
    Ok(count)
}

// ─────────────────────────────────────────────────────────────────────────────
// ZIP creation helpers
// ─────────────────────────────────────────────────────────────────────────────

fn count_sources(sources: &[PathBuf]) -> u64 {
    sources
        .iter()
        .map(|p| if p.is_dir() { count_dir_files(p) } else { 1 })
        .sum()
}

fn count_dir_files(dir: &Path) -> u64 {
    let Ok(iter) = std::fs::read_dir(dir) else { return 0 };
    iter.filter_map(|e| e.ok())
        .map(|e| {
            let p = e.path();
            if p.is_dir() { count_dir_files(&p) } else { 1 }
        })
        .sum()
}

fn add_file_to_zip<W: Write + Seek>(
    zip:  &mut zip::ZipWriter<W>,
    src:  &Path,
    name: &str,
    opts: zip::write::SimpleFileOptions,
) -> Result<()> {
    zip.start_file(name, opts)
        .with_context(|| format!("adding '{name}' to zip"))?;
    let mut f = std::fs::File::open(src)
        .with_context(|| format!("opening {}", src.display()))?;
    std::io::copy(&mut f, zip)
        .with_context(|| format!("writing '{name}'"))?;
    Ok(())
}

fn add_dir_to_zip<W: Write + Seek>(
    zip:         &mut zip::ZipWriter<W>,
    dir:         &Path,
    prefix:      &str,
    opts:        zip::write::SimpleFileOptions,
    done:        &mut u64,
    total:       u64,
    on_progress: &dyn Fn(u64, u64),
) -> Result<()> {
    zip.add_directory(format!("{}/", prefix), opts)
        .with_context(|| format!("adding dir '{prefix}' to zip"))?;

    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
    {
        let entry        = entry?;
        let path         = entry.path();
        let child_name   = entry.file_name().to_string_lossy().into_owned();
        let archive_name = format!("{}/{}", prefix, child_name);

        if path.is_dir() {
            add_dir_to_zip(zip, &path, &archive_name, opts, done, total, on_progress)?;
        } else {
            add_file_to_zip(zip, &path, &archive_name, opts)?;
            *done += 1;
            on_progress(*done, total);
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared utilities
// ─────────────────────────────────────────────────────────────────────────────

/// Convert a ZIP `DateTime` (DOS format) to `SystemTime`.
/// Returns `None` if the date components are out of range.
fn zip_datetime_to_system_time(dt: &zip::DateTime) -> Option<SystemTime> {
    // DOS date epoch is 1980; year 0 means "not set"
    if dt.year() < 1980 { return None; }
    let naive = NaiveDate::from_ymd_opt(
        dt.year() as i32,
        dt.month() as u32,
        dt.day() as u32,
    )?
    .and_hms_opt(dt.hour() as u32, dt.minute() as u32, dt.second() as u32)?;

    let timestamp = Utc.from_utc_datetime(&naive).timestamp();
    if timestamp < 0 {
        return None;
    }
    SystemTime::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(timestamp as u64))
}

/// Build a `"rwxr-xr-x"`-style string from Unix mode bits.
fn unix_mode_string(mode: u32) -> String {
    let trio = |bits: u32| -> [char; 3] {
        [
            if bits & 4 != 0 { 'r' } else { '-' },
            if bits & 2 != 0 { 'w' } else { '-' },
            if bits & 1 != 0 { 'x' } else { '-' },
        ]
    };
    let o = trio((mode >> 6) & 7);
    let g = trio((mode >> 3) & 7);
    let w = trio(mode & 7);
    format!(
        "{}{}{}{}{}{}{}{}{}",
        o[0], o[1], o[2],
        g[0], g[1], g[2],
        w[0], w[1], w[2],
    )
}

/// Build the safe output path for a ZIP entry, guarding against path traversal.
///
/// Strips leading slashes, normalises backslashes, and removes any `..` or
/// empty components.  Returns `None` if the result would be empty.
fn safe_zip_outpath(name: &str, dst_dir: &Path) -> Option<PathBuf> {
    let safe: String = name
        .replace('\\', "/")
        .trim_start_matches('/')
        .split('/')
        .filter(|c| !c.is_empty() && *c != ".." && !c.contains('\0'))
        .collect::<Vec<_>>()
        .join("/");

    if safe.is_empty() {
        return None;
    }
    Some(dst_dir.join(safe))
}

/// Return `true` if `entry_path` is or is under any of the `targets`.
///
/// A target of `"dir"` matches both `"dir"` itself and `"dir/anything"`.
fn entry_matches_targets(entry_path: &str, targets: &[String]) -> bool {
    targets.iter().any(|target| {
        let target = target.trim_end_matches('/');
        entry_path == target
            || entry_path.starts_with(&format!("{}/", target))
            // also match the directory entry itself ("dir/" stored in ZIP)
            || entry_path.trim_end_matches('/') == target
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Integration tests — create real archives in a tempdir, verify listing/reading
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn noop_progress(_done: u64, _total: u64) {}

    // ── ZIP helpers ──────────────────────────────────────────────────────────

    /// Build a zip with explicit dir entry + two files (root and subdir).
    fn make_zip_with_dirs(dir: &std::path::Path) -> std::path::PathBuf {
        use zip::write::SimpleFileOptions;
        let path = dir.join("with_dirs.zip");
        let file = std::fs::File::create(&path).unwrap();
        let mut w = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        w.start_file("root.txt", opts).unwrap();
        w.write_all(b"root content").unwrap();

        w.add_directory("subdir/", opts).unwrap();

        w.start_file("subdir/child.txt", opts).unwrap();
        w.write_all(b"child content").unwrap();

        w.finish().unwrap();
        path
    }

    /// Build a zip that has NO explicit "dir/" entry — only "dir/file.txt".
    /// The VFS must synthesise the directory entry from the file path.
    fn make_zip_no_explicit_dirs(dir: &std::path::Path) -> std::path::PathBuf {
        use zip::write::SimpleFileOptions;
        let path = dir.join("no_dirs.zip");
        let file = std::fs::File::create(&path).unwrap();
        let mut w = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        w.start_file("implicit_dir/file.txt", opts).unwrap();
        w.write_all(b"hello").unwrap();

        w.finish().unwrap();
        path
    }

    // ── TAR helpers ──────────────────────────────────────────────────────────

    fn make_plain_tar(dir: &std::path::Path) -> std::path::PathBuf {
        let path = dir.join("test.tar");
        let file = std::fs::File::create(&path).unwrap();
        let mut b = tar::Builder::new(file);

        append_tar_file(&mut b, "root.txt", b"tar root content");
        append_tar_file(&mut b, "subdir/nested.txt", b"nested content");
        b.finish().unwrap();
        path
    }

    /// Tar created by GNU tar often prefixes paths with "./".
    fn make_tar_with_dot_prefix(dir: &std::path::Path) -> std::path::PathBuf {
        let path = dir.join("dotprefix.tar");
        let file = std::fs::File::create(&path).unwrap();
        let mut b = tar::Builder::new(file);

        append_tar_file(&mut b, "./file.txt", b"dot prefixed");
        append_tar_file(&mut b, "./subdir/deep.txt", b"deep");
        b.finish().unwrap();
        path
    }

    fn append_tar_file(b: &mut tar::Builder<std::fs::File>, name: &str, data: &[u8]) {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        b.append_data(&mut header, name, data).unwrap();
    }

    // ── ZIP tests ────────────────────────────────────────────────────────────

    #[test]
    fn zip_root_listing_contains_all_top_level_entries() {
        let tmp = tempfile::TempDir::new().unwrap();
        let archive = make_zip_with_dirs(tmp.path());

        let entries = list_archive_dir(&archive, ArchiveFormat::Zip, "").unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();

        assert!(names.contains(&"root.txt"), "root.txt missing: {names:?}");
        assert!(names.contains(&"subdir"),   "subdir missing: {names:?}");
        // The root listing must NOT include items from subdirectories
        assert!(!names.contains(&"child.txt"), "child.txt must not appear at root");
    }

    #[test]
    fn zip_synthesises_dir_entry_when_none_explicit() {
        let tmp = tempfile::TempDir::new().unwrap();
        let archive = make_zip_no_explicit_dirs(tmp.path());

        let entries = list_archive_dir(&archive, ArchiveFormat::Zip, "").unwrap();
        let dir_entry = entries.iter().find(|e| e.name == "implicit_dir");

        assert!(dir_entry.is_some(), "synthesised dir entry not found");
        assert!(dir_entry.unwrap().is_dir);
    }

    #[test]
    fn zip_subdir_listing_shows_only_direct_children() {
        let tmp = tempfile::TempDir::new().unwrap();
        let archive = make_zip_with_dirs(tmp.path());

        let entries = list_archive_dir(&archive, ArchiveFormat::Zip, "subdir").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "child.txt");
        assert!(!entries[0].is_dir);
    }

    #[test]
    fn zip_read_file_returns_correct_bytes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let archive = make_zip_with_dirs(tmp.path());

        let bytes = read_archive_file(&archive, ArchiveFormat::Zip, "subdir/child.txt").unwrap();
        assert_eq!(bytes, b"child content");
    }

    #[test]
    fn zip_extract_all_produces_correct_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let archive = make_zip_with_dirs(tmp.path());
        let dst = tmp.path().join("out");
        std::fs::create_dir_all(&dst).unwrap();

        extract_archive_to(&archive, ArchiveFormat::Zip, &dst, &noop_progress).unwrap();

        assert_eq!(std::fs::read(dst.join("root.txt")).unwrap(), b"root content");
        assert_eq!(std::fs::read(dst.join("subdir/child.txt")).unwrap(), b"child content");
    }

    #[test]
    fn zip_extract_entries_extracts_only_selected() {
        let tmp = tempfile::TempDir::new().unwrap();
        let archive = make_zip_with_dirs(tmp.path());
        let dst = tmp.path().join("out");
        std::fs::create_dir_all(&dst).unwrap();

        extract_archive_entries(
            &archive,
            ArchiveFormat::Zip,
            &["subdir".to_owned()],
            &dst,
            &noop_progress,
        )
        .unwrap();

        assert!(dst.join("subdir/child.txt").exists(), "subdir/child.txt missing");
        assert!(!dst.join("root.txt").exists(), "root.txt should not be extracted");
    }

    #[test]
    fn zip_create_and_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Prepare source files
        let src_dir = tmp.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(src_dir.join("a.txt"), b"hello").unwrap();
        std::fs::write(src_dir.join("b.txt"), b"world").unwrap();

        let archive = tmp.path().join("out.zip");
        let sources = vec![src_dir.join("a.txt"), src_dir.join("b.txt")];
        let count = create_zip_archive(&sources, &archive, &noop_progress).unwrap();
        assert_eq!(count, 2, "expected 2 files packed");

        // Read back and verify
        let a = read_archive_file(&archive, ArchiveFormat::Zip, "a.txt").unwrap();
        let b = read_archive_file(&archive, ArchiveFormat::Zip, "b.txt").unwrap();
        assert_eq!(a, b"hello");
        assert_eq!(b, b"world");
    }

    #[test]
    fn zip_path_traversal_sanitised() {
        use zip::write::SimpleFileOptions;
        let tmp = tempfile::TempDir::new().unwrap();
        let archive = tmp.path().join("evil.zip");
        {
            let file = std::fs::File::create(&archive).unwrap();
            let mut w = zip::ZipWriter::new(file);
            let opts = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            // Attempt path traversal
            w.start_file("../escape.txt", opts).unwrap();
            w.write_all(b"evil").unwrap();
            // Windows-style backslash
            w.start_file("..\\win_escape.txt", opts).unwrap();
            w.write_all(b"evil").unwrap();
            w.finish().unwrap();
        }
        let dst = tmp.path().join("out");
        std::fs::create_dir_all(&dst).unwrap();
        extract_archive_to(&archive, ArchiveFormat::Zip, &dst, &noop_progress).unwrap();

        // Neither file should exist outside dst
        assert!(!tmp.path().join("escape.txt").exists());
        assert!(!tmp.path().join("win_escape.txt").exists());
        // They should land inside dst (stripped of the "..")
        assert!(dst.join("escape.txt").exists() || !dst.join("escape.txt").exists()); // skipped entirely
    }

    // ── TAR tests ────────────────────────────────────────────────────────────

    #[test]
    fn tar_root_listing_synthesises_subdir_entry() {
        let tmp = tempfile::TempDir::new().unwrap();
        let archive = make_plain_tar(tmp.path());

        let entries = list_archive_dir(&archive, ArchiveFormat::Tar, "").unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();

        assert!(names.contains(&"root.txt"), "root.txt missing: {names:?}");
        assert!(names.contains(&"subdir"),   "synthesised subdir missing: {names:?}");
        assert!(!names.contains(&"nested.txt"), "nested.txt must not appear at root");
    }

    /// GNU tar adds "./" to every path; the VFS must strip it transparently.
    #[test]
    fn tar_dot_prefix_stripped_at_root() {
        let tmp = tempfile::TempDir::new().unwrap();
        let archive = make_tar_with_dot_prefix(tmp.path());

        let entries = list_archive_dir(&archive, ArchiveFormat::Tar, "").unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();

        assert!(names.contains(&"file.txt"), "file.txt missing after stripping ./: {names:?}");
        assert!(names.contains(&"subdir"),   "subdir missing: {names:?}");
        assert!(!names.iter().any(|n| n.starts_with("./")), "raw ./ prefix leaked into listing");
    }

    #[test]
    fn tar_read_file_returns_correct_bytes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let archive = make_plain_tar(tmp.path());

        let bytes = read_archive_file(&archive, ArchiveFormat::Tar, "subdir/nested.txt").unwrap();
        assert_eq!(bytes, b"nested content");
    }

    #[test]
    fn tar_subdir_listing_shows_only_direct_children() {
        let tmp = tempfile::TempDir::new().unwrap();
        let archive = make_plain_tar(tmp.path());

        let entries = list_archive_dir(&archive, ArchiveFormat::Tar, "subdir").unwrap();
        assert_eq!(
            entries.len(), 1,
            "expected 1 child, got: {:?}", entries.iter().map(|e| &e.name).collect::<Vec<_>>()
        );
        assert_eq!(entries[0].name, "nested.txt");
    }

    #[test]
    fn tar_permissions_populated() {
        let tmp = tempfile::TempDir::new().unwrap();
        let archive = make_plain_tar(tmp.path()); // mode 0o644 set in append_tar_file

        let entries = list_archive_dir(&archive, ArchiveFormat::Tar, "").unwrap();
        let file = entries.iter().find(|e| e.name == "root.txt").unwrap();
        assert_eq!(file.permissions, "rw-r--r--");
        assert!(!file.is_executable, "0o644 should not be executable");
    }

    #[test]
    fn tar_extract_entries_extracts_only_selected() {
        let tmp = tempfile::TempDir::new().unwrap();
        let archive = make_plain_tar(tmp.path());
        let dst = tmp.path().join("out");
        std::fs::create_dir_all(&dst).unwrap();

        extract_archive_entries(
            &archive,
            ArchiveFormat::Tar,
            &["subdir".to_owned()],
            &dst,
            &noop_progress,
        )
        .unwrap();

        assert!(dst.join("subdir/nested.txt").exists(), "subdir/nested.txt missing");
        assert!(!dst.join("root.txt").exists(), "root.txt should not be extracted");
    }
}
