//! Single-instance enforcement via an advisory `flock` on a per-user lockfile.
//!
//! The lock is released automatically when the returned [`LockGuard`] is
//! dropped or when the process exits, including crashes, because the kernel
//! drops file locks on close.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::PathBuf;

use nix::fcntl::{Flock, FlockArg};

use crate::error::{Error, Result};

/// Holds the lock until dropped. Keep it alive for the duration you want
/// exclusive access.
pub struct LockGuard {
    // The Flock owns the File; dropping it releases the lock.
    _flock: Flock<File>,
    path: PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        tracing::debug!(path = %self.path.display(), "released single-instance lock");
    }
}

/// Try to acquire the lock. Returns [`Error::AlreadyRunning`] if another
/// instance holds it.
pub fn acquire() -> Result<LockGuard> {
    let path = lock_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)?;

    let fd = file.as_raw_fd();
    tracing::debug!(path = %path.display(), fd, "attempting single-instance lock");

    match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
        Ok(flock) => Ok(LockGuard {
            _flock: flock,
            path,
        }),
        Err((_file, nix::errno::Errno::EWOULDBLOCK)) => Err(Error::AlreadyRunning),
        Err((_file, errno)) => Err(Error::Io(std::io::Error::from_raw_os_error(errno as i32))),
    }
}

fn lock_path() -> Result<PathBuf> {
    // Prefer XDG_RUNTIME_DIR (tmpfs, cleaned on logout). Fall back to /tmp.
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    Ok(base.join("byt.lock"))
}
