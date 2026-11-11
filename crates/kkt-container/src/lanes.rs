//! `kkt lanes import` and `export` for kkt in Docker: the database is in the
//! container's volume, so the lane files are copied in or out with `docker
//! compose cp` and `kkt lanes` runs in the container. Each run makes its own
//! folder in the container with `mktemp -d` and removes only that folder
//! afterwards, so two runs at the same time never see or remove each other's
//! files.

use super::compose::compose;
use super::provision::run;
use super::{Command, ConfigReader, DockerError, Folder, Host};
use kkt_config_file::{CONFIG_DIR, CONFIG_FILE_NAME, XDG_CONFIG_HOME_DEFAULT};
use kkt_settings::Limits;
use std::path::{Path, PathBuf};

/// The start of the name of the folder in the kkt container that `kkt lanes
/// import` copies the lane files into, to import them there. Each run makes
/// its own with `mktemp -d`, which adds random characters, and removes only
/// that one afterwards, so runs at the same time do not share a folder.
const DOCKER_LANES_IMPORT_DIR: &str = "/tmp/kkt-lanes-import";
/// The start of the name of the folder in the kkt container that `kkt lanes
/// export` writes the lane files to, to copy them out; one per run, as for
/// [`DOCKER_LANES_IMPORT_DIR`].
const DOCKER_LANES_EXPORT_DIR: &str = "/tmp/kkt-lanes-export";

/// What to import: the lane files, and the source to record for them, which
/// is the folder on the host and its git commit, not the copy in the
/// container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanesImport {
    /// Absolute paths of the `.md` files to import.
    pub files: Vec<PathBuf>,
    pub source_folder: String,
    pub source_commit: Option<String>,
    pub retire_missing: bool,
}

/// Imports the lane files into kkt in Docker. Returns what `kkt lanes
/// import` printed in the container.
pub fn import_lanes(
    host: &dyn Host,
    folder: &Folder,
    import: &LanesImport,
) -> Result<String, DockerError> {
    let dir = make_dir(host, folder, DOCKER_LANES_IMPORT_DIR)?;
    let done = copy_in_and_import(host, folder, &dir, import);
    finish(host, folder, &dir, done)
}

fn copy_in_and_import(
    host: &dyn Host,
    folder: &Folder,
    dir: &str,
    import: &LanesImport,
) -> Result<String, DockerError> {
    for file in &import.files {
        let name =
            file.file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| DockerError::Unexpected {
                    what: "a lane file's name",
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
    let mut args = vec![
        "exec",
        "-T",
        "kkt",
        "kkt",
        "lanes",
        "import",
        dir,
        "--source-folder",
        &import.source_folder,
    ];
    if let Some(commit) = &import.source_commit {
        args.extend(["--source-commit", commit]);
    }
    if import.retire_missing {
        args.push("--retire-missing");
    }
    succeed(host, &compose(folder, &args))
}

/// Exports every current lane from kkt in Docker to `dest`, an absolute
/// folder on the host, created if missing. Returns what `kkt lanes export`
/// printed in the container.
pub fn export_lanes(host: &dyn Host, folder: &Folder, dest: &Path) -> Result<String, DockerError> {
    let dir = make_dir(host, folder, DOCKER_LANES_EXPORT_DIR)?;
    let done = export_and_copy_out(host, folder, &dir, dest);
    finish(host, folder, &dir, done)
}

fn export_and_copy_out(
    host: &dyn Host,
    folder: &Folder,
    dir: &str,
    dest: &Path,
) -> Result<String, DockerError> {
    let said = succeed(
        host,
        &compose(
            folder,
            &["exec", "-T", "kkt", "kkt", "lanes", "export", dir],
        ),
    )?;
    host.create_dir_all(dest)
        .map_err(|e| DockerError::io(format!("could not create {}", dest.display()), e))?;
    let from = format!("kkt:{dir}/.");
    let to = dest.display().to_string();
    succeed(host, &compose(folder, &["cp", &from, &to]))?;
    Ok(said)
}

/// The `max_lane_chars` that `kkt lanes import` in the kkt container uses:
/// from the config file it finds there, else the default. It reads the file
/// in a new container from the kkt image, on the same data volume, so the
/// install need not be running; Compose makes the volume if it is missing.
/// `reader` parses the file. An error names what could not be read,
/// without the file's contents.
///
/// The name pack ([`super::name_pack`]) is read differently, because it
/// answers a different question: what the running `kkt serve` uses. That
/// comes from serve's own command, with `--name-pack` and `--config`, so it
/// needs the running container. `kkt lanes import` in the container is run
/// with no `--config` ([`import_lanes`]), so it reads only the default
/// config file and never sees serve's flags; reading serve's command here
/// could give a limit the import does not use. Reading in a new container
/// also gives a limit while the install is stopped, which setup's lane
/// check needs. So the two stay apart.
pub fn lane_limit(
    host: &dyn Host,
    folder: &Folder,
    reader: &dyn ConfigReader,
) -> Result<usize, DockerError> {
    let script = config_script();
    let said = succeed(
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
    )?;
    // Only the lane limit is used, so the paths are not, and a `~/` path
    // need only be read as one: the home given is a stand-in for the
    // container's.
    let settings = reader
        .settings(&said, Path::new("/"), Some(Path::new("/")))
        .map_err(|detail| DockerError::Unexpected {
            what: "the kkt container's config file",
            // Names the setting that is wrong.
            detail,
        })?;
    Ok(settings
        .max_lane_chars
        .unwrap_or(Limits::DEFAULT.max_lane_chars))
}

/// A shell script that prints the config file `kkt` in the container reads
/// by default, if there is one, and nothing if not: under
/// `$XDG_CONFIG_HOME` when it is absolute, else `$HOME/.config`, as
/// [`kkt_settings::Env::config_file`] finds it.
fn config_script() -> String {
    format!(
        "case \"$XDG_CONFIG_HOME\" in /*) base=\"$XDG_CONFIG_HOME\" ;; *) base=\"$HOME/{XDG_CONFIG_HOME_DEFAULT}\" ;; esac; \
         file=\"$base/{CONFIG_DIR}/{CONFIG_FILE_NAME}\"; \
         if [ -e \"$file\" ]; then cat -- \"$file\"; fi"
    )
}

/// Makes a new, empty folder in the kkt container whose name starts with
/// `prefix` and a dot, as the kkt user, and returns its path. `mktemp`
/// picks a name no other folder has, so this run owns it.
fn make_dir(host: &dyn Host, folder: &Folder, prefix: &str) -> Result<String, DockerError> {
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
    host: &dyn Host,
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
fn remove(host: &dyn Host, folder: &Folder, dir: &str) -> Result<(), DockerError> {
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
fn succeed(host: &dyn Host, command: &Command) -> Result<String, DockerError> {
    let done = run(host, command)?;
    if !done.success() {
        return Err(DockerError::Command {
            command: command.to_string(),
            detail: done.detail(),
        });
    }
    Ok(done.stdout)
}
