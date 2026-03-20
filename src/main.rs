use anyhow::Context;
use erldash::{metrics, ui, Command, ReplayArgs, RunArgs};

struct Args {
    command: Command,
    logfile: Option<std::path::PathBuf>,
    loglevel: simplelog::LevelFilter,
    truncate_log: bool,
}

fn main() -> noargs::Result<()> {
    let args = match parse_args()? {
        Some(args) => args,
        None => return Ok(()),
    };

    run(args).map_err(noargs::Error::from)
}

fn run(args: Args) -> anyhow::Result<()> {
    setup_logger(args.logfile.as_deref(), args.loglevel, args.truncate_log)?;

    let poller = metrics::MetricsPoller::start_thread(args.command)?;
    let app = ui::App::new(poller)?;
    app.run()?;
    Ok(())
}

fn parse_args() -> noargs::Result<Option<Args>> {
    let mut args = noargs::raw_args();
    args.metadata_mut().app_name = env!("CARGO_PKG_NAME");
    args.metadata_mut().app_description = env!("CARGO_PKG_DESCRIPTION");

    if noargs::VERSION_FLAG.take(&mut args).is_present() {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        return Ok(None);
    }
    noargs::HELP_FLAG.take_help(&mut args);

    // Hidden options (parsed before subcommands).
    let logfile: Option<std::path::PathBuf> = noargs::opt("logfile")
        .ty("FILE")
        .doc("Log file path")
        .take(&mut args)
        .present_and_then(|o| Ok::<_, std::convert::Infallible>(o.value().into()))?;
    let loglevel: simplelog::LevelFilter = noargs::opt("loglevel")
        .doc("Log level")
        .default("info")
        .take(&mut args)
        .then(|o| o.value().parse::<simplelog::LevelFilter>())?;
    let truncate_log = noargs::flag("truncate-log")
        .doc("Truncate the log file instead of appending")
        .take(&mut args)
        .is_present();

    // Subcommands.
    let mut command = None;
    if command.is_none() {
        command = try_parse_run(&mut args)?;
    }
    if command.is_none() {
        command = try_parse_replay(&mut args)?;
    }

    if let Some(help) = args.finish()? {
        print!("{help}");
        return Ok(None);
    }

    let command = command.expect("unreachable: a command should have been parsed");
    Ok(Some(Args {
        command,
        logfile,
        loglevel,
        truncate_log,
    }))
}

fn try_parse_run(args: &mut noargs::RawArgs) -> noargs::Result<Option<Command>> {
    if !noargs::cmd("run")
        .doc("Run the dashboard")
        .take(args)
        .is_present()
    {
        return Ok(None);
    }

    let polling_interval: std::num::NonZeroUsize = noargs::opt("polling-interval")
        .short('i')
        .ty("SECONDS")
        .doc("Erlang metrics polling interval (in seconds)")
        .default("1")
        .take(args)
        .then(|o| o.value().parse())?;
    let cookie: Option<String> = noargs::opt("cookie")
        .short('c')
        .doc("Erlang cookie (default: content of $HOME/.erlang.cookie)")
        .take(args)
        .present_and_then(|o| Ok::<_, std::convert::Infallible>(o.value().to_owned()))?;
    let record: Option<std::path::PathBuf> = noargs::opt("record")
        .ty("FILE")
        .doc("Record collected metrics to a file for later replay")
        .take(args)
        .present_and_then(|o| Ok::<_, std::convert::Infallible>(o.value().into()))?;
    let port: Option<u16> = noargs::opt("port")
        .short('p')
        .doc("Port number on which the target node listens (bypasses EPMD)")
        .take(args)
        .present_and_then(|o| o.value().parse())?;
    let erlang_node: erl_dist::node::NodeName = noargs::arg("<ERLANG_NODE>")
        .doc("Target Erlang node name")
        .example("foo@localhost")
        .take(args)
        .then(|a| a.value().parse())?;

    if args.metadata().help_mode {
        return Ok(None);
    }

    Ok(Some(Command::Run(RunArgs {
        erlang_node,
        polling_interval,
        cookie,
        record,
        port,
    })))
}

fn try_parse_replay(args: &mut noargs::RawArgs) -> noargs::Result<Option<Command>> {
    if !noargs::cmd("replay")
        .doc("Replay a previously recorded dashboard session")
        .take(args)
        .is_present()
    {
        return Ok(None);
    }

    let file: std::path::PathBuf = noargs::arg("<FILE>")
        .doc("Path to a file containing recorded metrics")
        .example("recording.jsonl")
        .take(args)
        .then(|a| Ok::<_, std::convert::Infallible>(a.value().into()))?;

    if args.metadata().help_mode {
        return Ok(None);
    }

    Ok(Some(Command::Replay(ReplayArgs { file })))
}

fn setup_logger(
    logfile: Option<&std::path::Path>,
    loglevel: simplelog::LevelFilter,
    truncate_log: bool,
) -> anyhow::Result<()> {
    if let Some(logfile) = logfile {
        let file = std::fs::OpenOptions::new()
            .append(!truncate_log)
            .truncate(truncate_log)
            .create(true)
            .write(true)
            .open(logfile)
            .with_context(|| format!("failed to open log file {:?}", logfile))?;
        simplelog::WriteLogger::init(loglevel, Default::default(), file)?;
    }
    Ok(())
}
