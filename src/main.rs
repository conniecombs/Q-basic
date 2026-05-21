#![allow(dead_code)]

mod graphics;
mod interpreter;
mod lexer;
mod parser;

use arboard::Clipboard;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use tui_textarea::TextArea;

use crate::interpreter::ConsoleMsg;

const APP_NAME: &str = "QBasic Studio";
const DEFAULT_FILE_NAME: &str = "main.bas";
const DEFAULT_SOURCE: &str = "' Welcome to QBasic Studio
RANDOMIZE TIMER()
DATA \"Ada\", \"Grace\", \"Katherine\"

PRINT \"QBasic Studio is ready.\"
FOR I = 1 TO 3
    READ NAME$
    PRINT \"HELLO, \"; NAME$
NEXT I
";

enum CliCommand {
    Ide(Option<PathBuf>),
    Run(PathBuf),
    RunWindow(PathBuf),
    Check(PathBuf),
    Help,
    Version,
}

enum AppState {
    Editing,
    PromptingSave(String),
    PromptingLoad(String),
    RunningInput { prompt: String, input: String },
    ConfirmQuit(String),
    ConfirmNew(String),
}

struct RunningProgram {
    child: Child,
    source_path: PathBuf,
}

#[derive(Default)]
struct EditorView {
    top_row: usize,
    left_col: usize,
}

struct EditorRenderMeta<'a> {
    current_file: &'a Path,
    dirty: bool,
    is_running: bool,
    show_cursor: bool,
}

fn main() -> ExitCode {
    match real_main() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {}", err);
            ExitCode::FAILURE
        }
    }
}

fn real_main() -> Result<ExitCode, Box<dyn Error>> {
    match parse_cli(env::args().skip(1).collect())? {
        CliCommand::Ide(path) => {
            run_ide(path)?;
            Ok(ExitCode::SUCCESS)
        }
        CliCommand::Run(path) => match run_file(&path) {
            Ok(()) => Ok(ExitCode::SUCCESS),
            Err(err) => {
                eprintln!("{}", err);
                Ok(ExitCode::FAILURE)
            }
        },
        CliCommand::RunWindow(path) => {
            let code = match run_file(&path) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("{}", err);
                    ExitCode::FAILURE
                }
            };
            pause_before_close();
            Ok(code)
        }
        CliCommand::Check(path) => match check_file(&path) {
            Ok(()) => {
                println!("{}: syntax ok", path.display());
                Ok(ExitCode::SUCCESS)
            }
            Err(err) => {
                eprintln!("{}", err);
                Ok(ExitCode::FAILURE)
            }
        },
        CliCommand::Help => {
            print_help();
            Ok(ExitCode::SUCCESS)
        }
        CliCommand::Version => {
            println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn parse_cli(args: Vec<String>) -> Result<CliCommand, String> {
    match args.as_slice() {
        [] => Ok(CliCommand::Ide(None)),
        [flag] if flag == "-h" || flag == "--help" || flag == "help" => Ok(CliCommand::Help),
        [flag] if flag == "-V" || flag == "--version" || flag == "version" => {
            Ok(CliCommand::Version)
        }
        [cmd, path] if cmd == "run" => Ok(CliCommand::Run(PathBuf::from(path))),
        [cmd, path] if cmd == "run-window" => Ok(CliCommand::RunWindow(PathBuf::from(path))),
        [cmd, path] if cmd == "check" => Ok(CliCommand::Check(PathBuf::from(path))),
        [cmd] if cmd == "edit" || cmd == "ide" => Ok(CliCommand::Ide(None)),
        [cmd, path] if cmd == "edit" || cmd == "ide" => {
            Ok(CliCommand::Ide(Some(PathBuf::from(path))))
        }
        [path] if !path.starts_with('-') => Ok(CliCommand::Run(PathBuf::from(path))),
        _ => Err(format!("invalid arguments\n\n{}", help_text())),
    }
}

fn print_help() {
    println!("{}", help_text());
}

fn help_text() -> String {
    format!(
        "{name} {version}

USAGE:
    qbasic_interpreter                 Launch the terminal IDE
    qbasic_interpreter <file.bas>      Run a BASIC program
    qbasic_interpreter run <file.bas>  Run a BASIC program
    qbasic_interpreter check <file>    Parse and validate a program
    qbasic_interpreter edit [file]     Open the IDE, optionally with a file

SHORTCUTS:
    F5/Ctrl+R run   F6 stop   F2/Ctrl+S save   F12 save as
    F3/Ctrl+O open  Ctrl+N new   Ctrl+L clear console   Esc/Ctrl+Q quit",
        name = APP_NAME,
        version = env!("CARGO_PKG_VERSION")
    )
}

fn run_ide(open_file: Option<PathBuf>) -> Result<(), Box<dyn Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_ide_loop(&mut terminal, open_file);

    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    disable_raw_mode()?;
    terminal.show_cursor()?;

    result
}

