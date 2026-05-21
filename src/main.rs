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
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
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
    textarea.set_line_number_style(Style::default().fg(Color::DarkGray));

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
    let (console_tx, console_rx) = mpsc::channel();
    let graphics_state = Arc::new(Mutex::new(graphics::SharedGraphics::new()));
    let mut mf_window: Option<minifb::Window> = None;

    let mut app_state = AppState::Editing;
    let mut is_running = false;
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let mut input_tx_opt: Option<mpsc::Sender<String>> = None;
    let mut run_handle: Option<thread::JoinHandle<()>> = None;

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

            f.render_widget(&textarea, chunks[1]);

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

        while let Ok(msg) = console_rx.try_recv() {
            match msg {
                ConsoleMsg::Print { text, newline } => {
                    if console_output.is_empty() {
                        console_output.push(String::new());
                    }
                    if let Some(last) = console_output.last_mut() {
                        last.push_str(&text);
                    }
                    if newline {
                        console_output.push(String::new());
                    }
                    trim_console(&mut console_output);
                }
                ConsoleMsg::InputPrompt(p) => {
                    if console_output.is_empty() {
                        console_output.push(String::new());
                    }
                    app_state = AppState::RunningInput {
                        prompt: p,
                        input: String::new(),
                    };
                }
                ConsoleMsg::Clear => {
                    console_output.clear();
                    console_output.push(String::new());
                }
                ConsoleMsg::End(err_opt) => {
                    if let Some(err) = err_opt {
                        push_console_line(&mut console_output, format!("[RUNTIME ERROR] {}", err));
                        status_msg = String::from("Program ended with an error");
                        status_color = Color::Red;
                    } else {
                        status_msg = String::from("Program finished");
                        status_color = Color::Green;
                    }
                    push_console_line(&mut console_output, String::from("--- Program Ended ---"));
                    is_running = false;
                    input_tx_opt = None;
                    if let Some(handle) = run_handle.take() {
                        let _ = handle.join();
                    }
                    app_state = AppState::Editing;
                }
            }
        }

        update_graphics_window(
            &graphics_state,
            &mut mf_window,
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
                            if let Some(tx) = &input_tx_opt {
                                let _ = tx.send(val);
                            }
                            app_state = AppState::Editing;
                        }
                        KeyCode::Backspace => {
                            input.pop();
                        }
                        KeyCode::Char(c) => {
                            input.push(c);
                        }
                        KeyCode::Esc => {
                            request_stop(&cancel_flag, &input_tx_opt);
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
                            request_stop(&cancel_flag, &input_tx_opt);
                            if let Some(handle) = run_handle.take() {
                                let _ = handle.join();
                            }
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
                            request_stop(&cancel_flag, &input_tx_opt);
                            if let Some(handle) = run_handle.take() {
                                let _ = handle.join();
                            }
                            is_running = false;
                            input_tx_opt = None;
                            current_file = PathBuf::from(DEFAULT_FILE_NAME);
                            textarea = textarea_from_source("");
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
                                    &console_tx,
                                    &graphics_state,
                                    &cancel_flag,
                                    &mut input_tx_opt,
                                    &mut run_handle,
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
                                    &console_tx,
                                    &graphics_state,
                                    &cancel_flag,
                                    &mut input_tx_opt,
                                    &mut run_handle,
                                    &mut console_output,
                                    &mut is_running,
                                    &mut status_msg,
                                    &mut status_color,
                                );
                            }
                        }
                        KeyCode::F(6) => {
                            if is_running {
                                request_stop(&cancel_flag, &input_tx_opt);
                                status_msg = String::from("Stop requested");
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
    textarea.set_line_number_style(Style::default().fg(Color::DarkGray));
    textarea
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

#[allow(clippy::too_many_arguments)]
fn start_program(
    textarea: &TextArea<'static>,
    console_tx: &mpsc::Sender<ConsoleMsg>,
    graphics_state: &Arc<Mutex<graphics::SharedGraphics>>,
    cancel_flag: &Arc<AtomicBool>,
    input_tx_opt: &mut Option<mpsc::Sender<String>>,
    run_handle: &mut Option<thread::JoinHandle<()>>,
    console_output: &mut Vec<String>,
    is_running: &mut bool,
    status_msg: &mut String,
    status_color: &mut Color,
) {
    let source = textarea.lines().join("\n");
    let tokens = match lexer::tokenize(&source) {
        Ok(tokens) => tokens,
        Err(err) => {
            *status_msg = format!("Lexer error: {}", err);
            *status_color = Color::Red;
            return;
        }
    };
    let ast = match parser::parse(tokens) {
        Ok(program) => program,
        Err(err) => {
            *status_msg = format!("Syntax error: {}", err);
            *status_color = Color::Red;
            return;
        }
    };

    let tx = console_tx.clone();
    let (in_tx, in_rx) = mpsc::channel();
    *input_tx_opt = Some(in_tx);
    let graphics = Arc::clone(graphics_state);

    console_output.clear();
    console_output.push(String::from("--- Running Program ---"));
    console_output.push(String::new());
    *is_running = true;
    *status_msg = String::from("Running");
    *status_color = Color::Cyan;
    cancel_flag.store(false, Ordering::Relaxed);
    let c_flag = Arc::clone(cancel_flag);

    *run_handle = Some(thread::spawn(move || {
        let res = interpreter::run(ast, tx.clone(), in_rx, c_flag, graphics);
        let _ = tx.send(ConsoleMsg::End(res.err()));
    }));
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

    let result = interpreter::run(ast, console_tx.clone(), input_rx, cancel_flag, graphics);
    let _ = console_tx.send(ConsoleMsg::End(None));
    let _ = output_handle.join();
    result
}
