//! `kkt setup` and `kkt switch`: ask on the terminal if there is one, and
//! do the work through [`steps::OsSteps`], the program's real
//! implementation of setup's steps.

mod steps;

use crate::machine::settings::ServeArgs;
use crate::terminal::ask::TerminalAsk;
use kkt_config::{self as config, DEFAULT_BIND};
use kkt_provenance::Build;
use kkt_settings::Env;
use kkt_setup as setup;
use std::io::IsTerminal;
use steps::OsSteps;

/// `kkt setup`: asks on the terminal if there is one, and does the work
/// through [`OsSteps`].
pub async fn run(
    env: &Env,
    serve_args: ServeArgs,
    answers: setup::Answers,
    options: InstallOptions,
) -> Result<(), String> {
    let ask = TerminalAsk {
        interactive: std::io::stdin().is_terminal(),
    };
    let steps = OsSteps {
        env,
        serve_args,
        options,
    };
    // Steps are synchronous; doctor's checks are async, so they block on
    // this runtime from inside.
    tokio::task::block_in_place(|| {
        setup::run(&answers, Build::CURRENT.version, &ask, &steps).map(|_| ())
    })
}

/// `kkt switch`: asks on the terminal if there is one, and does the work
/// through [`OsSteps`].
pub async fn run_switch(
    env: &Env,
    serve_args: ServeArgs,
    how: setup::Switch,
) -> Result<(), String> {
    let ask = TerminalAsk {
        interactive: std::io::stdin().is_terminal(),
    };
    // Switching installs nothing, so these are never used.
    let options = InstallOptions {
        kkt_version: Build::CURRENT.version.to_string(),
        docker_project: config::DOCKER_PROJECT.to_string(),
        docker_port: DEFAULT_BIND.port(),
    };
    let steps = OsSteps {
        env,
        serve_args,
        options,
    };
    tokio::task::block_in_place(|| {
        setup::switch(&how, Build::CURRENT.version, &ask, &steps).map(|_| ())
    })
}

/// How to install, beyond what setup asks.
pub struct InstallOptions {
    pub kkt_version: String,
    pub docker_project: String,
    pub docker_port: u16,
}
