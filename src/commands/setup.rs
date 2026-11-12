//! Supplies native parts to the operator's setup and switch commands.
use crate::machine::settings::ServeArgs;
use kkt_machine::{Env, Platform};
pub use kkt_operate::setup::InstallOptions;
use kkt_setup as setup;

pub async fn run(
    env: &Env,
    platform: Platform,
    serve_args: ServeArgs,
    answers: setup::Answers,
    options: InstallOptions,
) -> Result<(), String> {
    let native =
        crate::machine::operate::NativeOperate::new(env, platform).with_service_args(serve_args);
    kkt_operate::setup::run(env, platform, &native.parts(), answers, options).await
}

pub async fn run_switch(env: &Env, platform: Platform, how: setup::Switch) -> Result<(), String> {
    let native = crate::machine::operate::NativeOperate::new(env, platform);
    kkt_operate::setup::run_switch(&native.parts(), env, how).await
}
