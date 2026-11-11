//! `kkt setup` and `kkt switch`: ask on the terminal if there is one, and
//! do the work through [`steps::OsSteps`] and [`steps::OsSwitchSteps`], the
//! program's real implementation of setup's steps.

mod steps;

use crate::machine::settings::ServeArgs;
use crate::terminal::TerminalLines;
use kkt_machine::{Env, Platform};
use kkt_provenance::Build;
use kkt_setup as setup;
use kkt_setup::ask::Prompter;
use steps::{OsSteps, OsSwitchSteps};

/// `kkt setup`: asks on the terminal if there is one, and does the work
/// through [`OsSteps`].
pub async fn run(
    env: &Env,
    platform: Platform,
    serve_args: ServeArgs,
    answers: setup::Answers,
    options: InstallOptions,
) -> Result<(), String> {
    let ask = Prompter::new(&TerminalLines);
    let steps = OsSteps {
        env,
        platform,
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
/// through [`OsSwitchSteps`].
pub async fn run_switch(env: &Env, how: setup::Switch) -> Result<(), String> {
    let ask = Prompter::new(&TerminalLines);
    let steps = OsSwitchSteps { env };
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
