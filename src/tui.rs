use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    io::{self, Cursor, stdout},
};

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use ratatui_image::{Resize, StatefulImage, picker::Picker, protocol::StatefulProtocol};

use crate::{
    app::AppService,
    cache,
    config::Config,
    document::{image_urls, statement_markdown, terminal_lines},
    model::{JudgeResult, Problem, ProblemSummary},
    workspace::{self, WorkspaceEntry},
};

enum View {
    Browser,
    Problem,
}

struct App {
    all: Vec<ProblemSummary>,
    visible: Vec<usize>,
    selected: usize,
    query: String,
    searching: bool,
    view: View,
    problem: Option<Problem>,
    document: Paragraph<'static>,
    scroll: u16,
    status: String,
    result: Option<JudgeResult>,
    image_urls: Vec<String>,
    image: Option<StatefulProtocol>,
    confirm_submit: bool,
    pending_submission: Option<WorkspaceEntry>,
}

pub async fn run(config: &Config, service: &AppService<'_>) -> Result<()> {
    let all = match cache::load_index(service.paths()) {
        Ok(index) => index,
        Err(error) if error.is_not_found() => {
            eprintln!("No problem cache found; syncing from LeetCode…");
            service.sync_problems().await?
        }
        Err(error) => return Err(error.into()),
    };
    let visible = all
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.paid_only)
        .map(|(i, _)| i)
        .collect();
    let mut app = App {
        all,
        visible,
        selected: 0,
        query: String::new(),
        searching: false,
        view: View::Browser,
        problem: None,
        document: Paragraph::default(),
        scroll: 0,
        status: "Enter open  / search  q quit".into(),
        result: None,
        image_urls: Vec::new(),
        image: None,
        confirm_submit: false,
        pending_submission: None,
    };

    let image_picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
    let mut guard = TerminalGuard::enter()?;
    guard.terminal.draw(|frame| draw(frame, &mut app))?;
    loop {
        if !event::poll(std::time::Duration::from_millis(200))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        if app.confirm_submit {
            match key.code {
                KeyCode::Char('y' | 'Y') => {
                    app.confirm_submit = false;
                    app.status = "Submitting…".into();
                    guard.terminal.draw(|frame| draw(frame, &mut app))?;
                    if let Err(error) = submit_current(&mut app, config, service).await {
                        app.set_error(error);
                    }
                }
                KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                    app.confirm_submit = false;
                    app.pending_submission = None;
                    app.status = "Submission cancelled".into();
                }
                _ => app.status = "Submit? Press y to confirm or n/Esc to cancel".into(),
            }
            guard.terminal.draw(|frame| draw(frame, &mut app))?;
            continue;
        }

        if app.searching {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => app.searching = false,
                KeyCode::Backspace => {
                    app.query.pop();
                    filter(&mut app);
                }
                KeyCode::Char(c) => {
                    app.query.push(c);
                    filter(&mut app);
                }
                _ => {}
            }
            guard.terminal.draw(|frame| draw(frame, &mut app))?;
            continue;
        }

        match app.view {
            View::Browser => match key.code {
                KeyCode::Char('q') => break,
                KeyCode::Char('/') => app.searching = true,
                KeyCode::Down | KeyCode::Char('j') => {
                    app.selected = (app.selected + 1).min(app.visible.len().saturating_sub(1));
                }
                KeyCode::Up | KeyCode::Char('k') => app.selected = app.selected.saturating_sub(1),
                KeyCode::Enter => {
                    app.status = "Loading problem…".into();
                    guard.terminal.draw(|frame| draw(frame, &mut app))?;
                    if let Err(error) = open_selected(&mut app, service).await {
                        app.set_error(error);
                    }
                }
                _ => {}
            },
            View::Problem => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    if app.image.take().is_some() {
                        app.status = "Closed image".into();
                        guard.terminal.draw(|frame| draw(frame, &mut app))?;
                        continue;
                    }
                    app.view = View::Browser;
                    app.problem = None;
                    app.document = Paragraph::default();
                }
                KeyCode::Down | KeyCode::Char('j') => app.scroll = app.scroll.saturating_add(1),
                KeyCode::Up | KeyCode::Char('k') => app.scroll = app.scroll.saturating_sub(1),
                KeyCode::PageDown => app.scroll = app.scroll.saturating_add(15),
                KeyCode::PageUp => app.scroll = app.scroll.saturating_sub(15),
                KeyCode::Home => app.scroll = 0,
                KeyCode::Char('e') => {
                    let result = edit_current(&mut app, config, service, &mut guard).await;
                    if let Err(error) = result {
                        app.set_error(error);
                    }
                }
                KeyCode::Char('r') => {
                    app.status = "Running sample testcase…".into();
                    guard.terminal.draw(|frame| draw(frame, &mut app))?;
                    if let Err(error) = run_current(&mut app, config, service).await {
                        app.set_error(error);
                    }
                }
                KeyCode::Char('S') => match prepare_submission(&app, config, service).await {
                    Ok(entry) => {
                        app.status = submission_prompt(&entry);
                        app.pending_submission = Some(entry);
                        app.confirm_submit = true;
                    }
                    Err(error) => app.set_error(error),
                },
                KeyCode::Tab => {
                    app.status = "Loading editorial…".into();
                    guard.terminal.draw(|frame| draw(frame, &mut app))?;
                    if let Err(error) = load_editorial(&mut app, service).await {
                        app.set_error(error);
                    }
                }
                KeyCode::Char('m') => {
                    app.status = "Loading submissions…".into();
                    guard.terminal.draw(|frame| draw(frame, &mut app))?;
                    if let Err(error) = load_submissions(&mut app, service).await {
                        app.set_error(error);
                    }
                }
                KeyCode::Char('i') => {
                    app.status = "Loading image…".into();
                    guard.terminal.draw(|frame| draw(frame, &mut app))?;
                    if let Err(error) = load_image(&mut app, service, &image_picker).await {
                        app.set_error(error);
                    }
                }
                _ => {}
            },
        }
        sync_document_scroll(&mut app);
        guard.terminal.draw(|frame| draw(frame, &mut app))?;
    }
    Ok(())
}

