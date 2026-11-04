//! `kkt key`: `create` makes a new access, web or admin key, `header`
//! prints the header clients send with the access key, and `show` prints a
//! key.

use crate::machine::keys::{
    KeyKind, admin_key_collision, admin_name_line, local_operator_key, new_key_file,
    optional_key_path, runs_in_docker, selected_key_path, write_admin_name,
};
use crate::machine::settings::ServeArgs;
use kkt_client_config as clients;
use kkt_config as config;
use kkt_settings::Env;
use kkt_setup as setup;
use std::path::{Path, PathBuf};

pub fn header(
    env: &Env,
    serve_args: ServeArgs,
    path: Option<PathBuf>,
    json: bool,
) -> Result<(), String> {
    let path = selected_key_path(env, serve_args, path, KeyKind::Access)?;
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("could not read key file {}: {e}", path.display()))?;
    println!("{}", clients::header(text.trim(), json)?);
    Ok(())
}

/// Prints the selected key. A missing admin key is not an error but has its
/// own exit code, [`config::NO_ADMIN_KEY_EXIT`], so `kkt setup` and
/// `kkt upgrade` can tell it from a key they failed to read.
pub fn show(
    env: &Env,
    serve_args: ServeArgs,
    path: Option<PathBuf>,
    kind: KeyKind,
) -> Result<std::process::ExitCode, String> {
    let Some(path) = optional_key_path(env, serve_args, path, kind)? else {
        eprintln!("there is no admin key; `kkt key create --admin` makes one");
        return Ok(std::process::ExitCode::from(config::NO_ADMIN_KEY_EXIT));
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if kind == KeyKind::Admin && e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "there is no admin key at {}; `kkt key create --admin` makes one",
                path.display()
            );
            return Ok(std::process::ExitCode::from(config::NO_ADMIN_KEY_EXIT));
        }
        Err(e) => return Err(format!("could not read key file {}: {e}", path.display())),
    };
    println!("{}", text.trim());
    if kind == KeyKind::Admin {
        // On standard error, so standard output stays the key alone.
        if let Some(line) = admin_name_line(&path, &text, "admin name")? {
            eprintln!("{line}");
        }
    }
    Ok(std::process::ExitCode::SUCCESS)
}

pub fn create(
    env: &Env,
    serve_args: ServeArgs,
    path: Option<PathBuf>,
    force: bool,
    show: bool,
    kind: KeyKind,
    name: Option<&str>,
) -> Result<(), String> {
    // Checked before anything is written, so a refused key or a bad name
    // leaves no key.
    let in_docker =
        setup::create_asks_for_docker(kind, path.is_some()) && runs_in_docker(env, serve_args)?;
    let name = setup::create_checks(kind, in_docker, name)?;
    if kind == KeyKind::Admin
        && let Some(destination) = path.as_ref()
        && let Some(access) = local_operator_key(env, None, None, KeyKind::Access)?
    {
        // Canonicalize existing files to also catch relative paths and
        // symlinks; a missing destination cannot overwrite an existing key.
        let same = if destination.exists() && access.exists() {
            let resolve = |path: &Path| {
                path.canonicalize()
                    .map_err(|e| format!("could not resolve key file {}: {e}", path.display()))
            };
            resolve(destination)? == resolve(&access)?
        } else {
            let resolve = |path: &Path| {
                std::path::absolute(path)
                    .map_err(|e| format!("could not resolve key file {}: {e}", path.display()))
            };
            resolve(destination)? == resolve(&access)?
        };
        if same {
            return Err(admin_key_collision());
        }
    }
    let path = match kind {
        KeyKind::Access => path
            .or_else(|| env.access_key_file())
            .ok_or("set HOME or pass --path")?,
        _ => optional_key_path(env, serve_args, path, kind)?.ok_or_else(admin_key_collision)?,
    };
    let key = new_key_file(&path, force)?;
    match kind {
        KeyKind::Web => println!(
            "Wrote a new web key to {}. Restart the server to use it and end existing web sessions.",
            path.display()
        ),
        KeyKind::Admin => {
            println!(
                "Wrote a new admin key to {}. Restart the server to use it: /mcp-admin then takes only this key. Read it with `kkt key show --admin`.",
                path.display()
            );
            println!("{}", write_admin_name(&path, &key, name.as_ref())?);
        }
        KeyKind::Access => {}
    }
    if kind != KeyKind::Access {
        if show {
            println!("{key}");
        }
        return Ok(());
    }
    let shown = path.display();
    println!("Wrote a new access key to {shown}");
    println!();
    println!("Start the server with it:");
    println!("  kkt serve --access-key-file {shown}");
    println!("or put this line in the config file:");
    println!("  access_key_file = \"{shown}\"");
    println!();
    println!("Clients send it as the header `Authorization: Bearer <key>`. For example:");
    println!("  export KKT_ACCESS_KEY=\"$(cat {shown})\"");
    if show {
        println!();
        println!("The key:");
        println!("{key}");
    }
    Ok(())
}