fn run_ide_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    open_file: Option<PathBuf>,
) -> Result<(), Box<dyn Error>> {
    let mut clipboard = Clipboard::new().ok();
    let mut textarea = TextArea::default();
    style_textarea(&mut textarea);

    let mut current_file = open_file.unwrap_or_else(|| PathBuf::from(DEFAULT_FILE_NAME));
    let mut dirty = false;
    let mut status_msg = String::from("Ready");
    let mut status_color = Color::Green;

    if current_file.exists() {
        match fs::read_to_string(&current_file) {
            Ok(content) => {
                textarea = textarea_from_source(&content);
                status_msg = format!("Loaded {}", display_path(&current_file));
            }
            Err(err) => {
                textarea = textarea_from_source(DEFAULT_SOURCE);
                status_msg = format!("Could not load {}: {}", display_path(&current_file), err);
                status_color = Color::Red;
            }
        }
    } else {
        textarea = textarea_from_source(DEFAULT_SOURCE);
        dirty = true;
    }
    configure_editor(&mut textarea, &current_file, dirty, false);

    let mut console_output: Vec<String> = vec![String::from("Console ready.")];
    let mut editor_view = EditorView::default();

    let mut app_state = AppState::Editing;
    let mut is_running = false;
    let mut running_program: Option<RunningProgram> = None;

    loop {
        configure_editor(&mut textarea, &current_file, dirty, is_running);
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(8),
                    Constraint::Percentage(30),
                    Constraint::Length(3),
                ])
                .split(f.area());

            let header = Paragraph::new(header_line(&current_file, dirty, is_running)).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {} ", APP_NAME)),
            );
            f.render_widget(header, chunks[0]);

            render_basic_editor(
                f,
                chunks[1],
                &textarea,
                EditorRenderMeta {
                    current_file: &current_file,
                    dirty,
                    is_running,
                    show_cursor: matches!(app_state, AppState::Editing),
                },
                &mut editor_view,
            );

            let console_lines: Vec<String> = console_output
                .iter()
                .rev()
                .take(chunks[2].height.saturating_sub(2) as usize)
                .rev()
                .cloned()
                .collect();
            let console_text = console_lines.join("\n");
            let console = Paragraph::new(console_text)
                .style(Style::default().fg(Color::White))
                .block(Block::default().borders(Borders::ALL).title(" Console "));
            f.render_widget(console, chunks[2]);

            let footer_text = footer_text(&app_state, &status_msg);
            let footer_color = footer_color(&app_state, status_color);
            let footer = Paragraph::new(footer_text)
                .style(Style::default().fg(footer_color).bg(Color::Black))
                .block(Block::default().borders(Borders::ALL).title(" Status "));
            f.render_widget(footer, chunks[3]);
        })?;

        poll_running_program(
            &mut running_program,
            &mut is_running,
            &mut status_msg,
            &mut status_color,
            &mut console_output,
        );

        if event::poll(std::time::Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                match &mut app_state {
                    AppState::RunningInput { prompt, input } => match key.code {
                        KeyCode::Enter => {
                            let val = input.clone();
                            if let Some(last) = console_output.last_mut() {
                                last.push_str(prompt);
                                last.push_str(&val);
                            }
                            console_output.push(String::new());
                            app_state = AppState::Editing;
                        }
                        KeyCode::Backspace => {
                            input.pop();
                        }
                        KeyCode::Char(c) => {
                            input.push(c);
                        }
                        KeyCode::Esc => {
                            app_state = AppState::Editing;
                        }
                        _ => {}
                    },
                    AppState::PromptingSave(input) => match key.code {
                        KeyCode::Enter => {
                            let target_file = prompt_target_path(input, &current_file);
                            match save_editor(&target_file, &textarea) {
                                Ok(()) => {
                                    current_file = target_file;
                                    dirty = false;
                                    status_msg = format!("Saved {}", display_path(&current_file));
                                    status_color = Color::Green;
                                }
                                Err(err) => {
                                    status_msg = format!("Save failed: {}", err);
                                    status_color = Color::Red;
                                }
                            }
                            app_state = AppState::Editing;
                        }
                        KeyCode::Esc => app_state = AppState::Editing,
                        KeyCode::Backspace => {
                            input.pop();
                        }
                        KeyCode::Char(c) => {
                            input.push(c);
                        }
                        _ => {}
                    },
                    AppState::PromptingLoad(input) => match key.code {
                        KeyCode::Enter => {
                            let target_file = prompt_target_path(input, &current_file);
                            match fs::read_to_string(&target_file) {
                                Ok(content) => {
                                    textarea = textarea_from_source(&content);
                                    editor_view = EditorView::default();
                                    current_file = target_file;
                                    dirty = false;
                                    status_msg = format!("Loaded {}", display_path(&current_file));
                                    status_color = Color::Green;
                                }
                                Err(err) => {
                                    status_msg = format!("Load failed: {}", err);
                                    status_color = Color::Red;
                                }
                            }
                            app_state = AppState::Editing;
                        }
                        KeyCode::Esc => app_state = AppState::Editing,
                        KeyCode::Backspace => {
                            input.pop();
                        }
                        KeyCode::Char(c) => {
                            input.push(c);
                        }
                        _ => {}
                    },
                    AppState::ConfirmQuit(_) => match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                            stop_running_program(&mut running_program);
                            break;
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                            app_state = AppState::Editing;
                            status_msg = String::from("Quit cancelled");
                            status_color = Color::Green;
                        }
                        _ => {}
                    },
                    AppState::ConfirmNew(_) => match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                            stop_running_program(&mut running_program);
                            is_running = false;
                            current_file = PathBuf::from(DEFAULT_FILE_NAME);
                            textarea = textarea_from_source("");
                            editor_view = EditorView::default();
                            dirty = false;
                            console_output.clear();
                            console_output.push(String::from("New program."));
                            status_msg = String::from("New program");
                            status_color = Color::Green;
                            app_state = AppState::Editing;
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                            app_state = AppState::Editing;
                            status_msg = String::from("New cancelled");
                            status_color = Color::Green;
                        }
                        _ => {}
                    },
                    AppState::Editing => match key.code {
                        KeyCode::Esc => {
                            if dirty || is_running {
                                app_state = AppState::ConfirmQuit(quit_reason(dirty, is_running));
                            } else {
                                break;
                            }
                        }
                        KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if dirty || is_running {
                                app_state = AppState::ConfirmQuit(quit_reason(dirty, is_running));
                            } else {
                                break;
                            }
                        }
                        KeyCode::F(5) => {
                            if is_running {
                                status_msg = String::from("Program is already running");
                                status_color = Color::Yellow;
                            } else {
                                start_program(
                                    &textarea,
                                    &current_file,
                                    &mut running_program,
                                    &mut console_output,
                                    &mut is_running,
                                    &mut status_msg,
                                    &mut status_color,
                                );
                            }
                        }
                        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if is_running {
                                status_msg = String::from("Program is already running");
                                status_color = Color::Yellow;
                            } else {
                                start_program(
                                    &textarea,
                                    &current_file,
                                    &mut running_program,
                                    &mut console_output,
                                    &mut is_running,
                                    &mut status_msg,
                                    &mut status_color,
                                );
                            }
                        }
                        KeyCode::F(6) => {
                            if is_running {
                                stop_running_program(&mut running_program);
                                is_running = false;
                                status_msg = String::from("Program window closed");
                                status_color = Color::Yellow;
                            }
                        }
                        KeyCode::F(2) => match save_editor(&current_file, &textarea) {
                            Ok(()) => {
                                dirty = false;
                                status_msg = format!("Saved {}", display_path(&current_file));
                                status_color = Color::Green;
                            }
                            Err(err) => {
                                status_msg = format!("Save failed: {}", err);
                                status_color = Color::Red;
                            }
                        },
                        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            match save_editor(&current_file, &textarea) {
                                Ok(()) => {
                                    dirty = false;
                                    status_msg = format!("Saved {}", display_path(&current_file));
                                    status_color = Color::Green;
                                }
                                Err(err) => {
                                    status_msg = format!("Save failed: {}", err);
                                    status_color = Color::Red;
                                }
                            }
                        }
                        KeyCode::F(12) => {
                            app_state =
                                AppState::PromptingSave(current_file.to_string_lossy().to_string());
                        }
                        KeyCode::F(3) => {
                            app_state =
                                AppState::PromptingLoad(current_file.to_string_lossy().to_string());
                        }
                        KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app_state =
                                AppState::PromptingLoad(current_file.to_string_lossy().to_string());
                        }
                        KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if dirty || is_running {
                                app_state = AppState::ConfirmNew(String::from(
                                    "Discard the current program and start over? y/n",
                                ));
                            } else {
                                current_file = PathBuf::from(DEFAULT_FILE_NAME);
                                textarea = textarea_from_source("");
                                editor_view = EditorView::default();
                                dirty = false;
                                console_output.clear();
                                console_output.push(String::from("New program."));
                                status_msg = String::from("New program");
                                status_color = Color::Green;
                            }
                        }
                        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            console_output.clear();
                            console_output.push(String::from("Console cleared."));
                            status_msg = String::from("Console cleared");
                            status_color = Color::Green;
                        }
                        KeyCode::Char('c') | KeyCode::Char('C')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            textarea.copy();
                            if let Some(ref mut cb) = clipboard {
                                let _ = cb.set_text(textarea.yank_text().to_string());
                                status_msg = String::from("Copied to clipboard");
                                status_color = Color::Green;
                            }
                        }
                        KeyCode::Char('x') | KeyCode::Char('X')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            textarea.cut();
                            if let Some(ref mut cb) = clipboard {
                                let _ = cb.set_text(textarea.yank_text().to_string());
                                dirty = true;
                                status_msg = String::from("Cut to clipboard");
                                status_color = Color::Green;
                            }
                        }
                        KeyCode::Char('v') | KeyCode::Char('V')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            if let Some(ref mut cb) = clipboard {
                                if let Ok(text) = cb.get_text() {
                                    textarea.set_yank_text(text);
                                    textarea.paste();
                                    dirty = true;
                                    status_msg = String::from("Pasted from clipboard");
                                    status_color = Color::Green;
                                }
                            }
                        }
                        _ => {
                            let changed = is_edit_key(&key);
                            textarea.input(key);
                            if changed {
                                dirty = true;
                                status_msg = String::from("Editing");
                                status_color = Color::Green;
                            }
                        }
                    },
                }
            }
        }
    }

    Ok(())
}

