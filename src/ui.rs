use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block as WidgetBlock, Borders, Paragraph, Wrap},
    Frame,
};
use syntect::{
    easy::HighlightLines,
    highlighting::{Theme, ThemeSet},
    parsing::SyntaxSet,
};

use crate::{
    mdn::{Block, Page},
    App, View,
};

const INK: Color = Color::Rgb(223, 229, 239);
const MUTED: Color = Color::Rgb(111, 125, 145);
const CYAN: Color = Color::Rgb(92, 226, 221);
const PURPLE: Color = Color::Rgb(183, 139, 255);
const PINK: Color = Color::Rgb(255, 132, 179);
const GOLD: Color = Color::Rgb(255, 196, 119);
const PANEL: Color = Color::Rgb(34, 43, 58);
const CONTENT_BG: Color = Color::Rgb(43, 47, 57);

pub struct Highlighter {
    syntaxes: SyntaxSet,
    theme: Theme,
}

impl Highlighter {
    pub fn new() -> Self {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let themes = ThemeSet::load_defaults();
        let theme = themes
            .themes
            .get("base16-ocean.dark")
            .cloned()
            .or_else(|| themes.themes.values().next().cloned())
            .expect("syntect ships a theme");
        Self { syntaxes, theme }
    }

    pub fn code_lines(&self, code: &str, language: &str) -> Vec<Line<'static>> {
        let syntax = self
            .syntaxes
            .find_syntax_by_token(language)
            .unwrap_or_else(|| self.syntaxes.find_syntax_plain_text());
        let mut highlighter = HighlightLines::new(syntax, &self.theme);
        code.lines()
            .map(|line| {
                let spans = highlighter
                    .highlight_line(&format!("{line}\n"), &self.syntaxes)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(style, part)| {
                        let color = style.foreground;
                        Span::styled(
                            part.trim_end_matches('\n').to_string(),
                            Style::default().fg(Color::Rgb(color.r, color.g, color.b)),
                        )
                    })
                    .collect::<Vec<_>>();
                Line::from(spans)
            })
            .collect()
    }
}

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < 42 || area.height < 18 {
        frame.render_widget(
            Paragraph::new("mdnq needs a little more room").style(Style::default().fg(MUTED)),
            area,
        );
        return;
    }
    header(frame, app, area);
    if app.view == View::Search {
        search_view(frame, app, area);
    } else {
        page_view(frame, app, area);
    }
    let footer = if app.status.is_empty() {
        match app.view {
            View::Search => String::new(),
            _ => app
                .page
                .as_ref()
                .map_or_else(|| "MDN · Loading".into(), |p| p.url.clone()),
        }
    } else {
        app.status.clone()
    };
    if !footer.is_empty() {
        let x = if app.view == View::Search { 3 } else { 5 };
        let width = if app.view == View::Search {
            area.width - 6
        } else {
            area.width - 10
        };
        frame.render_widget(
            Paragraph::new(footer).style(Style::default().fg(MUTED)),
            Rect::new(x, area.height - 1, width, 1),
        );
    }
}

fn header(frame: &mut Frame, app: &App, area: Rect) {
    let colors = [CYAN, PURPLE, PINK, GOLD];
    let brand = Line::from(vec![
        Span::styled(
            "mdnq ",
            Style::default().fg(INK).add_modifier(Modifier::BOLD),
        ),
        Span::styled("✦", Style::default().fg(colors[app.tick % colors.len()])),
    ]);
    frame.render_widget(Paragraph::new(brand), Rect::new(3, 1, 10, 1));
    let content_width = area.width.saturating_sub(13);
    match app.view {
        View::Search => {
            let input = if app.query.is_empty() {
                Line::from(Span::styled("Type a topic…", Style::default().fg(MUTED)))
            } else {
                let visible: String = app
                    .query
                    .chars()
                    .rev()
                    .take(content_width.saturating_sub(1) as usize)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                Line::from(vec![
                    Span::styled(visible, Style::default().fg(INK)),
                    Span::styled("▌", Style::default().fg(CYAN)),
                ])
            };
            frame.render_widget(Paragraph::new(input), Rect::new(10, 1, content_width, 1));
        }
        View::Read | View::Code => {
            let title = app
                .page
                .as_ref()
                .map_or("Reading page…", |page| &page.title);
            frame.render_widget(
                Paragraph::new(title.to_string())
                    .style(Style::default().fg(INK).add_modifier(Modifier::BOLD)),
                Rect::new(10, 1, content_width, 1),
            );
        }
    }
}

