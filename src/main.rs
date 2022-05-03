use clap::Parser;
use erldash::erlang;
use erldash::erlang::msacc;
use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

#[derive(Debug, Parser)]
struct Args {
    erlang_node: erl_dist::node::NodeName, // TODO: make this optional

    #[clap(long, default_value_t = 1.0)]
    interval: f32,

    #[clap(long)]
    cookie: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let cookie = if let Some(cookie) = &args.cookie {
        cookie.clone()
    } else {
        erlang::find_cookie()?
    };
    let interval = Duration::from_secs_f32(args.interval);

    // Setup terminal.
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = tui::backend::CrosstermBackend::new(stdout);
    let mut terminal = tui::Terminal::new(backend)?;

    let result: anyhow::Result<()> = smol::block_on(async {
        let mut client = erlang::RpcClient::connect(&args.erlang_node, &cookie).await?;

        let mut app = App {
            index: 0,
            history: VecDeque::new(),
            memory: Default::default(),
            stats: Default::default(),
        };
        loop {
            if crossterm::event::poll(std::time::Duration::from_secs(0))? {
                if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                    use crossterm::event::KeyCode;
                    match key.code {
                        KeyCode::Char('q') => {
                            return Ok(());
                        }
                        KeyCode::Up => {}
                        KeyCode::Down => {}
                        _ => {}
                    }
                }
            }

            let data = client.get_msacc_stats(interval).await?;
            app.history.push_back(data);
            if app.history.len() > 60 {
                // TODO:
                app.history.pop_front();
            }

            app.memory = client.get_memory_stats().await?;
            app.stats = client.get_stats().await?;

            terminal.draw(|f| ui(f, &mut app))?;
        }
    });

    // Restore terminal.
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

#[derive(Debug, Default)]
struct App {
    index: usize,
    history: VecDeque<msacc::MsaccData>,
    memory: erlang::memory::MemoryStats,
    stats: erlang::stats::Stats,
}

impl App {
    fn utilization_per_type(&self) -> BTreeMap<msacc::MsaccThreadType, Vec<(f64, f64)>> {
        let mut result = BTreeMap::<_, Vec<_>>::new();
        for (i, d) in self.history.iter().enumerate() {
            let x = i as f64;
            for (ty, y) in d.get_utilization_per_type() {
                result.entry(ty).or_default().push((x, y));
            }
        }
        result
    }

    fn utilization_per_state(&self) -> BTreeMap<msacc::MsaccThreadState, Vec<(f64, f64)>> {
        let mut result = BTreeMap::<_, Vec<_>>::new();
        for (i, d) in self.history.iter().enumerate() {
            let x = i as f64;
            for (state, y) in d.get_utilization_per_state() {
                result.entry(state).or_default().push((x, y));
            }
        }
        result
    }
}

fn ui<B: tui::backend::Backend>(f: &mut tui::Frame<B>, app: &mut App) {
    let tabs = tui::widgets::Tabs::new(vec!["Stats".into()])
        .select(app.index)
        .block(
            tui::widgets::Block::default()
                .borders(tui::widgets::Borders::ALL)
                .title("Tabs"),
        )
        .style(tui::style::Style::default().fg(tui::style::Color::Cyan))
        .highlight_style(
            tui::style::Style::default()
                .add_modifier(tui::style::Modifier::BOLD)
                .bg(tui::style::Color::Black),
        );
    let size = f.size();
    let chunks = tui::layout::Layout::default()
        .direction(tui::layout::Direction::Vertical)
        .margin(1)
        .constraints(
            [
                tui::layout::Constraint::Length(3),
                tui::layout::Constraint::Min(0),
            ]
            .as_ref(),
        )
        .split(size);
    f.render_widget(tabs, chunks[0]);

    match app.index {
        0 => ui_stats(f, app, chunks[1]),
        _ => unreachable!(),
    };
}

