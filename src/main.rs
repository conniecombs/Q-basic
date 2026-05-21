#![allow(dead_code)]

mod graphics;
mod interpreter;
mod lexer;
mod parser;

use arboard::Clipboard;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use tui_textarea::TextArea;

use crate::interpreter::ConsoleMsg;

enum AppState {
    Editing,
    PromptingSave(String),
    PromptingLoad(String),
    RunningInput { prompt: String, input: String },
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() == 2 {
        run_file(&args[1]);
        return Ok(());
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut clipboard = Clipboard::new().ok();

    let mut textarea = TextArea::default();
    let mut current_file = String::from("main.bas");

    // Enable line numbers
    textarea.set_line_number_style(Style::default().fg(Color::DarkGray));
    textarea.set_block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" QBASIC IDE - {} ", current_file))
            .style(Style::default().fg(Color::Cyan)),
    );
    textarea.insert_str("PRINT \"Welcome to the Feature-Complete QBasic IDE!\"\nINPUT \"What is your name? \", N$\nPRINT \"Hello, \"; N$\n");

    let mut status_msg = String::from(" F5: Run | F2: Save | F3: Load | Esc: Quit ");
    let mut status_color = Color::Green;

    let mut console_output: Vec<String> = vec![String::new()];
    let (console_tx, console_rx) = mpsc::channel();
    let graphics_state = Arc::new(Mutex::new(graphics::SharedGraphics::new()));
    let mut mf_window: Option<minifb::Window> = None;

    let mut app_state = AppState::Editing;
    let mut is_running = false;
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let mut input_tx_opt: Option<mpsc::Sender<String>> = None;
    let mut run_handle: Option<thread::JoinHandle<()>> = None;

    loop {
        // TUI Render
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(70),
                    Constraint::Percentage(30),
                    Constraint::Length(3),
                ])
                .split(f.area());

            f.render_widget(&textarea, chunks[0]);

            let console_lines: Vec<String> = console_output
                .iter()
                .rev()
                .take(chunks[1].height.saturating_sub(2) as usize)
                .rev()
                .cloned()
                .collect();
            let console_text = console_lines.join("\n");

            let console_block = Paragraph::new(console_text)
                .style(Style::default().fg(Color::White))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Console Output "),
                );
            f.render_widget(console_block, chunks[1]);

            let (display_status, draw_color) = match &app_state {
                AppState::PromptingSave(input) => {
                    (format!(" Save File As: {}_ ", input), Color::Yellow)
                }
                AppState::PromptingLoad(input) => {
                    (format!(" Load File: {}_ ", input), Color::Yellow)
                }
                AppState::RunningInput { prompt, input } => {
                    (format!(" {} {}_ ", prompt, input), Color::Magenta)
                }
                AppState::Editing => {
                    if is_running {
                        (
                            " Running... F6: Stop | F2: Save | F3: Load | Esc: Quit ".to_string(),
                            Color::Cyan,
                        )
                    } else {
                        (status_msg.clone(), status_color)
                    }
                }
            };

            let status_block = Paragraph::new(display_status)
                .style(Style::default().fg(draw_color).bg(Color::Black))
                .block(Block::default().borders(Borders::ALL).title(" Status "));
            f.render_widget(status_block, chunks[2]);
        })?;

        // Process asynchronous outputs from the background interpreter thread
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
                    if console_output.len() > 150 {
                        console_output.remove(0);
                    }
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
                        if console_output.is_empty() {
                            console_output.push(String::new());
                        }
                        console_output
                            .last_mut()
                            .unwrap()
                            .push_str(&format!("[RUNTIME ERROR]: {}", err));
                        console_output.push(String::new());
                    }
                    if console_output.is_empty() {
                        console_output.push(String::new());
                    }
                    console_output
                        .last_mut()
                        .unwrap()
                        .push_str("--- Program Ended ---");
                    console_output.push(String::new());
                    is_running = false;
                    if let Some(handle) = run_handle.take() {
                        let _ = handle.join();
                    }
                    app_state = AppState::Editing;
                }
            }
        }

        // Handle minifb window initialization and rendering synchronously to prevent OS deadlock
        {
            let mut g = graphics_state.lock().unwrap();
            if g.active {
                let needs_new_window = match &mf_window {
                    Some(w) => w.get_size() != (g.width, g.height),
                    None => true,
                };

                if needs_new_window {
                    match minifb::Window::new(
                        &g.title,
                        g.width,
                        g.height,
                        minifb::WindowOptions::default(),
                    ) {
                        Ok(mut w) => {
                            w.limit_update_rate(Some(std::time::Duration::from_millis(16))); // Force ~60 FPS
                            mf_window = Some(w);
                        }
                        Err(e) => {
                            status_msg = format!(" Graphics Error: {} ", e);
                            status_color = Color::Red;
                            console_output.push(format!("[GRAPHICS ERROR]: {}", e));
                            g.active = false;
                            mf_window = None;
                        }
                    }
                }

                if g.updated {
                    let update_result = if let Some(w) = mf_window.as_mut() {
                        w.update_with_buffer(&g.buffer, g.width, g.height)
                    } else {
                        Ok(())
                    };
                    if let Err(e) = update_result {
                        status_msg = format!(" Graphics Error: {} ", e);
                        status_color = Color::Red;
                        console_output.push(format!("[GRAPHICS ERROR]: {}", e));
                        g.active = false;
                        mf_window = None;
                    }
                    g.updated = false;
                }
            }

            if let Some(w) = mf_window.as_mut() {
                if !w.is_open() || w.is_key_down(minifb::Key::Escape) {
                    g.active = false;
                    mf_window = None;
                } else {
                    w.update();
                }
            }
        }

        // Standard Keyboard Input check
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
                            cancel_flag.store(true, Ordering::Relaxed);
                            if let Some(tx) = &input_tx_opt {
                                let _ = tx.send(String::new());
                            }
                            app_state = AppState::Editing;
                        }
                        _ => {}
                    },
                    AppState::PromptingSave(ref mut input) => match key.code {
                        KeyCode::Enter => {
                            let target_file = if input.trim().is_empty() {
                                current_file.clone()
                            } else {
                                input.clone()
                            };
                            let source = textarea.lines().join("\n");
                            match fs::write(&target_file, source) {
                                Ok(_) => {
                                    current_file = target_file;
                                    status_msg = format!(" Saved to {} ", current_file);
                                    status_color = Color::Green;
                                }
                                Err(e) => {
                                    status_msg = format!(" Save Error: {} ", e);
                                    status_color = Color::Red;
                                }
                            }
                            textarea.set_block(
                                Block::default()
                                    .borders(Borders::ALL)
                                    .title(format!(" QBASIC IDE - {} ", current_file))
                                    .style(Style::default().fg(Color::Cyan)),
                            );
                            app_state = AppState::Editing;
                        }
                        KeyCode::Esc => {
                            app_state = AppState::Editing;
                        }
                        KeyCode::Backspace => {
                            input.pop();
                        }
                        KeyCode::Char(c) => {
                            input.push(c);
                        }
                        _ => {}
                    },
                    AppState::PromptingLoad(ref mut input) => {
                        match key.code {
                            KeyCode::Enter => {
                                let target_file = if input.trim().is_empty() {
                                    current_file.clone()
                                } else {
                                    input.clone()
                                };
                                match fs::read_to_string(&target_file) {
                                    Ok(content) => {
                                        textarea =
                                            TextArea::from(content.lines().map(|s| s.to_string()));
                                        textarea.set_line_number_style(
                                            Style::default().fg(Color::DarkGray),
                                        ); // Keep Line numbers initialized
                                        current_file = target_file;
                                        textarea.set_block(
                                            Block::default()
                                                .borders(Borders::ALL)
                                                .title(format!(" QBASIC IDE - {} ", current_file))
                                                .style(Style::default().fg(Color::Cyan)),
                                        );
                                        status_msg = format!(" Loaded {} ", current_file);
                                        status_color = Color::Green;
                                    }
                                    Err(e) => {
                                        status_msg = format!(" Load Error: {} ", e);
                                        status_color = Color::Red;
                                    }
                                }
                                app_state = AppState::Editing;
                            }
                            KeyCode::Esc => {
                                app_state = AppState::Editing;
                            }
                            KeyCode::Backspace => {
                                input.pop();
                            }
                            KeyCode::Char(c) => {
                                input.push(c);
                            }
                            _ => {}
                        }
                    }
                    AppState::Editing => {
                        match key.code {
                            KeyCode::Esc => {
                                if is_running {
                                    cancel_flag.store(true, Ordering::Relaxed);
                                    if let Some(tx) = &input_tx_opt {
                                        let _ = tx.send(String::new());
                                    }
                                    if let Some(handle) = run_handle.take() {
                                        let _ = handle.join();
                                    }
                                }
                                break;
                            }

                            KeyCode::Char('c') | KeyCode::Char('C')
                                if key.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                textarea.copy();
                                if let Some(ref mut cb) = clipboard {
                                    let _ = cb.set_text(textarea.yank_text().to_string());
                                    status_msg = String::from(" Copied to system clipboard ");
                                    status_color = Color::Green;
                                }
                            }

                            KeyCode::Char('x') | KeyCode::Char('X')
                                if key.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                textarea.cut();
                                if let Some(ref mut cb) = clipboard {
                                    let _ = cb.set_text(textarea.yank_text().to_string());
                                    status_msg = String::from(" Cut to system clipboard ");
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
                                        status_msg = String::from(" Pasted from system clipboard ");
                                        status_color = Color::Green;
                                    }
                                }
                            }

                            KeyCode::F(5) => {
                                if is_running {
                                    continue;
                                } // Don't run multiple
                                let source = textarea.lines().join("\n");
                                let tokens = match lexer::tokenize(&source) {
                                    Ok(t) => t,
                                    Err(e) => {
                                        status_msg = format!(" Lexer Error: {} ", e);
                                        status_color = Color::Red;
                                        continue;
                                    }
                                };
                                let ast = match parser::parse(tokens) {
                                    Ok(p) => p,
                                    Err(e) => {
                                        status_msg = format!(" Syntax Error: {} ", e);
                                        status_color = Color::Red;
                                        continue;
                                    }
                                };

                                let tx = console_tx.clone();
                                let (in_tx, in_rx) = mpsc::channel();
                                input_tx_opt = Some(in_tx);
                                let g = Arc::clone(&graphics_state);

                                console_output.clear();
                                console_output.push("--- Running Program ---".to_string());
                                console_output.push(String::new());
                                is_running = true;
                                cancel_flag.store(false, Ordering::Relaxed);
                                let c_flag = Arc::clone(&cancel_flag);

                                // OPTIMIZATION: Threaded execution prevents UI freezing on loops
                                run_handle = Some(thread::spawn(move || {
                                    let res = interpreter::run(ast, tx.clone(), in_rx, c_flag, g);
                                    match res {
                                        Err(e) => {
                                            let _ = tx.send(ConsoleMsg::End(Some(e)));
                                        }
                                        Ok(_) => {
                                            let _ = tx.send(ConsoleMsg::End(None));
                                        }
                                    }
                                }));
                            }

                            KeyCode::F(6) => {
                                if is_running {
                                    cancel_flag.store(true, Ordering::Relaxed);
                                    if let Some(tx) = &input_tx_opt {
                                        let _ = tx.send(String::new()); // Unblock any pending UI prompts
                                    }
                                }
                            }

                            KeyCode::F(2) => {
                                app_state = AppState::PromptingSave(current_file.clone());
                                status_color = Color::Yellow;
                            }

                            KeyCode::F(3) => {
                                app_state = AppState::PromptingLoad(current_file.clone());
                                status_color = Color::Yellow;
                            }

                            _ => {
                                textarea.input(key);
                                status_msg = String::from(
                                    " Editing... F5: Run | F2: Save | F3: Load | Esc: Quit ",
                                );
                                status_color = Color::Green;
                            }
                        }
                    }
                }
            }
        }
    }

    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    disable_raw_mode()?;
    terminal.show_cursor()?;

    Ok(())
}

