use clap::Parser;
use erl_dist::term::{Atom, FixInteger, List, Map, Term};
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
        find_cookie()?
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
        let client = erl_rpc::RpcClient::connect(&args.erlang_node.to_string(), &cookie).await?;
        let handle = client.handle();
        smol::spawn(async {
            if let Err(e) = client.run().await {
                eprintln!("RpcClient Error: {}", e);
            }
        })
        .detach();

        let mut app = App {
            history: VecDeque::new(),
            interval,
        };
        let mut msacc = Msacc::start(handle).await?;
        loop {
            if crossterm::event::poll(std::time::Duration::from_secs(0))? {
                if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                    if let crossterm::event::KeyCode::Char('q') = key.code {
                        return Ok(());
                    }
                }
            }

            let data = msacc.get_stats(interval).await?;
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

fn find_cookie() -> anyhow::Result<String> {
    if let Some(dir) = dirs::home_dir().filter(|dir| dir.join(".erlang.cookie").exists()) {
        let cookie = std::fs::read_to_string(dir.join(".erlang.cookie"))?;
        Ok(cookie)
    } else {
        anyhow::bail!("Could not find the cookie file $HOME/.erlang.cookie. Please specify `-cookie` arg instead.");
    }
}

const TIME_UNIT: u32 = 1_000_000; // us

#[derive(Debug)]
struct Msacc {
    rpc: erl_rpc::RpcClientHandle,
}

impl Msacc {
    async fn start(mut rpc: erl_rpc::RpcClientHandle) -> anyhow::Result<Self> {
        Ok(Self { rpc })
    }

    async fn get_stats(&mut self, interval: Duration) -> anyhow::Result<MsaccData> {
        self.rpc
            .call(
                "msacc".into(),
                "start".into(),
                List::from(vec![FixInteger::from(interval.as_millis() as i32).into()]),
            )
            .await?;

        let stats = self
            .rpc
            .call("msacc".into(), "stats".into(), List::nil())
            .await?;
        MsaccData::from_term(stats, TIME_UNIT)
    }
}

#[derive(Debug)]
struct MsaccDataThread {
    id: i32,
    ty: MsaccType,
    counters: MsaccCounters,
}

impl MsaccDataThread {
    fn from_term(term: Term, time_unit: u32) -> anyhow::Result<Self> {
        let mut map: Map = term.try_into().map_err(|_| anyhow::anyhow!("not a map"))?;
        let id: FixInteger = remove_map_entry(&mut map, "id")?;

        let ty: Atom = remove_map_entry(&mut map, "type")?;
        let ty = match ty.name.as_str() {
            "scheduler" => MsaccType::Scheduler,
            "aux" => MsaccType::Aux,
            "async" => MsaccType::Async,
            "dirty_cpu_scheduler" => MsaccType::DirtyCpuScheduler,
            "dirty_io_scheduler" => MsaccType::DirtyIoScheduler,
            "poll" => MsaccType::Poll,
            ty => anyhow::bail!("unknown msacc type {:?}", ty),
        };

        let msacc_data_counters: Map = remove_map_entry(&mut map, "counters")?;
        let mut counters = MsaccCounters::default();
        counters.count = 1;
        for (k, v) in msacc_data_counters.entries {
            let v: u64 = match v {
                Term::FixInteger(v) => v.value.try_into()?,
                Term::BigInteger(v) => v.value.try_into()?,
                _ => anyhow::bail!("not an integer"),
            };
            let v = Duration::from_secs_f64(v as f64 / time_unit as f64);

            let k: Atom = k.try_into().map_err(|_| anyhow::anyhow!("not an atom"))?;
            match k.name.as_str() {
                "alloc" => counters.alloc += v,
                "aux" => counters.aux += v,
                "bif" => counters.bif += v,
                "busy_wait" => counters.busy_wait += v,
                "check_io" => counters.check_io += v,
                "emulator" => counters.emulator += v,
                "ets" => counters.ets += v,
                "gc" => counters.gc += v,
                "gc_fullsweep" => counters.gc_fullsweep += v,
                "nif" => counters.nif += v,
                "other" => counters.other += v,
                "port" => counters.port += v,
                "send" => counters.send += v,
                "sleep" => counters.sleep += v,
                "timers" => counters.timers += v,
                k => anyhow::bail!("unknown msacc state {:?}", k),
            }
        }
        Ok(Self {
            id: id.value,
            ty,
            counters,
        })
    }
}

