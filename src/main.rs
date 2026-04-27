use chrono::{Datelike, Local, NaiveDate};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};
use serde::{Deserialize, Serialize};
use std::{fs, io, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Todo {
    text: String,
    done: bool,
    date: NaiveDate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Daily {
    id: u64,
    text: String,
    done_dates: Vec<NaiveDate>,
}

impl Daily {
    fn is_done_on(&self, date: NaiveDate) -> bool {
        self.done_dates.contains(&date)
    }

    fn toggle_done(&mut self, date: NaiveDate) {
        if let Some(pos) = self.done_dates.iter().position(|d| *d == date) {
            self.done_dates.remove(pos);
        } else {
            self.done_dates.push(date);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SaveData {
    todos: Vec<Todo>,
    dailies: Vec<Daily>,
    next_daily_id: u64,
}

enum Mode {
    Normal,
    Insert,
}

#[derive(PartialEq)]
enum Focus {
    Dailies,
    Todos,
}

struct App {
    todos: Vec<Todo>,
    dailies: Vec<Daily>,
    next_daily_id: u64,
    daily_state: ListState,
    todo_state: ListState,
    focus: Focus,
    mode: Mode,
    input: String,
    current_date: NaiveDate,
    today: NaiveDate,
    save_path: PathBuf,
}

impl App {
    fn new() -> Self {
        let today = Local::now().date_naive();
        let save_path = save_path();
        let data = load_data(&save_path);

        let mut todos = data.todos;
        todos.retain(|t| t.date >= today);

        let mut app = App {
            todos,
            dailies: data.dailies,
            next_daily_id: data.next_daily_id,
            daily_state: ListState::default(),
            todo_state: ListState::default(),
            focus: Focus::Dailies,
            mode: Mode::Normal,
            input: String::new(),
            current_date: today,
            today,
            save_path,
        };
        app.sync_daily_selection();
        app.sync_todo_selection();
        app
    }

    // --- selection helpers ---

    fn sync_daily_selection(&mut self) {
        let n = self.dailies.len();
        if n == 0 {
            self.daily_state.select(None);
        } else {
            match self.daily_state.selected() {
                None => self.daily_state.select(Some(0)),
                Some(i) if i >= n => self.daily_state.select(Some(n - 1)),
                _ => {}
            }
        }
    }

    fn visible_todo_indices(&self) -> Vec<usize> {
        self.todos
            .iter()
            .enumerate()
            .filter_map(|(i, t)| if t.date == self.current_date { Some(i) } else { None })
            .collect()
    }

    fn sync_todo_selection(&mut self) {
        let n = self.visible_todo_indices().len();
        if n == 0 {
            self.todo_state.select(None);
        } else {
            match self.todo_state.selected() {
                None => self.todo_state.select(Some(0)),
                Some(i) if i >= n => self.todo_state.select(Some(n - 1)),
                _ => {}
            }
        }
    }

    // --- navigation ---

    fn select_next(&mut self) {
        match self.focus {
            Focus::Dailies => {
                let n = self.dailies.len();
                if n == 0 { return; }
                let i = self.daily_state.selected().map(|i| (i + 1) % n).unwrap_or(0);
                self.daily_state.select(Some(i));
            }
            Focus::Todos => {
                let n = self.visible_todo_indices().len();
                if n == 0 { return; }
                let i = self.todo_state.selected().map(|i| (i + 1) % n).unwrap_or(0);
                self.todo_state.select(Some(i));
            }
        }
    }

    fn select_prev(&mut self) {
        match self.focus {
            Focus::Dailies => {
                let n = self.dailies.len();
                if n == 0 { return; }
                let i = match self.daily_state.selected() {
                    Some(0) | None => n - 1,
                    Some(i) => i - 1,
                };
                self.daily_state.select(Some(i));
            }
            Focus::Todos => {
                let n = self.visible_todo_indices().len();
                if n == 0 { return; }
                let i = match self.todo_state.selected() {
                    Some(0) | None => n - 1,
                    Some(i) => i - 1,
                };
                self.todo_state.select(Some(i));
            }
        }
    }

    // --- actions ---

    fn toggle_done(&mut self) {
        match self.focus {
            Focus::Dailies => {
                if let Some(i) = self.daily_state.selected() {
                    let date = self.current_date;
                    self.dailies[i].toggle_done(date);
                    self.save();
                }
            }
            Focus::Todos => {
                if let Some(vis_i) = self.todo_state.selected() {
                    let indices = self.visible_todo_indices();
                    if let Some(&real_i) = indices.get(vis_i) {
                        self.todos[real_i].done = !self.todos[real_i].done;
                        self.save();
                    }
                }
            }
        }
    }

    fn delete_selected(&mut self) {
        match self.focus {
            Focus::Dailies => {
                if let Some(i) = self.daily_state.selected() {
                    self.dailies.remove(i);
                    self.sync_daily_selection();
                    self.save();
                }
            }
            Focus::Todos => {
                if let Some(vis_i) = self.todo_state.selected() {
                    let indices = self.visible_todo_indices();
                    if let Some(&real_i) = indices.get(vis_i) {
                        self.todos.remove(real_i);
                        self.sync_todo_selection();
                        self.save();
                    }
                }
            }
        }
    }

    fn confirm_input(&mut self) {
        let text = self.input.trim().to_string();
        if !text.is_empty() {
            match self.focus {
                Focus::Dailies => {
                    let id = self.next_daily_id;
                    self.next_daily_id += 1;
                    self.dailies.push(Daily { id, text, done_dates: vec![] });
                    self.daily_state.select(Some(self.dailies.len() - 1));
                }
                Focus::Todos => {
                    self.todos.push(Todo { text, done: false, date: self.current_date });
                    let n = self.visible_todo_indices().len();
                    self.todo_state.select(Some(n - 1));
                }
            }
            self.save();
        }
        self.input.clear();
        self.mode = Mode::Normal;
    }

    fn next_day(&mut self) {
        self.current_date = self.current_date.succ_opt().unwrap_or(self.current_date);
        self.todo_state.select(None);
        self.sync_todo_selection();
    }

    fn prev_day(&mut self) {
        let candidate = self.current_date.pred_opt().unwrap_or(self.current_date);
        if candidate >= self.today {
            self.current_date = candidate;
            self.todo_state.select(None);
            self.sync_todo_selection();
        }
    }

    fn save(&self) {
        let data = SaveData {
            todos: self.todos.clone(),
            dailies: self.dailies.clone(),
            next_daily_id: self.next_daily_id,
        };
        if let Ok(json) = serde_json::to_string_pretty(&data) {
            let _ = fs::write(&self.save_path, json);
        }
    }
}

fn save_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dir = PathBuf::from(home).join(".todor");
    let _ = fs::create_dir_all(&dir);
    dir.join("todos.json")
}

fn load_data(path: &PathBuf) -> SaveData {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(SaveData {
            todos: vec![],
            dailies: vec![],
            next_daily_id: 0,
        })
}

fn weekday_pt(date: NaiveDate) -> &'static str {
    match date.weekday() {
        chrono::Weekday::Mon => "segunda-feira",
        chrono::Weekday::Tue => "terça-feira",
        chrono::Weekday::Wed => "quarta-feira",
        chrono::Weekday::Thu => "quinta-feira",
        chrono::Weekday::Fri => "sexta-feira",
        chrono::Weekday::Sat => "sábado",
        chrono::Weekday::Sun => "domingo",
    }
}

fn main() -> Result<(), io::Error> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    let res = run(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    res
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<(), io::Error> {
    loop {
        terminal.draw(|f| ui(f, app))?;

        if let Event::Key(key) = event::read()? {
            match app.mode {
                Mode::Normal => match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Tab => {
                        app.focus = match app.focus {
                            Focus::Dailies => Focus::Todos,
                            Focus::Todos => Focus::Dailies,
                        };
                    }
                    KeyCode::Char('a') | KeyCode::Char('i') => {
                        app.mode = Mode::Insert;
                    }
                    KeyCode::Char('j') | KeyCode::Down => app.select_next(),
                    KeyCode::Char('k') | KeyCode::Up => app.select_prev(),
                    KeyCode::Char(' ') | KeyCode::Enter => app.toggle_done(),
                    KeyCode::Char('d') | KeyCode::Delete => app.delete_selected(),
                    KeyCode::Char('l') | KeyCode::Right => app.next_day(),
                    KeyCode::Char('h') | KeyCode::Left => app.prev_day(),
                    _ => {}
                },
                Mode::Insert => match key.code {
                    KeyCode::Enter => app.confirm_input(),
                    KeyCode::Esc => {
                        app.input.clear();
                        app.mode = Mode::Normal;
                    }
                    KeyCode::Backspace => {
                        app.input.pop();
                    }
                    KeyCode::Char(c) => {
                        app.input.push(c);
                    }
                    _ => {}
                },
            }
        }
    }
}

fn ui(f: &mut ratatui::Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(6),
            Constraint::Min(3),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(f.area());

    // Date header
    let is_today = app.current_date == app.today;
    let days_ahead = (app.current_date - app.today).num_days();
    let weekday = weekday_pt(app.current_date);
    let date_label = if is_today {
        format!(" hoje — {} — {} ", weekday, app.current_date.format("%d/%m/%Y"))
    } else if days_ahead == 1 {
        format!(" amanhã — {} — {} ", weekday, app.current_date.format("%d/%m/%Y"))
    } else {
        format!(" +{} dias — {} — {} ", days_ahead, weekday, app.current_date.format("%d/%m/%Y"))
    };
    let date_color = if is_today { Color::Cyan } else { Color::Yellow };
    let date_widget = Paragraph::new(date_label.as_str())
        .style(Style::default().fg(date_color).add_modifier(Modifier::BOLD))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(date_widget, chunks[0]);

    // Dailies panel
    let daily_focused = app.focus == Focus::Dailies;
    let daily_items: Vec<ListItem> = app
        .dailies
        .iter()
        .map(|d| {
            let done = d.is_done_on(app.current_date);
            let (check, style) = if done {
                ("[x]", Style::default().fg(Color::DarkGray).add_modifier(Modifier::CROSSED_OUT))
            } else {
                ("[ ]", Style::default().fg(Color::White))
            };
            ListItem::new(Line::from(vec![
                Span::styled(check, Style::default().fg(Color::Magenta)),
                Span::raw(" "),
                Span::styled(d.text.clone(), style),
            ]))
        })
        .collect();

    let daily_border = if daily_focused {
        Style::default().fg(Color::Magenta)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let daily_title = format!(" diária ({}) ", app.dailies.len());
    let daily_list = List::new(daily_items)
        .block(Block::default().borders(Borders::ALL).title(daily_title).border_style(daily_border))
        .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
        .highlight_symbol("► ");
    f.render_stateful_widget(daily_list, chunks[1], &mut app.daily_state);

    // Todos panel
    let todo_focused = app.focus == Focus::Todos;
    let visible_todos: Vec<&Todo> = app
        .todos
        .iter()
        .filter(|t| t.date == app.current_date)
        .collect();

    let todo_items: Vec<ListItem> = visible_todos
        .iter()
        .map(|t| {
            let (check, style) = if t.done {
                ("[x]", Style::default().fg(Color::DarkGray).add_modifier(Modifier::CROSSED_OUT))
            } else {
                ("[ ]", Style::default().fg(Color::White))
            };
            ListItem::new(Line::from(vec![
                Span::styled(check, Style::default().fg(Color::Green)),
                Span::raw(" "),
                Span::styled(t.text.clone(), style),
            ]))
        })
        .collect();

    let todo_border = if todo_focused {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let todo_title = format!(" todos ({}) ", visible_todos.len());
    let todo_list = List::new(todo_items)
        .block(Block::default().borders(Borders::ALL).title(todo_title).border_style(todo_border))
        .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
        .highlight_symbol("► ");
    f.render_stateful_widget(todo_list, chunks[2], &mut app.todo_state);

    // Input box
    let focused_label = match app.focus {
        Focus::Dailies => "diária",
        Focus::Todos => "todo",
    };
    let (input_title, input_style) = match app.mode {
        Mode::Insert => (
            format!(" novo {} (Enter=salvar, Esc=cancelar) ", focused_label),
            Style::default().fg(Color::Yellow),
        ),
        Mode::Normal => (
            format!(" input "),
            Style::default().fg(Color::DarkGray),
        ),
    };
    let input = Paragraph::new(app.input.as_str())
        .style(input_style)
        .block(Block::default().borders(Borders::ALL).title(input_title));
    f.render_widget(input, chunks[3]);

    if matches!(app.mode, Mode::Insert) {
        f.set_cursor_position((chunks[3].x + app.input.len() as u16 + 1, chunks[3].y + 1));
    }

    // Help bar
    let help = match app.mode {
        Mode::Normal => "  tab: trocar painel  a/i: add  j/k: mover  space: toggle  d: delete  h/l: dia  q: sair",
        Mode::Insert => "  digitando... enter salva, esc cancela",
    };
    let help_widget = Paragraph::new(help).style(Style::default().fg(Color::DarkGray));
    f.render_widget(help_widget, chunks[4]);
}
