use std::{fs, path::Path};

use anyhow::{Context, Result, bail};

use crate::{
    cache,
    client::LeetCodeClient,
    config::{Config, Paths},
    language,
    model::{Editorial, JudgeResult, Problem, SubmissionDetails, SubmissionSummary},
    workspace::{self, WorkspaceEntry},
};

/// Shared application operations used by both the CLI and TUI. Presentation
/// layers should not reimplement cache fallback or workspace selection rules.
pub struct AppService<'a> {
    paths: &'a Paths,
    client: &'a LeetCodeClient,
}

impl<'a> AppService<'a> {
    pub fn new(paths: &'a Paths, client: &'a LeetCodeClient) -> Self {
        Self { paths, client }
    }

    pub fn paths(&self) -> &Paths {
        self.paths
    }

    pub async fn sync_problems(&self) -> Result<Vec<crate::model::ProblemSummary>> {
        let problems = self.client.problem_list().await?;
        cache::save_index(self.paths, &problems)?;
        Ok(problems)
    }

    pub async fn problem(&self, target: &str, refresh: bool) -> Result<Problem> {
        let slug = resolve_slug(self.paths, target)?;
        if !refresh {
            match cache::load_problem(self.paths, &slug) {
                Ok(problem) => return Ok(problem),
                Err(error) if error.is_not_found() => {}
                Err(error) => return Err(error.into()),
            }
        }
        let problem = self.client.problem(&slug).await?;
        if problem.is_paid_only {
            bail!("`{slug}` is a premium problem");
        }
        cache::save_problem(self.paths, &problem)?;
        Ok(problem)
    }

    pub async fn editorial(&self, target: &str, refresh: bool) -> Result<Editorial> {
        let slug = resolve_slug(self.paths, target)?;
        if !refresh {
            match cache::load_editorial(self.paths, &slug) {
                Ok(editorial) => return Ok(editorial),
                Err(error) if error.is_not_found() => {}
                Err(error) => return Err(error.into()),
            }
        }
        let editorial = self.client.editorial(&slug).await?;
        if editorial.paid_only {
            bail!("this editorial requires LeetCode Premium");
        }
        cache::save_editorial(self.paths, &slug, &editorial)?;
        Ok(editorial)
    }

    pub async fn workspace(
        &self,
        config: &Config,
        target: &str,
        language: Option<&str>,
    ) -> Result<WorkspaceEntry> {
        let language = language.map(language::normalize).transpose()?;
        let path = Path::new(target);
        if path.exists() {
            let existing = workspace::from_path(path)?;
            if language.is_none() || language.as_deref() == Some(existing.meta.language.as_str()) {
                return Ok(existing);
            }
            let problem = self.problem(&existing.meta.slug, false).await?;
            return workspace::prepare_in(
                &existing.directory,
                &problem,
                language.as_deref(),
                &config.default_language,
            );
        }
        let problem = self.problem(target, false).await?;
        workspace::prepare(config, &problem, language.as_deref())
    }

    pub async fn run_workspace(&self, entry: &WorkspaceEntry, input: &str) -> Result<JudgeResult> {
        let problem = self.problem(&entry.meta.slug, false).await?;
        let code = fs::read_to_string(&entry.solution)
            .with_context(|| format!("failed to read {}", entry.solution.display()))?;
        Ok(self
            .client
            .run(&problem, &entry.meta.language, &code, input)
            .await?)
    }

    pub async fn submit_workspace(&self, entry: &WorkspaceEntry) -> Result<JudgeResult> {
        let problem = self.problem(&entry.meta.slug, false).await?;
        let code = fs::read_to_string(&entry.solution)
            .with_context(|| format!("failed to read {}", entry.solution.display()))?;
        Ok(self
            .client
            .submit(&problem, &entry.meta.language, &code)
            .await?)
    }

    pub async fn submissions(&self, slug: &str) -> Result<Vec<SubmissionSummary>> {
        cache::validate_slug(slug)?;
        Ok(self.client.submissions(slug).await?)
    }

    pub async fn submission_details(&self, id: &str) -> Result<SubmissionDetails> {
        Ok(self.client.submission_details(id).await?)
    }

    pub async fn image(&self, url: &str) -> Result<Vec<u8>> {
        Ok(self.client.download_image(url).await?)
    }
}

pub fn resolve_slug(paths: &Paths, target: &str) -> Result<String> {
    let slug = if let Some(rest) = target.split("/problems/").nth(1) {
        rest.split('/').next().unwrap_or(rest).to_owned()
    } else if target.chars().all(|character| character.is_ascii_digit()) {
        let index = match cache::load_index(paths) {
            Ok(index) => index,
            Err(error) if error.is_not_found() => {
                bail!("numeric IDs require `leetui sync` first")
            }
            Err(error) => return Err(error.into()),
        };
        index
            .into_iter()
            .find(|problem| problem.frontend_question_id == target)
            .map(|problem| problem.title_slug)
            .with_context(|| format!("problem ID {target} was not found"))?
    } else {
        target.to_owned()
    };
    cache::validate_slug(&slug)?;
    Ok(slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_path_like_targets() {
        let directory = tempfile::tempdir().unwrap();
        let paths = Paths {
            config: directory.path().join("config"),
            credentials: directory.path().join("credentials"),
            cache: directory.path().join("cache"),
        };
        assert!(resolve_slug(&paths, "../two-sum").is_err());
        assert!(resolve_slug(&paths, "two/sum").is_err());
        assert_eq!(resolve_slug(&paths, "two-sum").unwrap(), "two-sum");
    }
}