#[derive(Debug, Clone, Default)]
struct MsaccCounters {
    alloc: Duration,
    aux: Duration,
    bif: Duration,
    busy_wait: Duration,
    check_io: Duration,
    emulator: Duration,
    ets: Duration,
    gc: Duration,
    gc_fullsweep: Duration,
    nif: Duration,
    other: Duration,
    port: Duration,
    send: Duration,
    sleep: Duration,
    timers: Duration,
    count: usize,
}

impl MsaccCounters {
    fn realtime_sum(&self) -> Duration {
        self.alloc
            + self.aux
            + self.bif
            + self.busy_wait
            + self.check_io
            + self.emulator
            + self.ets
            + self.gc
            + self.gc_fullsweep
            + self.nif
            + self.other
            + self.port
            + self.send
            + self.sleep
            + self.timers
    }

    fn runtime_sum(&self) -> Duration {
        self.alloc
            + self.aux
            + self.bif
            + self.busy_wait
            + self.check_io
            + self.emulator
            + self.ets
            + self.gc
            + self.gc_fullsweep
            + self.nif
            + self.other
            + self.port
            + self.send
            + self.timers
    }

    fn runtime_sum_avg(&self) -> Duration {
        self.runtime_sum() / self.count as u32
    }

    fn merge(&mut self, other: &Self) {
        self.alloc += other.alloc;
        self.aux += other.aux;
        self.bif += other.bif;
        self.busy_wait += other.busy_wait;
        self.check_io += other.check_io;
        self.emulator += other.emulator;
        self.ets += other.ets;
        self.gc += other.gc;
        self.gc_fullsweep += other.gc_fullsweep;
        self.nif += other.nif;
        self.other += other.other;
        self.port += other.port;
        self.send += other.send;
        self.sleep += other.sleep;
        self.timers += other.timers;
        self.count += other.count;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum MsaccType {
    Scheduler,
    Aux,
    Async,
    DirtyCpuScheduler,
    DirtyIoScheduler,
    Poll,
}

#[derive(Debug, Clone, Default)]
struct MsaccTypeStats {
    scheduler: MsaccCounters,
    aux: MsaccCounters,
    r#async: MsaccCounters,
    dirty_cpu_scheduler: MsaccCounters,
    dirty_io_scheduler: MsaccCounters,
    poll: MsaccCounters,
}

#[derive(Debug)]
struct MsaccData {
    pub threads: Vec<MsaccDataThread>,
}

impl MsaccData {
    fn from_term(term: Term, time_unit: u32) -> anyhow::Result<Self> {
        let list: List = term.try_into().map_err(|_| anyhow::anyhow!("not a list"))?;
        let mut threads = Vec::new();
        for msacc_data_thread in list.elements {
            threads.push(MsaccDataThread::from_term(msacc_data_thread, time_unit)?);
        }
        Ok(Self { threads })
    }

    fn system_realtime(&self) -> Duration {
        self.threads.iter().map(|t| t.counters.realtime_sum()).sum()
    }

    fn system_runtime(&self) -> Duration {
        self.threads.iter().map(|t| t.counters.runtime_sum()).sum()
    }

    fn type_stats(&self) -> MsaccTypeStats {
        let mut stats = MsaccTypeStats::default();
        for thread in &self.threads {
            match thread.ty {
                MsaccType::Scheduler => {
                    stats.scheduler.merge(&thread.counters);
                }
                MsaccType::Async => {
                    stats.r#async.merge(&thread.counters);
                }
                MsaccType::Aux => {
                    stats.aux.merge(&thread.counters);
                }
                MsaccType::DirtyCpuScheduler => {
                    stats.dirty_cpu_scheduler.merge(&thread.counters);
                }
                MsaccType::DirtyIoScheduler => {
                    stats.dirty_io_scheduler.merge(&thread.counters);
                }
                MsaccType::Poll => {
                    stats.poll.merge(&thread.counters);
                }
            }
        }
        stats
    }
}

fn remove_map_entry<T>(map: &mut Map, key: &str) -> anyhow::Result<T>
where
    Term: TryInto<T, Error = Term>,
{
    let pos = map
        .entries
        .iter()
        .position(|(k, _)| {
            if let Term::Atom(k) = k {
                k.name == key
            } else {
                false
            }
        })
        .ok_or_else(|| anyhow::anyhow!("no such key: {:?}", key))?;
    let (_, v) = map.entries.swap_remove(pos);
    v.try_into()
        .map_err(|t| anyhow::anyhow!("unexpected term type: {}", t))
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
    let chunks = tui::layout::Layout::default()
        .direction(tui::layout::Direction::Horizontal)
        .constraints(
            [
                tui::layout::Constraint::Percentage(80),
                tui::layout::Constraint::Percentage(20),
            ]
            .as_ref(),
        )
        .split(size);
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
