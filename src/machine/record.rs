//! The program supplies native record files and the installed service/container providers.
use crate::machine::container::ContainerHost;
use crate::machine::operate::{NativeOutput, NativeSetupFiles};
use crate::machine::service::ServiceSystem;
use crate::machine::settings::ServeArgs;
use kkt_machine::{Env, Platform};
pub use kkt_operate::record::{DOCKER_KEY_FILE_NAME, docker_folder, docker_url, unfinished_mode};
use kkt_setup as setup;

pub fn read_unrecorded(
    env: &Env,
    platform: Platform,
    serve_args: ServeArgs,
) -> Result<Option<setup::Unrecorded>, (setup::Purpose, String)> {
    let native =
        crate::machine::operate::NativeOperate::new(env, platform).with_service_args(serve_args);
    kkt_operate::record::read_unrecorded(&native.parts(), env, platform)
}

pub fn recorded_setup(
    env: &Env,
    platform: Platform,
    serve_args: ServeArgs,
) -> Result<setup::Record, String> {
    let native =
        crate::machine::operate::NativeOperate::new(env, platform).with_service_args(serve_args);
    kkt_operate::record::recorded_setup(&native.parts(), env, platform)
}

pub fn read_setup(env: &Env) -> Result<Option<setup::Record>, String> {
    kkt_operate::record::read_setup(env, &NativeSetupFiles)
}

pub fn read_unfinished(env: &Env) -> Result<Option<setup::Unfinished>, String> {
    kkt_operate::record::read_unfinished(env, &NativeSetupFiles, &ContainerHost, &ServiceSystem)
}

pub fn check_setup(env: &Env) -> Result<(), String> {
    kkt_operate::record::check_setup(env, &NativeSetupFiles, &ServiceSystem, &NativeOutput)
}

pub fn write_setup_record(env: &Env, record: &setup::Record) -> Result<(), String> {
    kkt_operate::record::write_setup_record(env, &NativeSetupFiles, record)
}
