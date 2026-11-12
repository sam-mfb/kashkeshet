//! `kkt setup` and `kkt switch`: ask on the terminal if there is one, and
//! do the work through [`steps::OsSteps`] and [`steps::OsSwitchSteps`], the
//! assembly's implementation of setup's steps through the supplied providers.

mod steps;

use kkt_machine::{Env, Platform};
use kkt_provenance::Build;
use kkt_setup as setup;
use kkt_setup::ask::Prompter;
use kkt_setup::connection::ConnectionSource;
use steps::{OsSteps, OsSwitchSteps};

/// `kkt setup`: asks on the terminal if there is one, and does the work
/// through [`OsSteps`].
pub async fn run<D: crate::ServiceDefaults>(
    env: &Env,
    platform: Platform,
    parts: &crate::OperateParts<'_, impl ConnectionSource, D>,
    answers: setup::Answers,
    options: InstallOptions,
) -> Result<(), String> {
    let ask = Prompter::new(parts.lines);
    let steps = OsSteps {
        env,
        platform,
        parts,
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
pub async fn run_switch<D>(
    parts: &crate::OperateParts<'_, impl ConnectionSource, D>,
    env: &Env,
    how: setup::Switch,
) -> Result<(), String> {
    let ask = Prompter::new(parts.lines);
    let steps = OsSwitchSteps { env, parts };
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
