//! Running the server's own program in the kkt container, and reading a
//! file under its config home. Each run makes its own folder in the
//! container with `mktemp -d`, carries files in or out of it with `docker
//! compose cp`, and removes only that folder afterwards, so two runs at the
//! same time never see or remove each other's files.

use super::compose::compose;
use super::provision::run;
use super::{Command, DockerError, Folder, Runner};
use kkt_container::{Arg, Carry, InServer};
use kkt_machine::XDG_CONFIG_HOME_DEFAULT;
use std::path::Path;

/// Runs `kkt` in the kkt container with `command`'s arguments, in a new
/// folder whose name starts with its prefix, carrying files in before the
/// run or out after it. Returns what the program printed.
pub fn run_in_server(
    host: &dyn Runner,
    folder: &Folder,
    command: &InServer<'_>,
) -> Result<String, DockerError> {
    let dir = make_dir(host, folder, command.folder_prefix)?;
    let done = match &command.carry {
        Carry::In(files) => copy_in_and_run(host, folder, &dir, command.args, files),
        Carry::Out(dest) => run_and_copy_out(host, folder, &dir, command.args, dest),
    };
    finish(host, folder, &dir, done)
}

fn copy_in_and_run(
    host: &dyn Runner,
    folder: &Folder,
    dir: &str,
    args: &[Arg],
    files: &[std::path::PathBuf],
) -> Result<String, DockerError> {
    for file in files {
        let name =
            file.file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| DockerError::Unexpected {
                    what: "a file's name",
                    detail: file.display().to_string(),
                })?;
        let from = file.display().to_string();
        let to = format!("kkt:{dir}/{name}");
        succeed(host, &compose(folder, &["cp", &from, &to]))?;
    }
    // Copied files keep their host modes, and belong to root in the
    // container; the kkt user must be able to read them.
    succeed(
        host,
        &compose(
            folder,
            &[
                "exec", "-T", "-u", "root", "kkt", "chmod", "-R", "a+rX", dir,
            ],
        ),
    )?;
    run_kkt(host, folder, dir, args)
}

fn run_and_copy_out(
    host: &dyn Runner,
    folder: &Folder,
    dir: &str,
    args: &[Arg],
    dest: &Path,
) -> Result<String, DockerError> {
    let said = run_kkt(host, folder, dir, args)?;
    host.create_dir_all(dest)
        .map_err(|e| DockerError::io(format!("could not create {}", dest.display()), e))?;
    let from = format!("kkt:{dir}/.");
    let to = dest.display().to_string();
    succeed(host, &compose(folder, &["cp", &from, &to]))?;
    Ok(said)
}

/// Runs `kkt` with `args` in the kkt container, `dir` standing for
/// [`Arg::Folder`].
fn run_kkt(
    host: &dyn Runner,
    folder: &Folder,
    dir: &str,
    args: &[Arg],
) -> Result<String, DockerError> {
    let mut line = vec!["exec", "-T", "kkt", "kkt"];
    line.extend(args.iter().map(|arg| match arg {
        Arg::Text(text) => text.as_str(),
        Arg::Folder => dir,
    }));
    succeed(host, &compose(folder, &line))
}

/// The text of the file at `relative` under the config home of `kkt` in
/// the container, or nothing if there is none. It reads the file in a new
/// container from the kkt image, on the same data volume, so the install
/// need not be running; Compose makes the volume if it is missing.
///
/// The name pack ([`super::name_pack`]) is read differently, because it
/// answers a different question: what the running `kkt serve` uses. That
/// comes from serve's own command, with `--name-pack` and `--config`, so it
/// needs the running container. A program run in the container with no
/// `--config` reads only the default config file and never sees serve's
/// flags. Reading in a new container also works while the install is
/// stopped. So the two stay apart.
pub fn config_home_file(
    host: &dyn Runner,
    folder: &Folder,
    relative: &Path,
) -> Result<String, DockerError> {
    let script = config_script(relative)?;
    succeed(
        host,
        &compose(
            folder,
            &[
                "run",
                "--rm",
                "--no-deps",
                "-T",
                "--entrypoint",
                "sh",
                "kkt",
                "-c",
                &script,
            ],
        ),
    )
}