impl App {
    fn set_error(&mut self, error: anyhow::Error) {
        self.status = format!("Error: {error:#}");
    }
}

async fn open_selected(app: &mut App, service: &AppService<'_>) -> Result<()> {
    let summary = app
        .visible
        .get(app.selected)
        .and_then(|index| app.all.get(*index))
        .context("no problem is selected")?
        .clone();
    let problem = service.problem(&summary.title_slug, false).await?;
    let markdown = statement_markdown(&problem.content)?;
    app.image_urls = image_urls(&markdown);
    set_document(app, terminal_lines(&markdown), "Statement");
    app.problem = Some(problem);
    app.result = None;
    app.image = None;
    app.scroll = 0;
    app.view = View::Problem;
    app.status = "j/k scroll  Tab editorial  i image  e edit  r run sample  S submit  m submissions  Esc back".into();
    Ok(())
}

async fn edit_current(
    app: &mut App,
    config: &Config,
    service: &AppService<'_>,
    guard: &mut TerminalGuard,
) -> Result<()> {
    let problem = app.problem.as_ref().context("no problem is open")?;
    let entry = service.workspace(config, &problem.title_slug, None).await?;
    guard.suspend()?;
    let editor_result = workspace::open_editor(config, &entry).await;
    let resume_result = guard.resume();
    resume_result?;
    editor_result?;
    app.status = format!("Edited {}", entry.solution.display());
    Ok(())
}

async fn run_current(app: &mut App, config: &Config, service: &AppService<'_>) -> Result<()> {
    let problem = app.problem.as_ref().context("no problem is open")?;
    let entry = service.workspace(config, &problem.title_slug, None).await?;
    app.result = Some(
        service
            .run_workspace(&entry, &problem.sample_test_case)
            .await?,
    );
    app.status = "Run finished; r rerun  S submit".into();
    Ok(())
}

async fn submit_current(app: &mut App, config: &Config, service: &AppService<'_>) -> Result<()> {
    let entry = match app.pending_submission.take() {
        Some(entry) => entry,
        None => prepare_submission(app, config, service).await?,
    };
    app.result = Some(service.submit_workspace(&entry).await?);
    app.status = "Submission finished".into();
    Ok(())
}

async fn prepare_submission(
    app: &App,
    config: &Config,
    service: &AppService<'_>,
) -> Result<WorkspaceEntry> {
    let problem = app.problem.as_ref().context("no problem is open")?;
    service.workspace(config, &problem.title_slug, None).await
}

async fn load_editorial(app: &mut App, service: &AppService<'_>) -> Result<()> {
    let problem = app.problem.as_ref().context("no problem is open")?;
    let editorial = service.editorial(&problem.title_slug, false).await?;
    app.image_urls = image_urls(&editorial.content);
    set_document(
        app,
        terminal_lines(&editorial.content),
        "Official editorial",
    );
    app.status = "Showing editorial; reopen problem to return to statement".into();
    Ok(())
}

