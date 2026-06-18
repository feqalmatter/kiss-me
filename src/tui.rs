use crate::config::{Config, Profile};
use crate::nexus::{self, NxmLink};
use crate::ops::{self, CommandRunner, ModEntry, OperationContext, Reporter};
use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use std::cmp;
use std::io::{self, Stdout};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Mods,
    Buttons,
}

#[derive(Debug, Clone, Copy)]
enum Button {
    Assemble,
    ExtractDownloads,
    CheckLibrary,
    GenerateOverwrite,
    SaveManifest,
    DiffManifest,
    RemoveReadmes,
    DownloadNxm,
    Refresh,
}

#[derive(Debug)]
enum Mode {
    Normal,
    NxmInput(String),
}

struct App {
    config: Config,
    ctx: OperationContext,
    mods: Vec<ModEntry>,
    selected_mod: usize,
    selected_button: usize,
    focus: Focus,
    output: Vec<String>,
    mode: Mode,
    buttons: Vec<Button>,
}

struct TuiReporter<'a> {
    output: &'a mut Vec<String>,
}

impl Reporter for TuiReporter<'_> {
    fn line(&mut self, line: impl AsRef<str>) {
        self.output.push(line.as_ref().to_string());
    }
}

struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

pub fn run(config: Config, profile: Profile) -> Result<()> {
    let mut app = App::new(config, profile)?;
    let mut terminal = setup_terminal()?;

    loop {
        terminal.terminal.draw(|frame| draw(frame, &mut app))?;
        if !handle_event(&mut app)? {
            break;
        }
    }

    Ok(())
}

impl App {
    fn new(config: Config, profile: Profile) -> Result<Self> {
        let ctx = OperationContext::new(profile, CommandRunner::real());
        let mut app = Self {
            config,
            ctx,
            mods: Vec::new(),
            selected_mod: 0,
            selected_button: 0,
            focus: Focus::Mods,
            output: vec![
                "kiss-me TUI".to_string(),
                "Tab: switch panes  Space: enable/disable selected mod  Enter: run button  q: quit"
                    .to_string(),
            ],
            mode: Mode::Normal,
            buttons: vec![
                Button::Assemble,
                Button::ExtractDownloads,
                Button::CheckLibrary,
                Button::GenerateOverwrite,
                Button::SaveManifest,
                Button::DiffManifest,
                Button::RemoveReadmes,
                Button::DownloadNxm,
                Button::Refresh,
            ],
        };
        app.refresh_mods()?;
        Ok(app)
    }

    fn refresh_mods(&mut self) -> Result<()> {
        self.mods = ops::list_mods(&self.ctx)?;
        if self.mods.is_empty() {
            self.selected_mod = 0;
        } else {
            self.selected_mod = cmp::min(self.selected_mod, self.mods.len() - 1);
        }
        Ok(())
    }