fn textarea_from_source(source: &str) -> TextArea<'static> {
    let lines: Vec<String> = if source.is_empty() {
        vec![String::new()]
    } else {
        source.lines().map(|line| line.to_string()).collect()
    };
    let mut textarea = TextArea::from(lines);
    style_textarea(&mut textarea);
    textarea
}

fn style_textarea(textarea: &mut TextArea<'static>) {
    textarea.set_cursor_line_style(Style::default().bg(Color::Rgb(20, 28, 34)));
    textarea.set_cursor_style(Style::default().fg(Color::Black).bg(Color::White));
}

fn configure_editor(
    textarea: &mut TextArea<'static>,
    current_file: &Path,
    dirty: bool,
    is_running: bool,
) {
    let marker = if dirty { " *" } else { "" };
    let run_marker = if is_running { " [running]" } else { "" };
    textarea.set_block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(
                " Editor - {}{}{} ",
                display_path(current_file),
                marker,
                run_marker
            ))
            .style(Style::default().fg(Color::Cyan)),
    );
}

fn render_basic_editor(
    f: &mut Frame<'_>,
    area: Rect,
    textarea: &TextArea<'static>,
    meta: EditorRenderMeta<'_>,
    view: &mut EditorView,
) {
    let marker = if meta.dirty { " *" } else { "" };
    let run_marker = if meta.is_running { " [running]" } else { "" };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " Editor - {}{}{} ",
            display_path(meta.current_file),
            marker,
            run_marker
        ))
        .style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines = textarea.lines();
    let (cursor_row, cursor_col) = textarea.cursor();
    let height = inner.height as usize;
    if height == 0 {
        return;
    }
    let gutter_width = basic_gutter_width(lines.len());
    let text_width = (inner.width as usize).saturating_sub(gutter_width);

    if cursor_row < view.top_row {
        view.top_row = cursor_row;
    } else if cursor_row >= view.top_row.saturating_add(height) {
        view.top_row = cursor_row + 1 - height;
    }
    if text_width > 0 {
        if cursor_col < view.left_col {
            view.left_col = cursor_col;
        } else if cursor_col >= view.left_col.saturating_add(text_width) {
            view.left_col = cursor_col + 1 - text_width;
        }
    }

    let bottom = lines.len().min(view.top_row + height);
    let mut rendered = Vec::with_capacity(height);
    for (row, line) in lines.iter().enumerate().take(bottom).skip(view.top_row) {
        let is_cursor_row = row == cursor_row;
        let line_style = if is_cursor_row {
            Style::default().bg(Color::Rgb(20, 28, 34))
        } else {
            Style::default()
        };
        let gutter_style = if is_cursor_row {
            Style::default()
                .fg(Color::DarkGray)
                .bg(Color::Rgb(20, 28, 34))
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let mut spans = vec![Span::styled(
            format!(
                "{:>width$} ",
                display_basic_line_number(row, line),
                width = gutter_width.saturating_sub(1)
            ),
            gutter_style,
        )];
        spans.extend(highlight_basic_code(
            line,
            view.left_col,
            text_width,
            line_style,
        ));
        rendered.push(Line::from(spans));
    }
    while rendered.len() < height {
        rendered.push(Line::from(Span::raw("")));
    }

    f.render_widget(Paragraph::new(rendered), inner);

    if meta.show_cursor
        && cursor_row >= view.top_row
        && cursor_row < view.top_row + height
        && cursor_col >= view.left_col
        && text_width > 0
    {
        let x = inner.x + gutter_width as u16 + (cursor_col - view.left_col) as u16;
        let y = inner.y + (cursor_row - view.top_row) as u16;
        if x < inner.x.saturating_add(inner.width) && y < inner.y.saturating_add(inner.height) {
            f.set_cursor_position((x, y));
        }
    }
}