async fn load_submissions(app: &mut App, service: &AppService<'_>) -> Result<()> {
    let problem = app.problem.as_ref().context("no problem is open")?;
    let submissions = service.submissions(&problem.title_slug).await?;
    let markdown = submissions
        .into_iter()
        .map(|submission| {
            format!(
                "* `{}` — **{}**, {}, {}, {}",
                submission.id,
                submission.status_display,
                submission.lang,
                submission.runtime.unwrap_or_else(|| "-".into()),
                submission.memory.unwrap_or_else(|| "-".into())
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    set_document(
        app,
        terminal_lines(&format!("# My submissions\n\n{markdown}")),
        "My submissions",
    );
    app.image_urls.clear();
    app.status = "Showing submissions; reopen problem to return".into();
    Ok(())
}

async fn load_image(app: &mut App, service: &AppService<'_>, picker: &Picker) -> Result<()> {
    let url = app
        .image_urls
        .first()
        .context("this document contains no images")?;
    let name = image_cache_name(url);
    let bytes = match cache::load_image(service.paths(), &name) {
        Ok(bytes) => bytes,
        Err(error) if error.is_not_found() => {
            let bytes = service.image(url).await?;
            cache::save_image(service.paths(), &name, &bytes)?;
            bytes
        }
        Err(error) => return Err(error.into()),
    };
    let decoded = decode_image(&bytes)?;
    app.image = Some(picker.new_resize_protocol(decoded));
    app.status = "Image preview; Esc closes it".into();
    Ok(())
}

fn decode_image(bytes: &[u8]) -> Result<image::DynamicImage> {
    decode_image_with_limits(bytes, 4096, 4096, 64 * 1024 * 1024)
}

fn decode_image_with_limits(
    bytes: &[u8],
    max_width: u32,
    max_height: u32,
    max_alloc: u64,
) -> Result<image::DynamicImage> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .context("unsupported image format")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(max_width);
    limits.max_image_height = Some(max_height);
    limits.max_alloc = Some(max_alloc);
    reader.limits(limits);
    reader.decode().context("invalid or oversized image")
}

fn filter(app: &mut App) {
    let query = app.query.to_lowercase();
    app.visible = app
        .all
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.paid_only)
        .filter(|(_, p)| {
            query.is_empty()
                || p.title.to_lowercase().contains(&query)
                || p.title_slug.contains(&query)
                || p.frontend_question_id == query
        })
        .map(|(i, _)| i)
        .collect();
    app.selected = app.selected.min(app.visible.len().saturating_sub(1));
}

fn set_document(app: &mut App, lines: Vec<Line<'static>>, title: &'static str) {
    app.scroll = 0;
    app.document = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title(title));
}

fn sync_document_scroll(app: &mut App) {
    app.document = std::mem::take(&mut app.document).scroll((app.scroll, 0));
}

fn draw(frame: &mut Frame, app: &mut App) {
    match app.view {
        View::Browser => draw_browser(frame, app),
        View::Problem => draw_problem(frame, app),
    }
}

fn draw_browser(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(frame.area());
    let search_title = if app.searching {
        " Search (typing) "
    } else {
        " Search (/) "
    };
    frame.render_widget(
        Paragraph::new(app.query.as_str())
            .block(Block::default().borders(Borders::ALL).title(search_title)),
        chunks[0],
    );

    let capacity = chunks[1].height.saturating_sub(2).max(1) as usize;
    let start = app
        .selected
        .saturating_sub(capacity / 2)
        .min(app.visible.len().saturating_sub(capacity));
    let end = (start + capacity).min(app.visible.len());
    let items = app.visible[start..end]
        .iter()
        .map(|index| {
            let p = &app.all[*index];
            let status = match p.status.as_deref() {
                Some("ac") => "✓",
                Some("notac") => "·",
                _ => " ",
            };
            let color = match p.difficulty.as_str() {
                "Easy" => Color::Green,
                "Medium" => Color::Yellow,
                "Hard" => Color::Red,
                _ => Color::White,
            };
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{status} {:>4}  ", p.frontend_question_id),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(&p.title),
                Span::styled(
                    format!("  {}  {:.1}%", p.difficulty, p.ac_rate),
                    Style::default().fg(color),
                ),
            ]))
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default().with_selected(Some(app.selected.saturating_sub(start)));
    frame.render_stateful_widget(
        List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Free problems ({}) ", app.visible.len())),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("› "),
        chunks[1],
        &mut state,
    );
    frame.render_widget(
        Paragraph::new(app.status.as_str()).style(status_bar_style()),
        chunks[2],
    );
}