    fn reporter(&mut self) -> TuiReporter<'_> {
        TuiReporter {
            output: &mut self.output,
        }
    }

    fn run_button(&mut self, button: Button) {
        let ctx = self.ctx.clone();
        let result = match button {
            Button::Assemble => ops::assemble(&ctx, &mut self.reporter()),
            Button::ExtractDownloads => {
                let result = ops::extract_downloads(&ctx, &mut self.reporter());
                if result.is_ok() {
                    let _ = self.refresh_mods();
                }
                result
            }
            Button::CheckLibrary => ops::check_library(&ctx, &mut self.reporter()),
            Button::GenerateOverwrite => ops::generate_overwrite(&ctx, false, &mut self.reporter()),
            Button::SaveManifest => ops::save_manifest(&ctx, &mut self.reporter()),
            Button::DiffManifest => match ops::manifest_changes(&ctx) {
                Ok(lines) => {
                    if lines.is_empty() {
                        self.output.push("No manifest changes.".to_string());
                    } else {
                        self.output.extend(lines);
                    }
                    Ok(())
                }
                Err(error) => Err(error),
            },
            Button::RemoveReadmes => ops::remove_readmes(&ctx, &mut self.reporter()),
            Button::DownloadNxm => {
                self.mode = Mode::NxmInput(String::new());
                self.output
                    .push("Paste an nxm:// link, then press Enter.".to_string());
                Ok(())
            }
            Button::Refresh => self.refresh_mods(),
        };

        if let Err(error) = result {
            self.output.push(format!("error: {error:#}"));
        }
    }

    fn toggle_selected_mod(&mut self) {
        let Some(mod_entry) = self.mods.get(self.selected_mod).cloned() else {
            self.output.push("No mod selected.".to_string());
            return;
        };

        let ctx = self.ctx.clone();
        let result = if mod_entry.enabled {
            ops::disable_mod(&ctx, &mod_entry.name, &mut self.reporter())
        } else {
            ops::enable_mod(&ctx, &mod_entry.name, &mut self.reporter())
        };

        if let Err(error) = result {
            self.output.push(format!("error: {error:#}"));
        }
        if let Err(error) = self.refresh_mods() {
            self.output.push(format!("error: {error:#}"));
        }
    }

    fn submit_nxm(&mut self, raw: String) {
        let raw = raw.trim();
        if raw.is_empty() {
            self.output.push("NXM download canceled.".to_string());
            self.mode = Mode::Normal;
            return;
        }

        let result = (|| {
            let link = NxmLink::parse(raw).context("invalid nxm:// link")?;
            let profile = self.ctx.profile.clone();
            if let Some(expected) = profile.nexus_game_domain.as_deref() {
                if expected != link.game_domain {
                    anyhow::bail!(
                        "nxm game domain '{}' does not match active profile domain '{}'",
                        link.game_domain,
                        expected
                    );
                }
            }

            let api_key = self
                .config
                .nexus_api_key()
                .or_else(|| std::env::var("NEXUS_API_KEY").ok())
                .or_else(|| std::env::var("NEXUSMODS_API_KEY").ok())
                .filter(|value| !value.trim().is_empty())
                .context("missing Nexus API key; set nexus_api_key in config or NEXUS_API_KEY")?;
            nexus::download_nxm(&profile, &api_key, &link, &mut self.reporter())?;
            Ok(())
        })();

        if let Err(error) = result {
            self.output.push(format!("error: {error:#}"));
        }
        self.mode = Mode::Normal;
    }
}

fn setup_terminal() -> Result<TerminalGuard> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(TerminalGuard { terminal })
}

fn handle_event(app: &mut App) -> Result<bool> {
    let Event::Key(key) = event::read()? else {
        return Ok(true);
    };
    if key.kind == KeyEventKind::Release {
        return Ok(true);
    }

    match &mut app.mode {
        Mode::NxmInput(input) => match key.code {
            KeyCode::Esc => {
                app.output.push("NXM download canceled.".to_string());
                app.mode = Mode::Normal;
            }
            KeyCode::Enter => {
                let raw = std::mem::take(input);
                app.submit_nxm(raw);
            }
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Char(ch) => input.push(ch),
            _ => {}
        },
        Mode::Normal => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(false),
            KeyCode::Tab => {
                app.focus = match app.focus {
                    Focus::Mods => Focus::Buttons,
                    Focus::Buttons => Focus::Mods,
                };
            }
            KeyCode::Up | KeyCode::Char('k') => move_selection(app, -1),
            KeyCode::Down | KeyCode::Char('j') => move_selection(app, 1),
            KeyCode::Char(' ') if app.focus == Focus::Mods => app.toggle_selected_mod(),
            KeyCode::Enter if app.focus == Focus::Buttons => {
                if let Some(button) = app.buttons.get(app.selected_button).copied() {
                    app.run_button(button);
                }
            }
            KeyCode::Char('r') => {
                if let Err(error) = app.refresh_mods() {
                    app.output.push(format!("error: {error:#}"));
                }
            }
            _ => {}
        },
    }

    Ok(true)
}

