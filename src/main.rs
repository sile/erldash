use clap::Parser;
use msacc::erlang;
use msacc::erlang::msacc::MsaccData;
use std::collections::VecDeque;
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
            history: VecDeque::new(),
            interval,
        };
        loop {
            if crossterm::event::poll(std::time::Duration::from_secs(0))? {
                if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                    if let crossterm::event::KeyCode::Char('q') = key.code {
                        return Ok(());
                    }
                }
            }

            let data = client.get_msacc_stats(interval).await?;
            app.history.push_back(data);
            if app.history.len() > 60 {
                // TODO:
                app.history.pop_front();
            }

            terminal.draw(|f| ui(f, &app))?;
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
    history: VecDeque<MsaccData>,
    interval: Duration,
}

impl App {
    fn to_percent(&self, d: Duration) -> f64 {
        d.as_secs_f64() / self.interval.as_secs_f64() * 100.0
    }

    fn utilization(&self) -> Utilization {
        let mut scheduler_data = Vec::new();
        let mut aux_data = Vec::new();
        let mut async_data = Vec::new();
        let mut dirty_cpu_scheduler_data = Vec::new();
        let mut dirty_io_scheduler_data = Vec::new();
        let mut poll_data = Vec::new();

        for (i, d) in self.history.iter().enumerate() {
            let i = i as f64;
            let ts = d.type_stats();
            scheduler_data.push((i, self.to_percent(ts.scheduler.runtime_sum_avg())));
            aux_data.push((i, self.to_percent(ts.aux.runtime_sum_avg())));
            async_data.push((i, self.to_percent(ts.r#async.runtime_sum_avg())));
            dirty_cpu_scheduler_data
                .push((i, self.to_percent(ts.dirty_cpu_scheduler.runtime_sum_avg())));
            dirty_io_scheduler_data
                .push((i, self.to_percent(ts.dirty_io_scheduler.runtime_sum_avg())));
            poll_data.push((i, self.to_percent(ts.poll.runtime_sum_avg())));
        }

        Utilization {
            scheduler_data,
            aux_data,
            async_data,
            dirty_cpu_scheduler_data,
            dirty_io_scheduler_data,
            poll_data,
        }
    }
}

#[derive(Debug)]
struct Utilization {
    scheduler_data: Vec<(f64, f64)>,
    aux_data: Vec<(f64, f64)>,
    async_data: Vec<(f64, f64)>,
    dirty_cpu_scheduler_data: Vec<(f64, f64)>,
    dirty_io_scheduler_data: Vec<(f64, f64)>,
    poll_data: Vec<(f64, f64)>,
}

fn ui<B: tui::backend::Backend>(f: &mut tui::Frame<B>, app: &App) {
    let size = f.size();
    let top_chunks = tui::layout::Layout::default()
        .direction(tui::layout::Direction::Vertical)
        .constraints(
            [
                tui::layout::Constraint::Percentage(80),
                tui::layout::Constraint::Percentage(20),
            ]
            .as_ref(),
        )
        .split(size);
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
    let x_labels = vec![tui::text::Span::styled(
        "Utilization per Type",
        tui::style::Style::default().add_modifier(tui::style::Modifier::BOLD),
    )];

    let util = app.utilization();
    let datasets = vec![
        tui::widgets::Dataset::default()
            .name("Aux")
            .marker(tui::symbols::Marker::Braille)
            .style(tui::style::Style::default().fg(tui::style::Color::Cyan))
            .data(&util.aux_data),
        tui::widgets::Dataset::default()
            .name("Async")
            .marker(tui::symbols::Marker::Braille)
            .style(tui::style::Style::default().fg(tui::style::Color::Blue))
            .data(&util.async_data),
        tui::widgets::Dataset::default()
            .name("Poll")
            .marker(tui::symbols::Marker::Braille)
            .style(tui::style::Style::default().fg(tui::style::Color::White))
            .data(&util.poll_data),
        tui::widgets::Dataset::default()
            .name("Scheduler")
            .marker(tui::symbols::Marker::Braille)
            .style(tui::style::Style::default().fg(tui::style::Color::Yellow))
            .data(&util.scheduler_data)
            .graph_type(tui::widgets::GraphType::Line),
        tui::widgets::Dataset::default()
            .name("Dirty I/O Scheduler")
            .marker(tui::symbols::Marker::Braille)
            .style(tui::style::Style::default().fg(tui::style::Color::Green))
            .data(&util.dirty_io_scheduler_data)
            .graph_type(tui::widgets::GraphType::Line),
        tui::widgets::Dataset::default()
            .name("Dirty CPU Scheduler")
            .marker(tui::symbols::Marker::Braille)
            .style(tui::style::Style::default().fg(tui::style::Color::Gray))
            .data(&util.dirty_cpu_scheduler_data),
    ];

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
                .title("Time")
                .style(tui::style::Style::default().fg(tui::style::Color::Gray))
                .labels(x_labels)
                .bounds([0.0, 60.0]),
        )
        .y_axis(
            tui::widgets::Axis::default()
                .title("%")
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

    let mut items: Vec<tui::widgets::ListItem> = Vec::new();
    items.push(tui::widgets::ListItem::new(vec![
        tui::text::Spans::from(format!(
            "Aux:                {:6.2} %",
            util.aux_data.last().unwrap().1
        )),
        tui::text::Spans::from(format!(
            "Async:              {:6.2} %",
            util.async_data.last().unwrap().1
        )),
        tui::text::Spans::from(format!(
            "Poll:               {:6.2} %",
            util.poll_data.last().unwrap().1
        )),
        tui::text::Spans::from(format!(
            "Scheduler:          {:6.2} %",
            util.scheduler_data.last().unwrap().1
        )),
        tui::text::Spans::from(format!(
            "Dirty I/O Scheduler:{:6.2} %",
            util.dirty_io_scheduler_data.last().unwrap().1
        )),
        tui::text::Spans::from(format!(
            "Dirty CPU Scheduler:{:6.2} %",
            util.dirty_cpu_scheduler_data.last().unwrap().1
        )),
    ]));
    let list = tui::widgets::List::new(items)
        .block(
            tui::widgets::Block::default()
                .borders(tui::widgets::Borders::ALL)
                .title("List"),
        )
        .start_corner(tui::layout::Corner::TopLeft);
    f.render_widget(list, chunks[1]);
}
