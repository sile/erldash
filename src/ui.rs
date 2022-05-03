use crate::erlang::SystemVersion;
use crate::metrics::MetricsReceiver;
use crossterm::event::{KeyCode, KeyEvent};
use tui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use tui::style::{Modifier, Style};
use tui::text::{Span, Spans};
use tui::widgets::{Block, Borders, Paragraph};

type Terminal = tui::Terminal<tui::backend::CrosstermBackend<std::io::Stdout>>;
type Frame<'a> = tui::Frame<'a, tui::backend::CrosstermBackend<std::io::Stdout>>;

pub struct App {
    terminal: Terminal,
    rx: MetricsReceiver,
    ui: UiState,
}

impl App {
    pub fn new(system_version: SystemVersion, rx: MetricsReceiver) -> anyhow::Result<Self> {
        let terminal = Self::setup_terminal()?;
        log::debug!("setup terminal");
        Ok(Self {
            terminal,
            rx,
            ui: UiState::new(system_version),
        })
    }

    pub fn run(mut self) -> anyhow::Result<()> {
        loop {
            if self.handle_event()? {
                break;
            }
            self.render_ui()?;
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Ok(())
    }

    fn handle_event(&mut self) -> anyhow::Result<bool> {
        while crossterm::event::poll(std::time::Duration::from_secs(0))? {
            match crossterm::event::read()? {
                crossterm::event::Event::Key(key) => {
                    if self.handle_key_event(key)? {
                        return Ok(true);
                    }
                }
                crossterm::event::Event::Resize(_, _) => {
                    self.terminal.draw(|f| self.ui.render(f))?;
                }
                _ => {}
            }
        }
        Ok(false)
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> anyhow::Result<bool> {
        match key.code {
            KeyCode::Char('q') => {
                return Ok(true);
            }
            KeyCode::Char('p') => {}
            KeyCode::Left => {}
            KeyCode::Right => {}
            KeyCode::Up => {}
            KeyCode::Down => {}
            _ => {
                return Ok(false);
            }
        }
        self.render_ui()?;
        Ok(false)
    }

    fn render_ui(&mut self) -> anyhow::Result<()> {
        self.terminal.draw(|f| self.ui.render(f))?;
        Ok(())
    }

    fn setup_terminal() -> anyhow::Result<Terminal> {
        crossterm::terminal::enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen,)?;
        let backend = tui::backend::CrosstermBackend::new(stdout);
        let terminal = tui::Terminal::new(backend)?;
        Ok(terminal)
    }

    fn teardown_terminal(&mut self) -> anyhow::Result<()> {
        crossterm::terminal::disable_raw_mode()?;
        crossterm::execute!(
            self.terminal.backend_mut(),
            crossterm::terminal::LeaveAlternateScreen,
        )?;
        self.terminal.show_cursor()?;
        Ok(())
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if let Err(e) = self.teardown_terminal() {
            log::warn!("failed to tear down terminal: {e}");
        } else {
            log::debug!("tear down terminal");
        }
    }
}

#[derive(Debug)]
struct UiState {
    system_version: SystemVersion,
}

impl UiState {
    fn new(system_version: SystemVersion) -> Self {
        Self { system_version }
    }

    fn render(&mut self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)].as_ref())
            .split(f.size());

        self.render_header(f, chunks[0]);
        self.render_body(f, chunks[1]);
    }

    fn render_header(&mut self, f: &mut Frame, area: Rect) {
        let paragraph = Paragraph::new(vec![Spans::from(self.system_version.get())])
            .block(self.make_block("System Version"))
            .alignment(Alignment::Left);
        f.render_widget(paragraph, area);
    }

    fn render_body(&mut self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
            .split(area);

        self.render_body_left(f, chunks[0]);
        self.render_body_right(f, chunks[1]);
    }

    fn render_body_left(&mut self, f: &mut Frame, area: Rect) {
        self.render_metrics(f, area);
    }

    fn render_metrics(&mut self, f: &mut Frame, area: Rect) {
        let paragraph = Paragraph::new(vec![Spans::from("TODO")])
            .block(self.make_block("Metrics"))
            .alignment(Alignment::Left);
        f.render_widget(paragraph, area);
    }

    fn render_body_right(&mut self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(5)].as_ref())
            .split(area);
        let upper_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
            .split(chunks[0]);

        self.render_detail(f, upper_chunks[0]);
        self.render_chart(f, upper_chunks[1]);
        self.render_help(f, chunks[1]);
    }

    fn render_help(&mut self, f: &mut Frame, area: Rect) {
        let paragraph = Paragraph::new(vec![
            Spans::from("Quit:           'q' key"),
            Spans::from("Pause / Resume: 'p' key"),
            Spans::from("Move:           UP / DOWN / LEFT / RIGHT keys"),
        ])
        .block(self.make_block("Help"))
        .alignment(Alignment::Left);
        f.render_widget(paragraph, area);
    }

    fn render_chart(&mut self, f: &mut Frame, area: Rect) {
        let paragraph = Paragraph::new(vec![Spans::from("TODO")])
            .block(self.make_block("Chart"))
            .alignment(Alignment::Left);
        f.render_widget(paragraph, area);
    }

    fn render_detail(&mut self, f: &mut Frame, area: Rect) {
        let paragraph = Paragraph::new(vec![Spans::from("TODO")])
            .block(self.make_block("Detail"))
            .alignment(Alignment::Left);
        f.render_widget(paragraph, area);
    }

    fn make_block(&self, name: &str) -> Block<'static> {
        Block::default().borders(Borders::ALL).title(Span::styled(
            name.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ))
    }
}
