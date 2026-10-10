use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use directories::{BaseDirs, ProjectDirs};
use serde::{Deserialize, Serialize};

use crate::{fsutil::atomic_write, language};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub default_language: String,
    pub editor: Vec<String>,
    pub workspace: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        let home = BaseDirs::new()
            .map(|directories| directories.home_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            default_language: "rust".into(),
            editor: Vec::new(),
            workspace: home.join("leetcode"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub session: String,
    pub csrf: String,
}

#[derive(Debug, Clone)]
pub struct Paths {
    pub config: PathBuf,
    pub credentials: PathBuf,
    pub cache: PathBuf,
}

impl Paths {
    pub fn discover() -> Result<Self> {
        let dirs = ProjectDirs::from("dev", "leetui", "leetui")
            .context("could not determine the user configuration directory")?;
        Ok(Self {
            config: dirs.config_dir().join("config.toml"),
            credentials: dirs
                .state_dir()
                .unwrap_or(dirs.data_local_dir())
                .join("credentials.toml"),
            cache: dirs.cache_dir().to_path_buf(),
        })
    }

    pub fn ensure(&self) -> Result<()> {
        for path in [&self.config, &self.credentials] {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
        }
        fs::create_dir_all(self.cache.join("problems"))?;
        fs::create_dir_all(self.cache.join("editorials"))?;
        fs::create_dir_all(self.cache.join("images"))?;
        Ok(())
    }
}

impl Config {
    pub fn load(paths: &Paths) -> Result<Self> {
        if !paths.config.exists() {
            return Self::default().normalized();
        }
        let config = parse_config(&fs::read_to_string(&paths.config)?)
            .with_context(|| format!("invalid configuration in {}", paths.config.display()))?;
        config
            .normalized()
            .with_context(|| format!("invalid configuration in {}", paths.config.display()))
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        let config = self.clone().normalized()?;
        paths.ensure()?;
        atomic_write(&paths.config, toml::to_string_pretty(&config)?.as_bytes())?;
        Ok(())
    }

    fn normalized(mut self) -> Result<Self> {
        self.default_language = language::normalize(&self.default_language)?;
        self.workspace = expand_home(self.workspace);
        if self.workspace.as_os_str().is_empty() {
            bail!("workspace path cannot be empty");
        }
        validate_editor(&self.editor)?;
        Ok(self)
    }

    pub fn editor_command(&self) -> Result<Vec<String>> {
        if !self.editor.is_empty() {
            validate_editor(&self.editor)?;
            return Ok(self.editor.clone());
        }
        for key in ["VISUAL", "EDITOR"] {
            if let Ok(value) = std::env::var(key) {
                let command = shell_words::split(&value)
                    .with_context(|| format!("invalid command in ${key}"))?;
                if !command.is_empty() {
                    validate_editor(&command)?;
                    return Ok(command);
                }
            }
        }
        bail!("no editor configured; set $VISUAL, $EDITOR, or editor in config.toml")
    }
}

fn parse_config(contents: &str) -> Result<Config> {
    let mut table: toml::Table = toml::from_str(contents)?;
    // `page_size` existed in early versions but never affected behavior. Drop
    // it during loading so old generated configuration remains usable while
    // all other unknown keys are still reported as mistakes.
    table.remove("page_size");
    Ok(toml::Value::Table(table).try_into()?)
}

fn validate_editor(command: &[String]) -> Result<()> {
    match command.first() {
        None => Ok(()),
        Some(executable) if !executable.is_empty() => Ok(()),
        _ => bail!("the first editor array element must be a non-empty executable name"),
    }
}

pub fn expand_home(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let Ok(rest) = path.strip_prefix("~") else {
        return path.to_path_buf();
    };
    let Some(home) = BaseDirs::new().map(|directories| directories.home_dir().to_path_buf()) else {
        return path.to_path_buf();
    };
    if rest.as_os_str().is_empty() {
        home
    } else {
        home.join(rest)
    }
}

impl Credentials {
    pub fn load(paths: &Paths) -> Result<Option<Self>> {
        let session = std::env::var("LEETCODE_SESSION").ok();
        let csrf = std::env::var("LEETCODE_CSRF_TOKEN")
            .or_else(|_| std::env::var("LEETCODE_CSRF"))
            .ok();
        if let Some(credentials) = credentials_from_environment(session, csrf)? {
            return Ok(Some(credentials));
        }
        if !paths.credentials.exists() {
            return Ok(None);
        }
        Ok(Some(toml::from_str(&fs::read_to_string(
            &paths.credentials,
        )?)?))
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        paths.ensure()?;
        atomic_write(&paths.credentials, toml::to_string(self)?.as_bytes())?;
        Ok(())
    }
}

fn credentials_from_environment(
    session: Option<String>,
    csrf: Option<String>,
) -> Result<Option<Credentials>> {
    match (session, csrf) {
        (Some(session), Some(csrf)) => Ok(Some(Credentials { session, csrf })),
        (None, None) => Ok(None),
        _ => bail!(
            "LEETCODE_SESSION and LEETCODE_CSRF_TOKEN (or LEETCODE_CSRF) must be set together"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn test_paths(root: &std::path::Path) -> Paths {
        Paths {
            config: root.join("config/config.toml"),
            credentials: root.join("state/credentials.toml"),
            cache: root.join("cache"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn credentials_are_created_private() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        Credentials {
            session: "secret".into(),
            csrf: "csrf".into(),
        }
        .save(&paths)
        .unwrap();

        fs::set_permissions(&paths.credentials, fs::Permissions::from_mode(0o644)).unwrap();
        Credentials {
            session: "new-secret".into(),
            csrf: "new-csrf".into(),
        }
        .save(&paths)
        .unwrap();

        let mode = fs::metadata(paths.credentials)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn environment_credentials_require_a_complete_pair() {
        assert!(credentials_from_environment(Some("session".into()), None).is_err());
        assert!(credentials_from_environment(None, Some("csrf".into())).is_err());
        let credentials = credentials_from_environment(Some("session".into()), Some("csrf".into()))
            .unwrap()
            .unwrap();
        assert_eq!(credentials.session, "session");
        assert_eq!(credentials.csrf, "csrf");
    }

    #[test]
    fn config_normalizes_language_and_validates_editor() {
        let config: Config = toml::from_str(
            r#"
default_language = "C++"
editor = ["nvim", "{file}"]
workspace = "workspace"
"#,
        )
        .unwrap();
        let config = config.normalized().unwrap();
        assert_eq!(config.default_language, "cpp");
        assert_eq!(config.editor, ["nvim", "{file}"]);

        let invalid = Config {
            editor: vec![String::new()],
            ..Config::default()
        };
        assert!(invalid.normalized().is_err());
    }

    #[test]
    fn config_rejects_unknown_fields() {
        assert!(parse_config("editors = ['nvim']").is_err());
    }

    #[test]
    fn config_migrates_legacy_page_size() {
        let config = parse_config("page_size = 50").unwrap();
        assert_eq!(config.default_language, "rust");
    }
}
