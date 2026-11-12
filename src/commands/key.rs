//! The program wiring for operator key commands.
use crate::machine::keys::KeyKind;
use crate::machine::settings::ServeArgs;
use kkt_machine::{Env, Platform};
pub use kkt_operate::key::NewKey;
use std::path::PathBuf;

pub fn header(
    env: &Env,
    platform: Platform,
    serve_args: ServeArgs,
    path: Option<PathBuf>,
    json: bool,
) -> Result<(), String> {
    let native =
        crate::machine::operate::NativeOperate::new(env, platform).with_service_args(serve_args);
    kkt_operate::key::header(&native.parts(), env, platform, path, json)
}

pub fn show(
    env: &Env,
    platform: Platform,
    serve_args: ServeArgs,
    path: Option<PathBuf>,
    kind: KeyKind,
) -> Result<std::process::ExitCode, String> {
    let native =
        crate::machine::operate::NativeOperate::new(env, platform).with_service_args(serve_args);
    kkt_operate::key::show(&native.parts(), env, platform, path, kind)
}

pub fn create(
    env: &Env,
    platform: Platform,
    serve_args: ServeArgs,
    new: NewKey<'_>,
    force: bool,
    show: bool,
) -> Result<(), String> {
    let native =
        crate::machine::operate::NativeOperate::new(env, platform).with_service_args(serve_args);
    kkt_operate::key::create(&native.parts(), env, platform, new, force, show)
}
