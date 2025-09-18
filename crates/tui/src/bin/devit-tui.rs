use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{cursor::Show, execute};
use ratatui::backend::{Backend, CrosstermBackend, TestBackend};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Terminal;

#[derive(Parser, Debug, Clone)]
#[command(name = "devit-tui", version, about = "DevIt TUI: timeline + statusbar")]
struct Args {
    /// Path to journal JSONL (e.g., .devit/journal.jsonl)
    #[arg(long = "journal-path", value_name = "PATH")]
    journal_path: Option<PathBuf>,

    /// Follow new lines appended to journal
    #[arg(long, default_value_t = false)]
    follow: bool,

    /// Open a unified diff (path or '-' for stdin)
    #[arg(long = "open-diff", value_name = "PATH")]
    open_diff: Option<PathBuf>,

    /// Open a journal log (path or '-' for stdin)
    #[arg(long = "open-log", value_name = "PATH")]
    open_log: Option<PathBuf>,
}

#[derive(Default)]
struct App {
    lines: Vec<String>,
    selected: usize,
    follow: bool,
    journal_path: Option<PathBuf>,
    last_size: u64,
    status: String,
    help: bool,
    diff: Option<DiffState>,
}

impl App {
    fn new(journal_path: Option<PathBuf>, follow: bool) -> Self {
        Self {
            journal_path,
            follow,
            ..Default::default()
        }
    }

    fn load_initial(&mut self) -> Result<()> {
        let Some(p) = &self.journal_path else {
            return Ok(());
        };
        let meta =
            fs::metadata(p).with_context(|| format!("journal not found: {}", p.display()))?;
        let f = File::open(p).with_context(|| format!("open journal: {}", p.display()))?;
        let mut reader = BufReader::new(f);
        let mut buf = String::new();
        reader.read_to_string(&mut buf)?;
        self.lines = buf.lines().map(|s| s.to_string()).collect();
        self.last_size = meta.len();
        if self.selected >= self.lines.len() {
            self.selected = self.lines.len().saturating_sub(1);
        }
        Ok(())
    }

    fn poll_updates(&mut self) {
        let Some(journal_path) = &self.journal_path else {
            return;
        };
        if !self.follow {
            return;
        }
        let Ok(meta) = fs::metadata(journal_path) else {
            return;
        };
        if meta.len() <= self.last_size {
            return;
        }
        if let Ok(mut f) = File::open(journal_path) {
            use std::io::Seek;
            use std::io::SeekFrom;
            if f.seek(SeekFrom::Start(self.last_size)).is_ok() {
                let reader = BufReader::new(f);
                for line in reader.lines().map_while(Result::ok) {
                    self.lines.push(line);
                }
                self.last_size = meta.len();
            }
        }
    }
}

fn print_tool_error_journal_not_found(path: &PathBuf) {
    // Stable JSON error on stderr
    let _ = writeln!(
        std::io::stderr(),
        "{}",
        serde_json::json!({
            "type":"tool.error",
            "error":{
                "tui_io_error": true,
                "reason":"journal_not_found",
                "path": path,
            }
        })
    );
}

fn print_diff_error(reason: &str, path: &PathBuf) {
    let _ = writeln!(
        std::io::stderr(),
        "{}",
        serde_json::json!({
            "type":"tool.error",
            "error":{
                "diff_load_failed": true,
                "reason": reason,
                "path": path,
            }
        })
    );
}

fn print_diff_error_with_message(reason: &str, path: &PathBuf, message: &str) {
    let _ = writeln!(
        std::io::stderr(),
        "{}",
        serde_json::json!({
            "type":"tool.error",
            "error":{
                "diff_load_failed": true,
                "reason": reason,
                "path": path,
                "message": message,
            }
        })
    );
}

fn print_diff_error_stdin(reason: &str, message: &str) {
    let path = PathBuf::from("-");
    print_diff_error_with_message(reason, &path, message);
}

