//! Reading the running container's name setting through the Docker host.
//!
//! This reads the config file `kkt serve` in the running container uses,
//! following its `--name-pack` and `--config`. [`super::lane_limit`] reads
//! the container's config differently, and on purpose; its comment says
//! why the two are not one reader.

use super::compose::compose;
use super::{Command, Folder, Host};
use kkt_settings::{self as settings, Env, Limits};
use std::path::{Path, PathBuf};

pub(super) fn selected(host: &dyn Host, folder: &Folder) -> Result<String, String> {
    let id = output(host, &compose(folder, &["ps", "--quiet", "kkt"]))?;
    let id = id.trim();
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("could not identify the kkt container".into());
    }
    let text = output(
        host,
        &Command::docker(vec![
            "inspect".into(),
            "--format".into(),
            "{{json .Config}}".into(),
            id.into(),
        ]),
    )?;
    let config: serde_json::Value =
        serde_json::from_str(&text).map_err(|_| "unreadable container settings")?;
    let args = strings(&config["Cmd"]).ok_or("unreadable container command")?;
    let entry = strings(&config["Entrypoint"]).ok_or("unreadable container entrypoint")?;
    if entry.len() != 1
        || Path::new(entry[0]).file_name().and_then(|s| s.to_str()) != Some("kkt")
        || args.first() != Some(&"serve")
    {
        return Err("container does not run kkt serve directly".into());
    }
    if let Some(pack) = flag(&args, "--name-pack")? {
        return Ok(pack.to_string());
    }
    let vars = strings(&config["Env"]).ok_or("unreadable container environment")?;
    let variable = |name: &str| {
        vars.iter().find_map(|s| {
            let (key, value) = s.split_once('=')?;
            (key == name && !value.is_empty()).then(|| PathBuf::from(value))
        })
    };
    let env = Env {
        home: variable("HOME"),
        xdg_config_home: variable("XDG_CONFIG_HOME"),
        xdg_data_home: variable("XDG_DATA_HOME"),
        claude_config_dir: None,
        codex_home: None,
    };
    let explicit = flag(&args, "--config")?;
    let path = match explicit {
        Some(path) => PathBuf::from(path),
        None => env.config_file().ok_or("container has no config path")?,
    };
    let working = config["WorkingDir"].as_str().unwrap_or("");
    let path = if path.is_absolute() {
        path
    } else {
        Path::new(if working.is_empty() { "/" } else { working }).join(path)
    };
    let path_text = path.to_str().ok_or("unreadable config path")?;
    if explicit.is_none() {
        let exists = host
            .run(&compose(
                folder,
                &["exec", "-T", "kkt", "test", "-e", path_text],
            ))
            .map_err(|e| format!("could not check the container config: {e}"))?;
        match exists.code {
            Some(0) => {}
            Some(1) => return Ok(Limits::DEFAULT.name_pack.name().to_string()),
            _ => return Err("could not check the container config".into()),
        }
    }
    let text = output(
        host,
        &compose(folder, &["exec", "-T", "kkt", "cat", "--", path_text]),
    )?;
    let options = settings::parse_file(
        &text,
        path.parent().unwrap_or(Path::new("/")),
        env.home.as_deref(),
    )
    .map_err(|e| format!("unreadable container config: {e}"))?;
    Ok(options
        .name_pack
        .unwrap_or_else(|| Limits::DEFAULT.name_pack.name().to_string()))
}

fn strings(value: &serde_json::Value) -> Option<Vec<&str>> {
    value.as_array()?.iter().map(|v| v.as_str()).collect()
}

fn flag<'a>(args: &[&'a str], name: &str) -> Result<Option<&'a str>, String> {
    for (index, arg) in args.iter().enumerate() {
        if *arg == name {
            return args
                .get(index + 1)
                .copied()
                .map(Some)
                .ok_or_else(|| "container command has a setting without a value".to_string());
        }
        if let Some((key, value)) = arg.split_once('=')
            && key == name
        {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

fn output(host: &dyn Host, command: &Command) -> Result<String, String> {
    let done = host.run(command).map_err(|_| "container query failed")?;
    if !done.success() {
        // Container settings can contain secrets; do not include command output.
        return Err("container query failed".into());
    }
    Ok(done.stdout)
}
