//! An install's lifecycle: `kkt status`, `kkt upgrade`, `kkt uninstall`
//! and `kkt backup`, run by kkt-operate over this program's parts.

use crate::machine::operate::NativeOperate;
use crate::machine::settings::ServeArgs;
use kkt_machine::{Env, Platform};
use std::path::{Path, PathBuf};

pub fn backup(
    env: &Env,
    platform: Platform,
    folder: Option<&Path>,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<(), String> {
    let native = NativeOperate::new(env, platform);
    kkt_operate::lifecycle::backup(&native.parts(), env, folder, db, config)
}

pub async fn status(env: &Env, platform: Platform, serve_args: ServeArgs) -> Result<(), String> {
    let native = NativeOperate::new(env, platform).with_service_args(serve_args);
    kkt_operate::lifecycle::status(&native.parts(), env, platform).await
}

pub async fn upgrade(
    env: &Env,
    platform: Platform,
    serve_args: ServeArgs,
    allow_downgrade: bool,
) -> Result<(), String> {
    let native = NativeOperate::new(env, platform).with_service_args(serve_args);
    kkt_operate::lifecycle::upgrade(&native.parts(), env, platform, allow_downgrade).await
}

pub async fn uninstall(env: &Env, platform: Platform, serve_args: ServeArgs) -> Result<(), String> {
    let native = NativeOperate::new(env, platform).with_service_args(serve_args);
    kkt_operate::lifecycle::uninstall(&native.parts(), env, platform).await
}