fn basic_gutter_width(line_count: usize) -> usize {
    let max_auto = auto_basic_line_number(line_count.saturating_sub(1));
    max_auto.to_string().len().max(4) + 1
}

fn auto_basic_line_number(row: usize) -> u32 {
    ((row as u32).saturating_add(1)).saturating_mul(10)
}

fn display_basic_line_number(row: usize, line: &str) -> String {
    explicit_basic_line_number(line).unwrap_or_else(|| auto_basic_line_number(row).to_string())
}

fn explicit_basic_line_number(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let len = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if len == 0 {
        return None;
    }
    Some(trimmed.chars().take(len).collect())
}

fn source_with_auto_line_numbers(lines: &[String]) -> String {
    let mut source = String::new();
    for (row, line) in lines.iter().enumerate() {
        if line.trim().is_empty() || explicit_basic_line_number(line).is_some() {
            source.push_str(line);
        } else {
            source.push_str(&format!("{} {}", auto_basic_line_number(row), line));
        }
        source.push('\n');
    }
    source
}

fn highlight_basic_code(
    line: &str,
    left_col: usize,
    text_width: usize,
    line_style: Style,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    if text_width == 0 {
        return spans;
    }

    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut col = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            push_visible_span(
                &mut spans,
                chars[i..].iter().collect::<String>(),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
                &mut col,
                left_col,
                text_width,
            );
            break;
        }
        if c == '"' {
            let start = i;
            i += 1;
            while i < chars.len() {
                let ch = chars[i];
                i += 1;
                if ch == '"' {
                    break;
                }
            }
            push_visible_span(
                &mut spans,
                chars[start..i].iter().collect::<String>(),
                Style::default().fg(Color::Green),
                &mut col,
                left_col,
                text_width,
            );
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit())
        {
            let start = i;
            let mut saw_dot = c == '.';
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_digit() || (chars[i] == '.' && !saw_dot)) {
                if chars[i] == '.' {
                    saw_dot = true;
                }
                i += 1;
            }
            push_visible_span(
                &mut spans,
                chars[start..i].iter().collect::<String>(),
                Style::default().fg(Color::Magenta),
                &mut col,
                left_col,
                text_width,
            );
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            if i < chars.len() && matches!(chars[i], '$' | '%' | '!' | '#' | '&') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let upper = word.to_uppercase();
            if upper == "REM" {
                push_visible_span(
                    &mut spans,
                    chars[start..].iter().collect::<String>(),
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                    &mut col,
                    left_col,
                    text_width,
                );
                break;
            }
            let style = if is_basic_keyword(&upper) {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else if is_basic_builtin(&upper) {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };
            push_visible_span(&mut spans, word, style, &mut col, left_col, text_width);
            continue;
        }

        let style = if c.is_ascii_whitespace() {
            line_style
        } else {
            Style::default().fg(Color::Blue)
        };
        push_visible_span(
            &mut spans,
            c.to_string(),
            style,
            &mut col,
            left_col,
            text_width,
        );
        i += 1;
    }

    spans
}

