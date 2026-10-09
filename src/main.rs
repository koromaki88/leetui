mod app;
mod cache;
mod client;
mod config;
mod document;
mod fsutil;
mod model;
mod tui;
mod workspace;

use std::{fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};

use crate::{
    app::{AppService, resolve_slug},
    client::LeetCodeClient,
    config::{Config, Credentials, Paths},
    document::statement_markdown,
    model::JudgeResult,
};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Save and validate browser session cookies.
    Login {
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        csrf: Option<String>,
    },
    /// Remove locally stored credentials.
    Logout,
    /// Refresh the local problem index.
    Sync,
    /// List cached problems.
    List(ListArgs),
    /// Display a problem statement as Markdown.
    Show {
        target: String,
        #[arg(long)]
        refresh: bool,
    },
    /// Create a local solution workspace.
    Pick(TargetArgs),
    /// Create/reopen a solution in the configured editor.
    Edit(TargetArgs),
    /// Run code with LeetCode's custom-test judge.
    Run(RunArgs),
    /// Submit code to LeetCode.
    Submit(SubmitArgs),
    /// Display the free official editorial.
    Editorial {
        target: String,
        #[arg(long)]
        refresh: bool,
    },
    /// List your submissions or show one submission's code.
    Submissions {
        /// Problem ID or slug. Required unless --show is used.
        target: Option<String>,
        #[arg(long)]
        show: Option<String>,
    },
    /// Print or update configuration.
    Config {
        /// One of: language, workspace, editor.
        key: Option<String>,
        /// New value. Editor commands are parsed like a shell command line without expansion.
        value: Option<String>,
    },
}

#[derive(Args)]
struct ListArgs {
    #[arg(short, long)]
    query: Option<String>,
    #[arg(short, long)]
    difficulty: Option<String>,
    #[arg(short, long)]
    tag: Option<String>,
    #[arg(long)]
    include_paid: bool,
    #[arg(long)]
    solved: bool,
    #[arg(long)]
    attempted: bool,
    #[arg(short, long)]
    limit: Option<usize>,
}

#[derive(Args)]
struct TargetArgs {
    target: String,
    #[arg(short, long)]
    lang: Option<String>,
}

#[derive(Args)]
struct RunArgs {
    target: String,
    #[arg(short, long)]
    lang: Option<String>,
    #[arg(short, long, conflicts_with = "input_file")]
    input: Option<String>,
    #[arg(long, conflicts_with = "input")]
    input_file: Option<PathBuf>,
    #[arg(long, conflicts_with_all = ["input", "input_file"])]
    examples: bool,
}

#[derive(Args)]
struct SubmitArgs {
    target: String,
    #[arg(short, long)]
    lang: Option<String>,
    /// Skip the interactive confirmation.
    #[arg(long)]
    yes: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = Paths::discover()?;
    match cli.command {
        Some(Command::Login { session, csrf }) => login(&paths, session, csrf).await,
        Some(Command::Logout) => {
            if paths.credentials.exists() {
                fs::remove_file(&paths.credentials)?;
            }
            println!("Logged out; stored credentials removed.");
            Ok(())
        }
        Some(Command::List(args)) => list(&paths, args),
        Some(Command::Config { key, value }) => {
            let config = Config::load(&paths)?;
            configure(&paths, config, key, value)
        }
        command => {
            let config = if matches!(
                command,
                None | Some(
                    Command::Pick(_) | Command::Edit(_) | Command::Run(_) | Command::Submit(_)
                )
            ) {
                Some(Config::load(&paths)?)
            } else {
                None
            };
            let credentials = if matches!(
                command,
                Some(Command::Run(_) | Command::Submit(_) | Command::Submissions { .. })
            ) {
                Credentials::load(&paths)?
            } else if matches!(command, None | Some(Command::Sync)) {
                match Credentials::load(&paths) {
                    Ok(credentials) => credentials,
                    Err(error) => {
                        eprintln!("Warning: ignoring invalid credentials: {error}");
                        None
                    }
                }
            } else {
                None
            };
            let client = LeetCodeClient::new(credentials)?;
            let service = AppService::new(&paths, &client);
            dispatch(command, config.as_ref(), &service).await
        }
    }
}

