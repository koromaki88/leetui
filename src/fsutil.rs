use std::{
    fs,
    io::{self, Write},
    path::Path,
};

use atomicwrites::{AllowOverwrite, AtomicFile};

/// Replaces a file atomically using a uniquely named temporary file in the
/// destination directory. On Windows, `atomicwrites` uses `MoveFileExW` with
/// replacement and write-through flags, avoiding the concurrent replacement
/// race in `std::fs::rename`. Temporary files start with mode 0600 on Unix.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "destination has no parent"))?;
    fs::create_dir_all(parent)?;

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
