use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState},
    Terminal,
};
use std::io;
use std::process::Command;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Enforce,
    Complain,
    Audit,
    Disable,
    Kill,
}

struct App {
    profiles: Vec<(String, Mode)>,
    state: ListState,
}

impl App {
    fn new() -> App {
        App {
            profiles: Vec::new(),
            state: ListState::default(),
        }
    }

    fn load_profiles(&mut self) -> Result<()> {
        let output = Command::new("aa-status")
        .output()
        .context("Failed to execute aa-status")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!("aa-status failed"));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout.lines().collect();

        self.profiles.clear();
        let mut state = None;

        for line in lines {
            let trimmed = line.trim_end();
            if trimmed.contains("profiles are in enforce mode.") {
                state = Some(Mode::Enforce);
                continue;
            } else if trimmed.contains("profiles are in complain mode.") {
                state = Some(Mode::Complain);
                continue;
            } else if trimmed.contains("profiles are in kill mode.") {
                state = Some(Mode::Kill);
                continue;
            } else if trimmed.contains("profiles are in audit mode.") { // May not exist, but added for completeness
                state = Some(Mode::Audit);
                continue;
            }

            let profile_line = trimmed.trim();
            if !profile_line.is_empty() && (profile_line.starts_with('/') || profile_line.starts_with('{')) {
                if let Some(mode) = state {
                    self.profiles.push((profile_line.to_string(), mode));
                }
            }
        }

        Ok(())
    }

    fn next(&mut self) {
        let i = match self.state.selected() {
            Some(i) => if i >= self.profiles.len() - 1 { 0 } else { i + 1 },
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn previous(&mut self) {
        let i = match self.state.selected() {
            Some(i) => if i == 0 { self.profiles.len() - 1 } else { i - 1 },
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn change_mode(&mut self, new_mode: Mode) -> Result<()> {
        if let Some(i) = self.state.selected() {
            let profile = &self.profiles[i].0;
            let cmd = match new_mode {
                Mode::Enforce => "aa-enforce",
                Mode::Complain => "aa-complain",
                Mode::Audit => "aa-audit",
                Mode::Disable => "aa-disable",
                Mode::Kill => return Ok(()), // No command for kill mode
            };

            let status = Command::new("sudo")
            .args([cmd, profile])
            .status()
            .context(format!("Failed to execute {}", cmd))?;

            if status.success() {
                self.load_profiles()?; // Reload to update list and modes
            } else {
                return Err(anyhow::anyhow!("Command failed: {}", cmd));
            }
        }
        Ok(())
    }

    fn reload_all(&mut self) -> Result<()> {
        let status = Command::new("sudo")
        .args(["systemctl", "reload", "apparmor"])
        .status()
        .context("Failed to reload apparmor")?;

        if status.success() {
            self.load_profiles()?;
        }
        Ok(())
    }

    fn edit_profile(&mut self) -> Result<()> {
        if let Some(i) = self.state.selected() {
            let profile = &self.profiles[i].0;
            let file = if profile.starts_with('/') {
                profile[1..].replace('/', ".")
            } else {
                profile.to_string()
            };
            let path = format!("/etc/apparmor.d/{}", file);
            let status = Command::new("sudo")
            .args(["vim", &path]) // Change to your preferred editor if needed
            .status()?;

            if status.success() {
                self.reload_all()?;
            }
        }
        Ok(())
    }
}

fn main() -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    app.load_profiles()?;

    if !app.profiles.is_empty() {
        app.state.select(Some(0));
    }

    loop {
        terminal.draw(|f| {
            let size = f.area();
            let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(100)])
            .split(size);

            let items: Vec<ListItem> = app.profiles.iter().map(|(name, mode)| {
                let color = match mode {
                    Mode::Enforce => Color::Green,
                    Mode::Complain => Color::Yellow,
                    Mode::Audit => Color::Cyan,
                    Mode::Disable => Color::Gray,
                    Mode::Kill => Color::Red,
                };
                ListItem::new(name.as_str()).style(Style::default().fg(color))
            }).collect();

            let list = List::new(items)
            .block(Block::default().title("AppArmor Profiles").borders(Borders::ALL))
            .highlight_style(Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED))
            .highlight_symbol("> ");

            f.render_stateful_widget(list, chunks[0], &mut app.state);
        })?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Down => app.next(),
                    KeyCode::Up => app.previous(),
                    KeyCode::Char('e') => { let _ = app.change_mode(Mode::Enforce); },
                    KeyCode::Char('c') => { let _ = app.change_mode(Mode::Complain); },
                    KeyCode::Char('a') => { let _ = app.change_mode(Mode::Audit); },
                    KeyCode::Char('d') => { let _ = app.change_mode(Mode::Disable); },
                    KeyCode::Char('r') => { let _ = app.load_profiles(); },
                    KeyCode::Char('R') => { let _ = app.reload_all(); },
                    KeyCode::Char('v') => { let _ = app.edit_profile(); },
                    _ => {},
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}