fn push_visible_span(
    spans: &mut Vec<Span<'static>>,
    text: String,
    style: Style,
    col: &mut usize,
    left_col: usize,
    text_width: usize,
) {
    let current_len = *col;
    let end_col = current_len + text.chars().count();
    *col = end_col;
    if end_col <= left_col || current_len >= left_col + text_width {
        return;
    }
    let skip = left_col.saturating_sub(current_len);
    let take = (left_col + text_width).saturating_sub(current_len + skip);
    let visible: String = text.chars().skip(skip).take(take).collect();
    if !visible.is_empty() {
        spans.push(Span::styled(visible, style));
    }
}

fn is_basic_keyword(word: &str) -> bool {
    matches!(
        word,
        "APPEND"
            | "AS"
            | "BASE"
            | "BYREF"
            | "BYVAL"
            | "CALL"
            | "CASE"
            | "CLEAR"
            | "CLOSE"
            | "CONST"
            | "DATA"
            | "DECLARE"
            | "DEFDBL"
            | "DEFINT"
            | "DEFLNG"
            | "DEFSNG"
            | "DEFSTR"
            | "DIM"
            | "DO"
            | "DOUBLE"
            | "ELSE"
            | "ELSEIF"
            | "END"
            | "ENDIF"
            | "ERASE"
            | "EXIT"
            | "FOR"
            | "FUNCTION"
            | "GET"
            | "GOSUB"
            | "GOTO"
            | "IF"
            | "INPUT"
            | "INTEGER"
            | "IS"
            | "LET"
            | "LINE"
            | "LONG"
            | "LOOP"
            | "NEXT"
            | "ON"
            | "OPEN"
            | "OPTION"
            | "OUTPUT"
            | "PRESERVE"
            | "PRINT"
            | "PUT"
            | "RANDOM"
            | "READ"
            | "REDIM"
            | "RESTORE"
            | "RETURN"
            | "SELECT"
            | "SHARED"
            | "SINGLE"
            | "STATIC"
            | "STEP"
            | "STRING"
            | "SUB"
            | "THEN"
            | "TO"
            | "TYPE"
            | "UNTIL"
            | "WEND"
            | "WHILE"
    )
}

