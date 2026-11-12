//! `kkt key`: `create` makes a new access, web or admin key, `header`
//! prints the header clients send with the access key, and `show` prints a
//! key.

use crate::keys::{
    KeyKind, admin_key_collision, admin_name_line, local_operator_key, new_key_file,
    optional_key_path, runs_in_docker, selected_key_path, write_admin_name,
};
use kkt_client_config as clients;
use kkt_machine::{Env, Platform};
use kkt_setup as setup;
use kkt_setup::connection::ConnectionSource;
use std::path::{Path, PathBuf};

pub fn header(
    parts: &crate::OperateParts<'_, impl ConnectionSource>,
    env: &Env,
    platform: Platform,
    path: Option<PathBuf>,
    json: bool,
) -> Result<(), String> {
    let path = selected_key_path(parts, env, platform, path, KeyKind::Access)?;
    let text = parts
        .key_files
        .read(&path)
        .map_err(|e| format!("could not read key file {}: {}", path.display(), e))?;
    parts
        .output
        .output(format_args!("{}\n", clients::header(text.trim(), json)?));
    Ok(())
}

/// Prints the selected key. A missing admin key is not an error but has its
/// own exit code, [`kkt_container::NO_ADMIN_KEY_EXIT`], so `kkt setup` and
/// `kkt upgrade` can tell it from a key they failed to read.
pub fn show(
    parts: &crate::OperateParts<'_, impl ConnectionSource>,
    env: &Env,
    platform: Platform,
    path: Option<PathBuf>,
    kind: KeyKind,
) -> Result<std::process::ExitCode, String> {
    let Some(path) = optional_key_path(parts, env, platform, path, kind)? else {
        parts.output.errors(format_args!(
            "there is no admin key; `kkt key create --admin` makes one\n"
        ));
        return Ok(std::process::ExitCode::from(
            kkt_container::NO_ADMIN_KEY_EXIT,
        ));
    };
    let text = match parts.key_files.read(&path) {
        Ok(text) => text,
        Err(e) if kind == KeyKind::Admin && e.kind() == std::io::ErrorKind::NotFound => {
            parts.output.errors(format_args!(
                "there is no admin key at {}; `kkt key create --admin` makes one\n",
                path.display()
            ));
            return Ok(std::process::ExitCode::from(
                kkt_container::NO_ADMIN_KEY_EXIT,
            ));
        }
        Err(e) => {
            return Err(format!("could not read key file {}: {}", path.display(), e));
        }
    };
    parts.output.output(format_args!("{}\n", text.trim()));
    if kind == KeyKind::Admin {
        // On standard error, so standard output stays the key alone.
        if let Some(line) = admin_name_line(parts.key_files, &path, &text, "admin name")? {
            parts.output.errors(format_args!("{line}\n"));
        }
    }
    Ok(std::process::ExitCode::SUCCESS)
}

/// The key `kkt key create` makes: its kind, the file to write it to if
/// given, and the admin agent's name if given.
pub struct NewKey<'a> {
    pub kind: KeyKind,
    pub path: Option<PathBuf>,
    pub name: Option<&'a str>,
}

pub fn create(
    parts: &crate::OperateParts<'_, impl ConnectionSource>,
    env: &Env,
    platform: Platform,
    new: NewKey<'_>,
    force: bool,
    show: bool,
) -> Result<(), String> {
    let NewKey { kind, path, name } = new;
    // Checked before anything is written, so a refused key or a bad name
    // leaves no key.
    let in_docker = setup::create_asks_for_docker(kind, path.is_some())
        && runs_in_docker(parts, env, platform)?;
    let name = setup::create_checks(kind, in_docker, name)?;
    if kind == KeyKind::Admin
        && let Some(destination) = path.as_ref()
        && let Some(access) = local_operator_key(parts, env, None, None, KeyKind::Access)?
    {
        // Canonicalize existing files to also catch relative paths and
        // symlinks; a missing destination cannot overwrite an existing key.
        let (destination, access) =
            if parts.key_files.exists(destination) && parts.key_files.exists(&access) {
                let resolve = |path: &Path| {
                    parts
                        .setup_files
                        .canonicalize(path)
                        .map_err(|e| format!("could not resolve key file {}: {e}", path.display()))
                };
                (resolve(destination)?, resolve(&access)?)
            } else {
                let resolve = |path: &Path| {
                    parts
                        .setup_files
                        .absolute(path)
                        .map_err(|e| format!("could not resolve key file {}: {e}", path.display()))
                };
                (resolve(destination)?, resolve(&access)?)
            };
        setup::create_destination_check(&destination, &access)?;
    }
    let path = match kind {
        KeyKind::Access => path
            .or_else(|| parts.serve.default_access_key())
            .ok_or("set HOME or pass --path")?,
        _ => {
            optional_key_path(parts, env, platform, path, kind)?.ok_or_else(admin_key_collision)?
        }
    };
    let key = new_key_file(parts.key_files, &path, force)?;
    match kind {
        KeyKind::Web => parts.output.output(format_args!(
            "Wrote a new web key to {}. Restart the server to use it and end existing web sessions.\n",
            path.display()
        )),
        KeyKind::Admin => {
            parts.output.output(format_args!(
                "Wrote a new admin key to {}. Restart the server to use it: /mcp-admin then takes only this key. Read it with `kkt key show --admin`.\n",
                path.display()
            ));
            parts.output.output(format_args!("{}\n", write_admin_name(parts.key_files, &path, &key, name.as_ref())?));
        }
        KeyKind::Access => {}
    }
    if kind != KeyKind::Access {
        if show {
            parts.output.output(format_args!("{key}\n"));
        }
        return Ok(());
    }
    let shown = path.display();
    parts
        .output
        .output(format_args!("Wrote a new access key to {shown}\n"));
    parts.output.output(format_args!("\n"));
    parts
        .output
        .output(format_args!("Start the server with it:\n"));
    parts
        .output
        .output(format_args!("  kkt serve --access-key-file {shown}\n"));
    parts
        .output
        .output(format_args!("or put this line in the config file:\n"));
    parts
        .output
        .output(format_args!("  access_key_file = \"{shown}\"\n"));
    parts.output.output(format_args!("\n"));
    parts.output.output(format_args!(
        "Clients send it as the header `Authorization: Bearer <key>`. For example:\n"
    ));
    parts
        .output
        .output(format_args!("  export KKT_ACCESS_KEY=\"$(cat {shown})\"\n"));
    if show {
        parts.output.output(format_args!("\n"));
        parts.output.output(format_args!("The key:\n"));
        parts.output.output(format_args!("{key}\n"));
    }
    Ok(())
}