fn draw_problem(frame: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(frame.area());
    if let Some(problem) = &app.problem {
        let color = match problem.difficulty.as_str() {
            "Easy" => Color::Green,
            "Medium" => Color::Yellow,
            "Hard" => Color::Red,
            _ => Color::White,
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!("{}. {}", problem.question_frontend_id, problem.title),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {}", problem.difficulty),
                    Style::default().fg(color),
                ),
            ]))
            .block(Block::default().borders(Borders::ALL)),
            chunks[0],
        );
    }
    let body_chunks = if app.result.is_some() {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
            .split(chunks[1])
    } else {
        Layout::default()
            .constraints([Constraint::Percentage(100)])
            .split(chunks[1])
    };
    if let Some(image) = &mut app.image {
        frame.render_widget(
            Block::default().borders(Borders::ALL).title(" Image "),
            body_chunks[0],
        );
        frame.render_stateful_widget(
            StatefulImage::default().resize(Resize::Fit(None)),
            body_chunks[0].inner(ratatui::layout::Margin::new(1, 1)),
            image,
        );
    } else {
        frame.render_widget(&app.document, body_chunks[0]);
    }
    if let Some(result) = &app.result {
        frame.render_widget(
            Paragraph::new(judge_lines(result))
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Judge result "),
                ),
            body_chunks[1],
        );
    }
    frame.render_widget(
        Paragraph::new(app.status.as_str()).style(status_bar_style()),
        chunks[2],
    );
}

fn status_bar_style() -> Style {
    Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
}

struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    raw_mode: bool,
    alternate_screen: bool,
}

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        if let Err(error) = execute!(stdout(), EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(error.into());
        }
        let terminal = match Terminal::new(CrosstermBackend::new(stdout())) {
            Ok(terminal) => terminal,
            Err(error) => {
                let _ = execute!(stdout(), LeaveAlternateScreen);
                let _ = disable_raw_mode();
                return Err(error).context("failed to initialize terminal");
            }
        };
        Ok(Self {
            terminal,
            raw_mode: true,
            alternate_screen: true,
        })
    }

    fn suspend(&mut self) -> Result<()> {
        if self.raw_mode {
            disable_raw_mode()?;
            self.raw_mode = false;
        }
        if self.alternate_screen {
            execute!(self.terminal.backend_mut(), LeaveAlternateScreen)?;
            self.alternate_screen = false;
        }
        self.terminal.show_cursor()?;
        Ok(())
    }

    fn resume(&mut self) -> Result<()> {
        if !self.raw_mode {
            enable_raw_mode()?;
            self.raw_mode = true;
        }
        if !self.alternate_screen {
            execute!(self.terminal.backend_mut(), EnterAlternateScreen)?;
            self.alternate_screen = true;
        }
        self.terminal.clear()?;
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.raw_mode {
            let _ = disable_raw_mode();
        }
        if self.alternate_screen {
            let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        }
        let _ = self.terminal.show_cursor();
    }
}

fn judge_lines(result: &JudgeResult) -> Vec<Line<'static>> {
    let mut lines = vec![Line::styled(
        result.status.clone(),
        Style::default().add_modifier(Modifier::BOLD),
    )];
    if let Some(runtime) = &result.runtime {
        lines.push(Line::raw(format!("Runtime: {runtime}")));
    }
    if let Some(memory) = &result.memory {
        lines.push(Line::raw(format!("Memory: {memory}")));
    }
    if let Some(output) = &result.output {
        lines.push(Line::raw(format!("Output: {output}")));
    }
    if let Some(expected) = &result.expected {
        lines.push(Line::raw(format!("Expected: {expected}")));
    }
    if let Some(error) = &result.error {
        lines.push(Line::styled(error.clone(), Style::default().fg(Color::Red)));
    }
    lines
}

fn image_cache_name(url: &str) -> String {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    format!("{:016x}.img", hasher.finish())
}

fn submission_prompt(entry: &WorkspaceEntry) -> String {
    format!(
        "Submit {}: {} ({})? y confirm, n/Esc cancel",
        entry.meta.slug,
        entry.solution.display(),
        entry.meta.language
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_decode_enforces_dimensions() {
        let image = image::DynamicImage::new_rgba8(2, 2);
        let mut encoded = Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        assert!(decode_image_with_limits(encoded.get_ref(), 1, 1, 1024).is_err());
        assert!(decode_image_with_limits(encoded.get_ref(), 2, 2, 1024).is_ok());
    }

    #[test]
    fn confirmation_identifies_file_and_language() {
        let entry = WorkspaceEntry {
            directory: "/work/two-sum".into(),
            solution: "/work/two-sum/solution.py".into(),
            meta: crate::workspace::WorkspaceMeta {
                slug: "two-sum".into(),
                question_id: "1".into(),
                frontend_id: "1".into(),
                language: "python3".into(),
                solution: "solution.py".into(),
            },
        };
        let prompt = submission_prompt(&entry);
        assert!(prompt.contains("solution.py"));
        assert!(prompt.contains("python3"));
    }
}
