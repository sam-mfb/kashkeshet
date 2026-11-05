//! What `kkt doctor`, and setup's check of the server it set up, decide by
//! how this machine is set up: which setup doctor goes by, how the server
//! this machine's clients use is checked, and the fix for each problem. The
//! program reads the record, the note of an unfinished setup and what it
//! finds installed, and runs the checks.

use super::adopt::Unrecorded;
use super::record::{Mode, Record, Unfinished};
use kkt_diagnostics::shell_word;
use std::path::Path;

/// The setup `kkt doctor` goes by: the `recorded` one; else the one an
/// `unfinished` setup installed but did not record; else, with no note of
/// one, a server `found` installed here with no record, adopted as kkt
/// `version` would record it. `found` is looked for only as
/// [`super::status_looks_for_unrecorded`] says.
pub fn doctor_record(
    recorded: Option<Record>,
    unfinished: &Result<Option<Unfinished>, String>,
    found: Option<&Unrecorded>,
    version: &str,
) -> Option<Record> {
    match (recorded, unfinished) {
        (Some(record), _) => Some(record),
        (None, Ok(Some(unfinished))) => unfinished.record.clone(),
        (None, _) => found.map(|found| found.record(version)),
    }
}

/// How the server this machine's clients use is checked, on a machine set
/// up as `mode`: as a client's when the clients were `switched` to another
/// machine's server, otherwise as `mode` says.
pub fn clients_server_mode(switched: bool, mode: &Mode) -> Mode {
    match switched {
        true => Mode::Client,
        false => mode.clone(),
    }
}

/// The fix doctor gives for a server that does not answer, on a machine set
/// up as `mode` (`None` when not known) whose clients may have been
/// `switched`: see [`clients_server_mode`] and [`server_fix`].
pub fn clients_server_fix(switched: bool, mode: Option<&Mode>) -> Option<String> {
    mode.map(|mode| clients_server_mode(switched, mode))
        .as_ref()
        .and_then(server_fix)
}

/// The fix for a server that does not answer, checked as `mode`, where
/// starting the kkt service is not how kkt runs; `None` where it is.
pub fn server_fix(mode: &Mode) -> Option<String> {
    match mode {
        Mode::Client => Some(
            "the server runs on another machine: check its URL and key (running `kkt setup` again can change them)"
                .to_string(),
        ),
        // From inside the folder: Compose reads a relative COMPOSE_FILE in
        // .env against the current folder.
        Mode::Docker(install) => Some(format!(
            "this machine runs kkt in Docker: run `kkt status`, or start it with `cd {} && docker compose up -d`",
            shell_word(&install.folder.display().to_string())
        )),
        Mode::Service { .. } => None,
    }
}

/// The fix for the operator web key on a machine set up as `mode` (`None`
/// when not known).
pub fn web_key_fix(mode: Option<&Mode>) -> String {
    match mode {
        Some(Mode::Docker(install)) => docker_web_key_fix(&install.folder),
        _ => "check the operator web key on the server; `kkt key create --web` creates it if missing, then restart the server".to_string(),
    }
}

/// The fix for the admin key on a machine set up as `mode` (`None` when not
/// known).
pub fn admin_key_fix(mode: Option<&Mode>) -> String {
    match mode {
        Some(Mode::Docker(install)) => docker_admin_key_fix(&install.folder),
        _ => "check the admin key on the server; `kkt key create --admin --force` replaces it, then restart the server".to_string(),
    }
}

/// How to save, or read, the web key of kkt in Docker in `folder`.
pub fn docker_web_key_fix(folder: &Path) -> String {
    format!(
        "`kkt setup` (run again with the same answers) or `kkt upgrade` (to a new version) saves the server's web key here; to read it now, run `cd {} && docker compose exec kkt kkt key show --web`",
        shell_word(&folder.display().to_string())
    )
}

