use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

use crate::{
    config::Paths,
    fsutil::atomic_write,
    model::{Editorial, Problem, ProblemSummary},
};

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("cache entry not found: {0}")]
    NotFound(PathBuf),
    #[error("invalid cache key `{0}`")]
    InvalidKey(String),
    #[error("invalid cache entry {}: {source}", path.display())]
    Corrupt {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("cache I/O failed for {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to serialize cache entry: {0}")]
    Serialize(#[from] serde_json::Error),
}

impl CacheError {
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound(_))
    }
}

pub type CacheResult<T> = Result<T, CacheError>;

pub fn validate_slug(slug: &str) -> CacheResult<()> {
    let valid = !slug.is_empty()
        && slug.len() <= 200
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !slug.starts_with('-')
        && !slug.ends_with('-');
    if valid {
        Ok(())
    } else {
        Err(CacheError::InvalidKey(slug.into()))
    }
}

pub fn save_index(paths: &Paths, problems: &[ProblemSummary]) -> CacheResult<()> {
    save_json(&paths.cache.join("problems.json"), problems)
}

pub fn load_index(paths: &Paths) -> CacheResult<Vec<ProblemSummary>> {
    load_json(&paths.cache.join("problems.json"))
}

pub fn save_problem(paths: &Paths, problem: &Problem) -> CacheResult<()> {
    validate_slug(&problem.title_slug)?;
    save_json(&problem_path(paths, &problem.title_slug)?, problem)
}

pub fn load_problem(paths: &Paths, slug: &str) -> CacheResult<Problem> {
    load_json(&problem_path(paths, slug)?)
}

pub fn save_editorial(paths: &Paths, slug: &str, editorial: &Editorial) -> CacheResult<()> {
    save_json(&editorial_path(paths, slug)?, editorial)
}

pub fn load_editorial(paths: &Paths, slug: &str) -> CacheResult<Editorial> {
    load_json(&editorial_path(paths, slug)?)
}

pub fn save_image(paths: &Paths, name: &str, bytes: &[u8]) -> CacheResult<()> {
    validate_filename(name)?;
    let path = paths.cache.join("images").join(name);
    atomic_write(&path, bytes).map_err(|source| CacheError::Io { path, source })
}

pub fn load_image(paths: &Paths, name: &str) -> CacheResult<Vec<u8>> {
    validate_filename(name)?;
    read_bytes(&paths.cache.join("images").join(name))
}

fn problem_path(paths: &Paths, slug: &str) -> CacheResult<PathBuf> {
    validate_slug(slug)?;
    Ok(paths.cache.join("problems").join(format!("{slug}.json")))
}

fn editorial_path(paths: &Paths, slug: &str) -> CacheResult<PathBuf> {
    validate_slug(slug)?;
    Ok(paths.cache.join("editorials").join(format!("{slug}.json")))
}

fn validate_filename(name: &str) -> CacheResult<()> {
    if !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'.' || byte == b'-')
        && !name.contains("..")
    {
        Ok(())
    } else {
        Err(CacheError::InvalidKey(name.into()))
    }
}

fn save_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> CacheResult<()> {
    let bytes = serde_json::to_vec(value)?;
    atomic_write(path, &bytes).map_err(|source| CacheError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn load_json<T: DeserializeOwned>(path: &Path) -> CacheResult<T> {
    let bytes = read_bytes(path)?;
    serde_json::from_slice(&bytes).map_err(|source| CacheError::Corrupt {
        path: path.to_path_buf(),
        source,
    })
}

fn read_bytes(path: &Path) -> CacheResult<Vec<u8>> {
    fs::read(path).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            CacheError::NotFound(path.to_path_buf())
        } else {
            CacheError::Io {
                path: path.to_path_buf(),
                source,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(root: &Path) -> Paths {
        Paths {
            config: root.join("config"),
            credentials: root.join("credentials"),
            cache: root.join("cache"),
        }
    }

    #[test]
    fn rejects_unsafe_slugs() {
        for slug in [
            "../escape",
            "/absolute",
            "two/sum",
            "Two-Sum",
            "-bad",
            "bad-",
        ] {
            assert!(validate_slug(slug).is_err(), "accepted {slug}");
        }
        assert!(validate_slug("two-sum-2").is_ok());
    }

    #[test]
    fn distinguishes_missing_and_corrupt_entries() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        assert!(load_problem(&paths, "two-sum").unwrap_err().is_not_found());

        let path = problem_path(&paths, "two-sum").unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"not json").unwrap();
        assert!(matches!(
            load_problem(&paths, "two-sum"),
            Err(CacheError::Corrupt { .. })
        ));
    }

    #[test]
    fn concurrent_atomic_writes_remain_valid() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("value.json");
        let handles = (0..8)
            .map(|value| {
                let target = target.clone();
                std::thread::spawn(move || save_json(&target, &value).unwrap())
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }
        let value: u32 = load_json(&target).unwrap();
        assert!(value < 8);
    }
}
