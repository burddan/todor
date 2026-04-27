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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SubTask {
    text: String,
    done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Todo {
    text: String,
    done: bool,
    date: NaiveDate,
    #[serde(default)]
    children: Vec<SubTask>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Persistent {
    id: u64,
    text: String,
    done_dates: Vec<NaiveDate>,
    #[serde(default)]
    children: Vec<SubTask>,
}

impl Persistent {
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
    metas: Vec<Persistent>,
    dailies: Vec<Persistent>,
    next_id: u64,
}

// (parent_real_idx, child_idx_or_none)
type FlatRef = (usize, Option<usize>);

fn flat_persistent(items: &[Persistent]) -> Vec<FlatRef> {
    let mut v = vec![];
    for (pi, p) in items.iter().enumerate() {
        v.push((pi, None));
        for ci in 0..p.children.len() {
            v.push((pi, Some(ci)));
        }
    }
    v
}

fn flat_todos(todos: &[Todo], date: NaiveDate) -> Vec<FlatRef> {
    let mut v = vec![];
    for (pi, t) in todos.iter().enumerate() {
        if t.date != date { continue; }
        v.push((pi, None));
        for ci in 0..t.children.len() {
            v.push((pi, Some(ci)));
        }
    }
    v
}

enum Mode {
    Normal,
    Insert { as_child: bool },
}

#[derive(PartialEq)]
enum Focus { Metas, Dailies, Todos }

impl Focus {
    fn next(&self) -> Focus {
        match self {
            Focus::Metas => Focus::Dailies,
            Focus::Dailies => Focus::Todos,
            Focus::Todos => Focus::Metas,
        }
    }
}

struct App {
    todos: Vec<Todo>,
    metas: Vec<Persistent>,
    dailies: Vec<Persistent>,
    next_id: u64,
    meta_state: ListState,
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
            metas: data.metas,
            dailies: data.dailies,
            next_id: data.next_id,
            meta_state: ListState::default(),
            daily_state: ListState::default(),
            todo_state: ListState::default(),
            focus: Focus::Metas,
            mode: Mode::Normal,
            input: String::new(),
            current_date: today,
            today,
            save_path,
        };
        let nm = flat_persistent(&app.metas).len();
        let nd = flat_persistent(&app.dailies).len();
        let nt = flat_todos(&app.todos, today).len();
        sync_sel(&mut app.meta_state, nm);
        sync_sel(&mut app.daily_state, nd);
        sync_sel(&mut app.todo_state, nt);
        app
    }

    fn flat_metas(&self) -> Vec<FlatRef> { flat_persistent(&self.metas) }
    fn flat_dailies(&self) -> Vec<FlatRef> { flat_persistent(&self.dailies) }
    fn flat_todos(&self) -> Vec<FlatRef> { flat_todos(&self.todos, self.current_date) }

    fn current_flat_ref(&self) -> Option<FlatRef> {
        match self.focus {
            Focus::Metas => self.meta_state.selected().and_then(|i| self.flat_metas().get(i).copied()),
            Focus::Dailies => self.daily_state.selected().and_then(|i| self.flat_dailies().get(i).copied()),
            Focus::Todos => self.todo_state.selected().and_then(|i| self.flat_todos().get(i).copied()),
        }
    }

    fn select_next(&mut self) {
        let n = match self.focus {
            Focus::Metas => self.flat_metas().len(),
            Focus::Dailies => self.flat_dailies().len(),
            Focus::Todos => self.flat_todos().len(),
        };
        let state = self.focused_state_mut();
        step(state, n, 1);
    }

    fn select_prev(&mut self) {
        let n = match self.focus {
            Focus::Metas => self.flat_metas().len(),
            Focus::Dailies => self.flat_dailies().len(),
            Focus::Todos => self.flat_todos().len(),
        };
        let state = self.focused_state_mut();
        step(state, n, -1);
    }

    fn focused_state_mut(&mut self) -> &mut ListState {
        match self.focus {
            Focus::Metas => &mut self.meta_state,
            Focus::Dailies => &mut self.daily_state,
            Focus::Todos => &mut self.todo_state,
        }
    }

    fn toggle_done(&mut self) {
        let date = self.current_date;
        let fr = match self.current_flat_ref() { Some(r) => r, None => return };
        match self.focus {
            Focus::Metas => {
                let (pi, ci) = fr;
                match ci {
                    None => self.metas[pi].toggle_done(date),
                    Some(ci) => self.metas[pi].children[ci].done = !self.metas[pi].children[ci].done,
                }
            }
            Focus::Dailies => {
                let (pi, ci) = fr;
                match ci {
                    None => self.dailies[pi].toggle_done(date),
                    Some(ci) => self.dailies[pi].children[ci].done = !self.dailies[pi].children[ci].done,
                }
            }
            Focus::Todos => {
                let (pi, ci) = fr;
                match ci {
                    None => self.todos[pi].done = !self.todos[pi].done,
                    Some(ci) => self.todos[pi].children[ci].done = !self.todos[pi].children[ci].done,
                }
            }
        }
        self.save();
    }

    fn delete_selected(&mut self) {
        let fr = match self.current_flat_ref() { Some(r) => r, None => return };
        match self.focus {
            Focus::Metas => {
                let (pi, ci) = fr;
                match ci {
                    None => { self.metas.remove(pi); }
                    Some(ci) => { self.metas[pi].children.remove(ci); }
                }
                let n = self.flat_metas().len();
                sync_sel(&mut self.meta_state, n);
            }
            Focus::Dailies => {
                let (pi, ci) = fr;
                match ci {
                    None => { self.dailies.remove(pi); }
                    Some(ci) => { self.dailies[pi].children.remove(ci); }
                }
                let n = self.flat_dailies().len();
                sync_sel(&mut self.daily_state, n);
            }
            Focus::Todos => {
                let (pi, ci) = fr;
                match ci {
                    None => { self.todos.remove(pi); }
                    Some(ci) => { self.todos[pi].children.remove(ci); }
                }
                let n = self.flat_todos().len();
                sync_sel(&mut self.todo_state, n);
            }
        }
        self.save();
    }

    fn confirm_input(&mut self) {
        let text = self.input.trim().to_string();
        let as_child = matches!(self.mode, Mode::Insert { as_child: true });
        if !text.is_empty() {
            if as_child {
                let fr = self.current_flat_ref();
                let parent_idx = fr.map(|(pi, _)| pi);
                match self.focus {
                    Focus::Metas => {
                        if let Some(pi) = parent_idx {
                            self.metas[pi].children.push(SubTask { text, done: false });
                            let flat_idx = self.flat_metas().len() - 1; // last in flat
                            // find new child in flat
                            let new_flat_idx = self.flat_metas().iter().position(|&(p, c)| p == pi && c == Some(self.metas[pi].children.len() - 1));
                            if let Some(i) = new_flat_idx { self.meta_state.select(Some(i)); }
                            let _ = flat_idx;
                        }
                    }
                    Focus::Dailies => {
                        if let Some(pi) = parent_idx {
                            self.dailies[pi].children.push(SubTask { text, done: false });
                            let new_flat_idx = self.flat_dailies().iter().position(|&(p, c)| p == pi && c == Some(self.dailies[pi].children.len() - 1));
                            if let Some(i) = new_flat_idx { self.daily_state.select(Some(i)); }
                        }
                    }
                    Focus::Todos => {
                        if let Some(pi) = parent_idx {
                            self.todos[pi].children.push(SubTask { text, done: false });
                            let new_flat_idx = self.flat_todos().iter().position(|&(p, c)| p == pi && c == Some(self.todos[pi].children.len() - 1));
                            if let Some(i) = new_flat_idx { self.todo_state.select(Some(i)); }
                        }
                    }
                }
            } else {
                let id = self.next_id;
                self.next_id += 1;
                match self.focus {
                    Focus::Metas => {
                        self.metas.push(Persistent { id, text, done_dates: vec![], children: vec![] });
                        let n = self.flat_metas().len();
                        self.meta_state.select(Some(n - 1));
                    }
                    Focus::Dailies => {
                        self.dailies.push(Persistent { id, text, done_dates: vec![], children: vec![] });
                        let n = self.flat_dailies().len();
                        self.daily_state.select(Some(n - 1));
                    }
                    Focus::Todos => {
                        self.todos.push(Todo { text, done: false, date: self.current_date, children: vec![] });
                        let n = self.flat_todos().len();
                        self.todo_state.select(Some(n - 1));
                    }
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
        let n = self.flat_todos().len();
        sync_sel(&mut self.todo_state, n);
    }

    fn prev_day(&mut self) {
        let candidate = self.current_date.pred_opt().unwrap_or(self.current_date);
        if candidate >= self.today {
            self.current_date = candidate;
            self.todo_state.select(None);
            let n = self.flat_todos().len();
            sync_sel(&mut self.todo_state, n);
        }
    }

    fn save(&self) {
        let data = SaveData {
            todos: self.todos.clone(),
            metas: self.metas.clone(),
            dailies: self.dailies.clone(),
            next_id: self.next_id,
        };
        if let Ok(json) = serde_json::to_string_pretty(&data) {
            let _ = fs::write(&self.save_path, json);
        }
    }
}

fn sync_sel(state: &mut ListState, n: usize) {
    if n == 0 {
        state.select(None);
    } else {
        match state.selected() {
            None => state.select(Some(0)),
            Some(i) if i >= n => state.select(Some(n - 1)),
            _ => {}
        }
    }
}

fn step(state: &mut ListState, n: usize, dir: i64) {
    if n == 0 { return; }
    let i = match state.selected() {
        None => 0,
        Some(i) => ((i as i64 + dir).rem_euclid(n as i64)) as usize,
    };
    state.select(Some(i));
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
        .unwrap_or(SaveData { todos: vec![], metas: vec![], dailies: vec![], next_id: 0 })
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

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<(), io::Error> {
    loop {
        terminal.draw(|f| ui(f, app))?;
        if let Event::Key(key) = event::read()? {
            match app.mode {
                Mode::Normal => match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Tab => { app.focus = app.focus.next(); }
                    KeyCode::Char('a') | KeyCode::Char('i') => { app.mode = Mode::Insert { as_child: false }; }
                    KeyCode::Char('o') => { app.mode = Mode::Insert { as_child: true }; }
                    KeyCode::Char('j') | KeyCode::Down => app.select_next(),
                    KeyCode::Char('k') | KeyCode::Up => app.select_prev(),
                    KeyCode::Char(' ') | KeyCode::Enter => app.toggle_done(),
                    KeyCode::Char('d') | KeyCode::Delete => app.delete_selected(),
                    KeyCode::Char('l') | KeyCode::Right => app.next_day(),
                    KeyCode::Char('h') | KeyCode::Left => app.prev_day(),
                    _ => {}
                },
                Mode::Insert { .. } => match key.code {
                    KeyCode::Enter => app.confirm_input(),
                    KeyCode::Esc => { app.input.clear(); app.mode = Mode::Normal; }
                    KeyCode::Backspace => { app.input.pop(); }
                    KeyCode::Char(c) => { app.input.push(c); }
                    _ => {}
                },
            }
        }
    }
}

fn render_persistent_items(items: &[Persistent], flat: &[FlatRef], date: NaiveDate, check_color: Color) -> Vec<ListItem<'static>> {
    flat.iter().map(|&(pi, ci)| {
        match ci {
            None => {
                let p = &items[pi];
                let done = p.is_done_on(date);
                let (check, style) = if done {
                    ("[x]", Style::default().fg(Color::DarkGray).add_modifier(Modifier::CROSSED_OUT))
                } else {
                    ("[ ]", Style::default().fg(Color::White))
                };
                ListItem::new(Line::from(vec![
                    Span::styled(check, Style::default().fg(check_color)),
                    Span::raw(" "),
                    Span::styled(p.text.clone(), style),
                ]))
            }
            Some(ci) => {
                let child = &items[pi].children[ci];
                let (check, style) = if child.done {
                    ("•[x]", Style::default().fg(Color::DarkGray).add_modifier(Modifier::CROSSED_OUT))
                } else {
                    ("•[ ]", Style::default().fg(Color::White))
                };
                ListItem::new(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(check, Style::default().fg(check_color).add_modifier(Modifier::DIM)),
                    Span::raw(" "),
                    Span::styled(child.text.clone(), style),
                ]))
            }
        }
    }).collect()
}