fn best_effort_status() -> String {
    // Try to query versions/policy; fall back silently on errors
    fn run(cmd: &str, args: &[&str]) -> Option<String> {
        let out = std::process::Command::new(cmd).args(args).output().ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    let ver_devit = run("devit", &["--version"]).unwrap_or_else(|| "devit N/A".into());
    let ver_mcpd = run("devit-mcpd", &["--version"]).unwrap_or_else(|| "mcpd N/A".into());
    // policy is optional and not parsed deeply here
    let policy = run("devit-mcp", &["--policy"]).unwrap_or_else(|| "policy N/A".into());
    format!("{} | {} | {}", ver_devit, ver_mcpd, policy)
}

fn main() -> Result<()> {
    let args = Args::parse();
    run(args)
}

fn run(args: Args) -> Result<()> {
    let journal_path = args
        .open_log
        .clone()
        .or_else(|| args.journal_path.clone());

    if journal_path.is_none() && args.open_diff.is_none() {
        bail!("either --journal-path/--open-log or --open-diff must be provided");
    }

    if let Some(path) = &journal_path {
        if !path.exists() {
            print_tool_error_journal_not_found(path);
            bail!("journal missing");
        }
    }

    let headless = headless_mode();
    let initial_follow = if headless { false } else { args.follow };

    let mut app = App::new(journal_path.clone(), initial_follow);
    app.status = best_effort_status();
    app.load_initial()?;

    if let Some(open_diff) = args.open_diff.as_ref() {
        let source = if open_diff.as_os_str() == "-" {
            DiffSource::Stdin
        } else {
            DiffSource::Path
        };
        match load_diff(open_diff, source, 1_048_576) {
            Ok(diff_state) => {
                app.status = diff_state.status_line();
                app.diff = Some(diff_state);
                app.follow = false;
            }
            Err(DiffError::NotFound) => {
                print_diff_error("not_found", open_diff);
                std::process::exit(2);
            }
            Err(DiffError::TooLarge) => {
                print_diff_error("too_large", open_diff);
                std::process::exit(2);
            }
            Err(DiffError::Parse(msg)) => {
                if open_diff.as_os_str() == "-" {
                    print_diff_error_stdin("parse_error", &msg);
                } else {
                    print_diff_error_with_message("parse_error", open_diff, &msg);
                }
                std::process::exit(2);
            }
        }
    }

    if journal_path.is_some() && args.open_diff.is_none() && args.open_log.is_some() {
        app.follow = false;
    }

    if headless {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend)?;
        let mut control = LoopControl::headless();
        return run_app(&mut terminal, &mut app, &mut control);
    }

    let mut control = LoopControl::interactive(initial_follow)?;
    let guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;
    let result = run_app(&mut terminal, &mut app, &mut control);
    terminal.show_cursor().ok();
    drop(guard);
    result
}

fn headless_mode() -> bool {
    std::env::var("DEVIT_TUI_HEADLESS")
        .ok()
        .map(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return true;
            }
            matches!(
                trimmed.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        execute!(std::io::stdout(), EnterAlternateScreen)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        disable_raw_mode().ok();
        let mut stdout = std::io::stdout();
        execute!(stdout, LeaveAlternateScreen, Show).ok();
    }
}

struct LoopControl {
    headless: bool,
    allow_block_without_follow: bool,
    follow_stop: Option<FollowStop>,
}

impl LoopControl {
    fn headless() -> Self {
        Self {
            headless: true,
            allow_block_without_follow: false,
            follow_stop: None,
        }
    }

    fn interactive(initial_follow: bool) -> Result<Self> {
        let follow_stop = if initial_follow {
            Some(FollowStop::install_ctrlc_handler()?)
        } else {
            None
        };
        Ok(Self {
            headless: false,
            allow_block_without_follow: true,
            follow_stop,
        })
    }

    fn ensure_follow_stop(&mut self) -> Result<()> {
        if self.follow_stop.is_none() {
            self.follow_stop = Some(FollowStop::install_ctrlc_handler()?);
        }
        Ok(())
    }
}

struct FollowStop {
    rx: Receiver<()>,
}

impl FollowStop {
    fn install_ctrlc_handler() -> Result<Self> {
        let (tx, rx) = mpsc::channel();
        ctrlc::set_handler(move || {
            let _ = tx.send(());
        })
        .context("install ctrl+c handler for follow mode")?;
        Ok(Self { rx })
    }

