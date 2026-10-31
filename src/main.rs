mod mdn;
mod search;
mod ui;

use std::{
    io::{self, IsTerminal},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use anyhow::{anyhow, Result};
use crossterm::{
    event::{
        self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    },
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::text::Line;
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::{
    mdn::{Mdn, Page},
    search::{search, SearchItem},
};

enum Message {
    Index(Result<(Vec<SearchItem>, bool)>),
    Page(String, Result<Page>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Search,
    Read,
    Code,
}

struct App {
    query: String,
    index: Vec<SearchItem>,
    results: Vec<SearchItem>,
    selected: usize,
    result_offset: usize,
    view: View,
    page: Option<Page>,
    pending_url: Option<String>,
    example: usize,
    scroll: u16,
    horizontal: u16,
    reader_lines: Vec<Line<'static>>,
    code_lines: Vec<Vec<Line<'static>>>,
    reader_width: u16,
    status: String,
    tick: usize,
    sender: Sender<Message>,
}

impl App {
    fn new(query: String, sender: Sender<Message>) -> Self {
        Self {
            query,
            index: Vec::new(),
            results: Vec::new(),
            selected: 0,
            result_offset: 0,
            view: View::Search,
            page: None,
            pending_url: None,
            example: 0,
            scroll: 0,
            horizontal: 0,
            reader_lines: Vec::new(),
            code_lines: Vec::new(),
            reader_width: 0,
            status: "Connecting to MDN…".into(),
            tick: 0,
            sender,
        }
    }

    fn refresh_results(&mut self) {
        self.results = search(&self.index, &self.query, 30);
        self.selected = 0;
        self.result_offset = 0;
    }

    fn open_selected(&mut self) {
        if let Some(item) = self.results.get(self.selected).cloned() {
            self.open_page(&item.title, &item.url);
        }
    }

    fn open_page(&mut self, title: &str, raw_url: &str) {
        let url = match mdn::normalize_url(raw_url) {
            Ok(url) => url,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        self.pending_url = Some(url.clone());
        self.page = None;
        self.reader_lines.clear();
        self.code_lines.clear();
        self.reader_width = 0;
        self.view = View::Read;
        self.scroll = 0;
        self.example = 0;
        self.status = "Reading MDN…".into();
        let sender = self.sender.clone();
        let title = title.to_string();
        thread::spawn(move || {
            let page = Mdn::new().and_then(|mdn| mdn.load_page(&url, &title));
            let _ = sender.send(Message::Page(url, page));
        });
    }

    fn receive(&mut self, message: Message, highlighter: &ui::Highlighter) {
        match message {
            Message::Index(Ok((index, stale))) => {
                self.index = index;
                self.refresh_results();
                if self.view == View::Search {
                    self.status = if stale {
                        "Using saved MDN index · offline"
                    } else {
                        ""
                    }
                    .into();
                }
            }
            Message::Index(Err(error)) => self.status = format!("Search unavailable: {error:#}"),
            Message::Page(url, result) if self.pending_url.as_deref() == Some(&url) => {
                self.pending_url = None;
                match result {
                    Ok(page) => {
                        self.code_lines = page
                            .examples
                            .iter()
                            .map(|example| highlighter.code_lines(&example.code, &example.language))
                            .collect();
                        self.reader_width = 0;
                        self.status = String::new();
                        self.page = Some(page);
                    }
                    Err(error) => {
                        self.view = View::Search;
                        self.status = format!("Page unavailable: {error:#}");
                    }
                }
            }
            Message::Page(_, _) => {}
        }
    }

    fn select_next(&mut self, direction: i32) {
        if self.results.is_empty() {
            return;
        }
        self.selected =
            (self.selected as i32 + direction).rem_euclid(self.results.len() as i32) as usize;
    }

    fn next_example(&mut self, direction: i32) {
        let count = self.page.as_ref().map_or(0, |page| page.examples.len());
        if count == 0 {
            return;
        }
        self.example = (self.example as i32 + direction).rem_euclid(count as i32) as usize;
        self.scroll = 0;
        self.horizontal = 0;
    }

    fn switch_view(&mut self, view: View) {
        if self.page.is_some() {
            self.view = view;
            self.scroll = 0;
        }
    }

    fn key(&mut self, key: KeyEvent) -> bool {
        if key.kind != KeyEventKind::Press {
            return true;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return false;
        }
        match self.view {
            View::Search => match key.code {
                KeyCode::Esc => return false,
                KeyCode::Up | KeyCode::BackTab => self.select_next(-1),
                KeyCode::Down | KeyCode::Tab => self.select_next(1),
                KeyCode::Enter => self.open_selected(),
                KeyCode::Backspace => {
                    self.query.pop();
                    self.refresh_results();
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.query.clear();
                    self.refresh_results();
                }
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.query.push(c);
                    self.refresh_results();
                }
                _ => {}
            },
            View::Read | View::Code => match key.code {
                KeyCode::Esc | KeyCode::Char('/') => {
                    self.view = View::Search;
                    self.pending_url = None;
                    self.scroll = 0;
                }
                KeyCode::Char('q') => return false,
                KeyCode::Tab => self.switch_view(if self.view == View::Read {
                    View::Code
                } else {
                    View::Read
                }),
                KeyCode::Char('1') => self.switch_view(View::Read),
                KeyCode::Char('2') => self.switch_view(View::Code),
                KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
                KeyCode::Down => self.scroll = self.scroll.saturating_add(1),
                KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(12),
                KeyCode::PageDown => self.scroll = self.scroll.saturating_add(12),
                KeyCode::Home => self.scroll = 0,
                KeyCode::Left | KeyCode::Char('p') if self.view == View::Code => {
                    self.next_example(-1)
                }
                KeyCode::Right | KeyCode::Char('n') if self.view == View::Code => {
                    self.next_example(1)
                }
                KeyCode::Char('h') if self.view == View::Code => {
                    self.horizontal = self.horizontal.saturating_sub(8)
                }
                KeyCode::Char('l') if self.view == View::Code => {
                    self.horizontal = self.horizontal.saturating_add(8)
                }
                _ => {}
            },
        }
        true
    }

    fn mouse(&mut self, mouse: MouseEvent, width: u16, height: u16) {
        match mouse.kind {
            MouseEventKind::ScrollDown => {
                if self.view == View::Search {
                    self.select_next(1);
                } else {
                    self.scroll = self.scroll.saturating_add(3);
                }
            }
            MouseEventKind::ScrollUp => {
                if self.view == View::Search {
                    self.select_next(-1);
                } else {
                    self.scroll = self.scroll.saturating_sub(3);
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if self.view == View::Search {
                    let row = mouse.row;
                    if row >= 3
                        && row < height.saturating_sub(2)
                        && mouse.column >= 3
                        && mouse.column < width.saturating_sub(3)
                    {
                        let index = self.result_offset + (row - 3) as usize;
                        let visible = height.saturating_sub(5).max(1) as usize;
                        if index < self.results.len() && index < self.result_offset + visible {
                            self.selected = index;
                            self.open_selected();
                        }
                    }
                } else if mouse.row == 1 && (3..12).contains(&mouse.column) {
                    self.view = View::Search;
                    self.pending_url = None;
                } else if mouse.row == 3 {
                    if mouse.column < 14 {
                        self.switch_view(View::Read);
                    } else if mouse.column < 31 {
                        self.switch_view(View::Code);
                    }
                } else if self.view == View::Code && mouse.row == 5 {
                    if mouse.column >= width.saturating_sub(16)
                        && mouse.column < width.saturating_sub(9)
                    {
                        self.next_example(-1);
                    } else if mouse.column >= width.saturating_sub(9)
                        && mouse.column < width.saturating_sub(2)
                    {
                        self.next_example(1);
                    }
                }
            }
            _ => {}
        }
    }
}

struct ScreenGuard;
impl ScreenGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        if let Err(error) = execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture) {
            let _ = disable_raw_mode();
            return Err(error.into());
        }
        Ok(Self)
    }
}
impl Drop for ScreenGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

fn run(query: String, direct_url: Option<String>) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(anyhow!("mdnq needs an interactive terminal"));
    }
    let (sender, receiver): (Sender<Message>, Receiver<Message>) = mpsc::channel();
    let mut app = App::new(query, sender.clone());
    thread::spawn(move || {
        let _ = sender.send(Message::Index(Mdn::new().and_then(|mdn| mdn.load_index())));
    });
    if let Some(url) = direct_url {
        app.open_page(&url, &url);
    }
    let _guard = ScreenGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let highlighter = ui::Highlighter::new();
    let mut last_tick = Instant::now();
    loop {
        while let Ok(message) = receiver.try_recv() {
            app.receive(message, &highlighter);
        }
        let size = terminal.size()?;
        if app.view == View::Read && app.reader_width != size.width {
            if let Some(page) = app.page.as_ref() {
                app.reader_lines =
                    ui::reader_lines(page, size.width.saturating_sub(10) as usize, &highlighter);
                app.reader_width = size.width;
            }
        }
        let visible = size.height.saturating_sub(5).max(1) as usize;
        if app.selected < app.result_offset {
            app.result_offset = app.selected;
        }
        if app.selected >= app.result_offset + visible {
            app.result_offset = app.selected + 1 - visible;
        }
        terminal.draw(|frame| ui::draw(frame, &app))?;
        if event::poll(Duration::from_millis(80))? {
            match event::read()? {
                Event::Key(key) => {
                    if !app.key(key) {
                        break;
                    }
                }
                Event::Mouse(mouse) => app.mouse(mouse, size.width, size.height),
                _ => {}
            }
        }
        if last_tick.elapsed() >= Duration::from_millis(140) {
            app.tick = app.tick.wrapping_add(1);
            last_tick = Instant::now();
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut query = String::new();
    let mut url = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("mdnq [query] [--url MDN_URL]\n\nSearch and read MDN in your terminal.");
                return Ok(());
            }
            "--version" | "-V" => {
                println!("mdnq {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--url" => {
                url = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--url needs an MDN URL"))?,
                )
            }
            _ if arg.starts_with('-') => return Err(anyhow!("unknown option: {arg}")),
            _ => {
                if !query.is_empty() {
                    query.push(' ');
                }
                query.push_str(&arg);
            }
        }
    }
    if let Some(ref raw) = url {
        mdn::normalize_url(raw)?;
    }
    run(query, url)
}
