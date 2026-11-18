//! Running the server's own program in the service's container, and reading
//! a file under its config home. Each run makes its own folder in the
//! container with `mktemp -d`, carries files in or out of it, and removes
//! only that folder afterwards, so two runs at the same time never see or
//! remove each other's files.

use crate::services::worded;
use crate::{Arg, Carry, InServer};
use crate::{Tell, here};
use kkt_container::{Output, Project, Projects, User};
use kkt_machine::XDG_CONFIG_HOME_DEFAULT;
use std::path::{Path, PathBuf};

/// The service kkt's server runs in.
pub(crate) const SERVER: &str = "kkt";

/// Runs `kkt` in the server's container with `command`'s arguments, in a
/// new folder whose name starts with its prefix, carrying files in before
/// the run or out after it. Returns what the program printed.
pub(crate) fn run_in_server(
    projects: &dyn Projects,
    tell: &dyn Tell,
    project: &Project,
    command: &InServer<'_>,
) -> Result<String, String> {
    let dir = make_dir(projects, project, command.folder_prefix)?;
    let done = match &command.carry {
        Carry::In(files) => copy_in_and_run(projects, project, &dir, command.args, files),
        Carry::Out(dest) => run_and_copy_out(projects, project, &dir, command.args, dest),
    };
    finish(projects, tell, project, &dir, done)
}

fn copy_in_and_run(
    projects: &dyn Projects,
    project: &Project,
    dir: &str,
    args: &[Arg],
    files: &[PathBuf],
) -> Result<String, String> {
    for file in files {
        let name = file
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| unexpected("a file's name", &file.display().to_string()))?;
        projects
            .copy_in(project, SERVER, file, &format!("{dir}/{name}"))
            .map_err(|e| worded(projects, e))?;
    }
    // Copied files keep their host modes, and belong to root in the
    // container; the server's user must be able to read them.
    exec(projects, project, User::Root, &["chmod", "-R", "a+rX", dir])?;
    run_kkt(projects, project, dir, args)
}

fn run_and_copy_out(
    projects: &dyn Projects,
    project: &Project,
    dir: &str,
    args: &[Arg],
    dest: &Path,
) -> Result<String, String> {
    let said = run_kkt(projects, project, dir, args)?;
    let here_folder = here();
    projects
        .make_folders(&host(&here_folder), &[dest])
        .map_err(|e| e.to_string())?;
    projects
        .copy_out(project, SERVER, &format!("{dir}/."), dest)
        .map_err(|e| worded(projects, e))?;
    Ok(said)
}

/// Runs `kkt` with `args` in the server's container, `dir` standing for
/// [`Arg::Folder`].
fn run_kkt(
    projects: &dyn Projects,
    project: &Project,
    dir: &str,
    args: &[Arg],
) -> Result<String, String> {
    let mut line = vec!["kkt"];
    line.extend(args.iter().map(|arg| match arg {
        Arg::Text(text) => text.as_str(),
        Arg::Folder => dir,
    }));
    exec(projects, project, User::Default, &line)
}

/// The text of the file at `relative` under the config home of `kkt` in
/// the container, or nothing if there is none. It reads the file in a new
/// container of the server's service, on the same data volume, so the
/// install need not be running.
///
/// The name pack ([`crate::status`]) is read differently, because it
/// answers a different question: what the running `kkt serve` uses. That
/// comes from serve's own command, with `--name-pack` and `--config`, so it
/// needs the running container. A program run in the container with no
/// `--config` reads only the default config file and never sees serve's
/// flags. Reading in a new container also works while the install is
/// stopped. So the two stay apart.
pub(crate) fn config_home_file(
    projects: &dyn Projects,
    project: &Project,
    relative: &Path,
) -> Result<String, String> {
    let script = config_script(relative)?;
    projects
        .run_once(
            project,
            SERVER,
            User::Default,
            "sh",
            &["-c", &script],
            Output::Captured,
        )
        .and_then(|ran| ran.succeeded())
        .map_err(|e| worded(projects, e))
}

/// A shell script that prints the file at `relative` under the config home
/// `kkt` in the container uses, if there is one, and nothing if not: under
/// `$XDG_CONFIG_HOME` when it is absolute, else `$HOME/.config`. `relative`
/// goes into the script as it is, so it must be plain: a path that is not
/// UTF-8 is refused like any other that is not.
pub(crate) fn config_script(relative: &Path) -> Result<String, String> {
    let text = relative
        .to_str()
        .filter(|text| plain(text))
        .ok_or_else(|| {
            format!(
                "{} is not a plain path under the config home",
                relative.display()
            )
        })?;
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

/// Makes a new, empty folder in the server's container whose name starts
/// with `prefix` and a dot, as the server's user, and returns its path.
/// `mktemp` picks a name no other folder has, so this run owns it.
fn make_dir(projects: &dyn Projects, project: &Project, prefix: &str) -> Result<String, String> {
    let template = format!("{prefix}.XXXXXXXXXX");
    let said = exec(
        projects,
        project,
        User::Default,
        &["mktemp", "-d", &template],
    )?;
    let dir = said.trim();
    let start = format!("{prefix}.");
    let named_well = dir.len() > start.len()
        && dir.starts_with(&start)
        && dir[start.len()..]
            .chars()
            .all(|c| c.is_ascii_alphanumeric());
    if !named_well {
        return Err(unexpected("the folder mktemp made in the container", &said));
    }
    Ok(dir.to_string())
}

/// Removes this run's folder in the container, then returns `done`. A
/// failure to remove it after `done` worked is an error; after `done`
/// failed, it is said, and `done`'s error returned.
fn finish(
    projects: &dyn Projects,
    tell: &dyn Tell,
    project: &Project,
    dir: &str,
    done: Result<String, String>,
) -> Result<String, String> {
    match (done, remove(projects, project, dir)) {
        (Ok(said), Ok(())) => Ok(said),
        (Ok(_), Err(e)) => Err(e),
        (Err(e), Ok(())) => Err(e),
        (Err(e), Err(cleanup)) => {
            tell.say(&format!(
                "Could not remove {dir} in the container: {cleanup}"
            ));
            Err(e)
        }
    }
}

/// Removes `dir` in the server's container, if it is there.
fn remove(projects: &dyn Projects, project: &Project, dir: &str) -> Result<(), String> {
    exec(projects, project, User::Root, &["rm", "-rf", dir]).map(|_| ())
}

/// Runs `args` in the server's container as `user`; what it printed, or an
/// error saying how it failed.
fn exec(
    projects: &dyn Projects,
    project: &Project,
    user: User,
    args: &[&str],
) -> Result<String, String> {
    projects
        .exec(project, SERVER, user, args, Output::Captured)
        .and_then(|ran| ran.succeeded())
        .map_err(|e| worded(projects, e))
}

/// An answer that is not what was asked for, as the person is told it.
pub(crate) fn unexpected(what: &str, detail: &str) -> String {
    format!("unexpected output from {what}: {detail}")
}

/// Files on this machine, outside the managed folder, as a project with no
/// folder: a path is taken as it is, relative ones from where the person
/// ran kkt.
pub(crate) fn host(folder: &kkt_container::Folder) -> Project<'_> {
    Project {
        folder,
        shielded: &[],
    }
}