/// A shell script that prints the file at `relative` under the config home
/// `kkt` in the container uses, if there is one, and nothing if not: under
/// `$XDG_CONFIG_HOME` when it is absolute, else `$HOME/.config`. `relative`
/// goes into the script as it is, so it must be plain.
fn config_script(relative: &Path) -> Result<String, DockerError> {
    let text = relative
        .to_str()
        .filter(|text| plain(text))
        .ok_or_else(|| DockerError::NotPlain(relative.display().to_string()))?;
    Ok(format!(
        "case \"$XDG_CONFIG_HOME\" in /*) base=\"$XDG_CONFIG_HOME\" ;; *) base=\"$HOME/{XDG_CONFIG_HOME_DEFAULT}\" ;; esac; \
         file=\"$base/{text}\"; \
         if [ -e \"$file\" ]; then cat -- \"$file\"; fi"
    ))
}

/// Whether `text` is a relative path of letters, digits, `.`, `-`, `_` and
/// `/` that does not leave the folder it is under.
fn plain(text: &str) -> bool {
    !text.is_empty()
        && !text.starts_with('/')
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '/'))
        && text.split('/').all(|part| !part.is_empty() && part != "..")
}

/// Makes a new, empty folder in the kkt container whose name starts with
/// `prefix` and a dot, as the kkt user, and returns its path. `mktemp`
/// picks a name no other folder has, so this run owns it.
fn make_dir(host: &dyn Runner, folder: &Folder, prefix: &str) -> Result<String, DockerError> {
    let template = format!("{prefix}.XXXXXXXXXX");
    let said = succeed(
        host,
        &compose(folder, &["exec", "-T", "kkt", "mktemp", "-d", &template]),
    )?;
    let dir = said.trim();
    let start = format!("{prefix}.");
    let named_well = dir.len() > start.len()
        && dir.starts_with(&start)
        && dir[start.len()..]
            .chars()
            .all(|c| c.is_ascii_alphanumeric());
    if !named_well {
        return Err(DockerError::Unexpected {
            what: "the folder mktemp made in the container",
            detail: said,
        });
    }
    Ok(dir.to_string())
}

/// Removes this run's folder in the container, then returns `done`. A
/// failure to remove it after `done` worked is an error; after `done`
/// failed, it is said, and `done`'s error returned.
fn finish(
    host: &dyn Runner,
    folder: &Folder,
    dir: &str,
    done: Result<String, DockerError>,
) -> Result<String, DockerError> {
    match (done, remove(host, folder, dir)) {
        (Ok(said), Ok(())) => Ok(said),
        (Ok(_), Err(e)) => Err(e),
        (Err(e), Ok(())) => Err(e),
        (Err(e), Err(cleanup)) => {
            host.say(&format!(
                "Could not remove {dir} in the container: {cleanup}"
            ));
            Err(e)
        }
    }
}

/// Removes `dir` in the kkt container, if it is there.
fn remove(host: &dyn Runner, folder: &Folder, dir: &str) -> Result<(), DockerError> {
    succeed(
        host,
        &compose(
            folder,
            &["exec", "-T", "-u", "root", "kkt", "rm", "-rf", dir],
        ),
    )
    .map(|_| ())
}

/// Runs `command`; its output, or an error saying how it failed.
fn succeed(host: &dyn Runner, command: &Command) -> Result<String, DockerError> {
    let done = run(host, command)?;
    if !done.success() {
        return Err(DockerError::Command {
            command: command.to_string(),
            detail: done.detail(),
        });
    }
    Ok(done.stdout)
}
