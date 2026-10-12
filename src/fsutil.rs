use std::{
    fs,
    io::{self, Write},
    path::Path,
    sync::Mutex,
};

use atomicwrites::{AllowOverwrite, AtomicFile};

// Serialize this process's commits. Windows can reject simultaneous
// MoveFileExW replacements of the same directory entry with ERROR_ACCESS_DENIED
// even though each individual replacement is atomic.
static WRITE_MUTEX: Mutex<()> = Mutex::new(());

/// Replaces a file atomically using a uniquely named temporary file in the
/// destination directory. On Windows, `atomicwrites` uses `MoveFileExW` with
/// replacement and write-through flags. Commits are serialized in-process;
/// transient Windows replacement conflicts from other processes are retried.
/// Temporary files start with mode 0600 on Unix.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "destination has no parent"))?;
    fs::create_dir_all(parent)?;

    let _guard = WRITE_MUTEX
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    replace_with_retry(path, bytes)
}

fn replace_once(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    AtomicFile::new(path, AllowOverwrite)
        .write_with_options(
            |file| {
                file.write_all(bytes)?;
                file.flush()
            },
            options,
        )
        .map_err(Into::into)
}

#[cfg(not(windows))]
fn replace_with_retry(path: &Path, bytes: &[u8]) -> io::Result<()> {
    replace_once(path, bytes)
}

#[cfg(windows)]
fn replace_with_retry(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::{thread, time::Duration};

    const MAX_ATTEMPTS: u32 = 16;
    let mut delay_ms = 1_u64;
    for attempt in 0..MAX_ATTEMPTS {
        match replace_once(path, bytes) {
            Ok(()) => return Ok(()),
            Err(error)
                if attempt + 1 < MAX_ATTEMPTS && is_transient_windows_replace_error(&error) =>
            {
                // Process-specific jitter prevents separate leetui processes
                // from repeatedly colliding in lockstep.
                let jitter = (u64::from(std::process::id()) + u64::from(attempt)) % 7;
                thread::sleep(Duration::from_millis(delay_ms + jitter));
                delay_ms = (delay_ms * 2).min(64);
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("the retry loop always returns on its final attempt")
}

#[cfg(windows)]
fn is_transient_windows_replace_error(error: &io::Error) -> bool {
    // ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION.
    matches!(error.raw_os_error(), Some(5 | 32 | 33))
}
