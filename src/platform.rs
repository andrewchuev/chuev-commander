//! Platform-specific helpers that don't belong in the VFS layer.

use std::path::Path;

/// Returns available (free) bytes on the filesystem that contains `path`.
/// Returns `None` when the query fails or is not supported on this platform.
#[cfg(unix)]
pub fn free_space_bytes(path: &Path) -> Option<u64> {
    use std::ffi::CString;

    let c_path = CString::new(path.to_str()?).ok()?;
    // SAFETY: we pass a valid null-terminated path and a zeroed stat buffer.
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stat) == 0 {
            Some(stat.f_bavail as u64 * stat.f_frsize as u64)
        } else {
            None
        }
    }
}

#[cfg(not(unix))]
pub fn free_space_bytes(_path: &Path) -> Option<u64> {
    None
}
