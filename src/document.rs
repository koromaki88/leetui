use anyhow::Result;
use htmd::{Element, HtmlToMarkdown, element_handler::Handlers};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

pub fn statement_markdown(html: &str) -> Result<String> {
    let converter = HtmlToMarkdown::builder()
        .add_handler(
            vec!["sup"],
            |handlers: &dyn Handlers, element: Element<'_>| {
                let content = handlers.walk_children(element.node).content;
                Some(format!("^{{{}}}", content.trim()).into())
            },
        )
        .add_handler(
            vec!["sub"],
            |handlers: &dyn Handlers, element: Element<'_>| {
                let content = handlers.walk_children(element.node).content;
                Some(format!("_{{{}}}", content.trim()).into())
            },
        )
        .build();
    Ok(converter.convert(html)?)
}

pub fn html_fragment_markdown(html: &str) -> String {
    htmd::convert(html).unwrap_or_else(|_| html.to_owned())
}

pub fn terminal_lines(markdown: &str) -> Vec<Line<'static>> {
    let parser = Parser::new_ext(markdown, Options::all());
    let mut lines = Vec::new();
    let mut current = Vec::<Span<'static>>::new();
    let mut style = Style::default();
    let mut style_stack = Vec::new();
    let mut list_stack = Vec::<Option<u64>>::new();
    let mut link_stack = Vec::<String>::new();
    let mut code_block = false;
    let mut in_image = false;

    let flush = |lines: &mut Vec<Line<'static>>, current: &mut Vec<Span<'static>>| {
        if !current.is_empty() {
            lines.push(Line::from(std::mem::take(current)));
        }
    };

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    flush(&mut lines, &mut current);
                    style_stack.push(style);
                    style = heading_style(level);
                }
                Tag::Paragraph => {}
                Tag::Emphasis => {
                    style_stack.push(style);
                    style = style.add_modifier(Modifier::ITALIC);
                }
                Tag::Strong => {
                    style_stack.push(style);
                    style = style.add_modifier(Modifier::BOLD);
                }
                Tag::BlockQuote(_) => {
                    current.push(Span::styled("│ ", Style::default().fg(Color::DarkGray)))
                }
                Tag::CodeBlock(kind) => {
                    flush(&mut lines, &mut current);
                    code_block = true;
                    if let CodeBlockKind::Fenced(language) = kind
                        && !language.is_empty()
                    {
                        lines.push(Line::styled(
                            format!("[{language}]"),
                            Style::default().fg(Color::DarkGray),
                        ));
                    }
                }
                Tag::List(start) => list_stack.push(start),
                Tag::Item => {
                    flush(&mut lines, &mut current);
                    let prefix = match list_stack.last_mut() {
                        Some(Some(number)) => {
                            let prefix = format!("{number}. ");
                            *number += 1;
                            prefix
                        }
                        _ => "• ".into(),
                    };
                    current.push(Span::styled(prefix, Style::default().fg(Color::Cyan)));
                }
                Tag::Link { dest_url, .. } => {
                    style_stack.push(style);
                    style = style.fg(Color::Cyan).add_modifier(Modifier::UNDERLINED);
                    link_stack.push(dest_url.into_string());
                }
                Tag::Image {
                    dest_url, title, ..
                } => {
                    flush(&mut lines, &mut current);
                    let label = if title.is_empty() {
                        "image"
                    } else {
                        title.as_ref()
                    };
                    lines.push(Line::styled(
                        format!("🖼 {label}: {dest_url}"),
                        Style::default().fg(Color::Cyan),
                    ));
                    in_image = true;
                }
                Tag::Table(_) | Tag::TableHead | Tag::TableRow => flush(&mut lines, &mut current),
                Tag::TableCell => current.push(Span::raw("| ")),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Heading(_) => {
                    flush(&mut lines, &mut current);
                    lines.push(Line::default());
                    style = style_stack.pop().unwrap_or_default();
                }
                TagEnd::Paragraph => {
                    flush(&mut lines, &mut current);
                    lines.push(Line::default());
                }
                TagEnd::Emphasis | TagEnd::Strong => style = style_stack.pop().unwrap_or_default(),
                TagEnd::CodeBlock => {
                    flush(&mut lines, &mut current);
                    lines.push(Line::default());
                    code_block = false;
                }
                TagEnd::List(_) => {
                    list_stack.pop();
                    flush(&mut lines, &mut current);
                }
                TagEnd::Item => flush(&mut lines, &mut current),
                TagEnd::Link => {
                    style = style_stack.pop().unwrap_or_default();
                    if let Some(destination) = link_stack.pop() {
                        current.push(Span::styled(
                            format!(" <{destination}>"),
                            Style::default().fg(Color::DarkGray),
                        ));
                    }
                }
                TagEnd::Image => in_image = false,
                TagEnd::TableRow => {
                    current.push(Span::raw("|"));
                    flush(&mut lines, &mut current);
                }
                TagEnd::Table => {
                    flush(&mut lines, &mut current);
                    lines.push(Line::default());
                }
                _ => {}
            },
            Event::Text(text) => {
                if in_image {
                    continue;
                }
                let text_style = if code_block {
                    Style::default().fg(Color::Yellow)
                } else {
                    style
                };
                if code_block {
                    for (index, part) in text.split('\n').enumerate() {
                        if index > 0 {
                            flush(&mut lines, &mut current);
                        }
                        if !part.is_empty() {
                            current.push(Span::styled(format!("  {part}"), text_style));
                        }
                    }
                } else {
                    current.push(Span::styled(text.into_string(), text_style));
                }
            }
            Event::Code(code) => current.push(Span::styled(
                format!("`{code}`"),
                Style::default().fg(Color::Yellow),
            )),
            Event::SoftBreak => current.push(Span::raw(" ")),
            Event::HardBreak => flush(&mut lines, &mut current),
            Event::TaskListMarker(checked) => {
                current.push(Span::raw(if checked { "[x] " } else { "[ ] " }))
            }
            Event::Rule => {
                flush(&mut lines, &mut current);
                lines.push(Line::styled(
                    "─".repeat(40),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                let converted = html_fragment_markdown(&html);
                current.push(Span::raw(converted));
            }
            _ => {}
        }
    }
    flush(&mut lines, &mut current);
    lines
}

pub fn image_urls(markdown: &str) -> Vec<String> {
    Parser::new_ext(markdown, Options::all())
        .filter_map(|event| match event {
            Event::Start(Tag::Image { dest_url, .. }) => Some(dest_url.into_string()),
            _ => None,
        })
        .collect()
}

fn heading_style(level: HeadingLevel) -> Style {
    let color = match level {
        HeadingLevel::H1 => Color::Magenta,
        HeadingLevel::H2 => Color::Blue,
        _ => Color::Cyan,
    };
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_superscripts_and_images() {
        let markdown = statement_markdown(
            r#"<p>10<sup>9</sup></p><img src="https://assets.leetcode.com/example.png" alt="diagram">"#,
        )
        .unwrap();
        assert!(markdown.contains("10^{9}"));
        assert_eq!(
            image_urls(&markdown),
            ["https://assets.leetcode.com/example.png"]
        );
    }

    #[test]
    fn terminal_renderer_keeps_content() {
        let lines =
            terminal_lines("# Heading\n\n* one\n* two\n\n`code` and [docs](https://leetcode.com)");
        let rendered = lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("Heading"));
        assert!(rendered.contains("one"));
        assert!(rendered.contains("`code`"));
        assert!(rendered.contains("https://leetcode.com"));
    }
}