async fn dispatch(
    command: Option<Command>,
    config: Option<&Config>,
    service: &AppService<'_>,
) -> Result<()> {
    match command {
        None => tui::run(config.context("configuration was not loaded")?, service).await,
        Some(Command::Sync) => {
            let problems = service.sync_problems().await?;
            let free = problems.iter().filter(|p| !p.paid_only).count();
            println!("Cached {} problems ({free} free).", problems.len());
            Ok(())
        }
        Some(Command::Show { target, refresh }) => {
            let problem = service.problem(&target, refresh).await?;
            println!("# {}. {}\n", problem.question_frontend_id, problem.title);
            println!("**Difficulty:** {}  ", problem.difficulty);
            println!(
                "**Topics:** {}\n",
                problem
                    .topic_tags
                    .iter()
                    .map(|t| t.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            println!("{}", statement_markdown(&problem.content)?);
            Ok(())
        }
        Some(Command::Pick(args)) => {
            let entry = service
                .workspace(
                    config.context("configuration was not loaded")?,
                    &args.target,
                    args.lang.as_deref(),
                )
                .await?;
            println!("Created {}", entry.directory.display());
            println!("Solution: {}", entry.solution.display());
            Ok(())
        }
        Some(Command::Edit(args)) => {
            let config = config.context("configuration was not loaded")?;
            let entry = service
                .workspace(config, &args.target, args.lang.as_deref())
                .await?;
            workspace::open_editor(config, &entry).await
        }
        Some(Command::Run(args)) => {
            run_code(
                config.context("configuration was not loaded")?,
                service,
                args,
            )
            .await
        }
        Some(Command::Submit(args)) => {
            submit_code(
                config.context("configuration was not loaded")?,
                service,
                args,
            )
            .await
        }
        Some(Command::Editorial { target, refresh }) => {
            let editorial = service.editorial(&target, refresh).await?;
            println!("# {}\n\n{}", editorial.title, editorial.content);
            Ok(())
        }
        Some(Command::Submissions { target, show }) => {
            if let Some(id) = show {
                let details = service.submission_details(&id).await?;
                println!(
                    "Status: {}",
                    details.status_display.as_deref().unwrap_or("unknown")
                );
                if let Some(runtime) = details.runtime_display {
                    println!("Runtime: {runtime}");
                }
                if let Some(memory) = details.memory_display {
                    println!("Memory: {memory}");
                }
                println!("\n{}", details.code);
            } else {
                let target =
                    target.context("a problem target is required when listing submissions")?;
                let slug = resolve_slug(service.paths(), &target)?;
                for submission in service.submissions(&slug).await? {
                    println!(
                        "{:>12}  {:<18} {:<12} {:<12} {}",
                        submission.id,
                        submission.status_display,
                        submission.lang,
                        submission.runtime.as_deref().unwrap_or("-"),
                        submission.memory.as_deref().unwrap_or("-")
                    );
                }
            }
            Ok(())
        }
        Some(
            Command::Login { .. } | Command::Logout | Command::List(_) | Command::Config { .. },
        ) => unreachable!(),
    }
}

async fn login(paths: &Paths, session: Option<String>, csrf: Option<String>) -> Result<()> {
    let session = match session {
        Some(value) => value,
        None => rpassword::prompt_password("LEETCODE_SESSION: ")?,
    };
    let csrf = match csrf {
        Some(value) => value,
        None => rpassword::prompt_password("csrftoken: ")?,
    };
    if session.trim().is_empty() || csrf.trim().is_empty() {
        bail!("both cookie values are required");
    }
    let credentials = Credentials { session, csrf };
    let client = LeetCodeClient::new(Some(credentials.clone()))?;
    let username = client.validate_session().await?;
    credentials.save(paths)?;
    println!("Logged in as {username}.");
    Ok(())
}

fn list(paths: &Paths, args: ListArgs) -> Result<()> {
    let problems = match cache::load_index(paths) {
        Ok(problems) => problems,
        Err(error) if error.is_not_found() => bail!("run `leetui sync` first"),
        Err(error) => return Err(error.into()),
    };
    let query = args.query.unwrap_or_default().to_lowercase();
    let difficulty = args.difficulty.map(|v| v.to_lowercase());
    let tag = args.tag.map(|v| v.to_lowercase());
    let mut shown = 0;
    for problem in problems {
        if !args.include_paid && problem.paid_only {
            continue;
        }
        if !query.is_empty()
            && !problem.title.to_lowercase().contains(&query)
            && !problem.title_slug.contains(&query)
            && problem.frontend_question_id != query
        {
            continue;
        }
        if difficulty
            .as_ref()
            .is_some_and(|d| problem.difficulty.to_lowercase() != *d)
        {
            continue;
        }
        if tag.as_ref().is_some_and(|tag| {
            !problem
                .topic_tags
                .iter()
                .any(|t| t.slug == *tag || t.name.to_lowercase() == *tag)
        }) {
            continue;
        }
        if args.solved && problem.status.as_deref() != Some("ac") {
            continue;
        }
        if args.attempted && problem.status.as_deref() != Some("notac") {
            continue;
        }
        let status = match problem.status.as_deref() {
            Some("ac") => "✓",
            Some("notac") => "·",
            _ => " ",
        };
        let lock = if problem.paid_only { " 🔒" } else { "" };
        println!(
            "{status} {:>5}  {:<55} {:<6} {:>5.1}%{lock}",
            problem.frontend_question_id, problem.title, problem.difficulty, problem.ac_rate
        );
        shown += 1;
        if args.limit.is_some_and(|limit| shown >= limit) {
            break;
        }
    }
    Ok(())
}

async fn run_code(config: &Config, service: &AppService<'_>, args: RunArgs) -> Result<()> {
    let entry = service
        .workspace(config, &args.target, args.lang.as_deref())
        .await?;
    let problem = service.problem(&entry.meta.slug, false).await?;
    let input = if let Some(input) = args.input {
        input
    } else if let Some(path) = args.input_file {
        fs::read_to_string(path)?
    } else if args.examples {
        problem
            .example_testcases
            .clone()
            .unwrap_or_else(|| problem.sample_test_case.clone())
    } else {
        problem.sample_test_case.clone()
    };
    println!("Running {} with {}…", problem.title, entry.meta.language);
    let result = service.run_workspace(&entry, &input).await?;
    print_judge_result(&result);
    Ok(())
}

async fn submit_code(config: &Config, service: &AppService<'_>, args: SubmitArgs) -> Result<()> {
    let entry = service
        .workspace(config, &args.target, args.lang.as_deref())
        .await?;
    let problem = service.problem(&entry.meta.slug, false).await?;
    if !args.yes {
        eprint!("Submit {} ({})? [y/N] ", problem.title, entry.meta.language);
        use std::io::Write;
        std::io::stderr().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }
    println!("Submitting {}…", problem.title);
    let result = service.submit_workspace(&entry).await?;
    print_judge_result(&result);
    Ok(())
}

fn print_judge_result(result: &JudgeResult) {
    println!("\n{}", result.status);
    if let Some(runtime) = &result.runtime {
        println!("Runtime: {runtime}");
    }
    if let Some(memory) = &result.memory {
        println!("Memory: {memory}");
    }
    if let Some(input) = &result.input {
        println!("\nInput:\n{input}");
    }
    if let Some(output) = &result.output {
        println!("\nOutput:\n{output}");
    }
    if let Some(expected) = &result.expected {
        println!("\nExpected:\n{expected}");
    }
    if let Some(error) = &result.error {
        eprintln!("\n{error}");
    }
}

fn expand_home(value: &str) -> PathBuf {
    if value == "~" {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(value));
    }
    if let Some(rest) = value.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(value)
}

fn split_command(value: &str) -> Result<Vec<String>> {
    let args = shell_words::split(value).context("invalid editor command")?;
    if args.is_empty() {
        bail!("editor command cannot be empty");
    }
    Ok(args)
}

fn configure(
    paths: &Paths,
    mut config: Config,
    key: Option<String>,
    value: Option<String>,
) -> Result<()> {
    if let Some(key) = key {
        let value = value.context("a value is required when setting a configuration key")?;
        match key.as_str() {
            "language" => config.default_language = value,
            "workspace" => config.workspace = expand_home(&value),
            "editor" => config.editor = split_command(&value)?,
            _ => bail!("unknown configuration key `{key}`; use language, workspace, or editor"),
        }
        config.save(paths)?;
        println!("Updated {}.", paths.config.display());
        return Ok(());
    }
    println!("{}", toml::to_string_pretty(&config)?);
    println!("config_path = {:?}", paths.config);
    println!("cache_path = {:?}", paths.cache);
    println!("credentials_path = {:?}", paths.credentials);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_editor_commands() {
        assert_eq!(
            split_command("code --wait '{dir}'").unwrap(),
            ["code", "--wait", "{dir}"]
        );
        assert!(split_command("code '").is_err());
    }

    #[test]
    fn run_input_modes_are_mutually_exclusive() {
        assert!(
            Cli::try_parse_from(["leetui", "run", "two-sum", "--examples", "--input", "[]"])
                .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "leetui",
                "run",
                "two-sum",
                "--examples",
                "--input-file",
                "cases.txt"
            ])
            .is_err()
        );
    }

    #[test]
    fn submission_details_do_not_require_problem_target() {
        let cli = Cli::try_parse_from(["leetui", "submissions", "--show", "123"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Submissions {
                target: None,
                show: Some(id)
            }) if id == "123"
        ));
    }
}
