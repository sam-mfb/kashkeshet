//! `kkt-fake-product`: the fake product with two swarms, served on a
//! loopback port to look at by hand. For development only, never released.
//!
//! Usage: `kkt-fake-product <database file>`, a file that does not exist
//! yet. It prints the address it is bound to and the made-up bearers it
//! knows, and stops on SIGINT or SIGTERM.

use kkt_clock_system::SystemClock;
use kkt_db_sqlite::{CreateError, Options, SqlitePlace};
use kkt_entropy_os::OsEntropy;
use kkt_fake_product::{Bearers, Product, Settings};
use kkt_rules::Db;
use kkt_shutdown::StopSignals;
use kkt_terminal_os::Terminal;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            Terminal.errors(format_args!("kkt-fake-product: {e}\n"));
            ExitCode::FAILURE
        }
    }
}

async fn run(args: &[String]) -> Result<(), String> {
    let [path] = args else {
        return Err("usage: kkt-fake-product <database file>".into());
    };
    let path = PathBuf::from(path);
    let place = SqlitePlace::new(&path, Options::DEFAULT)
        .create(&[kkt_rules::schema()])
        .map_err(|error| match error {
            CreateError::Exists => format!("{} exists; give a new file", path.display()),
            CreateError::Open(error) => format!("could not open {}: {error}", path.display()),
        })?;
    let signals =
        StopSignals::listen().map_err(|e| format!("could not listen for stop signals: {e}"))?;
    let product = Product::start(
        Db::new(place),
        Arc::new(SystemClock),
        Arc::new(OsEntropy),
        Settings::DEFAULT,
    )
    .await;
    Terminal.output(format_args!(
        "kkt-fake-product bound to {}\n",
        product.address
    ));
    Terminal.output(format_args!(
        "agents' bearers: {} (sw_main), {} (sw_other)\n",
        Bearers::MAIN_AGENTS,
        Bearers::OTHER_AGENTS
    ));
    Terminal.output(format_args!(
        "operators' bearers, to sign in to the web page with: {} (sw_main), {} (sw_other)\n",
        Bearers::MAIN_OPERATOR,
        Bearers::OTHER_OPERATOR
    ));
    Terminal.output(format_args!(
        "admins' bearers, for /mcp-admin: {} (sw_main), {} (sw_other)\n",
        Bearers::MAIN_ADMIN,
        Bearers::OTHER_ADMIN
    ));
    match signals.stop_on_signal(product.shutdown()).await {
        Ok(signal) => Terminal.output(format_args!("received {signal}; stopping\n")),
        Err(e) => return Err(format!("could not listen for stop signals: {e}")),
    }
    product.stopped().await;
    Ok(())
}
