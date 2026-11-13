//! The web page with simulated data: no database, no MCP, no agents. For
//! working on the page's look and live behavior. It assembles kkt-web's page
//! over kkt-api-sim's simulated application and stand-in gate, with the real
//! clock, randomness and stop signals.
//!
//! ```text
//! cargo run -p kkt-web-preview -- --bind 127.0.0.1:7421 --seed 7 --rate 60 --page-messages 30
//! ```
//!
//! The same seed gives the same world and the same feed. Pass
//! `--access-key-file` to see the login page as well.

mod settings;

use clap::Parser;
use kkt_api_sim::{Preview, PreviewGate, PreviewSettings};
use kkt_clock_system::SystemClock;
use kkt_entropy::SeededEntropy;
use kkt_entropy_os::OsEntropy;
use kkt_hosts::AllowedHosts;
use kkt_log::{Level, Log};
use kkt_log_tracing::TracingLog;
use kkt_logs::LogLevel;
use kkt_server_keys::KeyFiles;
use kkt_shutdown::Shutdown;
use kkt_shutdown::StopSignals;
use kkt_web::{PageLimits, Sessions};
use settings::{PREVIEW_BIND, PREVIEW_SEED};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(about = "The kashkeshet web page with simulated data")]
struct Args {
    #[arg(long, help = format!(
        "Address to listen on [default: {PREVIEW_BIND}]. A non-loopback address, such as \
         0.0.0.0:3013, lets other machines see the preview; it has only simulated data"
    ))]
    bind: Option<SocketAddr>,
    #[arg(long, help = format!("Seed for the simulated world and feed [default: {PREVIEW_SEED}]"))]
    seed: Option<u64>,
    #[arg(long, help = format!(
        "Steps of the simulated feed per minute; most are one message [default: {}]",
        PreviewSettings::DEFAULT.steps_per_minute
    ))]
    rate: Option<u32>,
    #[arg(long, help = format!(
        "The most messages one page shows; small values show the live view dropping old ones [default: {}]",
        PageLimits::DEFAULT.page_messages
    ))]
    page_messages: Option<usize>,
    /// A host name to answer to besides loopback. Repeat for more.
    #[arg(long = "allow-host")]
    allow_hosts: Vec<String>,
    /// Answer to any host name, for viewing from another machine. Only the
    /// preview has this; it serves only simulated data.
    #[arg(long, conflicts_with = "allow_hosts")]
    any_host: bool,
    /// A file holding an access key, to preview the login page.
    #[arg(long)]
    access_key_file: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let log = TracingLog;
    // With no log there is nowhere to say why, so the exit code says the
    // preview did not run.
    if kkt_log_tracing::init_stderr_logging(LogLevel::Info).is_err() {
        return ExitCode::FAILURE;
    }
    match run(Args::parse(), &log).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log.record(Level::Error, module_path!(), format_args!("{e}"), &[]);
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args, log: &TracingLog) -> Result<(), String> {
    let invalid = |e: settings::SettingError| format!("invalid setting: {e}");
    let bind = args.bind.unwrap_or(PREVIEW_BIND);
    let seed = args.seed.unwrap_or(PREVIEW_SEED);
    let simulation = settings::simulation(
        args.rate
            .unwrap_or(PreviewSettings::DEFAULT.steps_per_minute),
    )
    .map_err(invalid)?;
    let page = settings::page(
        args.page_messages
            .unwrap_or(PageLimits::DEFAULT.page_messages),
    )
    .map_err(invalid)?;
    // Unlike the server, the preview may listen beyond loopback: it holds no
    // real data. Say so when it does.
    if !bind.ip().is_loopback() {
        log.record(
            Level::Warn,
            module_path!(),
            format_args!(
                "listening on {bind}, beyond this machine; the preview has only simulated data"
            ),
            &[],
        );
    }
    let hosts = if args.any_host {
        AllowedHosts::any()
    } else {
        AllowedHosts::new(&args.allow_hosts).map_err(|e| format!("invalid setting: {e}"))?
    };
    let clock = Arc::new(SystemClock);
    let gate = match &args.access_key_file {
        Some(path) => {
            let text = KeyFiles::read(path).map_err(|e| e.to_string())?;
            PreviewGate::with_key(&text).map_err(|e| invalid(e.into()))?
        }
        None => PreviewGate::open(),
    };
    let web_settings = kkt_web::Settings {
        page,
        ..kkt_web::Settings::DEFAULT
    };
    let sessions = Sessions::new(
        clock.clone(),
        Arc::new(OsEntropy),
        web_settings.session_lifetime,
        gate.access(),
    );

    let shutdown = Shutdown::new();
    let preview = Arc::new(
        Preview::new(
            clock,
            Arc::new(SeededEntropy::new(seed)),
            simulation.clone(),
            &shutdown,
        )
        .map_err(|e| invalid(e.into()))?,
    );
    let feed = {
        let preview = preview.clone();
        let shutdown = shutdown.clone();
        tokio::spawn(async move { preview.run(shutdown).await })
    };
    let app = kkt_server::page(
        preview,
        hosts,
        Arc::new(gate),
        Arc::new(sessions),
        Arc::new(TracingLog),
        web_settings,
    );

    let signals =
        StopSignals::listen().map_err(|e| format!("could not listen for shutdown signals: {e}"))?;
    let listening = kkt_server_listen::listen(app, bind)
        .await
        .map_err(|e| format!("could not listen on {bind}: {e}"))?;
    let bound = listening
        .address()
        .map_err(|e| format!("could not read the address listened on: {e}"))?;
    log.record(
        Level::Info,
        module_path!(),
        format_args!(
            "kashkeshet web preview listening: http://{bound}/ (seed {seed}, {} steps a minute)",
            simulation.steps_per_minute
        ),
        &[],
    );
    let served = listening
        .serve_until(
            {
                let shutdown = shutdown.clone();
                async move {
                    match signals.stop_on_signal(shutdown).await {
                        Ok(signal) => TracingLog.record(
                            Level::Info,
                            module_path!(),
                            format_args!("received {signal}; shutting down"),
                            &[],
                        ),
                        Err(e) => TracingLog.record(
                            Level::Error,
                            module_path!(),
                            format_args!("could not listen for shutdown signals: {e}"),
                            &[],
                        ),
                    }
                }
            },
            |e| {
                TracingLog.record(
                    Level::Warn,
                    module_path!(),
                    format_args!("could not set TCP_NODELAY on a connection: {e}"),
                    &[],
                )
            },
        )
        .await
        .map_err(|e| format!("server error: {e}"));
    shutdown.stop();
    feed.await
        .map_err(|e| format!("the simulated feed failed: {e}"))?;
    served
}