fn search_view(frame: &mut Frame, app: &App, area: Rect) {
    if !app.query.is_empty() && app.results.is_empty() && !app.index.is_empty() {
        frame.render_widget(
            Paragraph::new("No matches. Try a shorter search.").style(Style::default().fg(MUTED)),
            Rect::new(5, 3, area.width - 10, 1),
        );
    }
    let visible = area.height.saturating_sub(5).max(1) as usize;
    for (offset, item) in app
        .results
        .iter()
        .skip(app.result_offset)
        .take(visible)
        .enumerate()
    {
        let y = 3 + offset as u16;
        if y >= area.height - 2 {
            break;
        }
        let selected = app.result_offset + offset == app.selected;
        let fg = if selected { CYAN } else { INK };
        let row_style = if selected {
            Style::default().bg(PANEL)
        } else {
            Style::default()
        };
        frame.render_widget(
            Paragraph::new(format!("  {}", item.title)).style(row_style.fg(fg).add_modifier(
                if selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                },
            )),
            Rect::new(3, y, area.width - 6, 1),
        );
    }
}

fn page_view(frame: &mut Frame, app: &App, area: Rect) {
    let Some(page) = app.page.as_ref() else {
        return;
    };
    let read_style = if app.view == View::Read {
        Style::default().fg(CYAN).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(MUTED)
    };
    let code_style = if app.view == View::Code {
        Style::default().fg(CYAN).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(MUTED)
    };
    frame.render_widget(
        Paragraph::new("  Full  ").style(read_style),
        Rect::new(3, 3, 11, 1),
    );
    frame.render_widget(
        Paragraph::new("  Snippets  ").style(code_style),
        Rect::new(14, 3, 17, 1),
    );
    frame.render_widget(
        WidgetBlock::default()
            .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
            .style(Style::default().bg(CONTENT_BG))
            .border_style(Style::default().fg(PANEL)),
        Rect::new(3, 4, area.width - 6, area.height - 4),
    );
    match app.view {
        View::Read => draw_reader(frame, app, area),
        View::Code => draw_code(frame, page, app, area),
        View::Search => {}
    }
}

fn draw_reader(frame: &mut Frame, app: &App, area: Rect) {
    let viewport = area.height - 7;
    let max_scroll = app.reader_lines.len().saturating_sub(viewport as usize) as u16;
    frame.render_widget(
        Paragraph::new(app.reader_lines.clone()).scroll((app.scroll.min(max_scroll), 0)),
        Rect::new(5, 5, area.width - 10, viewport),
    );
    let progress = if max_scroll == 0 {
        "END".to_string()
    } else {
        format!(
            "{}%",
            (app.scroll.min(max_scroll) as usize * 100) / max_scroll as usize
        )
    };
    frame.render_widget(
        Paragraph::new(progress).style(Style::default().fg(MUTED)),
        Rect::new(area.width - 9, area.height - 2, 6, 1),
    );
}