    fn should_stop(&mut self) -> bool {
        match self.rx.try_recv() {
            Ok(_) | Err(TryRecvError::Disconnected) => true,
            Err(TryRecvError::Empty) => false,
        }
    }
}

#[derive(Debug)]
enum DiffSource {
    Path,
    Stdin,
}

#[derive(Debug)]
enum DiffError {
    NotFound,
    TooLarge,
    Parse(String),
}

#[derive(Debug, Clone)]
struct DiffState {
    files: Vec<DiffFile>,
    file_idx: usize,
    hunk_idx: usize,
}

impl DiffState {
    fn new(files: Vec<DiffFile>) -> Self {
        Self {
            files,
            file_idx: 0,
            hunk_idx: 0,
        }
    }

    fn status_line(&self) -> String {
        if self.files.is_empty() {
            return "Diff: empty".to_string();
        }
        let file = &self.files[self.file_idx];
        let file_total = self.files.len();
        if file.hunks.is_empty() {
            format!(
                "Diff {}/{}: {} — no hunks",
                self.file_idx + 1,
                file_total,
                file.display_name
            )
        } else {
            format!(
                "Diff {}/{}: {} — hunk {}/{} (j/k hunks, h/H files)",
                self.file_idx + 1,
                file_total,
                file.display_name,
                self.hunk_idx + 1,
                file.hunks.len()
            )
        }
    }

    fn current(&self) -> Option<(&DiffFile, Option<&DiffHunk>)> {
        let file = self.files.get(self.file_idx)?;
        let hunk = file.hunks.get(self.hunk_idx);
        Some((file, hunk))
    }

    fn next_hunk(&mut self) -> bool {
        if self.files.is_empty() {
            return false;
        }
        let file = &self.files[self.file_idx];
        if file.hunks.is_empty() {
            return false;
        }
        if self.hunk_idx + 1 < file.hunks.len() {
            self.hunk_idx += 1;
            true
        } else {
            false
        }
    }

    fn prev_hunk(&mut self) -> bool {
        if self.files.is_empty() {
            return false;
        }
        if self.hunk_idx > 0 {
            self.hunk_idx -= 1;
            true
        } else {
            false
        }
    }

    fn next_file(&mut self) -> bool {
        if self.file_idx + 1 < self.files.len() {
            self.file_idx += 1;
            self.hunk_idx = 0;
            true
        } else {
            false
        }
    }

