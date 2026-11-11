//! `kkt-fake-product`: the fake product with two swarms, served on a
//! loopback port to look at by hand. For development only, never released.
//!
//! Usage: `kkt-fake-product <database file>`, a file that does not exist
//! yet. It prints the address it is bound to and the made-up bearers it
//! knows, and stops on SIGINT or SIGTERM.

use kkt_clock::SystemClock;
use kkt_config::{DB_BUSY_TIMEOUT, DB_READERS};
use kkt_db_sqlite::{Options, SqlitePlace};
use kkt_fake_product::{Bearers, Product, Settings};
use kkt_names::OsEntropy;
use kkt_shutdown::StopSignals;
use kkt_store::Db;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("kkt-fake-product: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: &[String]) -> Result<(), String> {
    let [path] = args else {
        return Err("usage: kkt-fake-product <database file>".into());
    };
    let path = PathBuf::from(path);
    if path.exists() {
        return Err(format!("{} exists; give a new file", path.display()));
    }
    let place = SqlitePlace::new(
        &path,
        Options {
            readers: DB_READERS,
            busy_timeout: DB_BUSY_TIMEOUT,
        },
    )
    .open(&[kkt_store::schema()])
    .map_err(|e| format!("could not open {}: {e}", path.display()))?;
    let signals =
        StopSignals::listen().map_err(|e| format!("could not listen for stop signals: {e}"))?;
    let product = Product::start(
        Db::new(place),
        Arc::new(SystemClock),
        Arc::new(OsEntropy),
        Settings::DEFAULT,
    )
    .await;
    println!("kkt-fake-product bound to {}", product.address);
    println!(
        "agents' bearers: {} (sw_main), {} (sw_other)",
        Bearers::MAIN_AGENTS,
        Bearers::OTHER_AGENTS
    );
    println!(
        "operators' bearers, to sign in to the web page with: {} (sw_main), {} (sw_other)",
        Bearers::MAIN_OPERATOR,
        Bearers::OTHER_OPERATOR
    );
    println!(
        "admins' bearers, for /mcp-admin: {} (sw_main), {} (sw_other)",
        Bearers::MAIN_ADMIN,
        Bearers::OTHER_ADMIN
    );
    match signals.stop_on_signal(product.shutdown()).await {
        Ok(signal) => println!("received {signal}; stopping"),
        Err(e) => return Err(format!("could not listen for stop signals: {e}")),
    }
    product.stopped().await;
    Ok(())
}