pub fn reader_lines(page: &Page, width: usize, highlighter: &Highlighter) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for block in &page.blocks {
        match block {
            Block::Heading(level, value) => {
                if !lines.is_empty() {
                    lines.push(Line::raw(""));
                }
                let color = if *level <= 2 { CYAN } else { PURPLE };
                let style = Style::default().fg(color).add_modifier(Modifier::BOLD);
                push_wrapped(&mut lines, value, width, style);
                lines.push(Line::raw(""));
            }
            Block::Paragraph(value) => {
                push_wrapped(&mut lines, value, width, Style::default().fg(INK));
                lines.push(Line::raw(""));
            }
            Block::Bullet(value) => {
                push_wrapped(&mut lines, value, width, Style::default().fg(INK));
            }
            Block::Quote(value) => {
                push_wrapped(
                    &mut lines,
                    &format!("│  {value}"),
                    width,
                    Style::default().fg(PURPLE),
                );
                lines.push(Line::raw(""));
            }
            Block::Image(value) => {
                push_wrapped(
                    &mut lines,
                    &format!("▧  Image or media: {value}"),
                    width,
                    Style::default().fg(GOLD),
                );
                lines.push(Line::raw(""));
            }
            Block::Code(code, language) => {
                lines.push(Line::from(Span::styled(
                    format!(
                        "  {}",
                        if language.is_empty() {
                            "code"
                        } else {
                            language
                        }
                    ),
                    Style::default().fg(MUTED),
                )));
                for line in highlighter.code_lines(code, language) {
                    let mut spans = vec![Span::styled("  │ ", Style::default().fg(PANEL))];
                    spans.extend(line.spans);
                    lines.push(Line::from(spans));
                }
                lines.push(Line::raw(""));
            }
            Block::Table(rows) => {
                for row in rows {
                    push_wrapped(
                        &mut lines,
                        &row.join("  │  "),
                        width,
                        Style::default().fg(INK),
                    );
                }
                lines.push(Line::raw(""));
            }
            Block::Rule => {
                lines.push(Line::from(Span::styled(
                    "─".repeat(width),
                    Style::default().fg(PANEL),
                )));
                lines.push(Line::raw(""));
            }
        }
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "No readable content found.",
            Style::default().fg(MUTED),
        )));
    }
    lines
}

fn push_wrapped(lines: &mut Vec<Line<'static>>, value: &str, width: usize, style: Style) {
    for part in textwrap::wrap(value, width.max(8)) {
        lines.push(Line::from(Span::styled(part.into_owned(), style)));
    }
}