    fn prev_file(&mut self) -> bool {
        if self.file_idx > 0 {
            self.file_idx -= 1;
            self.hunk_idx = 0;
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone)]
struct DiffFile {
    display_name: String,
    header: Vec<String>,
    hunks: Vec<DiffHunk>,
}

#[derive(Debug, Clone)]
struct DiffHunk {
    header: String,
    lines: Vec<String>,
}

fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    control: &mut LoopControl,
) -> Result<()> {
    draw_frame(terminal, app)?;

    if control.headless {
        return Ok(());
    }

    let tick_rate = Duration::from_millis(150);
    let mut last_tick = Instant::now();

    'main: loop {
        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') => break Ok(()),
                        _ => {
                            if let Some(diff) = app.diff.as_mut() {
                                let mut updated = false;
                                match key.code {
                                    KeyCode::Char('j') | KeyCode::Char('J') | KeyCode::Down => {
                                        if diff.next_hunk() {
                                            updated = true;
                                        }
                                    }
                                    KeyCode::Char('k') | KeyCode::Char('K') | KeyCode::Up => {
                                        if diff.prev_hunk() {
                                            updated = true;
                                        }
                                    }
                                    KeyCode::Char('h') => {
                                        if diff.prev_file() {
                                            updated = true;
                                        }
                                    }
                                    KeyCode::Char('H') => {
                                        if diff.next_file() {
                                            updated = true;
                                        }
                                    }
                                    KeyCode::F(1) => app.help = !app.help,
                                    _ => {}
                                }
                                if updated {
                                    app.status = diff.status_line();
                                }
                                continue 'main;
                            } else {
                                match key.code {
                                    KeyCode::Char('f') => {
                                        app.follow = !app.follow;
                                        if app.follow {
                                            control.ensure_follow_stop()?;
                                        }
                                    }
                                    KeyCode::Up => {
                                        app.selected = app.selected.saturating_sub(1);
                                    }
                                    KeyCode::Down => {
                                        app.selected = app
                                            .selected
                                            .saturating_add(1)
                                            .min(app.lines.len().saturating_sub(1));
                                    }
                                    KeyCode::Char('/') => {
                                        app.status =
                                            format!("search: not implemented | {}", app.status);
                                    }
                                    KeyCode::F(1) => app.help = !app.help,
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }

        if !control.allow_block_without_follow && !app.follow {
            return Ok(());
        }

        if let Some(stop) = control.follow_stop.as_mut() {
            if stop.should_stop() {
                return Ok(());
            }
        }

        if last_tick.elapsed() >= tick_rate {
            app.poll_updates();
            last_tick = Instant::now();
        }

        draw_frame(terminal, app)?;
    }
}

fn draw_frame<B: Backend>(terminal: &mut Terminal<B>, app: &App) -> Result<()> {
    terminal.draw(|f| {
        let size = f.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(1)].as_ref())
            .split(size);

        if let Some(diff) = &app.diff {
            draw_diff_view(f, chunks[0], diff);
        } else {
            let title = Span::raw("Timeline");
            let block = Block::default().title(title).borders(Borders::ALL);
            let items: Vec<ListItem> = app
                .lines
                .iter()
                .rev()
                .map(|l| ListItem::new(Line::from(l.as_str())))
                .collect();
            let list = List::new(items)
                .block(block)
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
            f.render_widget(list, chunks[0]);
        }

        let status = Paragraph::new(Line::from(app.status.clone())).block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::raw("Status")),
        );
        f.render_widget(status, chunks[1]);

        if app.help {
            let help_text = if app.diff.is_some() {
                "Diff keys: q=quit, j/k hunk ±, h/H file ±"
            } else {
                "Keys: q=quit, f=follow toggle, ↑/↓ navigate, /=search, F1=help"
            };
            let area = centered_rect(60, 40, size);
            let help = Paragraph::new(help_text)
                .block(Block::default().title("Help").borders(Borders::ALL));
            f.render_widget(help, area);
        }
    })?;
    Ok(())
}

fn draw_diff_view(frame: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect, diff: &DiffState) {
    let block_title = if let Some((file, _)) = diff.current() {
        format!(
            "Diff: {} ({}/{})",
            file.display_name,
            diff.file_idx + 1,
            diff.files.len()
        )
    } else {
        "Diff".to_string()
    };

    let mut lines: Vec<Line> = Vec::new();
    if let Some((file, hunk_opt)) = diff.current() {
        if !file.header.is_empty() {
            for header in &file.header {
                lines.push(Line::from(header.clone()));
            }
        }
        if let Some(hunk) = hunk_opt {
            lines.push(Line::from(hunk.header.clone()));
            for body_line in &hunk.lines {
                let style = if body_line.starts_with('+') {
                    Style::default().fg(Color::Green)
                } else if body_line.starts_with('-') {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default()
                };
                lines.push(Line::from(Span::styled(body_line.clone(), style)));
            }
        } else {
            lines.push(Line::from("(no hunks)"));
        }
    } else {
        lines.push(Line::from("No diff content"));
    }

    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .title(Span::raw(block_title))
            .borders(Borders::ALL),
    );
    frame.render_widget(paragraph, area);
}

fn load_diff(path: &PathBuf, source: DiffSource, max_size: usize) -> Result<DiffState, DiffError> {
    let content = match source {
        DiffSource::Path => {
            let metadata = fs::metadata(path).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    DiffError::NotFound
                } else {
                    DiffError::Parse(e.to_string())
                }
            })?;
            if metadata.len() as usize > max_size {
                return Err(DiffError::TooLarge);
            }
            let mut buf = String::new();
            File::open(path)
                .and_then(|mut f| f.read_to_string(&mut buf))
                .map_err(|e| DiffError::Parse(e.to_string()))?;
            buf
        }
        DiffSource::Stdin => {
            let mut buf = String::new();
            let mut handle = std::io::stdin().lock();
            handle
                .read_to_string(&mut buf)
                .map_err(|e| DiffError::Parse(e.to_string()))?;
            if buf.len() > max_size {
                return Err(DiffError::TooLarge);
            }
            buf
        }
    };

    let files = parse_unified_diff(&content).map_err(DiffError::Parse)?;
    if files.is_empty() {
        return Err(DiffError::Parse("empty diff".to_string()));
    }
    Ok(DiffState::new(files))
}