fn is_basic_builtin(word: &str) -> bool {
    matches!(
        word,
        "ABS"
            | "ASC"
            | "ATN"
            | "CHR$"
            | "CDBL"
            | "CINT"
            | "CLNG"
            | "COS"
            | "CSNG"
            | "CSTR"
            | "CSTR$"
            | "EXP"
            | "FIX"
            | "HEX$"
            | "INSTR"
            | "INT"
            | "LCASE$"
            | "LEFT$"
            | "LEN"
            | "LOCATE"
            | "LOG"
            | "LTRIM"
            | "LTRIM$"
            | "MID$"
            | "OCT$"
            | "RIGHT$"
            | "RND"
            | "RTRIM"
            | "RTRIM$"
            | "SGN"
            | "SIN"
            | "SPACE$"
            | "SPC"
            | "SQR"
            | "STR$"
            | "STRING$"
            | "TAB"
            | "TAN"
            | "TIMER"
            | "TRIM$"
            | "TRIM"
            | "UCASE$"
            | "VAL"
    )
}

fn header_line(path: &Path, dirty: bool, is_running: bool) -> Line<'static> {
    let state = if is_running { "RUNNING" } else { "READY" };
    let saved = if dirty { "modified" } else { "saved" };
    Line::from(vec![
        Span::styled(
            format!(" {} ", display_path(path)),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{} ", saved),
            Style::default().fg(if dirty { Color::Yellow } else { Color::Green }),
        ),
        Span::raw(" "),
        Span::styled(state, Style::default().fg(Color::Cyan)),
    ])
}

fn footer_text(app_state: &AppState, status_msg: &str) -> String {
    match app_state {
        AppState::PromptingSave(input) => format!("Save as: {}_", input),
        AppState::PromptingLoad(input) => format!("Open file: {}_", input),
        AppState::RunningInput { prompt, input } => format!("{}{}_", prompt, input),
        AppState::ConfirmQuit(message) => message.clone(),
        AppState::ConfirmNew(message) => message.clone(),
        AppState::Editing => format!(
            "{} | F5 Run  F6 Stop  F2 Save  F3 Open  Ctrl+L Clear  Esc Quit",
            status_msg
        ),
    }
}