fn render_todo_items(todos: &[Todo], flat: &[FlatRef]) -> Vec<ListItem<'static>> {
    flat.iter().map(|&(pi, ci)| {
        match ci {
            None => {
                let t = &todos[pi];
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
            }
            Some(ci) => {
                let child = &todos[pi].children[ci];
                let (check, style) = if child.done {
                    ("•[x]", Style::default().fg(Color::DarkGray).add_modifier(Modifier::CROSSED_OUT))
                } else {
                    ("•[ ]", Style::default().fg(Color::White))
                };
                ListItem::new(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(check, Style::default().fg(Color::Green).add_modifier(Modifier::DIM)),
                    Span::raw(" "),
                    Span::styled(child.text.clone(), style),
                ]))
            }
        }
    }).collect()
}

fn ui(f: &mut ratatui::Frame, app: &mut App) {
    let flat_m = app.flat_metas();
    let flat_d = app.flat_dailies();
    let flat_t = app.flat_todos();

    // +2 for borders, min 3 lines tall, grow with content
    let meta_h = (flat_m.len() as u16 + 2).max(3);
    let daily_h = (flat_d.len() as u16 + 2).max(3);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(meta_h),
            Constraint::Length(daily_h),
            Constraint::Min(8),
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
    f.render_widget(
        Paragraph::new(date_label.as_str())
            .style(Style::default().fg(date_color).add_modifier(Modifier::BOLD))
            .block(Block::default().borders(Borders::ALL)),
        chunks[0],
    );

    // Metas
    let meta_items = render_persistent_items(&app.metas, &flat_m, app.current_date, Color::Blue);
    let meta_focused = app.focus == Focus::Metas;
    let meta_title = format!(" metas ({}) ", app.metas.len());
    f.render_stateful_widget(
        List::new(meta_items)
            .block(Block::default().borders(Borders::ALL).title(meta_title)
                .border_style(focused_border(meta_focused, Color::Blue)))
            .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
            .highlight_symbol("► "),
        chunks[1],
        &mut app.meta_state,
    );

    // Dailies
    let daily_items = render_persistent_items(&app.dailies, &flat_d, app.current_date, Color::Magenta);
    let daily_focused = app.focus == Focus::Dailies;
    let daily_title = format!(" diária ({}) ", app.dailies.len());
    f.render_stateful_widget(
        List::new(daily_items)
            .block(Block::default().borders(Borders::ALL).title(daily_title)
                .border_style(focused_border(daily_focused, Color::Magenta)))
            .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
            .highlight_symbol("► "),
        chunks[2],
        &mut app.daily_state,
    );

    // Todos
    let todo_items = render_todo_items(&app.todos, &flat_t);
    let todo_focused = app.focus == Focus::Todos;
    let todo_count = app.todos.iter().filter(|t| t.date == app.current_date).count();
    let todo_title = format!(" todos ({}) ", todo_count);
    f.render_stateful_widget(
        List::new(todo_items)
            .block(Block::default().borders(Borders::ALL).title(todo_title)
                .border_style(focused_border(todo_focused, Color::Green)))
            .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
            .highlight_symbol("► "),
        chunks[3],
        &mut app.todo_state,
    );

    // Input
    let as_child = matches!(app.mode, Mode::Insert { as_child: true });
    let focused_label = match app.focus {
        Focus::Metas => "meta",
        Focus::Dailies => "diária",
        Focus::Todos => "todo",
    };
    let (input_title, input_style) = match app.mode {
        Mode::Insert { .. } => {
            let label = if as_child {
                format!(" novo bullet em {} (Enter=salvar, Esc=cancelar) ", focused_label)
            } else {
                format!(" novo(a) {} (Enter=salvar, Esc=cancelar) ", focused_label)
            };
            (label, Style::default().fg(Color::Yellow))
        }
        Mode::Normal => (" input ".to_string(), Style::default().fg(Color::DarkGray)),
    };
    f.render_widget(
        Paragraph::new(app.input.as_str())
            .style(input_style)
            .block(Block::default().borders(Borders::ALL).title(input_title)),
        chunks[4],
    );
    if matches!(app.mode, Mode::Insert { .. }) {
        f.set_cursor_position((chunks[4].x + app.input.len() as u16 + 1, chunks[4].y + 1));
    }

    // Help
    let help = match app.mode {
        Mode::Normal => "  tab: painel  a/i: add  o: bullet  j/k: mover  space: toggle  d: delete  h/l: dia  q: sair",
        Mode::Insert { .. } => "  digitando... enter salva, esc cancela",
    };
    f.render_widget(
        Paragraph::new(help).style(Style::default().fg(Color::DarkGray)),
        chunks[5],
    );
}

fn focused_border(focused: bool, color: Color) -> Style {
    if focused { Style::default().fg(color) } else { Style::default().fg(Color::DarkGray) }
}
