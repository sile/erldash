use crate::erlang::SystemVersion;
use crate::metrics::{format_u64, MetricValue, Metrics, MetricsPoller};
use crossterm::event::{KeyCode, KeyEvent};
use std::collections::{BTreeMap, VecDeque};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use tui::style::{Modifier, Style};
use tui::symbols::Marker;
use tui::text::{Span, Spans};
use tui::widgets::{
    Axis, Block, Borders, Cell, Chart, Dataset, GraphType, Paragraph, Row, Table, TableState,
};

type Terminal = tui::Terminal<tui::backend::CrosstermBackend<std::io::Stdout>>;
type Frame<'a> = tui::Frame<'a, tui::backend::CrosstermBackend<std::io::Stdout>>;

const ONE_MINUTE: u64 = 60;
const CHART_DURATION: u64 = ONE_MINUTE;
const POLL_TIMEOUT: Duration = Duration::from_millis(10);

pub struct App {
    terminal: Terminal,
    poller: MetricsPoller,
    ui: UiState,
    replay_cursor_time: Duration,
}

impl App {
    pub fn new(poller: MetricsPoller) -> anyhow::Result<Self> {
        let terminal = Self::setup_terminal()?;
        log::debug!("setup terminal");

        let replay_mode = poller.is_replay();
        let system_version = poller.header().system_version.clone();
        let start_time = poller.header().start_time;
        Ok(Self {
            terminal,
            poller,
            ui: UiState::new(system_version, start_time, replay_mode),
            replay_cursor_time: Duration::default(),
        })
    }

    pub fn run(mut self) -> anyhow::Result<()> {
        self.render_replay_ui_if_need()?;
        loop {
            if self.handle_event()? {
                break;
            }
            if self.ui.pause || self.ui.replay_mode {
                std::thread::sleep(POLL_TIMEOUT);
            } else {
                self.handle_poll()?;
            }
        }
        Ok(())
    }

    fn handle_poll(&mut self) -> anyhow::Result<()> {
        match self.poller.poll_metrics(POLL_TIMEOUT) {
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                anyhow::bail!("Erlang metrics polling thread terminated unexpectedly");
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Ok(metrics) => {
                log::debug!("recv new metrics");

                for (name, item) in &metrics.items {
                    if let Some(avg) = self.ui.averages.get_mut(name) {
                        avg.add(item.clone());
                    } else {
                        self.ui
                            .averages
                            .insert(name.clone(), AvgValue::new(item.clone()));
                    }
                }

                let timestamp = metrics.timestamp;
                self.ui.history.push_back(metrics);
                while let Some(metrics) = self.ui.history.pop_front() {
                    let duration = (timestamp - metrics.timestamp).as_secs();
                    if duration <= CHART_DURATION {
                        self.ui.history.push_front(metrics);
                        break;
                    }
                    for (name, item) in metrics.items {
                        self.ui
                            .averages
                            .get_mut(&name)
                            .expect("unreachable")
                            .sub(item.clone());
                    }
                    log::debug!("remove old metrics");
                }
                self.ui.elapsed = self.ui.start.elapsed();
                self.render_ui()?;
            }
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
                    self.render_ui()?;
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
            KeyCode::Char('p') => {
                self.ui.pause = !self.ui.pause;
            }
            KeyCode::Char('h') => {
                self.replay_cursor_time = self
                    .replay_cursor_time
                    .saturating_sub(Duration::from_secs(1));
                self.render_replay_ui_if_need()?;
            }
            KeyCode::Char('l') => {
                if (self.replay_cursor_time + Duration::from_secs(1))
                    < self.poller.replay_last_time()
                {
                    self.replay_cursor_time += Duration::from_secs(1);
                    self.render_replay_ui_if_need()?;
                }
            }
            KeyCode::Left => {
                self.ui.focus = Focus::Main;
            }
            KeyCode::Right => {
                self.ui.focus = Focus::Sub;
            }
            KeyCode::Up => {
                let table = if self.ui.focus == Focus::Main {
                    &mut self.ui.metrics_table_state
                } else {
                    &mut self.ui.detail_table_state
                };

                let i = table.selected().unwrap_or(0).saturating_sub(1);
                table.select(Some(i));
            }
            KeyCode::Down => {
                let table = if self.ui.focus == Focus::Main {
                    &mut self.ui.metrics_table_state
                } else {
                    &mut self.ui.detail_table_state
                };

                let i = table.selected().unwrap_or(0) + 1;
                table.select(Some(i));
            }
            _ => {
                return Ok(false);
            }
        }
        self.render_ui()?;
        Ok(false)
    }