fn ui_stats<B: tui::backend::Backend>(f: &mut tui::Frame<B>, app: &App, area: tui::layout::Rect) {
    let top_chunks = tui::layout::Layout::default()
        .direction(tui::layout::Direction::Vertical)
        .constraints(
            [
                tui::layout::Constraint::Percentage(40),
                tui::layout::Constraint::Percentage(60),
            ]
            .as_ref(),
        )
        .split(area);
    let chunks = tui::layout::Layout::default()
        .direction(tui::layout::Direction::Horizontal)
        .constraints(
            [
                tui::layout::Constraint::Percentage(80),
                tui::layout::Constraint::Percentage(20),
            ]
            .as_ref(),
        )
        .split(top_chunks[0]);

    let stats_chunks = tui::layout::Layout::default()
        .direction(tui::layout::Direction::Horizontal)
        .constraints(
            [
                tui::layout::Constraint::Percentage(40),
                tui::layout::Constraint::Percentage(30),
                tui::layout::Constraint::Percentage(30),
            ]
            .as_ref(),
        )
        .split(top_chunks[1]);

    let util = app.utilization_per_type();
    let datasets = util
        .iter()
        .enumerate()
        .map(|(i, (ty, data))| {
            let color = erldash::color::PALETTE[i % erldash::color::PALETTE.len()];
            tui::widgets::Dataset::default()
                .name(ty)
                .marker(tui::symbols::Marker::Braille)
                .graph_type(tui::widgets::GraphType::Line)
                .style(tui::style::Style::default().fg(color))
                .data(data)
        })
        .collect::<Vec<_>>();

    let chart = tui::widgets::Chart::new(datasets)
        .block(
            tui::widgets::Block::default()
                .title(tui::text::Span::styled(
                    "Utilization per Type",
                    tui::style::Style::default()
                        .fg(tui::style::Color::Cyan)
                        .add_modifier(tui::style::Modifier::BOLD),
                ))
                .borders(tui::widgets::Borders::ALL),
        )
        .x_axis(
            tui::widgets::Axis::default()
                .style(tui::style::Style::default().fg(tui::style::Color::Gray))
                .bounds([0.0, 60.0]),
        )
        .y_axis(
            tui::widgets::Axis::default()
                .title("Utilization (%)")
                .style(tui::style::Style::default().fg(tui::style::Color::Gray))
                .labels(vec![
                    tui::text::Span::styled(
                        "0",
                        tui::style::Style::default().add_modifier(tui::style::Modifier::BOLD),
                    ),
                    tui::text::Span::styled(
                        "50",
                        tui::style::Style::default().add_modifier(tui::style::Modifier::BOLD),
                    ),
                    tui::text::Span::styled(
                        "100",
                        tui::style::Style::default().add_modifier(tui::style::Modifier::BOLD),
                    ),
                ])
                .bounds([0.0, 100.0]),
        );
    f.render_widget(chart, chunks[0]);

    let width = util.keys().map(|k| k.len()).max().unwrap(); // TODO
    let mut items: Vec<tui::widgets::ListItem> = Vec::new();
    items.push(tui::widgets::ListItem::new(
        util.iter()
            .enumerate()
            .map(|(i, (ty, data))| {
                let color = erldash::color::PALETTE[i % erldash::color::PALETTE.len()];
                let s = tui::style::Style::default()
                    .fg(color)
                    .add_modifier(tui::style::Modifier::BOLD);
                let span = tui::text::Span::styled(
                    format!(
                        "{:width$}:{:6.2} %",
                        ty,
                        data.last().unwrap().1,
                        width = width
                    ),
                    s,
                );
                tui::text::Spans::from(vec![span])
            })
            .collect::<Vec<_>>(),
    ));
    let list = tui::widgets::List::new(items)
        .block(
            tui::widgets::Block::default()
                .borders(tui::widgets::Borders::ALL)
                .title("Type"),
        )
        .start_corner(tui::layout::Corner::TopLeft);
    f.render_widget(list, chunks[1]);

    let datasets = util
        .iter()
        .enumerate()
        .map(|(i, (ty, data))| {
            let color = erldash::color::PALETTE[i % erldash::color::PALETTE.len()];
            tui::widgets::Dataset::default()
                .name(ty)
                .marker(tui::symbols::Marker::Braille)
                .graph_type(tui::widgets::GraphType::Line)
                .style(tui::style::Style::default().fg(color))
                .data(data)
        })
        .collect::<Vec<_>>();
    let chart = tui::widgets::Chart::new(datasets)
        .block(
            tui::widgets::Block::default()
                .title(tui::text::Span::styled(
                    "Per Type Utilization(%)",
                    tui::style::Style::default()
                        .fg(tui::style::Color::Cyan)
                        .add_modifier(tui::style::Modifier::BOLD),
                ))
                .borders(tui::widgets::Borders::ALL),
        )
        .x_axis(
            tui::widgets::Axis::default()
                .style(tui::style::Style::default().fg(tui::style::Color::Gray))
                .bounds([0.0, 60.0]),
        )
        .y_axis(
            tui::widgets::Axis::default()
                .style(tui::style::Style::default().fg(tui::style::Color::Gray))
                .labels(vec![
                    tui::text::Span::styled(
                        "0",
                        tui::style::Style::default().add_modifier(tui::style::Modifier::BOLD),
                    ),
                    tui::text::Span::styled(
                        "50",
                        tui::style::Style::default().add_modifier(tui::style::Modifier::BOLD),
                    ),
                    tui::text::Span::styled(
                        "100",
                        tui::style::Style::default().add_modifier(tui::style::Modifier::BOLD),
                    ),
                ])
                .bounds([0.0, 100.0]),
        );
    f.render_widget(chart, chunks[0]);

    //
    let util = app.utilization_per_state();
    let width = util.keys().map(|k| k.len()).max().unwrap(); // TODO
    let mut items: Vec<tui::widgets::ListItem> = Vec::new();
    items.push(tui::widgets::ListItem::new(
        util.iter()
            .enumerate()
            .map(|(i, (ty, data))| {
                let color = erldash::color::PALETTE[i % erldash::color::PALETTE.len()];
                let s = tui::style::Style::default()
                    .fg(color)
                    .add_modifier(tui::style::Modifier::BOLD);
                let span = tui::text::Span::styled(
                    format!(
                        "{:width$}:{:6.2} %",
                        ty,
                        data.last().unwrap().1,
                        width = width
                    ),
                    s,
                );
                tui::text::Spans::from(vec![span])
            })
            .collect::<Vec<_>>(),
    ));
    let list = tui::widgets::List::new(items)
        .block(
            tui::widgets::Block::default()
                .borders(tui::widgets::Borders::ALL)
                .title("State"),
        )
        .start_corner(tui::layout::Corner::TopLeft);
    f.render_widget(list, stats_chunks[2]);

    // memory
    // TODO: use table
    let mut items: Vec<tui::widgets::ListItem> = Vec::new();
    items.push(tui::widgets::ListItem::new(
        app.memory
            .iter()
            .enumerate()
            .map(|(i, (ty, data))| {
                let color = erldash::color::PALETTE[i % erldash::color::PALETTE.len()];
                let s = tui::style::Style::default()
                    .fg(color)
                    .add_modifier(tui::style::Modifier::BOLD);
                let span = tui::text::Span::styled(
                    format!(
                        "{}: {}",
                        ty,
                        byte_unit::Byte::from_bytes(data.into()).get_appropriate_unit(true)
                    ),
                    s,
                );
                tui::text::Spans::from(vec![span])
            })
            .collect::<Vec<_>>(),
    ));
    let list = tui::widgets::List::new(items)
        .block(
            tui::widgets::Block::default()
                .borders(tui::widgets::Borders::ALL)
                .title("Memory"),
        )
        .start_corner(tui::layout::Corner::TopLeft);
    f.render_widget(list, stats_chunks[1]);

    // stats
    // TODO: use table
    let mut items: Vec<tui::widgets::ListItem> = Vec::new();
    items.push(tui::widgets::ListItem::new(
        app.stats
            .iter()
            .enumerate()
            .map(|(i, (ty, data))| {
                let color = erldash::color::PALETTE[i % erldash::color::PALETTE.len()];
                let s = tui::style::Style::default()
                    .fg(color)
                    .add_modifier(tui::style::Modifier::BOLD);
                let span = tui::text::Span::styled(
                    format!(
                        "{}: {}",
                        ty,
                        byte_unit::Byte::from_bytes(data.into()).get_appropriate_unit(true)
                    ),
                    s,
                );
                tui::text::Spans::from(vec![span])
            })
            .collect::<Vec<_>>(),
    ));
    let list = tui::widgets::List::new(items)
        .block(
            tui::widgets::Block::default()
                .borders(tui::widgets::Borders::ALL)
                .title("Statistics"),
        )
        .start_corner(tui::layout::Corner::TopLeft);
    f.render_widget(list, stats_chunks[0]);
}
