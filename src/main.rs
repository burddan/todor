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
use std::io;

#[derive(Debug)]
struct Todo {
    text: String,
    done: bool,
}

enum Mode {
    Normal,
    Insert,
}

struct App {
    todos: Vec<Todo>,
    list_state: ListState,
    mode: Mode,
    input: String,
}

impl App {
    fn new() -> Self {
        let mut list_state = ListState::default();
        list_state.select(None);
        App {
            todos: Vec::new(),
            list_state,
            mode: Mode::Normal,
            input: String::new(),
        }
    }

    fn selected(&self) -> Option<usize> {
        self.list_state.selected()
    }

    fn select_next(&mut self) {
        if self.todos.is_empty() {
            return;
        }
        let i = match self.selected() {
            Some(i) => (i + 1) % self.todos.len(),
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    fn select_prev(&mut self) {
        if self.todos.is_empty() {
            return;
        }
        let i = match self.selected() {
            Some(0) | None => self.todos.len() - 1,
            Some(i) => i - 1,
        };
        self.list_state.select(Some(i));
    }

    fn toggle_done(&mut self) {
        if let Some(i) = self.selected() {
            self.todos[i].done = !self.todos[i].done;
        }
    }

    fn delete_selected(&mut self) {
        if let Some(i) = self.selected() {
            self.todos.remove(i);
            let new_sel = if self.todos.is_empty() {
                None
            } else {
                Some(i.saturating_sub(1))
            };
            self.list_state.select(new_sel);
        }
    }

    fn confirm_input(&mut self) {
        let text = self.input.trim().to_string();
        if !text.is_empty() {
            self.todos.push(Todo { text, done: false });
            let last = self.todos.len() - 1;
            self.list_state.select(Some(last));
        }
        self.input.clear();
        self.mode = Mode::Normal;
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
                    KeyCode::Char('a') | KeyCode::Char('i') => {
                        app.mode = Mode::Insert;
                    }
                    KeyCode::Char('j') | KeyCode::Down => app.select_next(),
                    KeyCode::Char('k') | KeyCode::Up => app.select_prev(),
                    KeyCode::Char(' ') | KeyCode::Enter => app.toggle_done(),
                    KeyCode::Char('d') | KeyCode::Delete => app.delete_selected(),
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
            Constraint::Min(3),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(f.area());

    let items: Vec<ListItem> = app
        .todos
        .iter()
        .map(|t| {
            let (check, style) = if t.done {
                (
                    "[x]",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::CROSSED_OUT),
                )
            } else {
                ("[ ]", Style::default().fg(Color::White))
            };
            ListItem::new(Line::from(vec![
                Span::styled(check, Style::default().fg(Color::Green)),
                Span::raw(" "),
                Span::styled(&t.text, style),
            ]))
        })
        .collect();

    let title = format!(" todor ({} items) ", app.todos.len());
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("► ");

    f.render_stateful_widget(list, chunks[0], &mut app.list_state);

    let (input_title, input_style) = match app.mode {
        Mode::Insert => (
            " new todo (Enter=save, Esc=cancel) ",
            Style::default().fg(Color::Yellow),
        ),
        Mode::Normal => (" input ", Style::default().fg(Color::DarkGray)),
    };
    let input = Paragraph::new(app.input.as_str())
        .style(input_style)
        .block(Block::default().borders(Borders::ALL).title(input_title));
    f.render_widget(input, chunks[1]);

    if matches!(app.mode, Mode::Insert) {
        f.set_cursor_position((chunks[1].x + app.input.len() as u16 + 1, chunks[1].y + 1));
    }

    let help = match app.mode {
        Mode::Normal => "  a/i: add  j/k: move  space/enter: toggle  d: delete  q: quit",
        Mode::Insert => "  typing... enter to save, esc to cancel",
    };
    let help_widget = Paragraph::new(help).style(Style::default().fg(Color::DarkGray));
    f.render_widget(help_widget, chunks[2]);
}
