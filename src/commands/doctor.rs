//! The program wiring for this machine's diagnostic command.
use crate::machine::connection::ConnectionOptions;
use crate::machine::settings::ServeArgs;
use kkt_machine::{Env, Platform};
pub(crate) use kkt_operate::doctor::server_fix;

pub async fn run(
    env: &Env,
    platform: Platform,
    serve_args: ServeArgs,
    flags: ConnectionOptions,
) -> Result<(), String> {
    let native =
        crate::machine::operate::NativeOperate::new(env, platform).with_service_args(serve_args);
    kkt_operate::doctor::run(&native.parts(), env, platform, flags).await
}