fn run_file(path: &str) {
    let src = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error reading file: {}", e);
            return;
        }
    };
    let tokens = match lexer::tokenize(&src) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Lexer error: {}", e);
            return;
        }
    };
    let ast = match parser::parse(tokens) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Parser error: {}", e);
            return;
        }
    };

    let (console_tx, console_rx) = mpsc::channel();
    let (input_tx, input_rx) = mpsc::channel();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let g = Arc::new(Mutex::new(graphics::SharedGraphics::new()));

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
                ConsoleMsg::InputPrompt(p) => {
                    use std::io::Write;
                    print!("{}", p);
                    let _ = std::io::stdout().flush();
                    let mut buf = String::new();
                    if std::io::stdin().read_line(&mut buf).is_err() {
                        let _ = input_tx.send(String::new());
                        continue;
                    }
                    let _ = input_tx.send(buf.trim_end().to_string());
                }
                ConsoleMsg::Clear => {
                    print!("\x1B[2J\x1B[1;1H");
                }
                ConsoleMsg::End(e) => {
                    if let Some(err) = e {
                        eprintln!("Runtime error: {}", err);
                    }
                    break;
                }
            }
        }
    });

    let res = interpreter::run(ast, console_tx.clone(), input_rx, cancel_flag, g);
    match res {
        Err(e) => {
            let _ = console_tx.send(ConsoleMsg::End(Some(e)));
        }
        Ok(_) => {
            let _ = console_tx.send(ConsoleMsg::End(None));
        }
    }
    let _ = output_handle.join();
}
