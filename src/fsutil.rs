use std::{
    fs,
    io::{self, Write},
    path::Path,
};

use tempfile::Builder;

/// Replaces a file atomically using a uniquely named temporary file in the
/// destination directory. `NamedTempFile` is created as mode 0600 on Unix,
/// which is required for credentials and is suitably restrictive elsewhere.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "destination has no parent"))?;
    fs::create_dir_all(parent)?;

    let mut temporary = Builder::new().prefix(".leetui-").tempfile_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;

    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;

    Ok(())
}