/// How to save, or read, the admin key of kkt in Docker in `folder`.
fn docker_admin_key_fix(folder: &Path) -> String {
    format!(
        "`kkt setup --admin-key` (run again with the same answers) saves the server's admin key here; to read it now, run `cd {} && docker compose exec kkt kkt key show --admin`",
        shell_word(&folder.display().to_string())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DockerSetup, Purpose, Server, status_looks_for_unrecorded};
    use std::path::PathBuf;

    fn docker(tailscale: bool) -> Mode {
        Mode::Docker(DockerSetup {
            folder: PathBuf::from("/data/kkt docker"),
            tailscale,
            hostname: None,
            tailnet: None,
        })
    }

    fn service() -> Mode {
        Mode::Service { tailscale: false }
    }

    fn record(mode: Mode) -> Record {
        Record {
            mode,
            written_by: "0.5.0".into(),
            url: "http://127.0.0.1:8080/mcp".into(),
            key_file: None,
            clients: None,
        }
    }

    fn switched(mode: Mode) -> Record {
        let mut record = record(mode);
        record.clients = Some(Server {
            url: "https://other/mcp".into(),
            key_file: None,
        });
        record
    }

    fn unfinished(record: Option<Record>) -> Unfinished {
        Unfinished {
            purpose: Purpose::Docker,
            tailscale: false,
            allow_hosts: Vec::new(),
            clients: Vec::new(),
            tailscale_login: None,
            record,
        }
    }

    fn found(mode: Mode) -> Unrecorded {
        Unrecorded {
            mode,
            url: "http://127.0.0.1:9000/mcp".into(),
            key_file: None,
        }
    }

    const CLIENT_FIX: &str = "the server runs on another machine: check its URL and key (running `kkt setup` again can change them)";

    fn docker_fix() -> Option<String> {
        server_fix(&docker(false))
    }

    #[test]
    fn doctor_goes_by_the_record_even_of_a_switched_server() {
        let record = switched(service());
        let found = found(docker(false));
        let chosen = doctor_record(
            Some(record.clone()),
            &Ok(Some(unfinished(None))),
            Some(&found),
            "0.7.0",
        );
        assert_eq!(chosen, Some(record.clone()));
        // Its clients use another machine's server, so that is checked as a
        // client's.
        assert_eq!(
            clients_server_fix(record.clients.is_some(), Some(&record.mode)),
            Some(CLIENT_FIX.into())
        );
        assert_eq!(clients_server_mode(true, &docker(false)), Mode::Client);
        assert_eq!(clients_server_fix(false, Some(&service())), None);
    }

    #[test]
    fn doctor_goes_by_what_an_unfinished_setup_installed() {
        let installed = record(docker(false));
        let noted = Ok(Some(unfinished(Some(installed.clone()))));
        assert!(!status_looks_for_unrecorded(None, &noted));
        assert_eq!(doctor_record(None, &noted, None, "0.7.0"), Some(installed));
        // One that stopped before installing has no record to go by.
        assert_eq!(
            doctor_record(None, &Ok(Some(unfinished(None))), None, "0.7.0"),
            None
        );
        // Its mode comes from where it goes, and was never switched.
        assert_eq!(clients_server_mode(false, &docker(false)), docker(false));
        assert_eq!(
            clients_server_fix(false, Some(&docker(false))),
            docker_fix()
        );
    }

    #[test]
    fn an_unreadable_note_leaves_nothing_to_go_by() {
        let unreadable = Err("bad note".to_string());
        assert!(!status_looks_for_unrecorded(None, &unreadable));
        assert_eq!(doctor_record(None, &unreadable, None, "0.7.0"), None);
        assert_eq!(clients_server_fix(false, None), None);
    }

    #[test]
    fn doctor_goes_by_a_server_found_installed() {
        assert!(status_looks_for_unrecorded(None, &Ok(None)));
        let found = found(docker(false));
        let chosen = doctor_record(None, &Ok(None), Some(&found), "0.7.0");
        assert_eq!(chosen, Some(found.record("0.7.0")));
        assert_eq!(
            clients_server_fix(false, chosen.as_ref().map(|record| &record.mode)),
            docker_fix()
        );
    }

    #[test]
    fn with_nothing_found_doctor_has_no_setup() {
        assert_eq!(doctor_record(None, &Ok(None), None, "0.7.0"), None);
        assert_eq!(clients_server_fix(false, None), None);
    }

    #[test]
    fn the_fix_for_a_silent_server_names_how_this_machine_runs_kkt() {
        for tailscale in [false, true] {
            let fix = server_fix(&docker(tailscale)).expect("a Docker fix");
            assert!(
                fix.contains("`cd '/data/kkt docker' && docker compose up -d`"),
                "{fix}"
            );
            assert!(!fix.contains("kkt service"), "{fix}");
            assert!(!fix.contains("kkt serve"), "{fix}");
        }
        let fix = server_fix(&Mode::Client).expect("a client fix");
        assert!(fix.contains("another machine"), "{fix}");
        // A service machine keeps the service fix.
        for tailscale in [false, true] {
            assert_eq!(server_fix(&Mode::Service { tailscale }), None);
        }
    }

    #[test]
    fn each_key_fix_names_the_docker_folder_only_for_docker() {
        let docker = docker(false);
        for (fix, show) in [
            (web_key_fix(Some(&docker)), "kkt key show --web"),
            (admin_key_fix(Some(&docker)), "kkt key show --admin"),
        ] {
            assert!(fix.contains("cd '/data/kkt docker' && "), "{fix}");
            assert!(fix.contains(show), "{fix}");
        }
        for mode in [None, Some(service()), Some(Mode::Client)] {
            assert!(
                web_key_fix(mode.as_ref()).starts_with("check the operator web key on the server")
            );
            assert!(admin_key_fix(mode.as_ref()).starts_with("check the admin key on the server"));
        }
    }
}