    fn render_ui(&mut self) -> anyhow::Result<()> {
        if !self.ui.history.is_empty() {
            self.terminal.draw(|f| self.ui.render(f))?;
        }
        Ok(())
    }

    fn render_replay_ui_if_need(&mut self) -> anyhow::Result<()> {
        if !self.ui.replay_mode {
            return Ok(());
        }

        let time = self.replay_cursor_time;

        self.ui.history.clear();
        for metrics in self
            .poller
            .get_metrics_range(time, time + Duration::from_secs(CHART_DURATION))?
        {
            self.ui.history.push_back(metrics.clone());
        }

        self.ui.averages.clear();
        for metrics in self.poller.get_metrics_range(
            time.saturating_sub(Duration::from_secs(CHART_DURATION)),
            time,
        )? {
            for (name, item) in &metrics.items {
                if let Some(avg) = self.ui.averages.get_mut(name) {
                    avg.add(item.clone());
                } else {
                    self.ui
                        .averages
                        .insert(name.clone(), AvgValue::new(item.clone()));
                }
            }
        }
        self.ui.elapsed = self
            .ui
            .history
            .back()
            .map(|x| x.timestamp)
            .unwrap_or_default();

        self.render_ui()?;
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
    start: Instant,
    system_version: SystemVersion,
    start_time: chrono::DateTime<chrono::Local>,
    elapsed: Duration,
    pause: bool,
    history: VecDeque<Metrics>,
    averages: BTreeMap<String, AvgValue>,
    focus: Focus,
    metrics_table_state: TableState,
    detail_table_state: TableState,
    replay_mode: bool,
}

impl UiState {
    fn new(
        system_version: SystemVersion,
        start_time: chrono::DateTime<chrono::Local>,
        replay_mode: bool,
    ) -> Self {
        Self {
            start: Instant::now(),
            system_version,
            start_time,
            elapsed: Duration::default(),
            pause: false,
            history: VecDeque::new(),
            averages: BTreeMap::new(),
            focus: Focus::Main,
            metrics_table_state: TableState::default(),
            detail_table_state: TableState::default(),
            replay_mode,
        }
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
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(75), Constraint::Percentage(25)].as_ref())
            .split(area);

        let paragraph = Paragraph::new(vec![Spans::from(self.system_version.get())])
            .block(self.make_block("System Version"))
            .alignment(Alignment::Left);
        f.render_widget(paragraph, chunks[0]);