fn footer_color(app_state: &AppState, status_color: Color) -> Color {
    match app_state {
        AppState::PromptingSave(_) | AppState::PromptingLoad(_) => Color::Yellow,
        AppState::RunningInput { .. } => Color::Magenta,
        AppState::ConfirmQuit(_) => Color::Yellow,
        AppState::ConfirmNew(_) => Color::Yellow,
        AppState::Editing => status_color,
    }
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn prompt_target_path(input: &str, current_file: &Path) -> PathBuf {
    if input.trim().is_empty() {
        current_file.to_path_buf()
    } else {
        PathBuf::from(input.trim())
    }
}

fn save_editor(path: &Path, textarea: &TextArea<'static>) -> Result<(), String> {
    fs::write(path, textarea.lines().join("\n")).map_err(|err| err.to_string())
}

fn push_console_line(console_output: &mut Vec<String>, line: String) {
    if console_output.is_empty() {
        console_output.push(String::new());
    }
    console_output.push(line);
    trim_console(console_output);
}

fn trim_console(console_output: &mut Vec<String>) {
    while console_output.len() > 250 {
        console_output.remove(0);
    }
}

fn quit_reason(dirty: bool, is_running: bool) -> String {
    match (dirty, is_running) {
        (true, true) => String::from("Program is running and has unsaved changes. Quit? y/n"),
        (true, false) => String::from("Unsaved changes will be lost. Quit? y/n"),
        (false, true) => String::from("Program is running. Stop and quit? y/n"),
        (false, false) => String::from("Quit? y/n"),
    }
}

fn is_edit_key(key: &KeyEvent) -> bool {
    match key.code {
        KeyCode::Char(_) => !key.modifiers.contains(KeyModifiers::CONTROL),
        KeyCode::Backspace | KeyCode::Delete | KeyCode::Enter | KeyCode::Tab => true,
        _ => false,
    }
}

fn request_stop(cancel_flag: &Arc<AtomicBool>, input_tx_opt: &Option<mpsc::Sender<String>>) {
    cancel_flag.store(true, Ordering::Relaxed);
    if let Some(tx) = input_tx_opt {
        let _ = tx.send(String::new());
    }
}

fn start_program(
    textarea: &TextArea<'static>,
    current_file: &Path,
    running_program: &mut Option<RunningProgram>,
    console_output: &mut Vec<String>,
    is_running: &mut bool,
    status_msg: &mut String,
    status_color: &mut Color,
) {
    let source = source_with_auto_line_numbers(textarea.lines());
    let tokens = match lexer::tokenize(&source) {
        Ok(tokens) => tokens,
        Err(err) => {
            *status_msg = format!("Lexer error: {}", err);
            *status_color = Color::Red;
            return;
        }
    };
    match parser::parse(tokens) {
        Ok(_) => {}
        Err(err) => {
            *status_msg = format!("Syntax error: {}", err);
            *status_color = Color::Red;
            return;
        }
    };

    let source_path = match write_run_source(&source) {
        Ok(path) => path,
        Err(err) => {
            *status_msg = format!("Run setup failed: {}", err);
            *status_color = Color::Red;
            return;
        }
    };
    let run_cwd = current_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let child = match spawn_program_window(&source_path, &run_cwd) {
        Ok(child) => child,
        Err(err) => {
            let _ = fs::remove_file(&source_path);
            *status_msg = format!("Run launch failed: {}", err);
            *status_color = Color::Red;
            return;
        }
    };

    console_output.clear();
    console_output.push(String::from("--- Program opened in a separate window ---"));
    console_output.push(String::from(
        "Close that window or press F6 here to stop it.",
    ));
    *is_running = true;
    *running_program = Some(RunningProgram { child, source_path });
    *status_msg = String::from("Running in separate window");
    *status_color = Color::Cyan;
}

fn write_run_source(source: &str) -> Result<PathBuf, String> {
    let mut path = env::temp_dir();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    path.push(format!(
        "qbasic_studio_run_{}_{}.bas",
        std::process::id(),
        stamp
    ));
    fs::write(&path, source).map_err(|err| err.to_string())?;
    Ok(path)
}

fn spawn_program_window(source_path: &Path, run_cwd: &Path) -> Result<Child, String> {
    let exe = env::current_exe().map_err(|err| err.to_string())?;
    let mut command = Command::new(exe);
    command
        .arg("run-window")
        .arg(source_path)
        .current_dir(run_cwd);
    configure_new_console(&mut command);
    command.spawn().map_err(|err| err.to_string())
}

#[cfg(windows)]
fn configure_new_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    command.creation_flags(CREATE_NEW_CONSOLE);
}

#[cfg(not(windows))]
fn configure_new_console(_command: &mut Command) {}

fn poll_running_program(
    running_program: &mut Option<RunningProgram>,
    is_running: &mut bool,
    status_msg: &mut String,
    status_color: &mut Color,
    console_output: &mut Vec<String>,
) {
    let Some(program) = running_program.as_mut() else {
        return;
    };
    match program.child.try_wait() {
        Ok(Some(status)) => {
            let path = program.source_path.clone();
            *running_program = None;
            *is_running = false;
            let _ = fs::remove_file(path);
            if status.success() {
                *status_msg = String::from("Program finished");
                *status_color = Color::Green;
                push_console_line(console_output, String::from("--- Program finished ---"));
            } else {
                *status_msg = format!("Program exited with {}", status);
                *status_color = Color::Red;
                push_console_line(
                    console_output,
                    format!("--- Program exited with {} ---", status),
                );
            }
        }
        Ok(None) => {}
        Err(err) => {
            let path = program.source_path.clone();
            *running_program = None;
            *is_running = false;
            let _ = fs::remove_file(path);
            *status_msg = format!("Program status failed: {}", err);
            *status_color = Color::Red;
        }
    }
}

fn stop_running_program(running_program: &mut Option<RunningProgram>) {
    if let Some(mut program) = running_program.take() {
        let _ = program.child.kill();
        let _ = program.child.wait();
        let _ = fs::remove_file(program.source_path);
    }
}