fn move_selection(app: &mut App, delta: isize) {
    match app.focus {
        Focus::Mods if !app.mods.is_empty() => {
            app.selected_mod = wrap_index(app.selected_mod, app.mods.len(), delta);
        }
        Focus::Buttons if !app.buttons.is_empty() => {
            app.selected_button = wrap_index(app.selected_button, app.buttons.len(), delta);
        }
        _ => {}
    }
}

fn wrap_index(index: usize, len: usize, delta: isize) -> usize {
    let len = len as isize;
    (index as isize + delta).rem_euclid(len) as usize
}

fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(10),
        ])
        .split(frame.area());

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            "kiss-me",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::raw(app.ctx.profile.game_source.display().to_string()),
    ]))
    .block(Block::default().borders(Borders::ALL).title("Profile"));
    frame.render_widget(header, root[0]);

    let middle = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(root[1]);

    draw_mods(frame, app, middle[0]);
    draw_buttons(frame, app, middle[1]);
    draw_output(frame, app, root[2]);

    if let Mode::NxmInput(input) = &app.mode {
        draw_nxm_input(frame, input);
    }
}

fn draw_mods(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let items: Vec<_> = if app.mods.is_empty() {
        vec![ListItem::new(
            "No mods found. Run extract-downloads or add Library entries.",
        )]
    } else {
        app.mods
            .iter()
            .map(|entry| {
                let marker = if entry.enabled { "[x]" } else { "[ ]" };
                let style = if entry.enabled {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                ListItem::new(Line::from(vec![
                    Span::styled(marker, style),
                    Span::raw(" "),
                    Span::raw(entry.name.clone()),
                ]))
            })
            .collect()
    };

    let title = if app.focus == Focus::Mods {
        "Mods (focused)"
    } else {
        "Mods"
    };
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
        .highlight_symbol("> ");
    let mut state = ListState::default();
    if !app.mods.is_empty() {
        state.select(Some(app.selected_mod));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_buttons(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let items: Vec<_> = app
        .buttons
        .iter()
        .map(|button| ListItem::new(button.label()))
        .collect();
    let title = if app.focus == Focus::Buttons {
        "Commands (focused)"
    } else {
        "Commands"
    };
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
        .highlight_symbol("> ");
    let mut state = ListState::default();
    state.select(Some(app.selected_button));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_output(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let height = area.height.saturating_sub(2) as usize;
    let start = app.output.len().saturating_sub(height);
    let lines: Vec<_> = app.output[start..]
        .iter()
        .map(|line| Line::raw(line.clone()))
        .collect();
    let output = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Output"))
        .wrap(Wrap { trim: false });
    frame.render_widget(output, area);
}

fn draw_nxm_input(frame: &mut Frame<'_>, input: &str) {
    let area = centered_rect(80, 20, frame.area());
    frame.render_widget(Clear, area);
    let prompt = Paragraph::new(vec![
        Line::raw("Paste nxm:// link and press Enter. Esc cancels."),
        Line::raw(""),
        Line::raw(input.to_string()),
    ])
    .block(Block::default().borders(Borders::ALL).title("Download NXM"));
    frame.render_widget(prompt, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

impl Button {
    fn label(self) -> &'static str {
        match self {
            Button::Assemble => "Assemble",
            Button::ExtractDownloads => "Extract downloads",
            Button::CheckLibrary => "Check library",
            Button::GenerateOverwrite => "Generate overwrite",
            Button::SaveManifest => "Save manifest",
            Button::DiffManifest => "Diff manifest",
            Button::RemoveReadmes => "Remove readmes",
            Button::DownloadNxm => "Download nxm:// link",
            Button::Refresh => "Refresh",
        }
    }
}