fn parse_unified_diff(content: &str) -> Result<Vec<DiffFile>, String> {
    #[derive(Default)]
    struct PartialFile {
        header: Vec<String>,
        hunks: Vec<DiffHunk>,
        old_path: Option<String>,
        new_path: Option<String>,
        diff_header: Option<String>,
    }

    impl PartialFile {
        fn with_diff_header(line: &str) -> Self {
            let mut pf = PartialFile::default();
            pf.diff_header = Some(line.to_string());
            pf.header.push(line.to_string());
            pf
        }

        fn finalize(self) -> DiffFile {
            let display = self
                .new_path
                .as_ref()
                .or(self.old_path.as_ref())
                .cloned()
                .or_else(|| {
                    self.diff_header
                        .as_ref()
                        .and_then(|h| extract_from_diff_header(h))
                })
                .unwrap_or_else(|| "(unknown)".to_string());
            DiffFile {
                display_name: clean_diff_path(&display),
                header: self.header,
                hunks: self.hunks,
            }
        }
    }

    let mut files: Vec<DiffFile> = Vec::new();
    let mut current_file: Option<PartialFile> = None;
    let mut current_hunk: Option<DiffHunk> = None;

    let flush_hunk = |file: &mut Option<PartialFile>, hunk: &mut Option<DiffHunk>| {
        if let Some(h) = hunk.take() {
            if file.is_none() {
                *file = Some(PartialFile::default());
            }
            if let Some(f) = file.as_mut() {
                f.hunks.push(h);
            }
        }
    };

    let flush_file =
        |files: &mut Vec<DiffFile>, file: &mut Option<PartialFile>, hunk: &mut Option<DiffHunk>| {
            flush_hunk(file, hunk);
            if let Some(pf) = file.take() {
                files.push(pf.finalize());
            }
        };

    for line in content.lines() {
        if line.starts_with("diff --git") {
            flush_file(&mut files, &mut current_file, &mut current_hunk);
            current_file = Some(PartialFile::with_diff_header(line));
            continue;
        }

        if line.starts_with("@@") {
            if current_file.is_none() {
                current_file = Some(PartialFile::default());
            }
            flush_hunk(&mut current_file, &mut current_hunk);
            current_hunk = Some(DiffHunk {
                header: line.to_string(),
                lines: Vec::new(),
            });
            continue;
        }

        if let Some(hunk) = current_hunk.as_mut() {
            hunk.lines.push(line.to_string());
            continue;
        }

        if current_file.is_none() {
            current_file = Some(PartialFile::default());
        }

        if let Some(file) = current_file.as_mut() {
            if line.starts_with("--- ") {
                file.old_path = extract_path_after_prefix(line);
            }
            if line.starts_with("+++ ") {
                file.new_path = extract_path_after_prefix(line);
            }
            file.header.push(line.to_string());
        }
    }

    flush_file(&mut files, &mut current_file, &mut current_hunk);

    Ok(files)
}

fn extract_path_after_prefix(line: &str) -> Option<String> {
    line.split_whitespace().nth(1).map(|p| clean_diff_path(p))
}

fn clean_diff_path(raw: &str) -> String {
    let trimmed = raw.trim_matches('"');
    let without_prefix = trimmed
        .strip_prefix("a/")
        .or_else(|| trimmed.strip_prefix("b/"))
        .unwrap_or(trimmed);
    without_prefix.to_string()
}

fn extract_from_diff_header(line: &str) -> Option<String> {
    let mut parts = line.split_whitespace();
    // Expect format: diff --git a/path b/path
    let first = parts.find(|part| part.starts_with('a'))?;
    let second = parts.next();
    second.or(Some(first)).map(clean_diff_path)
}

fn centered_rect(
    percent_x: u16,
    percent_y: u16,
    r: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1]);
    horizontal[1]
}

// tests moved to integration tests