fn update_graphics_window(
    graphics_state: &Arc<Mutex<graphics::SharedGraphics>>,
    mf_window: &mut Option<minifb::Window>,
    status_msg: &mut String,
    status_color: &mut Color,
    console_output: &mut Vec<String>,
) {
    let mut g = match graphics_state.lock() {
        Ok(guard) => guard,
        Err(_) => {
            *status_msg = String::from("Graphics state unavailable");
            *status_color = Color::Red;
            return;
        }
    };

    if g.active {
        let needs_new_window = match mf_window {
            Some(window) => window.get_size() != (g.width, g.height),
            None => true,
        };

        if needs_new_window {
            match minifb::Window::new(
                &g.title,
                g.width,
                g.height,
                minifb::WindowOptions::default(),
            ) {
                Ok(mut window) => {
                    window.limit_update_rate(Some(std::time::Duration::from_millis(16)));
                    *mf_window = Some(window);
                }
                Err(err) => {
                    *status_msg = format!("Graphics error: {}", err);
                    *status_color = Color::Red;
                    push_console_line(console_output, format!("[GRAPHICS ERROR] {}", err));
                    g.active = false;
                    *mf_window = None;
                }
            }
        }

        if g.updated {
            let update_result = if let Some(window) = mf_window.as_mut() {
                window.update_with_buffer(&g.buffer, g.width, g.height)
            } else {
                Ok(())
            };
            if let Err(err) = update_result {
                *status_msg = format!("Graphics error: {}", err);
                *status_color = Color::Red;
                push_console_line(console_output, format!("[GRAPHICS ERROR] {}", err));
                g.active = false;
                *mf_window = None;
            }
            g.updated = false;
        }
    }

    if let Some(window) = mf_window.as_mut() {
        if !window.is_open() || window.is_key_down(minifb::Key::Escape) {
            g.active = false;
            *mf_window = None;
        } else {
            window.update();
        }
    }
}

fn check_file(path: &Path) -> Result<(), String> {
    let src = fs::read_to_string(path)
        .map_err(|err| format!("Error reading {}: {}", path.display(), err))?;
    let tokens = lexer::tokenize(&src).map_err(|err| format!("Lexer error: {}", err))?;
    parser::parse(tokens).map_err(|err| format!("Parser error: {}", err))?;
    Ok(())
}

fn run_file(path: &Path) -> Result<(), String> {
    let src = fs::read_to_string(path)
        .map_err(|err| format!("Error reading {}: {}", path.display(), err))?;
    let tokens = lexer::tokenize(&src).map_err(|err| format!("Lexer error: {}", err))?;
    let ast = parser::parse(tokens).map_err(|err| format!("Parser error: {}", err))?;

    let (console_tx, console_rx) = mpsc::channel();
    let (input_tx, input_rx) = mpsc::channel();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let graphics = Arc::new(Mutex::new(graphics::SharedGraphics::new()));

    let output_handle = thread::spawn(move || {
        while let Ok(msg) = console_rx.recv() {
            match msg {
                ConsoleMsg::Print { text, newline } => {
                    use std::io::Write;
                    print!("{}", text);
                    if newline {
                        println!();
                    }
                    let _ = std::io::stdout().flush();
                }
                ConsoleMsg::InputPrompt(prompt) => {
                    use std::io::Write;
                    print!("{}", prompt);
                    let _ = std::io::stdout().flush();
                    let mut buf = String::new();
                    let value = if std::io::stdin().read_line(&mut buf).is_ok() {
                        buf.trim_end().to_string()
                    } else {
                        String::new()
                    };
                    let _ = input_tx.send(value);
                }
                ConsoleMsg::Clear => {
                    print!("\x1B[2J\x1B[1;1H");
                }
                ConsoleMsg::End(err) => {
                    if let Some(err) = err {
                        eprintln!("Runtime error: {}", err);
                    }
                    break;
                }
            }
        }
    });

    let (result_tx, result_rx) = mpsc::channel();
    let run_tx = console_tx.clone();
    let run_graphics = Arc::clone(&graphics);
    thread::spawn(move || {
        let result = interpreter::run(ast, run_tx.clone(), input_rx, cancel_flag, run_graphics);
        let _ = result_tx.send(result);
    });

    let mut mf_window: Option<minifb::Window> = None;
    let result = loop {
        match result_rx.try_recv() {
            Ok(result) => break result,
            Err(mpsc::TryRecvError::Empty) => {
                let mut status_msg = String::new();
                let mut status_color = Color::Green;
                let mut ignored_console = Vec::new();
                update_graphics_window(
                    &graphics,
                    &mut mf_window,
                    &mut status_msg,
                    &mut status_color,
                    &mut ignored_console,
                );
                thread::sleep(std::time::Duration::from_millis(16));
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                break Err("Program runner disconnected".to_string());
            }
        }
    };

    let _ = console_tx.send(ConsoleMsg::End(None));
    let _ = output_handle.join();
    result
}

fn pause_before_close() {
    println!();
    println!("Program window can now be closed. Press Enter to close it.");
    let mut line = String::new();
    let _ = io::stdin().read_line(&mut line);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_source_gets_virtual_basic_line_numbers() {
        let lines = vec![
            "PRINT \"HELLO\"".to_string(),
            "GOTO 10".to_string(),
            "100 PRINT \"EXPLICIT\"".to_string(),
        ];
        assert_eq!(
            source_with_auto_line_numbers(&lines),
            "10 PRINT \"HELLO\"\n20 GOTO 10\n100 PRINT \"EXPLICIT\"\n"
        );
    }
}