        let now = self.start_time + self.elapsed;
        let paragraph = Paragraph::new(vec![Spans::from(
            now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        )])
        .block(self.make_block("Time"))
        .alignment(Alignment::Left);
        f.render_widget(paragraph, chunks[1]);
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
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(5)].as_ref())
            .split(area);
        self.render_metrics(f, chunks[0]);
        self.render_help(f, chunks[1]);
    }

    fn render_metrics(&mut self, f: &mut Frame, area: Rect) {
        let block = if self.replay_mode {
            self.make_block("Metrics (REPLAY)")
        } else if self.pause {
            self.make_block("Metrics (PAUSED)")
        } else {
            self.make_block("Metrics")
        };

        let header_cells = ["Name", "Value", "Avg (1m)"]
            .into_iter()
            .map(|h| Cell::from(h).style(Style::default().add_modifier(Modifier::BOLD)));
        let header = Row::new(header_cells).bottom_margin(1);

        let items = self.latest_metrics().root_items().collect::<Vec<_>>();
        let is_avg_available = self.start.elapsed().as_secs() >= ONE_MINUTE;
        let mut value_width = 0;
        let mut avg_width = 0;
        let mut row_items = Vec::with_capacity(items.len());
        for (name, item) in &items {
            let value = item.to_string();
            let avg = if is_avg_available {
                self.averages
                    .get(*name)
                    .map(|v| v.get().to_string())
                    .unwrap_or("".to_string())
            } else {
                "".to_string()
            };
            value_width = std::cmp::max(value_width, value.len());
            avg_width = std::cmp::max(avg_width, avg.len());
            row_items.push((name.to_string(), value, avg));
        }

        let rows = row_items.into_iter().map(|(name, value, avg)| {
            Row::new(vec![
                Cell::from(name),
                Cell::from(format!("{:>value_width$}", value)),
                Cell::from(format!("{:>avg_width$}", avg)),
            ])
        });

        let widths = [
            Constraint::Percentage(50),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ];
        let highlight_style = if self.focus == Focus::Main {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        let selected = std::cmp::min(
            self.metrics_table_state.selected().unwrap_or(0),
            items.len().saturating_sub(1),
        );
        self.metrics_table_state.select(Some(selected));

        let table = Table::new(rows)
            .header(header)
            .block(block)
            .highlight_style(highlight_style)
            .highlight_symbol("> ")
            .widths(&widths);
        f.render_stateful_widget(table, area, &mut self.metrics_table_state);
    }

    fn render_body_right(&mut self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
            .split(area);

        self.render_detail(f, chunks[0]);
        self.render_chart(f, chunks[1]);
    }

    fn render_help(&mut self, f: &mut Frame, area: Rect) {
        let paragraph = if self.replay_mode {
            Paragraph::new(vec![
                Spans::from("Quit:           'q' key"),
                Spans::from("Prev / Next:    'h' / 'l' keys"),
                Spans::from("Move:           UP / DOWN / LEFT / RIGHT keys"),
            ])
        } else {
            Paragraph::new(vec![
                Spans::from("Quit:           'q' key"),
                Spans::from("Pause / Resume: 'p' key"),
                Spans::from("Move:           UP / DOWN / LEFT / RIGHT keys"),
            ])
        }
        .block(self.make_block("Help"))
        .alignment(Alignment::Left);
        f.render_widget(paragraph, area);
    }

    fn chart_data(&self) -> (&str, Vec<(f64, f64)>) {
        let root_metric_name = self
            .latest_metrics()
            .root_items()
            .nth(self.metrics_table_state.selected().unwrap_or(0))
            .expect("unreachable")
            .0;

        let metric_name = match self.focus {
            Focus::Main => root_metric_name,
            Focus::Sub => self
                .latest_metrics()
                .child_items(root_metric_name)
                .nth(self.detail_table_state.selected().unwrap_or(0))
                .map(|(k, _)| k)
                .unwrap_or(root_metric_name),
        };

        let start = self.history[0].timestamp;
        let mut data = Vec::with_capacity(self.history.len());
        for metrics in &self.history {
            let x = (metrics.timestamp - start).as_secs_f64();
            if let Some(y) = metrics.items.get(metric_name).and_then(|x| x.as_f64()) {
                data.push((x, y));
            }
        }
        (metric_name, data)
    }

    fn render_chart(&mut self, f: &mut Frame, area: Rect) {
        let (metric_name, data) = self.chart_data();
        let block = self.make_block(&format!("Chart of {:?}", metric_name));

        if data.is_empty() {
            f.render_widget(block, area);
            return;
        }

        let datasets = vec![Dataset::default()
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .data(&data)];

        let lower_bound = data
            .iter()
            .map(|(_, y)| *y)
            .min_by(|a, b| a.total_cmp(b))
            .expect("unreachable")
            .floor();
        let mut upper_bound = data
            .iter()
            .map(|(_, y)| *y)
            .max_by(|a, b| a.total_cmp(b))
            .expect("unreachable")
            .ceil();
        let is_constant = lower_bound == upper_bound;
        if is_constant {
            upper_bound = lower_bound + 1.0;
        }

        let y_labels = if is_constant {
            vec![
                Span::from(format_u64(lower_bound as u64, "")),
                Span::from(""),
            ]
        } else {
            vec![
                Span::from(format_u64(lower_bound as u64, "")),
                Span::from(format_u64(upper_bound as u64, "")),
            ]
        };

        let chart = Chart::new(datasets)
            .block(block)
            .x_axis(
                Axis::default()
                    .labels(vec![Span::from("0s"), Span::from("60s")])
                    .bounds([0.0, 60.0]),
            )
            .y_axis(
                Axis::default()
                    .labels(y_labels)
                    .bounds([lower_bound, upper_bound]),
            );
        f.render_widget(chart, area);
    }

    fn collect_detailed_items(&self) -> (&str, Vec<(&str, &MetricValue)>) {
        let root_name = self
            .latest_metrics()
            .root_items()
            .nth(self.metrics_table_state.selected().unwrap_or(0))
            .expect("unreachable")
            .0;
        let children = self.latest_metrics().child_items(root_name).collect();
        (root_name, children)
    }

    fn render_detail(&mut self, f: &mut Frame, area: Rect) {
        let (root_metric_name, items) = self.collect_detailed_items();
        let block = self.make_block(&format!("Detail of {:?}", root_metric_name));

        let header_cells = ["Name", "Value", "Avg (1m)"]
            .into_iter()
            .map(|h| Cell::from(h).style(Style::default().add_modifier(Modifier::BOLD)));
        let header = Row::new(header_cells).bottom_margin(1);

        let is_avg_available = self.start.elapsed().as_secs() >= ONE_MINUTE;
        let mut value_width = 0;
        let mut avg_width = 0;
        let mut row_items = Vec::with_capacity(items.len());
        for (name, item) in &items {
            let value = item.to_string();
            let avg = if is_avg_available {
                self.averages
                    .get(*name)
                    .map(|v| v.get().to_string())
                    .unwrap_or("".to_string())
            } else {
                "".to_string()
            };
            value_width = std::cmp::max(value_width, value.len());
            avg_width = std::cmp::max(avg_width, avg.len());
            row_items.push((name.to_string(), value, avg));
        }

        let rows = row_items.into_iter().map(|(name, value, avg)| {
            Row::new(vec![
                Cell::from(name),
                Cell::from(format!("{:>value_width$}", value)),
                Cell::from(format!("{:>avg_width$}", avg)),
            ])
        });

        let widths = [
            Constraint::Percentage(50),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ];

        let highlight_style = if self.focus == Focus::Sub {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        let highlight_symbol = if self.focus == Focus::Sub { "> " } else { "  " };

        let selected = std::cmp::min(
            self.detail_table_state.selected().unwrap_or(0),
            items.len().saturating_sub(1),
        );
        self.detail_table_state.select(Some(selected));

        let table = Table::new(rows)
            .header(header)
            .block(block)
            .highlight_style(highlight_style)
            .highlight_symbol(highlight_symbol)
            .widths(&widths);
        f.render_stateful_widget(table, area, &mut self.detail_table_state);
    }

    fn make_block(&self, name: &str) -> Block<'static> {
        Block::default().borders(Borders::ALL).title(Span::styled(
            name.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ))
    }

    fn latest_metrics(&self) -> &Metrics {
        self.history.back().expect("unreachable")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Focus {
    Main,
    Sub,
}

#[derive(Debug, Clone)]
struct AvgValue {
    sum: MetricValue,
    cnt: usize,
}

impl AvgValue {
    fn new(value: MetricValue) -> Self {
        Self { sum: value, cnt: 1 }
    }

    fn add(&mut self, v: MetricValue) {
        self.sum += v;
        self.cnt += 1;
    }

    fn sub(&mut self, v: MetricValue) {
        self.sum -= v;
        self.cnt -= 1;
    }

    fn get(&self) -> MetricValue {
        match self.sum {
            MetricValue::Gauge { value, .. } => {
                let value = (value as f64 / self.cnt as f64).round() as u64;
                MetricValue::Gauge {
                    value,
                    parent: None,
                }
            }
            MetricValue::Counter {
                value: Some(value), ..
            } => {
                let value = value / self.cnt as f64;
                MetricValue::Counter {
                    raw_value: 0,
                    value: Some(value),
                    parent: None,
                }
            }
            MetricValue::Counter { .. } => MetricValue::Counter {
                raw_value: 0,
                value: None,
                parent: None,
            },
            MetricValue::Utilization { value, .. } => {
                let value = value / self.cnt as f64;
                MetricValue::Utilization {
                    value,
                    parent: None,
                }
            }
        }
    }
}
