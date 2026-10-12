use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::{
    fsutil::atomic_write_new,
    language,
    model::{SubmissionDetails, SubmissionSummary},
    workspace::{self, WorkspaceEntry},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmissionSave {
    Saved { path: PathBuf },
    Unchanged { path: PathBuf },
    Conflict { path: PathBuf },
}

pub fn save(
    entry: &WorkspaceEntry,
    summary: &SubmissionSummary,
    details: &SubmissionDetails,
) -> Result<SubmissionSave> {
    validate_id(&summary.id)?;
    if let Some(details_id) = &details.id
        && details_id != &summary.id
    {
        bail!(
            "submission response ID `{details_id}` does not match requested ID `{}`",
            summary.id
        );
    }

    let language = language::normalize(&summary.lang)
        .with_context(|| format!("invalid language for submission {}", summary.id))?;
    let directory = entry.directory.join("submissions");
    let path = directory.join(format!(
        "{}.{}",
        summary.id,
        workspace::extension(&language)
    ));
    let bytes = details.code.as_bytes();

    if let Some(outcome) = classify_existing(&path, bytes)? {
        return Ok(outcome);
    }

    match atomic_write_new(&path, bytes) {
        Ok(()) => Ok(SubmissionSave::Saved { path }),
        Err(error) if is_already_exists(&error) => classify_existing(&path, bytes)?
            .context("submission file appeared but could not be read"),
        Err(error) => {
            Err(error).with_context(|| format!("failed to save submission to {}", path.display()))
        }
    }
}

fn classify_existing(path: &Path, bytes: &[u8]) -> Result<Option<SubmissionSave>> {
    match fs::read(path) {
        Ok(existing) if existing == bytes => Ok(Some(SubmissionSave::Unchanged {
            path: path.to_path_buf(),
        })),
        Ok(_) => Ok(Some(SubmissionSave::Conflict {
            path: path.to_path_buf(),
        })),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => {
            Err(error).with_context(|| format!("failed to read submission file {}", path.display()))
        }
    }
}

fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("invalid submission ID `{id}`")
    }
    Ok(())
}

fn is_already_exists(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        return true;
    }
    #[cfg(windows)]
    return matches!(error.raw_os_error(), Some(80 | 183));
    #[cfg(not(windows))]
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::WorkspaceMeta;

    fn summary(id: &str) -> SubmissionSummary {
        SubmissionSummary {
            id: id.into(),
            status_display: "Accepted".into(),
            lang: "C++".into(),
            runtime: Some("0 ms".into()),
            timestamp: "1700000000".into(),
            url: None,
            memory: Some("2 MB".into()),
        }
    }

    fn details(id: &str, code: &str) -> SubmissionDetails {
        SubmissionDetails {
            id: Some(id.into()),
            code: code.into(),
            status_display: Some("Accepted".into()),
            runtime_display: None,
            memory_display: None,
            runtime_error: None,
            compile_error: None,
        }
    }

    #[test]
    fn saves_once_and_never_overwrites_local_changes() {
        let directory = tempfile::tempdir().unwrap();
        let entry = WorkspaceEntry {
            directory: directory.path().into(),
            solution: directory.path().join("solution.rs"),
            meta: WorkspaceMeta {
                slug: "two-sum".into(),
                question_id: "1".into(),
                frontend_id: "1".into(),
                language: "rust".into(),
                solution: "solution.rs".into(),
            },
        };
        let summary = summary("123");
        let details = details("123", "int main() {}\n");

        let SubmissionSave::Saved { path } = save(&entry, &summary, &details).unwrap() else {
            panic!("expected a newly saved submission");
        };
        assert_eq!(path.file_name().unwrap(), "123.cpp");
        assert!(matches!(
            save(&entry, &summary, &details).unwrap(),
            SubmissionSave::Unchanged { .. }
        ));

        fs::write(&path, "local changes").unwrap();
        assert!(matches!(
            save(&entry, &summary, &details).unwrap(),
            SubmissionSave::Conflict { .. }
        ));
        assert_eq!(fs::read_to_string(path).unwrap(), "local changes");
    }

    #[test]
    fn rejects_unsafe_or_mismatched_ids() {
        let directory = tempfile::tempdir().unwrap();
        let entry = WorkspaceEntry {
            directory: directory.path().into(),
            solution: directory.path().join("solution.rs"),
            meta: WorkspaceMeta {
                slug: "two-sum".into(),
                question_id: "1".into(),
                frontend_id: "1".into(),
                language: "rust".into(),
                solution: "solution.rs".into(),
            },
        };
        assert!(save(&entry, &summary("../123"), &details("../123", "code")).is_err());
        assert!(save(&entry, &summary("123"), &details("456", "code")).is_err());
    }
}