fn draw_code(frame: &mut Frame, page: &Page, app: &App, area: Rect) {
    if page.examples.is_empty() {
        frame.render_widget(
            Paragraph::new("This page has no code snippets. The reader is still available.")
                .style(Style::default().fg(MUTED)),
            Rect::new(5, 5, area.width - 10, 2),
        );
        return;
    }
    let example = &page.examples[app.example.min(page.examples.len() - 1)];
    let language = if example.language.is_empty() {
        "code"
    } else {
        &example.language
    };
    frame.render_widget(
        Paragraph::new(format!(
            "{} / {}  ·  {language}",
            app.example + 1,
            page.examples.len()
        ))
        .style(Style::default().fg(PURPLE).add_modifier(Modifier::BOLD)),
        Rect::new(5, 5, area.width - 20, 1),
    );
    frame.render_widget(
        Paragraph::new("‹ Prev").style(Style::default().fg(CYAN)),
        Rect::new(area.width - 16, 5, 7, 1),
    );
    frame.render_widget(
        Paragraph::new("Next ›").style(Style::default().fg(CYAN)),
        Rect::new(area.width - 9, 5, 7, 1),
    );
    let caption_rows = if example.caption.is_empty() {
        0
    } else {
        textwrap::wrap(&example.caption, area.width.saturating_sub(10) as usize)
            .len()
            .min(2) as u16
    };
    if caption_rows > 0 {
        frame.render_widget(
            Paragraph::new(example.caption.clone())
                .style(Style::default().fg(INK))
                .wrap(Wrap { trim: true }),
            Rect::new(5, 7, area.width - 10, caption_rows),
        );
    }
    let code_y = if caption_rows == 0 {
        7
    } else {
        8 + caption_rows
    };
    let lines = app.code_lines.get(app.example).cloned().unwrap_or_default();
    let viewport = area.height.saturating_sub(code_y + 1);
    let max_scroll = lines.len().saturating_sub(viewport as usize) as u16;
    let snippet_height = viewport.min(lines.len() as u16);
    frame.render_widget(
        Paragraph::new(lines)
            .scroll((app.scroll.min(max_scroll), app.horizontal))
            .block(
                WidgetBlock::default()
                    .borders(Borders::LEFT)
                    .border_style(Style::default().fg(PURPLE)),
            ),
        Rect::new(5, code_y, area.width - 10, snippet_height),
    );
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::{backend::TestBackend, Terminal};

    use super::*;
    use crate::{mdn::Example, search::SearchItem};

    fn render(app: &App) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        (0u16..24)
            .map(|y| {
                (0u16..80)
                    .map(|x| terminal.backend().buffer()[(x, y)].symbol().to_string())
                    .collect::<Vec<_>>()
                    .concat()
            })
            .collect()
    }

    #[test]
    fn search_and_tightly_spaced_titles_share_the_header() {
        let (sender, _) = mpsc::channel();
        let mut app = App::new("media".into(), sender);
        app.results.push(SearchItem {
            title: "Using media queries".into(),
            url: "/en-US/docs/Web/CSS/Guides/Media_queries/Using".into(),
        });
        app.results.push(SearchItem {
            title: "MediaQueryList".into(),
            url: "/en-US/docs/Web/API/MediaQueryList".into(),
        });
        let rows = render(&app);
        assert_eq!(rows[1].matches("mdnq").count(), 1);
        assert_eq!(rows[1].matches('✦').count(), 1);
        assert!(rows[1].contains("mdnq ✦ media"));
        assert!(!rows[1].contains("Search ↵"));
        assert!(!rows[1].contains('⌕'));
        assert!(rows[3].contains("Using media queries"));
        assert!(rows[4].contains("MediaQueryList"));
        assert!(!rows.join("\n").contains("/en-US/docs/"));
        assert!(!rows.join("\n").contains("Results"));
    }

    #[test]
    fn page_has_frame_and_short_snippet_edge_without_caption_gap() {
        let (sender, _) = mpsc::channel();
        let mut app = App::new(String::new(), sender);
        app.view = View::Code;
        app.page = Some(Page {
            title: "A page".into(),
            url: "https://developer.mozilla.org/en-US/docs/example".into(),
            blocks: vec![],
            examples: vec![Example {
                code: "let x = 1;".into(),
                language: "js".into(),
                caption: String::new(),
            }],
        });
        app.code_lines = vec![vec![Line::raw("let x = 1;")]];
        let rows = render(&app);
        assert_eq!(rows[1].matches("mdnq").count(), 1);
        assert!(rows[1].contains("mdnq ✦ A page"));
        assert!(rows[3].contains("Full"));
        assert!(rows[3].contains("Snippets"));
        assert!(rows[4].contains('┌'));
        assert!(rows[5].contains("1 / 1  ·  js"));
        assert!(rows[7].contains("let x = 1;"));
        assert_eq!(rows[7].chars().nth(5), Some('│'));
        assert_eq!(rows[8].chars().nth(5), Some(' '));

        for view in [View::Code, View::Read] {
            app.view = view;
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            let buffer = terminal.backend().buffer();
            assert_eq!(buffer[(20, 23)].bg, CONTENT_BG);
            assert_eq!(buffer[(20, 3)].bg, Color::Reset);
        }

        app.view = View::Read;
        app.mouse(click(22, 3), 80, 24);
        assert!(app.view == View::Code);
        app.mouse(click(5, 1), 80, 24);
        assert!(app.view == View::Search);
    }

    fn click(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::empty(),
        }
    }
}
